use clap::{Parser, Subcommand};
use martino::app::{flow_job, FlowRunBody};
use martino::config::Config;
use martino::state::Core;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "martino", about = "Martino: cliente HTTP local con flujos visuales")]
struct Cli {
    /// Carpeta de trabajo (colecciones, flujos y .workbench). Por defecto la última abierta.
    #[arg(long, global = true)]
    root: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Levanta la UI en http://127.0.0.1:4400
    Serve {
        #[arg(long)]
        open: bool,
        #[arg(long)]
        port: Option<u16>,
        /// Acepta peticiones desde un túnel (*.trycloudflare.com o Tailscale *.ts.net), protegidas con una llave
        #[arg(long)]
        public: bool,
    },
    /// Ejecuta y valida un flujo sin UI (sale con 1 si falla o la comparación no coincide)
    Flow {
        id: String,
        #[arg(long)]
        env: Option<String>,
        #[arg(long)]
        compare: bool,
        /// JSON con la entrada
        #[arg(long)]
        input: Option<PathBuf>,
        /// Ejecuta todos los casos QA
        #[arg(long)]
        cases: bool,
    },
    /// Importa una colección o entorno Postman v2.1
    ImportPostman {
        file: PathBuf,
        #[arg(long)]
        name: Option<String>,
        /// Colección destino (para entornos o para mezclar)
        #[arg(long)]
        collection: Option<String>,
    },
    /// Genera los iconos de la PWA
    Icons {
        #[arg(long, default_value = "crates/web/icons")]
        out: PathBuf,
    },
}

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("✗ {msg}");
    std::process::exit(1)
}

/// `serve` sin `--root`: la carpeta actual si ya es de Martino; si no, la última abierta;
/// si no hay, `~/Martino` (el doble clic arranca en la carpeta personal y no queremos llenarla).
fn default_root() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    if cwd.join("collections").is_dir() || cwd.join("workbench.config.json").exists() {
        return None;
    }
    martino::config::recent().into_iter().next().or_else(|| {
        let d = PathBuf::from(std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?).join("Martino");
        std::fs::create_dir_all(&d).ok()?;
        Some(d)
    })
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let serve = matches!(cli.cmd, None | Some(Cmd::Serve { .. }));
    let root = cli.root.or_else(|| serve.then(default_root).flatten()).unwrap_or_else(|| ".".into());
    let root = std::fs::canonicalize(&root).unwrap_or_else(|e| die(format!("{}: {e}", root.display())));
    // Sin subcomando = doble clic en el ejecutable: abre el navegador.
    match cli.cmd.unwrap_or(Cmd::Serve { open: true, port: None, public: false }) {
        Cmd::Serve { open, port, public } => {
            let over = port.map(|p| json!({"port": p})).unwrap_or(json!({}));
            let mut cfg = Config::load(&root, over);
            if public {
                let key = std::env::var("MARTINO_PUBLIC_KEY").ok().filter(|k| k.len() >= 24).unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
                println!("🔑 Modo público: entra con https://<túnel>/?key={key} (Cloudflare o Tailscale)");
                cfg.public_key = Some(key);
            }
            martino::config::remember(&root);
            let core = Core::new(cfg);
            let (port, h) = martino::serve(core).await.unwrap_or_else(|e| die(format!("No se pudo abrir el puerto: {e}")));
            let url = format!("http://127.0.0.1:{port}");
            println!("Martino escuchando en {url}\nCarpeta de trabajo: {}", root.display());
            if open {
                let cmd = if cfg!(target_os = "macos") { "open" } else if cfg!(windows) { "explorer" } else { "xdg-open" };
                let _ = std::process::Command::new(cmd).arg(&url).spawn();
            }
            let _ = h.await;
        }
        Cmd::Flow { id, env, compare, input, cases } => {
            let core = Core::new(Config::load(&root, json!({})));
            let flow = core.flows.get(&id).unwrap_or_else(|e| die(e));
            let input: Option<Value> = input.map(|p| {
                let s = std::fs::read_to_string(&p).unwrap_or_else(|e| die(format!("{}: {e}", p.display())));
                serde_json::from_str(&s).unwrap_or_else(|e| die(format!("{}: {e}", p.display())))
            });
            let failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let f2 = failed.clone();
            let emit: Arc<dyn Fn(Value) + Send + Sync> = Arc::new(move |v: Value| match v["type"].as_str() {
                Some("node") => {
                    let s = &v["step"];
                    let mark = if s["ok"] == json!(true) { "✓" } else { "✗" };
                    let st = s["out"]["status"].as_u64().map(|x| format!(" {x}")).unwrap_or_default();
                    let err = s["error"].as_str().map(|e| format!("  {e}")).unwrap_or_default();
                    println!("  {mark} {} ({}){st} {}ms{err}", s["key"].as_str().unwrap_or(""), s["kind"].as_str().unwrap_or(""), s["ms"]);
                }
                Some("done") | Some("case") => {
                    let r = &v["result"];
                    let verdict = r["verdict"].as_str().unwrap_or("?");
                    let who = v["caseId"].as_str().map(|c| format!("[{c}] ")).unwrap_or_default();
                    println!("{who}{verdict} · status {} · {}ms", r["status"], r["ms"]);
                    if let Some(e) = r["error"].as_str() {
                        println!("  {e}");
                    }
                    for d in r["compare"]["diffs"].as_array().into_iter().flatten() {
                        println!("  ≠ {} ({}): esperado {} · real {}", d["path"].as_str().unwrap_or(""), d["kind"].as_str().unwrap_or(""), d["expected"], d["actual"]);
                    }
                    if verdict != "OK" {
                        f2.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                }
                _ => {}
            });
            let body = FlowRunBody { env, compare, input, cases: if cases { vec!["*".into()] } else { vec![] }, ..Default::default() };
            println!("▶ {} ({id})", flow.name);
            flow_job(core, flow, body, Default::default(), emit).await;
            std::process::exit(failed.load(std::sync::atomic::Ordering::SeqCst) as i32);
        }
        Cmd::ImportPostman { file, name, collection } => {
            let core = Core::new(Config::load(&root, json!({})));
            let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap_or_else(|e| die(e))).unwrap_or_else(|e| die(e));
            let mut c = martino_core::postman::convert(&v).unwrap_or_else(|e| die(e));
            if let Some(n) = name {
                c.name = n;
            }
            if c.items.is_empty() && collection.is_none() {
                die("Es un entorno: indica la colección con --collection <id>");
            }
            let id = core.store.import(&c, collection.as_deref()).unwrap_or_else(|e| die(e));
            println!("✓ Importado en collections/{id} ({} entornos)", c.environments.len());
        }
        Cmd::Icons { out } => {
            let files = martino::icons::generate(&root.join(out)).unwrap_or_else(|e| die(e));
            println!("✓ {}", files.join(", "));
        }
    }
}
