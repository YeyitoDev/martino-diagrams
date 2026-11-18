//! API REST + streaming NDJSON + canal de eventos.
use crate::config::Settings;
use crate::executor::{Overrides, RunResult};
use crate::flows::{run_flow, run_node, FlowResult, RunOpts, Seed};
use crate::config::{self, Config};
use crate::state::{Core, Hub};
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Query, Request as HttpRequest, State};
use axum::http::{header, HeaderValue, Method, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::StreamExt;
use martino_core::flow::{Flow, NodeOut};
use martino_core::model::{Environment, Request};
use martino_core::postman;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

type S = State<Arc<Core>>;
type Q = Query<HashMap<String, String>>;
type ApiResult = Result<Json<Value>, ApiErr>;

pub struct ApiErr(StatusCode, String);
impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}
impl From<String> for ApiErr {
    fn from(e: String) -> Self {
        ApiErr(StatusCode::BAD_REQUEST, e)
    }
}

fn q<'a>(q: &'a HashMap<String, String>, k: &str) -> Result<&'a str, ApiErr> {
    q.get(k).map(String::as_str).filter(|s| !s.is_empty()).ok_or_else(|| ApiErr(StatusCode::BAD_REQUEST, format!("Falta el parámetro «{k}»")))
}

fn ok(v: impl serde::Serialize) -> ApiResult {
    Ok(Json(serde_json::to_value(v).unwrap()))
}

pub fn router(hub: Hub) -> Router {
    let api = Router::new()
        .route("/api/settings", get(get_settings).put(put_settings))
        .route("/api/collections", get(list_collections).post(create_collection).delete(delete_collection))
        .route("/api/collection-root", get(get_root).put(put_root))
        .route("/api/import", post(import))
        .route("/api/request", get(get_request).put(put_request).post(create_request).delete(delete_request))
        .route("/api/request/move", post(move_request))
        .route("/api/request/next-id", get(next_id))
        .route("/api/tables", get(list_tables).put(put_table).delete(delete_table))
        .route("/api/crypto/keygen", post(keygen))
        .route("/api/folder", post(create_folder))
        .route("/api/environment", get(get_env).put(put_env).delete(delete_env))
        .route("/api/runtime", get(get_runtime).put(put_runtime).delete(clear_runtime))
        .route("/api/resolve", post(resolve))
        .route("/api/flows", get(list_flows).post(create_flow))
        .route("/api/flow", get(get_flow).put(put_flow).delete(delete_flow))
        .route("/api/workspaces", get(get_ws).put(put_ws))
        .route("/api/run", post(run))
        .route("/api/run-async", post(run_async))
        .route("/api/events", get(events))
        .route("/api/cancel", post(cancel))
        .route("/api/flow/run", post(flow_run))
        .route("/api/flow/run-async", post(flow_run_async))
        .route("/api/flow/node-run", post(node_run))
        .route("/api/root", get(get_dir).post(open_dir));
    api.fallback(ui)
        .layer(DefaultBodyLimit::max(50 * 1024 * 1024))
        .layer(middleware::from_fn_with_state(hub.clone(), guard))
        .with_state(hub)
}

/// La UI va dentro del binario (en debug se lee del disco, así `trunk watch` sigue sirviendo).
#[derive(rust_embed::Embed)]
#[folder = "../web/dist"]
struct Ui;

async fn ui(uri: Uri) -> Response {
    let Some(file) = Ui::get(uri.path().trim_start_matches('/')).or_else(|| Ui::get("index.html")) else {
        return (StatusCode::NOT_FOUND, "No hay UI compilada: corre `trunk build --release` en crates/web").into_response();
    };
    ([(header::CONTENT_TYPE, file.metadata.mimetype().to_string())], file.data).into_response()
}

async fn get_dir(State(c): S) -> ApiResult {
    ok(json!({"root": c.cfg.read().unwrap().root, "recent": config::recent()}))
}

#[derive(Deserialize)]
struct DirBody {
    path: Option<String>,
}

/// Cambia la carpeta de trabajo. Sin `path` abre el selector nativo del sistema.
async fn open_dir(State(hub): State<Hub>, Json(b): Json<DirBody>) -> ApiResult {
    let (port, public) = {
        let cur = hub.0.read().unwrap();
        let c = cur.cfg.read().unwrap();
        (c.port, c.public_key.is_some())
    };
    if public {
        return Err(ApiErr(StatusCode::FORBIDDEN, "En modo público no se puede cambiar de carpeta".into()));
    }
    let path = match b.path.filter(|p| !p.trim().is_empty()) {
        Some(p) => p.trim().to_string(),
        None => match pick_dir().await? {
            Some(p) => p,
            None => return ok(json!({"cancelled": true})),
        },
    };
    let root = std::fs::canonicalize(&path).ok().filter(|p| p.is_dir()).ok_or_else(|| format!("No existe la carpeta «{path}»"))?;
    *hub.0.write().unwrap() = Core::new(Config::load(&root, json!({"port": port})));
    config::remember(&root);
    ok(json!({"root": root}))
}

/// Diálogo nativo sin dependencias: osascript / PowerShell / zenity. `None` = cancelado.
async fn pick_dir() -> Result<Option<String>, ApiErr> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = tokio::process::Command::new("osascript");
        c.args(["-e", "POSIX path of (choose folder with prompt \"Carpeta de trabajo de Martino\")"]);
        c
    } else if cfg!(windows) {
        let mut c = tokio::process::Command::new("powershell");
        c.args(["-NoProfile", "-STA", "-Command", "Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.FolderBrowserDialog; $d.Description = 'Carpeta de trabajo de Martino'; $top = New-Object System.Windows.Forms.Form -Property @{TopMost = $true}; if ($d.ShowDialog($top) -eq 'OK') { $d.SelectedPath }"]);
        c
    } else {
        let mut c = tokio::process::Command::new("zenity");
        c.args(["--file-selection", "--directory", "--title=Carpeta de trabajo de Martino"]);
        c
    };
    let out = cmd.output().await.map_err(|e| format!("No se pudo abrir el selector de carpetas: {e}. Escribe la ruta a mano."))?;
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok((out.status.success() && !p.is_empty()).then_some(p))
}

fn local_host(h: &str, port: u16) -> bool {
    ["127.0.0.1", "localhost", "[::1]"].iter().any(|n| h == format!("{n}:{port}"))
}

/// Solo Host/Origin locales (DNS rebinding) y `X-Workbench: 1` en /api (fuerza un preflight
/// CORS que nunca se aprueba, así que otra web no puede llamar a la API).
async fn guard(State(core): S, req: HttpRequest, next: Next) -> Response {
    let (port, key) = {
        let c = core.cfg.read().unwrap();
        (c.port, c.public_key.clone())
    };
    let set_cookie = match denied(&req, port, key.as_deref(), EXPOSED.get().copied().unwrap_or(false)) {
        Err(msg) => return (StatusCode::FORBIDDEN, msg).into_response(),
        Ok(s) => s,
    };
    let api = req.uri().path().starts_with("/api/");
    let mut res = next.run(req).await;
    if api || res.headers().get(header::CONTENT_TYPE).is_some_and(|c| c.as_bytes().starts_with(b"text/html")) {
        res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    if let (true, Some(k)) = (set_cookie, key) {
        let c = format!("martino_key={k}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=43200");
        res.headers_mut().insert(header::SET_COOKIE, HeaderValue::from_str(&c).unwrap());
    }
    res
}

/// Túneles aceptados en modo `--public`: Cloudflare y Tailscale (`tailscale serve`).
const TUNNEL_SUFFIXES: [&str; 2] = [".trycloudflare.com", ".ts.net"];

static EXPOSED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// El servidor escucha fuera de la máquina (MARTINO_BIND): ningún Host cuenta como local.
pub fn set_exposed(v: bool) {
    let _ = EXPOSED.set(v);
}

/// Host público exacto aceptado además de los túneles (`MARTINO_PUBLIC_HOST=martino.fly.dev`).
fn public_host_ok(name: &str) -> bool {
    TUNNEL_SUFFIXES.iter().any(|s| name.ends_with(s)) || std::env::var("MARTINO_PUBLIC_HOST").is_ok_and(|h| !h.is_empty() && h.eq_ignore_ascii_case(name))
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Local: Host/Origin locales. Público (`--public`): solo el host del túnel y con la llave
/// (en `?key=` la primera vez; luego en una cookie HttpOnly). `Ok(true)` = hay que fijar la cookie.
fn denied(req: &HttpRequest, port: u16, key: Option<&str>, exposed: bool) -> Result<bool, &'static str> {
    let hv = |n: header::HeaderName| req.headers().get(n).and_then(|v| v.to_str().ok());
    let host = hv(header::HOST).unwrap_or("");
    let mut set_cookie = false;
    if !exposed && local_host(host, port) {
        if hv(header::ORIGIN).is_some_and(|o| !o.strip_prefix("http://").is_some_and(|h| local_host(h, port))) {
            return Err("Origin no permitido");
        }
    } else {
        let name = host.rsplit_once(':').map_or(host, |(h, _)| h);
        let Some(key) = key.filter(|_| public_host_ok(name)) else { return Err("Host no permitido") };
        let cookie_ok = req.headers().get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|c| c.split(';')).any(|c| c.trim().strip_prefix("martino_key=").is_some_and(|v| same(v, key)));
        let query_ok = req.uri().query().unwrap_or("").split('&').any(|kv| kv.strip_prefix("key=").is_some_and(|v| same(v, key)));
        if !cookie_ok && !query_ok {
            return Err("Acceso restringido: abre el enlace con la llave (?key=…)");
        }
        set_cookie = !cookie_ok;
        if hv(header::ORIGIN).is_some_and(|o| o != format!("https://{host}")) {
            return Err("Origin no permitido");
        }
    }
    let api = req.uri().path().starts_with("/api/");
    if api && (req.method() == Method::OPTIONS || req.headers().get("x-workbench").map(|v| v.as_bytes()) != Some(b"1")) {
        return Err("Falta X-Workbench: 1");
    }
    Ok(set_cookie)
}

/// Respuesta NDJSON: lo que `f` mande por el canal sale como una línea.
fn ndjson<F, Fut>(f: F) -> Response
where
    F: FnOnce(mpsc::UnboundedSender<String>) -> Fut,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let (tx, rx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(f(tx));
    let stream = futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|l| (Ok::<_, std::convert::Infallible>(l + "\n"), rx)) });
    Response::builder()
        .header(header::CONTENT_TYPE, "application/x-ndjson; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(stream))
        .unwrap()
}

// ---------- ajustes y colecciones ----------

async fn get_settings(State(c): S) -> ApiResult {
    ok(&c.cfg.read().unwrap().settings)
}

async fn put_settings(State(c): S, Json(s): Json<Settings>) -> ApiResult {
    c.exec.reconfigure(&s)?;
    c.cfg.write().unwrap().save_settings(s.clone()).map_err(|e| e.to_string())?;
    ok(s)
}

async fn list_collections(State(c): S) -> ApiResult {
    ok(c.store.list())
}

#[derive(Deserialize)]
struct NameBody {
    name: String,
}

async fn create_collection(State(c): S, Json(b): Json<NameBody>) -> ApiResult {
    let id = c.store.create_collection(&b.name)?;
    c.emit(json!({"type": "collections"}));
    ok(json!({"id": id}))
}

async fn delete_collection(State(c): S, Query(p): Q) -> ApiResult {
    let dest = c.store.delete_collection(q(&p, "collection")?)?;
    c.emit(json!({"type": "collections"}));
    ok(json!({"trash": dest}))
}

async fn get_root(State(c): S, Query(p): Q) -> ApiResult {
    ok(c.store.root(q(&p, "collection")?)?)
}

async fn put_root(State(c): S, Query(p): Q, Json(r): Json<Request>) -> ApiResult {
    c.store.put_root(q(&p, "collection")?, &r)?;
    ok(json!({"ok": true}))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ImportBody {
    /// JSON de Postman (colección o entorno) o el texto de una petición (.yml o .bru)
    data: Value,
    name: Option<String>,
    collection: Option<String>,
    folder: String,
    remap: Option<postman::Remap>,
}

async fn import(State(c): S, Json(b): Json<ImportBody>) -> ApiResult {
    if let Value::String(text) = &b.data {
        let col = b.collection.as_deref().ok_or_else(|| "Para importar una petición elige una colección".to_string())?;
        let mut r = martino_core::yaml_req::parse_request(text).ok().filter(|r| !r.method.is_empty()).unwrap_or_else(|| martino_core::bru::parse_request(text));
        if r.method.is_empty() {
            return Err("El texto no es una petición .yml ni .bru".to_string().into());
        }
        if let Some(n) = &b.name {
            r.name = n.clone();
        }
        let path = c.store.create_request(col, &b.folder, &r)?;
        c.emit(json!({"type": "collections"}));
        return ok(json!({"collection": col, "path": path}));
    }
    let mut conv = postman::convert(&b.data)?;
    if let Some(n) = b.name {
        conv.name = n;
    }
    if let Some(r) = &b.remap {
        postman::remap_variables(&mut conv, r);
    }
    let into = b.collection.as_deref().or(if conv.items.is_empty() { Some("") } else { None });
    if into == Some("") {
        return Err("Un entorno se importa dentro de una colección: elige una".to_string().into());
    }
    let col = c.store.import(&conv, into)?;
    c.emit(json!({"type": "collections"}));
    ok(json!({"collection": col, "requests": count(&conv.items), "environments": conv.environments.len()}))
}

fn count(items: &[postman::Item]) -> usize {
    items.iter().map(|i| if i.folder { count(&i.items) } else { 1 }).sum()
}

// ---------- peticiones ----------

async fn get_request(State(c): S, Query(p): Q) -> ApiResult {
    ok(c.store.read_request(q(&p, "collection")?, q(&p, "path")?)?)
}

async fn put_request(State(c): S, Query(p): Q, Json(r): Json<Request>) -> ApiResult {
    c.store.write_request(q(&p, "collection")?, q(&p, "path")?, &r)?;
    c.emit(json!({"type": "collections"}));
    ok(json!({"ok": true}))
}

async fn create_request(State(c): S, Query(p): Q, Json(r): Json<Request>) -> ApiResult {
    let col = q(&p, "collection")?;
    let path = c.store.create_request(col, p.get("folder").map(String::as_str).unwrap_or(""), &r)?;
    c.emit(json!({"type": "collections"}));
    ok(json!({"id": c.store.read_request(col, &path)?.id, "path": path}))
}

/// El ID que tendría una petición nueva en `folder` (respeta `wanted` si está libre).
async fn next_id(State(c): S, Query(p): Q) -> ApiResult {
    let opt = |k: &str| p.get(k).map(String::as_str).unwrap_or("");
    ok(json!({"id": c.store.next_id(q(&p, "collection")?, opt("folder"), opt("wanted"))?}))
}

// ---------- tablas de equivalencia ----------

async fn list_tables(State(c): S, Query(p): Q) -> ApiResult {
    ok(c.store.tables(q(&p, "collection")?))
}

#[derive(Deserialize)]
struct TableBody {
    csv: String,
}

async fn put_table(State(c): S, Query(p): Q, Json(b): Json<TableBody>) -> ApiResult {
    c.store.put_table(q(&p, "collection")?, q(&p, "name")?, &b.csv)?;
    ok(json!({"ok": true}))
}

async fn delete_table(State(c): S, Query(p): Q) -> ApiResult {
    c.store.delete_table(q(&p, "collection")?, q(&p, "name")?)?;
    ok(json!({"ok": true}))
}

#[derive(Deserialize)]
struct KeygenBody {
    kind: String,
}

/// Par RSA 2048 o llave AES de prueba (no se guarda en ningún lado).
async fn keygen(Json(b): Json<KeygenBody>) -> ApiResult {
    let v = tokio::task::spawn_blocking(move || crate::crypto::keygen(&b.kind, 2048)).await.map_err(|e| e.to_string())??;
    ok(v)
}

async fn delete_request(State(c): S, Query(p): Q) -> ApiResult {
    c.store.delete_request(q(&p, "collection")?, q(&p, "path")?)?;
    c.emit(json!({"type": "collections"}));
    ok(json!({"ok": true}))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoveBody {
    collection: String,
    from: String,
    #[serde(default)]
    to_folder: String,
    before: Option<String>,
}

async fn move_request(State(c): S, Json(b): Json<MoveBody>) -> ApiResult {
    let path = c.store.move_request(&b.collection, &b.from, &b.to_folder, b.before.as_deref())?;
    let flows = if path != b.from { c.flows.rename_ref(&b.collection, &b.from, &path) } else { 0 };
    c.emit(json!({"type": "collections"}));
    ok(json!({"path": path, "flowsUpdated": flows}))
}

#[derive(Deserialize)]
struct FolderBody {
    collection: String,
    #[serde(default)]
    parent: String,
    name: String,
}

async fn create_folder(State(c): S, Json(b): Json<FolderBody>) -> ApiResult {
    let path = c.store.create_folder(&b.collection, &b.parent, &b.name)?;
    c.emit(json!({"type": "collections"}));
    ok(json!({"path": path}))
}

// ---------- entornos y variables ----------

async fn get_env(State(c): S, Query(p): Q) -> ApiResult {
    ok(c.store.environment(q(&p, "collection")?, q(&p, "env")?)?)
}

async fn put_env(State(c): S, Query(p): Q, Json(e): Json<Environment>) -> ApiResult {
    c.store.put_environment(q(&p, "collection")?, q(&p, "env")?, &e)?;
    c.emit(json!({"type": "collections"}));
    ok(json!({"ok": true}))
}

async fn delete_env(State(c): S, Query(p): Q) -> ApiResult {
    c.store.delete_environment(q(&p, "collection")?, q(&p, "env")?)?;
    c.emit(json!({"type": "collections"}));
    ok(json!({"ok": true}))
}

async fn get_runtime(State(c): S) -> ApiResult {
    ok(c.runtime.read().unwrap().iter().map(|(k, v)| (k.clone(), json!(v))).collect::<serde_json::Map<_, _>>())
}

/// `{ nombre: valor }`; `{ nombre: null }` borra.
async fn put_runtime(State(c): S, Json(b): Json<serde_json::Map<String, Value>>) -> ApiResult {
    for (k, v) in b {
        c.set_runtime(&k, (!v.is_null()).then(|| martino_core::interp::value_to_text(&v)));
    }
    c.emit(json!({"type": "runtime"}));
    get_runtime(State(c)).await
}

async fn clear_runtime(State(c): S) -> ApiResult {
    c.runtime.write().unwrap().clear();
    c.emit(json!({"type": "runtime"}));
    ok(json!({}))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ResolveBody {
    collection: String,
    env: Option<String>,
    request: Option<Request>,
}

/// Tabla de variables (para colorear y el tooltip) y, si llega una petición, cómo quedaría.
async fn resolve(State(c): S, Json(b): Json<ResolveBody>) -> ApiResult {
    let scope = c.scope(&b.collection, b.env.as_deref(), b.request.as_ref());
    let mut out = json!({"vars": scope.table(), "environments": c.store.env_names(&b.collection)});
    if let Some(r) = &b.request {
        let used = crate::executor::used_vars(r);
        let missing: Vec<_> = used.iter().filter(|n| scope.resolve(n).is_none()).collect();
        let it = |s: &str| martino_core::interp::interpolate(s, &|n| scope.resolve(n));
        out["resolved"] = json!({"url": it(&r.url), "body": r.body_text().map(|b| it(&b))});
        out["used"] = json!(used);
        out["missing"] = json!(missing);
    }
    Ok(Json(out))
}

// ---------- flujos ----------

async fn list_flows(State(c): S) -> ApiResult {
    ok(c.flows.list())
}

async fn create_flow(State(c): S, Json(f): Json<Flow>) -> ApiResult {
    let id = c.flows.create(&f)?;
    ok(json!({"id": id, "flow": f}))
}

async fn get_flow(State(c): S, Query(p): Q) -> ApiResult {
    ok(c.flows.get(q(&p, "id")?)?)
}

async fn put_flow(State(c): S, Query(p): Q, Json(f): Json<Flow>) -> ApiResult {
    c.flows.put(q(&p, "id")?, &f)?;
    ok(json!({"ok": true}))
}

async fn delete_flow(State(c): S, Query(p): Q) -> ApiResult {
    c.flows.delete(q(&p, "id")?)?;
    ok(json!({"ok": true}))
}

async fn get_ws(State(c): S) -> ApiResult {
    ok(c.store.workspaces())
}

async fn put_ws(State(c): S, Json(v): Json<Value>) -> ApiResult {
    c.store.put_workspaces(&v)?;
    ok(json!({"ok": true}))
}

// ---------- ejecución ----------

#[derive(Deserialize, Clone, Default)]
#[serde(default)]
pub struct RunItem {
    /// id del panel (vuelve en el resultado)
    pub id: String,
    pub collection: String,
    pub path: Option<String>,
    pub request: Option<Request>,
    pub env: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct RunBody {
    pub items: Vec<RunItem>,
    pub concurrency: Option<usize>,
}

pub async fn run_item(c: &Core, it: &RunItem, cancel: &CancellationToken) -> RunResult {
    let req = match (&it.request, &it.path) {
        (Some(r), _) => Ok(r.clone()),
        (None, Some(p)) => c.store.read_request(&it.collection, p),
        _ => Err("Falta la petición".to_string()),
    };
    let req = match req {
        Ok(r) => r,
        Err(e) => return RunResult { error: Some(e), ..Default::default() },
    };
    let scope = c.scope(&it.collection, it.env.as_deref(), Some(&req));
    let root = c.store.root(&it.collection).ok();
    let r = c.exec.execute(&req, root.as_ref(), &|n| scope.resolve(n), &Overrides::default(), cancel).await;
    if !r.vars.is_empty() {
        for v in &r.vars {
            c.set_runtime(&v.name, Some(v.value.clone()));
        }
        c.emit(json!({"type": "runtime"}));
    }
    r
}

/// Ejecuta en paralelo (límite de concurrencia) y llama a `on` por cada resultado.
async fn run_many(c: Arc<Core>, b: RunBody, cancel: CancellationToken, on: impl Fn(&str, RunResult)) {
    let n = b.concurrency.unwrap_or_else(|| c.cfg.read().unwrap().settings.concurrency).clamp(1, 50);
    let mut s = futures::stream::iter(b.items.into_iter().map(|it| {
        let c = c.clone();
        let cancel = cancel.clone();
        async move { (it.id.clone(), run_item(&c, &it, &cancel).await) }
    }))
    .buffer_unordered(n);
    while let Some((id, r)) = s.next().await {
        on(&id, r);
    }
}

async fn run(State(c): S, Json(b): Json<RunBody>) -> Response {
    let (run_id, cancel) = c.start_run();
    ndjson(move |tx| async move {
        let _ = tx.send(json!({"type": "start", "runId": run_id}).to_string());
        run_many(c.clone(), b, cancel, |id, r| {
            let _ = tx.send(json!({"type": "result", "id": id, "result": r}).to_string());
        })
        .await;
        let _ = tx.send(json!({"type": "done"}).to_string());
        c.end_run(&run_id);
    })
}

async fn run_async(State(c): S, Json(b): Json<RunBody>) -> ApiResult {
    let (run_id, cancel) = c.start_run();
    let rid = run_id.clone();
    tokio::spawn(async move {
        let c2 = c.clone();
        run_many(c.clone(), b, cancel, |id, r| c2.emit(json!({"type": "result", "runId": rid, "id": id, "result": r}))).await;
        c.emit(json!({"type": "run-done", "runId": rid}));
        c.end_run(&rid);
    });
    ok(json!({"runId": run_id}))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CancelBody {
    run_id: String,
}

async fn cancel(State(c): S, Json(b): Json<CancelBody>) -> ApiResult {
    let t = c.runs.lock().unwrap().get(&b.run_id).cloned();
    if let Some(t) = &t {
        t.cancel();
    }
    ok(json!({"cancelled": t.is_some()}))
}

/// Canal único de la UI (NDJSON; un `ping` cada 15 s mantiene viva la conexión).
async fn events(State(c): S) -> Response {
    ndjson(move |tx| async move {
        let mut rx = c.events.subscribe();
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(15));
        loop {
            let line = tokio::select! {
                m = rx.recv() => match m {
                    Ok(s) => s,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                },
                _ = tick.tick() => json!({"type": "ping"}).to_string(),
            };
            if tx.send(line).is_err() {
                break;
            }
        }
    })
}

#[derive(Deserialize, Default, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct FlowRunBody {
    pub id: Option<String>,
    pub flow: Option<Flow>,
    pub env: Option<String>,
    pub compare: bool,
    pub input: Option<Value>,
    /// ids de casos QA a ejecutar (vacío = ejecución normal)
    pub cases: Vec<String>,
    pub concurrency: Option<usize>,
}

fn load_flow(c: &Core, b: &FlowRunBody) -> Result<Flow, String> {
    match (&b.flow, &b.id) {
        (Some(f), _) => Ok(f.clone()),
        (None, Some(id)) => c.flows.get(id),
        _ => Err("Falta el flujo".into()),
    }
}

/// Ejecuta el flujo (o sus casos QA) mandando cada evento a `emit`.
pub async fn flow_job(c: Arc<Core>, flow: Flow, b: FlowRunBody, cancel: CancellationToken, emit: Arc<dyn Fn(Value) + Send + Sync>) {
    let opts = RunOpts { env: b.env.clone(), input: b.input.clone(), compare: b.compare, case: None };
    if b.cases.is_empty() {
        let e = emit.clone();
        run_flow(&c, &flow, &opts, &cancel, &move |v| e(v)).await;
        return;
    }
    let n = b.concurrency.unwrap_or(4).clamp(1, 50);
    let cases: Vec<_> = flow.cases.iter().filter(|k| b.cases.contains(&k.id) || b.cases.iter().any(|x| x == "*")).cloned().collect();
    let total = cases.len();
    let mut s = futures::stream::iter(cases.into_iter().map(|case| {
        let (c, flow, cancel) = (c.clone(), flow.clone(), cancel.clone());
        let opts = RunOpts { input: Some(case.input.clone()), case: Some(case.clone()), ..opts.clone() };
        async move {
            let r: FlowResult = run_flow(&c, &flow, &opts, &cancel, &|_| {}).await;
            (case.id, r)
        }
    }))
    .buffer_unordered(n);
    let mut done = 0;
    while let Some((id, mut r)) = s.next().await {
        done += 1;
        r.outs.clear();
        emit(json!({"type": "case", "caseId": id, "result": r, "done": done, "total": total}));
    }
    emit(json!({"type": "cases-done"}));
}

async fn flow_run(State(c): S, Json(b): Json<FlowRunBody>) -> Response {
    let flow = match load_flow(&c, &b) {
        Ok(f) => f,
        Err(e) => return ApiErr(StatusCode::BAD_REQUEST, e).into_response(),
    };
    let (run_id, cancel) = c.start_run();
    ndjson(move |tx| async move {
        let _ = tx.send(json!({"type": "start", "runId": run_id}).to_string());
        let emit: Arc<dyn Fn(Value) + Send + Sync> = Arc::new(move |v: Value| {
            let _ = tx.send(v.to_string());
        });
        flow_job(c.clone(), flow, b, cancel, emit).await;
        c.end_run(&run_id);
    })
}

async fn flow_run_async(State(c): S, Json(b): Json<FlowRunBody>) -> ApiResult {
    let flow = load_flow(&c, &b)?;
    let (run_id, cancel) = c.start_run();
    let rid = run_id.clone();
    tokio::spawn(async move {
        let c2 = c.clone();
        let r2 = rid.clone();
        let emit: Arc<dyn Fn(Value) + Send + Sync> = Arc::new(move |mut v: Value| {
            v["runId"] = json!(r2);
            c2.emit(v);
        });
        flow_job(c.clone(), flow, b, cancel, emit).await;
        c.end_run(&rid);
    });
    ok(json!({"runId": run_id}))
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct NodeRunBody {
    id: Option<String>,
    flow: Option<Flow>,
    node_id: String,
    env: Option<String>,
    seed: Seed,
}

async fn node_run(State(c): S, Json(b): Json<NodeRunBody>) -> ApiResult {
    let flow = load_flow(&c, &FlowRunBody { id: b.id, flow: b.flow, ..Default::default() })?;
    let step = run_node(&c, &flow, &b.node_id, b.seed, b.env.as_deref()).await?;
    let vars: serde_json::Map<String, Value> = step.set_vars.clone();
    ok(json!({"step": step, "vars": vars, "out": NodeOut { ..step.out.clone() }}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(host: &str, path: &str) -> HttpRequest {
        axum::http::Request::builder().uri(path).header(header::HOST, host).header("x-workbench", "1").body(axum::body::Body::empty()).unwrap()
    }

    #[test]
    fn expuesto_pide_llave_aunque_digan_ser_local() {
        let key = Some("llave-de-prueba-de-24-caracteres");
        assert!(denied(&req("127.0.0.1:4400", "/api/flows"), 4400, key, false).is_ok(), "en la Mac, lo local entra");
        assert!(denied(&req("127.0.0.1:4400", "/api/flows"), 4400, key, true).is_err(), "expuesto: Host local sin llave no entra");
        assert!(denied(&req("evil.example.com", "/api/flows?key=llave-de-prueba-de-24-caracteres"), 4400, key, true).is_err(), "host ajeno no entra ni con llave");
        assert!(denied(&req("x.ts.net", "/api/flows"), 4400, key, true).is_err(), "túnel sin llave no entra");
        assert_eq!(denied(&req("x.ts.net", "/api/flows?key=llave-de-prueba-de-24-caracteres"), 4400, key, true), Ok(true), "túnel con llave entra y fija la cookie");
    }
}
