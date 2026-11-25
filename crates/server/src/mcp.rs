//! Servidor MCP (JSON-RPC por stdio, una línea por mensaje): las mismas operaciones que la CLI,
//! para que un agente explore colecciones, escriba flujos en YAML y los ejecute.
use martino::app::{flow_job, run_item, FlowRunBody, RunItem};
use martino::state::Core;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};

const GUIDE: &str = include_str!("../../../.claude/skills/martino-flujo/SKILL.md");

fn tools() -> Value {
    let s = |props: Value, req: &[&str]| json!({"type": "object", "properties": props, "required": req});
    json!([
        {"name": "martino_listar", "description": "Colecciones (peticiones con método y ruta, entornos, tablas) y flujos guardados.",
         "inputSchema": s(json!({}), &[])},
        {"name": "martino_peticion", "description": "Una petición guardada en YAML: método, URL, headers, body, variables {{x}} y su documentación (docs).",
         "inputSchema": s(json!({"coleccion": {"type": "string"}, "ruta": {"type": "string", "description": "p. ej. Cuentas/Consultar-cuenta.yml"}}), &["coleccion", "ruta"])},
        {"name": "martino_flujo", "description": "Un flujo en YAML: pasos, para qué sirve cada uno (desc), conexiones y casos QA. Sirve de plantilla.",
         "inputSchema": s(json!({"id": {"type": "string"}}), &["id"])},
        {"name": "martino_importar_flujo", "description": "Crea o reemplaza un flujo desde YAML (formato en martino_guia). Valida que se pueda ejecutar y lo ordena en el lienzo.",
         "inputSchema": s(json!({"yaml": {"type": "string"}, "id": {"type": "string", "description": "<coleccion>--<nombre>; si existe, se reemplaza"}}), &["yaml"])},
        {"name": "martino_ejecutar_flujo", "description": "Ejecuta un flujo (o todos sus casos QA) y devuelve cada paso con status, tiempo y error; con detalle, también lo que respondió cada paso.",
         "inputSchema": s(json!({"id": {"type": "string"}, "entorno": {"type": "string"}, "entrada": {"type": "object"},
                                  "casos": {"type": "boolean"}, "detalle": {"type": "boolean"}}), &["id"])},
        {"name": "martino_enviar", "description": "Envía una petición HTTP (cURL o YAML de petición) y la guarda en una colección, que se crea si no existe. Si ya hay una con el mismo nombre en esa carpeta, la actualiza. Usa {{variables}} del entorno para tokens y claves: lo que mandes queda escrito en la colección.",
         "inputSchema": s(json!({"coleccion": {"type": "string", "description": "id o nombre; si no existe, se crea"}, "carpeta": {"type": "string"},
                                  "nombre": {"type": "string"}, "curl": {"type": "string"}, "peticion": {"type": "string", "description": "YAML como el de martino_peticion"},
                                  "entorno": {"type": "string"}}), &["coleccion"])},
        {"name": "martino_casos_endpoint", "description": "Crea una carpeta «<endpoint> · casos» con un caso de prueba por petición y los ejecuta. Mandas sólo un YAML:\ncoleccion: banco\nendpoint: Cuentas/Retener-fondos.yml   # petición guardada (o curl: \"curl …\")\nentorno: local                      # opcional\ncarpeta: Cuentas/Retener fondos · casos   # opcional\ncasos:\n  - nombre: Monto válido\n    datos: {nroCuenta: \"001\", monto: 250}   # valores de sus {{variables}}\n    espera: {status: 201, body.retencionId: isDefined}   # campo: valor | operador valor (eq, neq, gt, contains, isDefined…)\nVolver a mandarlo actualiza los casos con el mismo nombre.",
         "inputSchema": s(json!({"yaml": {"type": "string"}}), &["yaml"])},
        {"name": "martino_guia", "description": "Guía completa del YAML de flujos: tipos de paso, operadores, casos QA y errores típicos. Léela antes de escribir un flujo.",
         "inputSchema": s(json!({}), &[])}
    ])
}

async fn call(core: &Arc<Core>, name: &str, a: &Value) -> Result<String, String> {
    let arg = |k: &str| a[k].as_str().map(String::from).ok_or(format!("falta «{k}»"));
    match name {
        "martino_listar" => {
            let cols: Vec<Value> = core.store.list().into_iter().map(|c| {
                let tables = core.store.tables(&c.id);
                json!({"id": c.id, "nombre": c.name, "entornos": c.environments, "peticiones": c.items, "tablas": tables})
            }).collect();
            Ok(serde_json::to_string_pretty(&json!({"colecciones": cols, "flujos": core.flows.list()})).unwrap())
        }
        "martino_peticion" => Ok(martino_core::yaml_req::serialize_request(&core.store.read_request(&arg("coleccion")?, &arg("ruta")?)?)),
        "martino_flujo" => Ok(martino_core::flow_yaml::flow_to_yaml(&core.flows.get(&arg("id")?)?)),
        "martino_importar_flujo" => {
            let id = crate::import_flow(core, &arg("yaml")?, a["id"].as_str().map(String::from))?;
            Ok(format!("✓ flows/{id}.flow.json guardado. Ejecútalo con martino_ejecutar_flujo {{id: \"{id}\", casos: true}}; en la UI aparece en Flujos al refrescar."))
        }
        "martino_ejecutar_flujo" => {
            let id = arg("id")?;
            let flow = core.flows.get(&id)?;
            let detail = a["detalle"] == json!(true);
            let lines = Arc::new(Mutex::new(vec![format!("▶ {} ({id})", flow.name)]));
            let l2 = lines.clone();
            let emit: Arc<dyn Fn(Value) + Send + Sync> = Arc::new(move |v: Value| {
                let Some((line, _)) = crate::run_line(&v) else { return };
                let mut l = l2.lock().unwrap();
                l.push(line);
                if detail && v["type"] == "node" {
                    let out = v["step"]["out"].to_string();
                    // shortcut: corta respuestas largas a 1500 caracteres para no llenar el contexto del agente.
                    l.push(format!("      {}", out.chars().take(1500).collect::<String>()));
                }
            });
            let input = Some(a["entrada"].clone()).filter(|v| v.is_object());
            let cases = if a["casos"] == json!(true) { vec!["*".into()] } else { vec![] };
            let body = FlowRunBody { env: a["entorno"].as_str().map(String::from), input, cases, ..Default::default() };
            flow_job(core.clone(), flow, body, Default::default(), emit).await;
            let text = lines.lock().unwrap().join("\n");
            Ok(text)
        }
        "martino_enviar" => {
            let mut r = match (a["curl"].as_str(), a["peticion"].as_str()) {
                (Some(c), _) => martino_core::curl::parse_curl(c)?,
                (None, Some(y)) => martino_core::yaml_req::parse_request(y)?,
                _ => return Err("falta «curl» o «peticion»".into()),
            };
            // Sin nombre: «MÉTODO /ruta» (el del cURL sería sólo el último tramo, p. ej. «1»).
            if let Some(n) = a["nombre"].as_str().filter(|n| !n.trim().is_empty()) {
                r.name = n.trim().into();
            } else {
                let path = r.url.split("://").last().unwrap_or(&r.url).splitn(2, '/').nth(1).unwrap_or("").split('?').next().unwrap_or("");
                r.name = format!("{} /{path}", r.method.to_uppercase());
            }
            let col = collection_id(core, &arg("coleccion")?)?;
            let folder = a["carpeta"].as_str().unwrap_or("").trim_matches('/').to_string();
            let it = RunItem { collection: col.clone(), request: Some(r.clone()), env: a["entorno"].as_str().map(String::from), ..Default::default() };
            let res = run_item(core, &it, &Default::default()).await;
            let path = save(core, &col, &folder, r)?;
            let mut out = vec![format!("✓ guardada en {col}/{path}")];
            match &res.error {
                Some(e) => out.push(format!("✗ {e}")),
                None => out.push(format!("{} {} · {} ms · {} bytes", res.status.unwrap_or(0), res.status_text, res.time_ms, res.size)),
            }
            if !res.missing.is_empty() {
                out.push(format!("variables sin valor: {}", res.missing.join(", ")));
            }
            // shortcut: corta el body a 1500 caracteres para no llenar el contexto del agente.
            out.extend((!res.body.is_empty()).then(|| res.body.chars().take(1500).collect::<String>()));
            Ok(out.join("\n"))
        }
        "martino_casos_endpoint" => endpoint_cases(core, &arg("yaml")?).await,
        "martino_guia" => Ok(GUIDE.into()),
        _ => Err(format!("herramienta desconocida: {name}")),
    }
}

/// Id de la colección por id o nombre; si no existe, la crea.
fn collection_id(core: &Core, name: &str) -> Result<String, String> {
    match core.store.list().into_iter().find(|c| c.id == name || c.name == name) {
        Some(c) => Ok(c.id),
        None => core.store.create_collection(name),
    }
}

/// Guarda la petición en la carpeta; si ya hay una con el mismo nombre, la actualiza (conserva código y orden).
fn save(core: &Core, col: &str, folder: &str, mut r: martino_core::model::Request) -> Result<String, String> {
    let same = format!("{}{}{}", if folder.is_empty() { String::new() } else { format!("{folder}/") }, martino::store::slug(&r.name), martino_core::yaml_req::EXT);
    match core.store.read_request(col, &same) {
        Ok(old) => {
            (r.id, r.seq) = (old.id, old.seq);
            core.store.write_request(col, &same, &r).map(|_| same)
        }
        Err(_) => core.store.create_request(col, folder, &r),
    }
}

/// Un caso = copia del endpoint con sus `datos` como vars de la petición y `espera` como asserts.
async fn endpoint_cases(core: &Arc<Core>, src: &str) -> Result<String, String> {
    let y: Value = serde_yaml::from_str(src).map_err(|e| format!("YAML inválido: {e}"))?;
    let text = |v: &Value| v.as_str().map(String::from).unwrap_or_else(|| v.to_string());
    let col = collection_id(core, y["coleccion"].as_str().ok_or("falta «coleccion»")?)?;
    let (base, parent) = match (y["endpoint"].as_str(), y["curl"].as_str()) {
        (Some(p), _) => (core.store.read_request(&col, p)?, p.rsplit_once('/').map(|x| x.0).unwrap_or("").to_string()),
        (None, Some(c)) => (martino_core::curl::parse_curl(c)?, String::new()),
        _ => return Err("falta «endpoint» (ruta de una petición guardada) o «curl»".into()),
    };
    let name = if base.name.is_empty() { "Endpoint".to_string() } else { base.name.clone() };
    let folder = match y["carpeta"].as_str() {
        Some(f) => f.trim_matches('/').to_string(),
        None => [parent.as_str(), &format!("{name} · casos")].iter().filter(|x| !x.is_empty()).copied().collect::<Vec<_>>().join("/"),
    };
    let cases = y["casos"].as_array().filter(|c| !c.is_empty()).ok_or("falta «casos» (al menos uno)")?;
    let env = y["entorno"].as_str().map(String::from);
    let mut out = vec![format!("Casos de {name} en {col}/{folder}")];
    let mut failed = 0;
    for (i, c) in cases.iter().enumerate() {
        let mut r = base.clone();
        r.name = format!("CP-{:02} · {}", i + 1, c["nombre"].as_str().unwrap_or("caso"));
        r.docs = format!("Caso de prueba de «{name}». {}", c["desc"].as_str().unwrap_or("")).trim().to_string();
        let pairs = |v: &Value, f: &dyn Fn(&Value) -> String| v.as_object().into_iter().flatten().map(|(k, v)| martino_core::model::Pair::new(k, &f(v))).collect::<Vec<_>>();
        let datos = pairs(&c["datos"], &text);
        r.vars_pre.retain(|p| !datos.iter().any(|d| d.name == p.name));
        r.vars_pre.extend(datos);
        // `status: 201` → `eq 201`: Bruno guarda operador y valor por separado.
        r.asserts = pairs(&c["espera"], &|v| {
            let (op, val) = martino_core::flow::split_assert(&text(v));
            format!("{op} {val}").trim().to_string()
        });
        if r.asserts.is_empty() {
            return Err(format!("casos[{i}]: falta «espera» (p. ej. status: 200)"));
        }
        let it = RunItem { collection: col.clone(), request: Some(r.clone()), env: env.clone(), ..Default::default() };
        let res = run_item(core, &it, &Default::default()).await;
        let path = save(core, &col, &folder, r.clone())?;
        let ok = res.error.is_none() && res.asserts.iter().all(|a| a.pass);
        failed += usize::from(!ok);
        out.push(format!("{} {} · status {} · {} ms · {path}", if ok { "✓" } else { "✗" }, r.name, res.status.map(|s| s.to_string()).unwrap_or("—".into()), res.time_ms));
        out.extend(res.error.map(|e| format!("    {e}")));
        out.extend(res.asserts.iter().filter(|a| !a.pass).map(|a| format!("    ≠ {} {} {} · real {}", a.name, a.op, a.expected, a.actual)));
        if !res.missing.is_empty() {
            out.push(format!("    variables sin valor: {}", res.missing.join(", ")));
        }
    }
    out.push(format!("{} de {} casos OK", cases.len() - failed, cases.len()));
    Ok(out.join("\n"))
}

async fn handle(core: &Arc<Core>, m: &Value) -> Option<Value> {
    let id = m.get("id")?.clone(); // sin id es una notificación: no se responde
    let result = match m["method"].as_str().unwrap_or("") {
        "initialize" => json!({
            "protocolVersion": m["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "martino", "version": env!("CARGO_PKG_VERSION")},
            "instructions": "Martino orquesta servicios HTTP como flujos. Para explicar o probar un servicio: martino_listar → martino_peticion → escribe un flujo en YAML (martino_guia) con `desc` en cada paso → martino_importar_flujo → martino_ejecutar_flujo con casos hasta que todo dé OK."
        }),
        "tools/list" => json!({"tools": tools()}),
        "tools/call" => {
            let p = &m["params"];
            let r = call(core, p["name"].as_str().unwrap_or(""), &p["arguments"]).await;
            let (text, err) = match r { Ok(t) => (t, false), Err(e) => (e, true) };
            json!({"content": [{"type": "text", "text": text}], "isError": err})
        }
        "ping" => json!({}),
        other => return Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("método no soportado: {other}")}})),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

pub async fn serve(core: Arc<Core>) {
    let stdin = std::io::stdin();
    for line in stdin.lock().lines().map_while(Result::ok) {
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(m) => handle(&core, &m).await,
            Err(e) => Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": e.to_string()}})),
        };
        if let Some(r) = reply {
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "{r}");
            let _ = out.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use martino::config::Config;

    #[tokio::test]
    async fn enviar_crea_la_coleccion_y_no_duplica() {
        let d = tempfile::tempdir().unwrap();
        let core = Core::new(Config::load(d.path(), json!({})));
        let a = json!({"coleccion": "Agente", "carpeta": "Usuarios", "curl": "curl http://127.0.0.1:9/users/1?x=1"});
        for _ in 0..2 {
            let t = call(&core, "martino_enviar", &a).await.unwrap();
            assert!(t.starts_with("✓ guardada en Agente/Usuarios/") && t.contains("✗"), "{t}");
        }
        let col = core.store.list().into_iter().find(|c| c.name == "Agente").expect("colección creada");
        assert_eq!(col.items[0].items.len(), 1, "la segunda vez actualiza, no duplica");
    }

    #[tokio::test]
    async fn casos_de_un_endpoint_en_su_carpeta() {
        let d = tempfile::tempdir().unwrap();
        let core = Core::new(Config::load(d.path(), json!({})));
        let y = "coleccion: Agente\ncurl: curl http://127.0.0.1:9/cuentas/{{nro}}\ncasos:\n  - nombre: Existe\n    datos: {nro: 7}\n    espera: {status: 200, body.id: isDefined}\n  - nombre: No existe\n    datos: {nro: 0}\n    espera: {status: 404}\n";
        for _ in 0..2 {
            let t = call(&core, "martino_casos_endpoint", &json!({"yaml": y})).await.unwrap();
            assert!(t.contains("✗ CP-01 · Existe") && t.ends_with("0 de 2 casos OK"), "{t}");
        }
        let dir = core.store.list().into_iter().find(|c| c.name == "Agente").unwrap().items.into_iter().find(|i| i.folder).expect("carpeta de casos");
        assert_eq!(dir.items.len(), 2, "un archivo por caso, sin duplicar al repetir");
        let r = core.store.read_request("Agente", &dir.items[0].path).unwrap();
        assert_eq!((r.vars_pre[0].value.as_str(), r.asserts[0].value.as_str()), ("7", "eq 200"));
    }
}
