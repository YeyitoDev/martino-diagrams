//! FlowStore (`flows/<id>.flow.json`) y el motor `run_flow`.
use crate::executor::{Overrides, RunResult};
use crate::state::Core;
use crate::store::{safe_join, slug, Res};
use futures::stream::{FuturesUnordered, StreamExt};
use martino_core::flow::{self, compare, service_port, Case, CompareResult, Flow, Node, NodeOut, RefCtx, Scheduler};
use martino_core::interp::{value_to_text, whole_ref};
use martino_core::qa::{case_checks, Check};
use martino_core::model::{Request, SentRequest};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

// ---------- almacenamiento ----------

#[derive(Clone)]
pub struct FlowStore {
    pub dir: PathBuf,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowSummary {
    pub id: String,
    pub name: String,
    pub collection: String,
    pub nodes: usize,
}

impl FlowStore {
    pub fn new(dir: PathBuf) -> FlowStore {
        let _ = std::fs::create_dir_all(&dir);
        FlowStore { dir }
    }

    fn path(&self, id: &str) -> Res<PathBuf> {
        if id.is_empty() || id.contains(['/', '\\']) {
            return Err(format!("id de flujo inválido: «{id}»"));
        }
        safe_join(&self.dir, &format!("{id}.flow.json"))
    }

    pub fn list(&self) -> Vec<FlowSummary> {
        let mut out: Vec<FlowSummary> = std::fs::read_dir(&self.dir)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".flow.json").map(String::from))
                    .filter_map(|id| self.get(&id).ok().map(|f| FlowSummary { name: f.name, collection: f.collection, nodes: f.nodes.len(), id }))
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        out
    }

    pub fn get(&self, id: &str) -> Res<Flow> {
        let s = std::fs::read_to_string(self.path(id)?).map_err(|_| format!("No existe el flujo «{id}»"))?;
        serde_json::from_str(&s).map_err(|e| format!("{id}.flow.json inválido: {e}"))
    }

    pub fn put(&self, id: &str, f: &Flow) -> Res<()> {
        let errs = flow::validate(f);
        if !errs.is_empty() {
            return Err(errs.join("\n"));
        }
        std::fs::write(self.path(id)?, serde_json::to_string_pretty(f).unwrap()).map_err(|e| e.to_string())
    }

    pub fn create(&self, f: &Flow) -> Res<String> {
        let base = slug(&f.name).replace(' ', "-").to_lowercase();
        let id = (1..).map(|i| if i == 1 { base.clone() } else { format!("{base}-{i}") }).find(|id| !self.dir.join(format!("{id}.flow.json")).exists()).unwrap();
        self.put(&id, f)?;
        Ok(id)
    }

    pub fn delete(&self, id: &str) -> Res<()> {
        std::fs::remove_file(self.path(id)?).map_err(|e| e.to_string())
    }

    /// Una petición se movió: actualiza los nodos que la referencian. Devuelve cuántos flujos cambió.
    pub fn rename_ref(&self, collection: &str, from: &str, to: &str) -> usize {
        let mut n = 0;
        for s in self.list() {
            let Ok(mut f) = self.get(&s.id) else { continue };
            if f.collection != collection {
                continue;
            }
            let mut changed = false;
            for node in f.nodes.iter_mut().filter(|x| x.req_ref == from) {
                node.req_ref = to.into();
                changed = true;
            }
            if let Some(g) = f.global.as_mut().filter(|g| g.req_ref == from) {
                g.req_ref = to.into();
                changed = true;
            }
            if changed && self.put(&s.id, &f).is_ok() {
                n += 1;
            }
        }
        n
    }
}

// ---------- motor ----------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Seed {
    pub outs: HashMap<String, NodeOut>,
    pub vars: Map<String, Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RunOpts {
    pub env: Option<String>,
    pub input: Option<Value>,
    pub compare: bool,
    /// Caso QA: su entrada ya va en `input`; aquí se usa lo que espera.
    pub case: Option<Case>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Step {
    pub id: String,
    pub key: String,
    pub kind: String,
    pub name: String,
    pub ok: bool,
    pub port: Option<String>,
    pub out: NodeOut,
    pub ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sent: Option<SentRequest>,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub detail: Value,
    #[serde(skip)]
    pub set_vars: Map<String, Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FlowResult {
    pub ok: bool,
    /// OK | DIFERENTE | ERROR | SIN RESPUESTA
    pub verdict: String,
    pub status: Option<u16>,
    pub body: Value,
    pub response_node: Option<String>,
    pub ended: bool,
    pub steps: Vec<Step>,
    pub vars: Map<String, Value>,
    pub outs: HashMap<String, NodeOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compare: Option<CompareResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub ms: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<Check>,
}

pub type Emit<'a> = &'a (dyn Fn(Value) + Send + Sync);

fn effective_input(flow: &Flow, input: Option<&Value>) -> Value {
    let mut data = flow.input_node().map(|n| n.data.clone()).unwrap_or(json!({}));
    if let (Some(Value::Object(over)), Value::Object(base)) = (input, &mut data) {
        base.extend(over.clone());
    } else if let Some(i) = input {
        data = i.clone();
    }
    data
}

/// Lo que ve un nodo al empezar (copia: los nodos en paralelo no se pisan).
#[derive(Clone)]
struct Snap {
    outs: HashMap<String, NodeOut>,
    vars: Map<String, Value>,
}

/// Petición del nodo: `ref` al .bru o inline.
fn node_request(core: &Core, flow: &Flow, n: &Node) -> Res<Request> {
    if !n.req_ref.is_empty() {
        return core.store.read_request(&flow.collection, &n.req_ref);
    }
    n.request.clone().ok_or_else(|| format!("{}: el servicio no tiene petición", n.key))
}

async fn exec_node(core: &Core, flow: &Flow, n: &Node, snap: Snap, env: Option<&str>, cancel: &CancellationToken) -> Step {
    let t0 = Instant::now();
    let mut step = Step { id: n.id.clone(), key: n.key.clone(), kind: n.kind.clone(), name: n.name.clone(), ok: true, port: Some("out".into()), ..Default::default() };
    let scope = core.scope(&flow.collection, env, None);
    let flow_vars: Vec<(String, String)> = flow.vars.iter().filter(|p| p.enabled).map(|p| (p.name.clone(), p.value.clone())).collect();
    let env_fb = |name: &str| flow_vars.iter().find(|x| x.0 == name).map(|x| x.1.clone()).or_else(|| scope.resolve(name)).or_else(|| crate::executor::dynamic(name));
    let ctx = RefCtx { flow, outs: &snap.outs, vars: &snap.vars };
    let res: Result<(), String> = async {
        match n.kind.as_str() {
            "start" | "end" => {}
            "input" => step.out.value = snap.outs.get(&n.key).map(|o| o.value.clone()).unwrap_or_else(|| n.data.clone()),
            "data" => step.out.value = ctx.render_json(&n.data, &env_fb),
            "extract" => {
                let mut m = Map::new();
                for p in n.vars.iter().filter(|p| p.enabled) {
                    m.insert(p.name.clone(), ctx.value(&p.value, &env_fb, true));
                }
                step.set_vars = m.clone();
                step.out.value = Value::Object(m);
            }
            "condition" => {
                let mut results = vec![];
                for r in &n.rules {
                    let left = match whole_ref(&r.left).or_else(|| (!r.left.contains("{{")).then_some(r.left.trim())) {
                        Some(p) => ctx.lookup(p),
                        None => Some(Value::String(ctx.render(&r.left, &env_fb))),
                    };
                    let right = ctx.render(&r.right, &env_fb);
                    let pass = flow::check(left.as_ref(), &r.op, &right);
                    results.push(json!({"left": r.left, "op": r.op, "right": right, "actual": left, "pass": pass}));
                }
                let passes = results.iter().map(|r| r["pass"] == json!(true));
                let ok = if n.mode == "any" { passes.clone().any(|x| x) } else { passes.clone().all(|x| x) };
                step.out.value = json!(ok);
                step.detail = json!(results);
                step.port = Some(if ok { "true" } else { "false" }.into());
            }
            "transform" => {
                let nodes: Map<String, Value> = snap.outs.iter().map(|(k, o)| (k.clone(), node_view(o))).collect();
                let env_map: Map<String, Value> = scope.table().into_iter().map(|v| (v.name, json!(v.value))).chain(snap.vars.clone()).collect();
                let input = ctx.lookup(&flow.input_key()).unwrap_or(Value::Null);
                let code = n.code.clone();
                let v = tokio::task::spawn_blocking(move || run_js(&code, &Value::Object(nodes), &input, &Value::Object(env_map)))
                    .await
                    .map_err(|e| e.to_string())??;
                step.out.status = v.get("status").and_then(Value::as_u64).map(|s| s as u16);
                step.out.value = v;
            }
            "crypto" => {
                let mut s = n.crypto.clone().ok_or("Cifrado: falta configurarlo")?;
                s.key = ctx.render(&s.key, &env_fb);
                s.signature = ctx.render(&s.signature, &env_fb);
                let input = match ctx.value(&s.input, &env_fb, true) {
                    Value::String(t) => t,
                    v => v.to_string(),
                };
                // RSA es lento para el hilo de red.
                step.out.value = tokio::task::spawn_blocking(move || crate::crypto::run(&s, &input)).await.map_err(|e| e.to_string())??;
            }
            "lookup" => {
                let s = n.lookup.as_ref().ok_or("Equivalencia: elige una tabla")?;
                let table = martino_core::lookup::parse_table(&core.store.read_table(&flow.collection, &s.table)?);
                step.out.value = table.lookup(s, &value_to_text(&ctx.value(&s.value, &env_fb, true)), &s.table)?;
            }
            "response" => {
                let status = ctx.render(if n.status.is_empty() { "200" } else { &n.status }, &env_fb);
                let status: u16 = status.trim().parse().map_err(|_| format!("status inválido: «{status}»"))?;
                step.out.status = Some(status);
                step.out.response = ctx.render_json(&n.body, &env_fb);
                step.out.value = json!({"status": status, "body": step.out.response});
            }
            "request" => {
                let req = node_request(core, flow, n)?;
                let node_vars: Vec<(String, String)> = n.vars.iter().filter(|p| p.enabled).map(|p| (p.name.clone(), ctx.render(&p.value, &env_fb))).collect();
                let req_vars: Vec<(String, String)> = req.vars_pre.iter().filter(|p| p.enabled).map(|p| (p.name.clone(), p.value.clone())).collect();
                let resolve = |name: &str| -> Option<String> {
                    node_vars
                        .iter()
                        .chain(req_vars.iter())
                        .find(|x| x.0 == name)
                        .map(|x| ctx.render(&x.1, &env_fb))
                        .or_else(|| ctx.lookup(name).map(|v| value_to_text(&v)))
                        .or_else(|| env_fb(name))
                };
                let ov = Overrides {
                    body: n.body_overrides.iter().filter(|p| p.enabled).map(|p| (p.name.clone(), ctx.value(&p.value, &env_fb, false))).collect(),
                    headers: n.header_overrides.iter().filter(|p| p.enabled).map(|p| martino_core::model::Pair::new(&p.name, ctx.render(&p.value, &env_fb))).collect(),
                    query: n.query_overrides.iter().filter(|p| p.enabled).map(|p| martino_core::model::Pair::new(&p.name, ctx.render(&p.value, &env_fb))).collect(),
                };
                let root = core.store.root(&flow.collection).ok();
                let r: RunResult = core.exec.execute(&req, root.as_ref(), &resolve, &ov, cancel).await;
                fill_service(&mut step, &r);
                for p in n.save_vars.iter().filter(|p| p.enabled) {
                    let v = if p.value.contains("{{") {
                        ctx.value(&p.value, &env_fb, false)
                    } else {
                        flow::service_value(&step.out, p.value.trim()).unwrap_or(Value::Null)
                    };
                    step.set_vars.insert(p.name.clone(), v);
                }
                if let Some(e) = r.error.clone().filter(|_| r.status.is_none()) {
                    return Err(e);
                }
            }
            "note" | "image" => step.port = None,
            k => return Err(format!("tipo de nodo desconocido: {k}")),
        }
        Ok(())
    }
    .await;
    if let Err(e) = res {
        step.ok = false;
        step.error = Some(e);
        step.port = if n.kind == "request" { Some("error".into()) } else { None };
    }
    step.ms = t0.elapsed().as_millis() as u64;
    step
}

fn fill_service(step: &mut Step, r: &RunResult) {
    let sent_query: Vec<martino_core::model::Pair> = r
        .sent
        .url
        .split_once('?')
        .map(|(_, q)| q.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| martino_core::model::Pair::new(martino_core::curl::pct_decode(k), martino_core::curl::pct_decode(v))).collect())
        .unwrap_or_default();
    step.out = NodeOut {
        input: flow::request_input(&sent_query, r.sent.body.as_deref()),
        response: r.body_value(),
        status: r.status,
        headers: r.headers_json(),
        value: Value::Null,
    };
    step.port = Some(service_port(r.status).into());
    step.ok = r.status.is_some_and(|s| s < 400);
    step.sent = Some(r.sent.clone());
    if !r.asserts.is_empty() || !r.missing.is_empty() {
        step.detail = json!({"asserts": r.asserts, "missing": r.missing});
    }
}

/// Lo que ve `nodes.<clave>` dentro de un Transformar.
fn node_view(o: &NodeOut) -> Value {
    let mut m = Map::new();
    for (k, v) in [("input", &o.input), ("response", &o.response), ("headers", &o.headers), ("value", &o.value)] {
        if !v.is_null() {
            m.insert(k.into(), v.clone());
        }
    }
    if let Some(s) = o.status {
        m.insert("status".into(), json!(s));
    }
    if let Value::Object(v) = &o.value {
        for (k, x) in v {
            m.entry(k.clone()).or_insert(x.clone());
        }
    }
    Value::Object(m)
}

const JS_PRELUDE: &str = r#"
"use strict";
delete globalThis.eval;
for (const f of [function(){}, async function(){}, function*(){}, async function*(){}]) {
  Object.defineProperty(Object.getPrototypeOf(f), 'constructor', { value: undefined });
}
globalThis.Function = undefined;
const __b64 = (s) => {
  const t = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
  s = s.replace(/-/g, '+').replace(/_/g, '/').replace(/=+$/, '');
  let bits = 0, n = 0, bytes = [];
  for (const c of s) { n = (n << 6) | t.indexOf(c); bits += 6; if (bits >= 8) { bits -= 8; bytes.push((n >> bits) & 255); } }
  let out = '';
  for (let i = 0; i < bytes.length; i++) out += '%' + bytes[i].toString(16).padStart(2, '0');
  return decodeURIComponent(out);
};
const jwt = (token) => {
  const p = String(token || '').replace(/^Bearer\s+/i, '').split('.');
  if (p.length < 2) throw new Error('jwt(): el token no tiene el formato a.b.c');
  return JSON.parse(__b64(p[1]));
};
const paginate = (list, page = 1, size = 10) => {
  const items = Array.isArray(list) ? list : [];
  const pages = Math.max(1, Math.ceil(items.length / size));
  return { items: items.slice((page - 1) * size, page * size), page, size, total: items.length, pages };
};
"#;

/// Ejecuta el código de un Transformar en QuickJS: sin objetos del host, sin eval, 1 s máximo.
pub fn run_js(code: &str, nodes: &Value, input: &Value, env: &Value) -> Result<Value, String> {
    use rquickjs::{Context, Runtime};
    let rt = Runtime::new().map_err(|e| e.to_string())?;
    rt.set_memory_limit(64 * 1024 * 1024);
    let deadline = Instant::now() + Duration::from_secs(1);
    rt.set_interrupt_handler(Some(Box::new(move || Instant::now() > deadline)));
    let ctx = Context::full(&rt).map_err(|e| e.to_string())?;
    let lit = |v: &Value| serde_json::to_string(&v.to_string()).unwrap();
    let script = format!(
        "{JS_PRELUDE}\nconst nodes = JSON.parse({}); const input = JSON.parse({}); const env = JSON.parse({});\n\
         const __r = (function(nodes, input, env) {{\n{code}\n}})(nodes, input, env);\n\
         JSON.stringify(__r === undefined ? null : __r)",
        lit(nodes),
        lit(input),
        lit(env)
    );
    ctx.with(|ctx| {
        let r: Result<String, _> = ctx.eval(script);
        match r {
            Ok(s) => serde_json::from_str(&s).map_err(|e| e.to_string()),
            Err(rquickjs::Error::Exception) => {
                let ex = ctx.catch();
                let msg = ex.as_exception().and_then(|e| e.message()).or_else(|| ex.as_string().and_then(|s| s.to_string().ok())).unwrap_or_else(|| "error de JavaScript".into());
                if Instant::now() > deadline {
                    Err("Transformar: se pasó de 1 s (¿bucle infinito?)".into())
                } else {
                    Err(format!("Transformar: {msg}"))
                }
            }
            Err(e) => Err(format!("Transformar: {e}")),
        }
    })
}

/// Ejecuta un solo nodo con el contexto (`seed`) de la última ejecución.
pub async fn run_node(core: &Core, flow: &Flow, node_id: &str, seed: Seed, env: Option<&str>) -> Res<Step> {
    let n = flow.node(node_id).ok_or_else(|| format!("No existe el nodo {node_id}"))?;
    let mut outs = seed.outs;
    if let Some(i) = flow.input_node() {
        outs.entry(i.key.clone()).or_insert(NodeOut { value: i.data.clone(), ..Default::default() });
    }
    Ok(exec_node(core, flow, n, Snap { outs, vars: seed.vars }, env, &CancellationToken::new()).await)
}

pub async fn run_flow(core: &Core, flow: &Flow, opts: &RunOpts, cancel: &CancellationToken, emit: Emit<'_>) -> FlowResult {
    let t0 = Instant::now();
    let env = opts.env.as_deref();
    let mut res = FlowResult::default();
    let mut outs: HashMap<String, NodeOut> = HashMap::new();
    for n in flow.nodes.iter().filter(|n| n.kind == "input") {
        let data = if Some(n.id.as_str()) == flow.input_node().map(|x| x.id.as_str()) { effective_input(flow, opts.input.as_ref()) } else { n.data.clone() };
        outs.insert(n.key.clone(), NodeOut { value: data, ..Default::default() });
    }
    let mut vars = Map::new();
    let mut sched = Scheduler::new(flow);
    let mut pending = FuturesUnordered::new();
    loop {
        for id in sched.take_ready() {
            let Some(n) = flow.node(&id) else { continue };
            emit(json!({"type": "node-start", "id": id}));
            let snap = Snap { outs: outs.clone(), vars: vars.clone() };
            pending.push(exec_node(core, flow, n, snap, env, cancel));
        }
        if sched.over_limit() {
            res.error = Some(format!("Se superó el límite de {} pasos (¿ciclo?)", flow::MAX_STEPS));
            break;
        }
        if pending.is_empty() {
            break;
        }
        let step = tokio::select! {
            _ = cancel.cancelled() => { res.error = Some("Cancelado".into()); break; }
            s = pending.next() => match s { Some(s) => s, None => break },
        };
        outs.insert(step.key.clone(), step.out.clone());
        vars.extend(step.set_vars.clone());
        emit(json!({"type": "node", "step": step}));
        let node = flow.node(&step.id).cloned().unwrap_or_default();
        let mut port = step.port.clone();
        if node.kind == "request" && port.as_deref() == Some("error") && flow.outgoing(&node.id).all(|e| e.port != "error") {
            port = None; // sin rama de error: la rama termina aquí
        }
        res.steps.push(step.clone());
        match node.kind.as_str() {
            "response" if res.response_node.is_none() => {
                res.response_node = Some(node.id.clone());
                res.status = step.out.status;
                res.body = step.out.response.clone();
                // La primera Respuesta gana: solo sigue su propia salida (si tiene).
                sched.stop();
                pending.clear();
                sched.running.clear();
                let has_out = flow.outgoing(&node.id).next().is_some();
                sched.complete(flow, &node.id, has_out.then_some("out"));
            }
            "response" => sched.complete(flow, &node.id, None),
            "end" => {
                res.ended = true;
                sched.stop();
                pending.clear();
                sched.running.clear();
                sched.complete(flow, &node.id, None);
            }
            _ => sched.complete(flow, &node.id, port.as_deref()),
        }
    }
    res.outs = outs;
    res.vars = vars;

    if opts.compare && res.error.is_none() {
        if let Some(g) = &flow.global {
            match core.store.read_request(&flow.collection, &g.req_ref) {
                Ok(req) => {
                    let scope = core.scope(&flow.collection, env, Some(&req));
                    let ctx = RefCtx { flow, outs: &res.outs, vars: &res.vars };
                    let resolve = |n: &str| ctx.lookup(n).map(|v| value_to_text(&v)).or_else(|| scope.resolve(n));
                    let root = core.store.root(&flow.collection).ok();
                    let r = core.exec.execute(&req, root.as_ref(), &resolve, &Overrides::default(), cancel).await;
                    res.compare = Some(compare(&flow.compare, res.status, &res.body, r.status, &r.body_value()));
                    res.global = Some(json!({"status": r.status, "body": r.body_value(), "error": r.error, "ms": r.time_ms}));
                }
                Err(e) => res.error = Some(format!("Servicio global: {e}")),
            }
        }
    }

    res.ms = t0.elapsed().as_millis() as u64;
    if let Some(c) = &opts.case {
        let end_key = res.response_node.as_deref().and_then(|id| flow.node(id)).map(|n| n.key.as_str());
        res.checks = case_checks(c, res.status, &res.body, res.ms, end_key);
    }
    let failed_step = res.steps.iter().find(|s| !s.ok && s.port.is_none() && s.kind != "condition");
    res.verdict = if res.error.is_some() || failed_step.is_some() {
        if res.error.is_none() {
            res.error = failed_step.and_then(|s| s.error.clone().map(|e| format!("{}: {e}", s.key)));
        }
        "ERROR".into()
    } else if res.compare.as_ref().is_some_and(|c| !c.ok) || res.checks.iter().any(|k| !k.ok) {
        "DIFERENTE".into()
    } else if res.response_node.is_none() && !res.ended {
        "SIN RESPUESTA".into()
    } else {
        "OK".into()
    };
    res.ok = res.verdict == "OK";
    emit(json!({"type": "done", "result": res}));
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn js_sandbox() {
        let v = run_js("return { status: 200, body: { n: input.a + nodes.s.response.x, sub: jwt(env.t).sub, p: paginate([1,2,3], 2, 2) } }",
            &json!({"s": {"response": {"x": 2}}}), &json!({"a": 1}), &json!({"t": "Bearer x.eyJzdWIiOiJhbmEifQ.y"})).unwrap();
        assert_eq!(v, json!({"status": 200, "body": {"n": 3, "sub": "ana", "p": {"items": [3], "page": 2, "size": 2, "total": 3, "pages": 2}}}));
        assert!(run_js("return eval('1')", &json!({}), &json!({}), &json!({})).is_err());
        assert!(run_js("return (function(){}).constructor('return 1')()", &json!({}), &json!({}), &json!({})).is_err());
        let t = Instant::now();
        assert!(run_js("while(true){}", &json!({}), &json!({}), &json!({})).unwrap_err().contains("1 s"));
        assert!(t.elapsed() < Duration::from_secs(3));
    }
}
