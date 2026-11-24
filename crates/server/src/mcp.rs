//! Servidor MCP (JSON-RPC por stdio, una línea por mensaje): las mismas operaciones que la CLI,
//! para que un agente explore colecciones, escriba flujos en YAML y los ejecute.
use martino::app::{flow_job, FlowRunBody};
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
        "martino_guia" => Ok(GUIDE.into()),
        _ => Err(format!("herramienta desconocida: {name}")),
    }
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
