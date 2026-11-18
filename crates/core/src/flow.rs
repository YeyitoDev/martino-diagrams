//! Núcleo de flujos compartido servidor/navegador: tipos, referencias, operadores,
//! comparación, CSV de casos y el planificador (paralelo + unión).
pub use crate::csv::{csv_template, csv_to_cases, parse_csv, to_csv};
use crate::interp::{get_path, interpolate, leaf_paths, parse_path, segments, value_to_text, whole_ref, PathSeg, Seg};
use crate::model::{Pair, Request};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

pub const MAX_STEPS: usize = 2000;
pub const DEFAULT_INPUT_KEY: &str = "entrada";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Flow {
    pub name: String,
    pub collection: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global: Option<Global>,
    pub compare: Compare,
    pub cases: Vec<Case>,
    /// Variables propias del flujo (panel "Variables").
    pub vars: Vec<Pair>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Global {
    /// Ruta de la petición, .yml o .bru (normalmente con `{{globalUrl}}`).
    #[serde(rename = "ref")]
    pub req_ref: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Compare {
    /// expected | strict | fields
    pub mode: String,
    pub fields: Vec<String>,
    pub ignore: Vec<String>,
}
impl Default for Compare {
    fn default() -> Self {
        Compare { mode: "expected".into(), fields: vec![], ignore: vec![] }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Case {
    pub id: String,
    pub name: String,
    pub input: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expect_status: Option<u16>,
    /// Qué se espera obtener. `left`: `status`, `ms`, `nodo` o una ruta del body (`error.codigo`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub expect: Vec<Rule>,
    /// Por qué importa este caso.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub port: String,
    pub to: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Origin {
    pub layer: String,
    #[serde(rename = "ref")]
    pub reference: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rule {
    pub left: String,
    pub op: String,
    pub right: String,
}

fn empty<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Node {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub key: String,
    pub name: String,
    #[serde(skip_serializing_if = "empty")]
    pub desc: String,
    #[serde(skip_serializing_if = "empty")]
    pub color: String,
    pub x: f64,
    pub y: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
    /// input: campos de la entrada · data: JSON fijo
    #[serde(skip_serializing_if = "Value::is_null")]
    pub data: Value,
    /// request: ruta de la petición (.yml o .bru) en la colección
    #[serde(rename = "ref", skip_serializing_if = "empty")]
    pub req_ref: String,
    /// request: petición inline (si no hay `ref`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<Request>,
    /// request: variables antes de enviar · extract: variables a guardar
    #[serde(skip_serializing_if = "empty")]
    pub vars: Vec<Pair>,
    #[serde(skip_serializing_if = "empty")]
    pub body_overrides: Vec<Pair>,
    #[serde(skip_serializing_if = "empty")]
    pub header_overrides: Vec<Pair>,
    #[serde(skip_serializing_if = "empty")]
    pub query_overrides: Vec<Pair>,
    #[serde(skip_serializing_if = "empty")]
    pub save_vars: Vec<Pair>,
    #[serde(skip_serializing_if = "empty")]
    pub rules: Vec<Rule>,
    /// condition: all | any
    #[serde(skip_serializing_if = "empty")]
    pub mode: String,
    #[serde(skip_serializing_if = "empty")]
    pub code: String,
    /// response: status (admite plantilla)
    #[serde(skip_serializing_if = "empty")]
    pub status: String,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub body: Value,
    #[serde(skip_serializing_if = "empty")]
    pub text: String,
    #[serde(skip_serializing_if = "empty")]
    pub src: String,
    #[serde(skip_serializing_if = "empty")]
    pub caption: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    /// Identificador visible y fijo del paso (SRV-01, CND-02…). Se asigna al crearlo.
    #[serde(skip_serializing_if = "empty")]
    pub sid: String,
    /// Alto elegido al redimensionar el nodo en el lienzo (si no, el de su tipo).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    /// Campos del resultado que el nodo muestra siempre, aunque nadie los use.
    #[serde(skip_serializing_if = "empty")]
    pub pins: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crypto: Option<crate::crypto::CryptoSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lookup: Option<crate::lookup::LookupSpec>,
}

pub const NODE_TYPES: [&str; 13] =
    ["start", "input", "request", "extract", "condition", "data", "transform", "crypto", "lookup", "response", "end", "note", "image"];

/// Puertos de salida de cada tipo.
pub fn ports(kind: &str) -> &'static [&'static str] {
    match kind {
        "request" => &["out", "error"],
        "condition" => &["true", "false"],
        "end" | "note" | "image" => &[],
        _ => &["out"],
    }
}

pub fn is_annotation(kind: &str) -> bool {
    matches!(kind, "note" | "image")
}

/// Resultado de un nodo ya ejecutado (lo que pueden leer los siguientes).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NodeOut {
    #[serde(skip_serializing_if = "Value::is_null")]
    pub input: Value,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub response: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub headers: Value,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub value: Value,
}

impl Flow {
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }
    pub fn by_key(&self, key: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.key == key).or_else(|| self.node(key))
    }
    pub fn input_node(&self) -> Option<&Node> {
        self.nodes.iter().find(|n| n.kind == "input")
    }
    pub fn input_key(&self) -> String {
        self.input_node().map(|n| n.key.clone()).unwrap_or_else(|| DEFAULT_INPUT_KEY.into())
    }
    pub fn outgoing<'a>(&'a self, id: &'a str) -> impl Iterator<Item = &'a Edge> + 'a {
        self.edges.iter().filter(move |e| e.from == id)
    }
    pub fn incoming<'a>(&'a self, id: &'a str) -> impl Iterator<Item = &'a Edge> + 'a {
        self.edges.iter().filter(move |e| e.to == id)
    }
    pub fn keys(&self) -> Vec<String> {
        self.nodes.iter().filter(|n| !is_annotation(&n.kind)).map(|n| n.key.clone()).collect()
    }
}

// ---------- referencias ----------

pub fn ref_key(path: &str) -> &str {
    let end = path.find(['.', '[']).unwrap_or(path.len());
    &path[..end]
}

/// `snake_case` único entre las claves existentes.
pub fn unique_key(base: &str, existing: &[String]) -> String {
    let mut k: String = base
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c.to_ascii_lowercase() } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string();
    if k.is_empty() || k.starts_with(|c: char| c.is_ascii_digit()) {
        k = format!("n_{k}");
    }
    if !existing.contains(&k) {
        return k;
    }
    (2..).map(|i| format!("{k}_{i}")).find(|c| !existing.contains(c)).unwrap()
}

pub struct RefCtx<'a> {
    pub flow: &'a Flow,
    /// Resultados por clave de nodo.
    pub outs: &'a HashMap<String, NodeOut>,
    pub vars: &'a Map<String, Value>,
}

fn sub(v: &Value, rest: &str) -> Option<Value> {
    if rest.is_empty() { Some(v.clone()) } else { get_path(v, rest).cloned() }
}

/// Valor de un nodo de servicio: `input.x`, `response.x` (alias `body.x`), `status`, `headers.x`.
pub fn service_value(out: &NodeOut, rest: &str) -> Option<Value> {
    let segs = parse_path(rest);
    let (head, tail) = match segs.first() {
        Some(PathSeg::Key(k)) => (k.as_str(), rest[k.len()..].trim_start_matches('.')),
        _ => return sub(&out.response, rest),
    };
    match head {
        "input" => sub(&out.input, tail),
        "response" | "body" => sub(&out.response, tail),
        "status" => out.status.map(|s| json!(s)),
        "headers" => {
            let h = &out.headers;
            if tail.is_empty() {
                return Some(h.clone());
            }
            h.as_object()?.iter().find(|(k, _)| k.eq_ignore_ascii_case(tail)).map(|x| x.1.clone())
        }
        _ => sub(&out.response, rest),
    }
}

pub fn data_value(out: &NodeOut, rest: &str) -> Option<Value> {
    sub(&out.value, rest)
}

impl RefCtx<'_> {
    /// Resuelve una referencia del flujo (`clave.ruta`, `input.x`, `nodes.clave.x`, `variable`).
    pub fn lookup(&self, path: &str) -> Option<Value> {
        let path = path.trim().strip_prefix("nodes.").unwrap_or(path.trim());
        let head = ref_key(path);
        let rest = path[head.len()..].trim_start_matches('.');
        let head = if head == "input" && self.flow.by_key("input").is_none() { self.flow.input_key() } else { head.to_string() };
        if let Some(n) = self.flow.by_key(&head) {
            let out = self.outs.get(&n.key);
            return match (n.kind.as_str(), out) {
                ("request", Some(o)) => service_value(o, rest),
                ("input", None) => sub(&n.data, rest),
                (_, Some(o)) => data_value(o, rest),
                ("data", None) => sub(&n.data, rest),
                _ => None,
            };
        }
        let v = self.vars.get(&head)?;
        sub(v, rest)
    }

    /// Plantilla → texto; `fallback` resuelve lo que no es del flujo (entorno, dinámicas).
    pub fn render(&self, s: &str, fallback: &dyn Fn(&str) -> Option<String>) -> String {
        interpolate(s, &|n| self.lookup(n).map(|v| value_to_text(&v)).or_else(|| fallback(n)))
    }

    /// Como `render` pero conserva el tipo si el texto es solo `{{ref}}` o una ruta sin llaves.
    pub fn value(&self, s: &str, fallback: &dyn Fn(&str) -> Option<String>, bare_paths: bool) -> Value {
        if let Some(r) = whole_ref(s) {
            if let Some(v) = self.lookup(r) {
                return v;
            }
        } else if bare_paths && !s.contains("{{") && !s.trim().is_empty() {
            if let Some(v) = self.lookup(s) {
                return v;
            }
        }
        let t = self.render(s, fallback);
        if whole_ref(s).is_some() {
            return serde_json::from_str(&t).unwrap_or(Value::String(t));
        }
        Value::String(t)
    }

    /// Interpola cada hoja de texto de un JSON (manteniendo tipos de `{{ref}}` completos).
    pub fn render_json(&self, v: &Value, fallback: &dyn Fn(&str) -> Option<String>) -> Value {
        match v {
            Value::String(s) => self.value(s, fallback, false),
            Value::Array(a) => Value::Array(a.iter().map(|x| self.render_json(x, fallback)).collect()),
            Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), self.render_json(x, fallback))).collect()),
            o => o.clone(),
        }
    }
}

/// Query + body (JSON) de una petición en un solo objeto (lo que muestra `servicio.input`).
pub fn request_input(query: &[Pair], body: Option<&str>) -> Value {
    let mut m = Map::new();
    for p in query.iter().filter(|p| p.enabled) {
        m.insert(p.name.clone(), Value::String(p.value.clone()));
    }
    if let Some(Value::Object(b)) = body.and_then(|b| serde_json::from_str::<Value>(b).ok()) {
        m.extend(b);
    }
    Value::Object(m)
}

// ---------- reescritura de plantillas ----------

const PROTECTED: [&str; 7] = ["id", "key", "type", "port", "from", "to", "src"];

/// Aplica `f` a cada texto editable de los nodos (no toca id/key/type/port).
pub fn map_node_strings(nodes: &mut [Node], f: &mut dyn FnMut(&str) -> String) {
    // Sólo las claves del nodo están protegidas: un `id` dentro de un body es un dato más.
    fn walk(v: &mut Value, f: &mut dyn FnMut(&str) -> String, top: bool) {
        match v {
            Value::String(s) => *s = f(s),
            Value::Array(a) => a.iter_mut().for_each(|x| walk(x, f, false)),
            Value::Object(m) => {
                for (k, x) in m.iter_mut() {
                    if !(top && PROTECTED.contains(&k.as_str())) {
                        walk(x, f, false)
                    }
                }
            }
            _ => {}
        }
    }
    for n in nodes.iter_mut() {
        let mut v = serde_json::to_value(&*n).unwrap();
        walk(&mut v, f, true);
        *n = serde_json::from_value(v).unwrap();
    }
}

/// Convierte la sintaxis antigua (`{{input.x}}`, `{{svc.body.x}}`, `{{nodes.k.x}}`) a la actual.
/// Devuelve cuántos cambios hizo.
pub fn modernize_refs(flow: &mut Flow) -> usize {
    let input_key = flow.input_key();
    let services: HashSet<String> = flow.nodes.iter().filter(|n| n.kind == "request").map(|n| n.key.clone()).collect();
    let has_input_node = flow.by_key("input").is_some();
    let mut count = 0;
    map_node_strings(&mut flow.nodes, &mut |s| {
        let mut out = String::new();
        for seg in segments(s) {
            match seg {
                Seg::Text(t) => out.push_str(t),
                Seg::Var(n, raw) => {
                    let mut name = n.strip_prefix("nodes.").unwrap_or(n).to_string();
                    let head = ref_key(&name).to_string();
                    if head == "input" && !has_input_node && input_key != "input" {
                        name = format!("{input_key}{}", &name[5..]);
                    }
                    let head = ref_key(&name).to_string();
                    if services.contains(&head) && name[head.len()..].starts_with(".body") {
                        name = format!("{head}.response{}", &name[head.len() + 5..]);
                    }
                    if name == n {
                        out.push_str(raw)
                    } else {
                        count += 1;
                        out.push_str(&format!("{{{{{name}}}}}"))
                    }
                }
            }
        }
        out
    });
    count
}

// ---------- operadores (asserts y condiciones) ----------

pub const OPS: [&str; 22] = [
    "eq", "neq", "gt", "gte", "lt", "lte", "contains", "notContains", "startsWith", "endsWith", "in", "notIn", "isDefined",
    "isUndefined", "isNull", "isNotNull", "isEmpty", "isNotEmpty", "isTruthy", "isFalsy", "isNumber", "isString",
];

fn unquote(s: &str) -> &str {
    let t = s.trim();
    if t.len() >= 2 && ((t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\''))) {
        &t[1..t.len() - 1]
    } else {
        t
    }
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        Value::Bool(b) => Some(*b as u8 as f64),
        _ => None,
    }
}

fn loose_eq(a: &Value, expected: &str) -> bool {
    let e = expected.trim();
    if let (Some(x), Ok(y)) = (num(a), e.parse::<f64>()) {
        if !a.is_string() || e == unquote(e) {
            return x == y;
        }
    }
    match a {
        Value::Bool(b) => e == b.to_string(),
        Value::Null => e == "null",
        Value::String(s) => s == unquote(e),
        other => serde_json::from_str::<Value>(e).map(|v| &v == other).unwrap_or(false) || other.to_string() == e,
    }
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::String(s)) => !s.is_empty(),
        _ => true,
    }
}

fn is_empty_val(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => s.is_empty(),
        Some(Value::Array(a)) => a.is_empty(),
        Some(Value::Object(m)) => m.is_empty(),
        _ => false,
    }
}

/// Evalúa `actual <op> expected`.
pub fn check(actual: Option<&Value>, op: &str, expected: &str) -> bool {
    let text = || actual.map(value_to_text).unwrap_or_default();
    let cmp = |f: fn(f64, f64) -> bool| match (actual.and_then(num), expected.trim().parse::<f64>()) {
        (Some(a), Ok(b)) => f(a, b),
        _ => false,
    };
    let list = || -> Vec<String> {
        match serde_json::from_str::<Value>(expected) {
            Ok(Value::Array(a)) => a.iter().map(value_to_text).collect(),
            _ => expected.split(',').map(|s| unquote(s).to_string()).collect(),
        }
    };
    match op {
        "eq" => actual.map(|a| loose_eq(a, expected)).unwrap_or(false),
        "neq" => !actual.map(|a| loose_eq(a, expected)).unwrap_or(false),
        "gt" => cmp(|a, b| a > b),
        "gte" => cmp(|a, b| a >= b),
        "lt" => cmp(|a, b| a < b),
        "lte" => cmp(|a, b| a <= b),
        "contains" | "notContains" => {
            let e = unquote(expected);
            let has = match actual {
                Some(Value::Array(a)) => a.iter().any(|x| value_to_text(x) == e),
                Some(Value::Object(m)) => m.contains_key(e),
                _ => text().contains(e),
            };
            has == (op == "contains")
        }
        "startsWith" => text().starts_with(unquote(expected)),
        "endsWith" => text().ends_with(unquote(expected)),
        "in" => list().contains(&text()),
        "notIn" => !list().contains(&text()),
        "isDefined" => actual.is_some(),
        "isUndefined" => actual.is_none(),
        "isNull" => matches!(actual, Some(Value::Null)),
        "isNotNull" => !matches!(actual, Some(Value::Null) | None),
        "isEmpty" => is_empty_val(actual),
        "isNotEmpty" => !is_empty_val(actual),
        "isTruthy" => truthy(actual),
        "isFalsy" => !truthy(actual),
        "isNumber" => matches!(actual, Some(Value::Number(_))),
        "isString" => matches!(actual, Some(Value::String(_))),
        _ => false,
    }
}

/// `eq 200` → ("eq", "200"). Sin operador conocido se asume `eq`.
pub fn split_assert(v: &str) -> (String, String) {
    let t = v.trim();
    let (op, rest) = t.split_once(' ').unwrap_or((t, ""));
    if OPS.contains(&op) { (op.into(), rest.trim().into()) } else { ("eq".into(), t.into()) }
}

// ---------- comparación con el servicio global ----------

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Diff {
    pub path: String,
    /// falta | distinto | sobra | status
    pub kind: String,
    pub expected: Value,
    pub actual: Value,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CompareResult {
    pub ok: bool,
    pub mode: String,
    pub diffs: Vec<Diff>,
}

fn wild(p: &str) -> String {
    let mut out = String::new();
    let mut in_idx = false;
    for c in p.chars() {
        match c {
            '[' => {
                in_idx = true;
                out.push_str("[*]")
            }
            ']' => in_idx = false,
            _ if in_idx => {}
            _ => out.push(c),
        }
    }
    out
}

fn ignored(path: &str, ignore: &[String]) -> bool {
    let w = wild(path);
    ignore.iter().any(|i| {
        let i = i.trim();
        [path, w.as_str()].iter().any(|p| *p == i || p.starts_with(&format!("{i}.")) || p.starts_with(&format!("{i}[")))
    })
}

/// Compara la respuesta del flujo (lo esperado) con la del servicio global real.
pub fn compare(cfg: &Compare, flow_status: Option<u16>, flow_body: &Value, real_status: Option<u16>, real_body: &Value) -> CompareResult {
    let mut diffs = vec![];
    if flow_status.is_some() && flow_status != real_status && !ignored("status", &cfg.ignore) {
        diffs.push(Diff { path: "status".into(), kind: "status".into(), expected: json!(flow_status), actual: json!(real_status) });
    }
    let leaves: Vec<(String, Value)> = match cfg.mode.as_str() {
        "fields" => cfg.fields.iter().map(|f| (f.clone(), get_path(flow_body, f).cloned().unwrap_or(Value::Null))).collect(),
        _ => leaf_paths(flow_body),
    };
    for (p, exp) in leaves {
        if p.is_empty() && cfg.mode != "fields" && flow_body.is_null() {
            continue;
        }
        if ignored(&p, &cfg.ignore) {
            continue;
        }
        match if p.is_empty() { Some(real_body) } else { get_path(real_body, &p) } {
            None => diffs.push(Diff { path: p, kind: "falta".into(), expected: exp, actual: Value::Null }),
            Some(a) if *a != exp && !(exp.is_string() && value_to_text(a) == value_to_text(&exp)) => {
                diffs.push(Diff { path: p, kind: "distinto".into(), expected: exp, actual: a.clone() })
            }
            _ => {}
        }
    }
    if cfg.mode == "strict" {
        for (p, a) in leaf_paths(real_body) {
            if !ignored(&p, &cfg.ignore) && get_path(flow_body, &p).is_none() {
                diffs.push(Diff { path: p, kind: "sobra".into(), expected: Value::Null, actual: a });
            }
        }
    }
    CompareResult { ok: diffs.is_empty(), mode: cfg.mode.clone(), diffs }
}

// ---------- grafo ----------

/// ¿Se puede llegar de `from` a `to` siguiendo conexiones?
pub fn reaches(flow: &Flow, from: &str, to: &str) -> bool {
    let mut seen = HashSet::new();
    let mut q = VecDeque::from([from.to_string()]);
    while let Some(n) = q.pop_front() {
        for e in flow.outgoing(&n) {
            if e.to == to {
                return true;
            }
            if seen.insert(e.to.clone()) {
                q.push_back(e.to.clone());
            }
        }
    }
    false
}

/// Nodos anteriores a `id` ordenados por distancia (más cercano primero). Las entradas
/// (Data) siempre aparecen porque son globales.
pub fn upstream_by_distance(flow: &Flow, id: &str) -> Vec<(String, usize)> {
    let mut out = vec![];
    let mut seen = HashSet::from([id.to_string()]);
    let mut q = VecDeque::from([(id.to_string(), 0usize)]);
    while let Some((n, d)) = q.pop_front() {
        for e in flow.incoming(&n) {
            if seen.insert(e.from.clone()) {
                out.push((e.from.clone(), d + 1));
                q.push_back((e.from.clone(), d + 1));
            }
        }
    }
    for n in flow.nodes.iter().filter(|n| n.kind == "input" && !seen.contains(&n.id)) {
        out.push((n.id.clone(), usize::MAX));
    }
    out.retain(|(i, _)| flow.node(i).is_some_and(|n| !matches!(n.kind.as_str(), "start" | "end")));
    out
}

pub fn validate(flow: &Flow) -> Vec<String> {
    let mut errs = vec![];
    let mut ids = HashSet::new();
    let mut keys = HashSet::new();
    for n in &flow.nodes {
        if n.id.is_empty() {
            errs.push(format!("Hay un nodo «{}» sin id", n.name));
        }
        if !ids.insert(&n.id) {
            errs.push(format!("id repetido: {}", n.id));
        }
        if !NODE_TYPES.contains(&n.kind.as_str()) {
            errs.push(format!("{}: tipo desconocido «{}»", n.id, n.kind));
        }
        if !is_annotation(&n.kind) && !keys.insert(&n.key) {
            errs.push(format!("clave repetida: {}", n.key));
        }
        if !is_annotation(&n.kind) && n.key.is_empty() {
            errs.push(format!("{}: falta la clave", n.id));
        }
    }
    if flow.nodes.iter().filter(|n| n.kind == "start").count() > 1 {
        errs.push("Solo puede haber un nodo Inicio".into());
    }
    for e in &flow.edges {
        match (flow.node(&e.from), flow.node(&e.to)) {
            (Some(f), Some(_)) if !ports(&f.kind).contains(&e.port.as_str()) => {
                errs.push(format!("{} no tiene el puerto «{}»", e.from, e.port))
            }
            (None, _) | (_, None) => errs.push(format!("conexión rota {} → {}", e.from, e.to)),
            _ => {}
        }
    }
    errs
}

// ---------- planificador ----------

/// Decide qué nodos se ejecutan y cuándo. Lo usan el motor del servidor (todo de corrido)
/// y el modo «Paso a paso» del navegador (una oleada por F10).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Scheduler {
    pub ready: Vec<String>,
    pub running: BTreeSet<String>,
    pub waiting: BTreeSet<String>,
    pub done: BTreeSet<String>,
    pub steps: usize,
}

impl Scheduler {
    pub fn new(flow: &Flow) -> Self {
        let start: Vec<String> = match flow.nodes.iter().find(|n| n.kind == "start") {
            Some(s) => vec![s.id.clone()],
            None => flow.nodes.iter().filter(|n| n.kind == "input").map(|n| n.id.clone()).collect(),
        };
        Scheduler { ready: start, ..Default::default() }
    }

    /// Siguiente oleada: pasa de `ready` a `running`.
    pub fn take_ready(&mut self) -> Vec<String> {
        let wave: Vec<String> = std::mem::take(&mut self.ready);
        self.steps += wave.len();
        self.running.extend(wave.iter().cloned());
        wave
    }

    pub fn finished(&self) -> bool {
        self.ready.is_empty() && self.running.is_empty() && self.waiting.is_empty()
    }

    pub fn over_limit(&self) -> bool {
        self.steps > MAX_STEPS
    }

    /// Un nodo terminó y sale por `port` (None = rama cortada).
    pub fn complete(&mut self, flow: &Flow, id: &str, port: Option<&str>) {
        self.running.remove(id);
        self.done.insert(id.to_string());
        if let Some(p) = port {
            for e in flow.outgoing(id).filter(|e| e.port == p) {
                if self.done.contains(&e.to) || self.running.contains(&e.to) || self.ready.contains(&e.to) {
                    continue;
                }
                if flow.incoming(&e.to).count() > 1 {
                    self.waiting.insert(e.to.clone());
                } else {
                    self.ready.push(e.to.clone());
                }
            }
        }
        self.release_joins(flow);
    }

    /// Una unión espera hasta que ninguna rama en curso pueda llegar a ella.
    fn release_joins(&mut self, flow: &Flow) {
        loop {
            let active: Vec<&String> = self.running.iter().chain(self.ready.iter()).collect();
            let free = self.waiting.iter().find(|w| !active.iter().any(|a| a != w && reaches(flow, a, w))).cloned();
            match free {
                Some(w) => {
                    self.waiting.remove(&w);
                    self.ready.push(w);
                }
                None => break,
            }
        }
    }

    /// Detiene todo (p. ej. la primera Respuesta gana).
    pub fn stop(&mut self) {
        self.ready.clear();
        self.waiting.clear();
    }
}

/// Puerto por el que sale un servicio según su status.
pub fn service_port(status: Option<u16>) -> &'static str {
    match status {
        Some(s) if s < 400 => "out",
        _ => "error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, kind: &str) -> Node {
        Node { id: id.into(), key: id.into(), kind: kind.into(), name: id.into(), ..Default::default() }
    }
    fn edge(f: &str, p: &str, t: &str) -> Edge {
        Edge { from: f.into(), port: p.into(), to: t.into() }
    }

    #[test]
    fn refs_resolve_against_nodes_and_vars() {
        let mut flow = Flow::default();
        let mut inp = node("entrada", "input");
        inp.data = json!({"doc": "123", "tipo": "DNI"});
        flow.nodes = vec![inp, node("svc", "request"), node("t", "transform")];
        let mut outs = HashMap::new();
        outs.insert(
            "svc".into(),
            NodeOut { input: json!({"q": 1}), response: json!({"Lista": [{"Campo": "x"}]}), status: Some(200), headers: json!({"X-Id": "h"}), ..Default::default() },
        );
        outs.insert("t".into(), NodeOut { value: json!({"status": 200, "body": {"ok": true}}), ..Default::default() });
        let vars: Map<String, Value> = [("token".to_string(), json!("abc"))].into_iter().collect();
        let ctx = RefCtx { flow: &flow, outs: &outs, vars: &vars };
        assert_eq!(ctx.lookup("entrada.doc"), Some(json!("123")));
        assert_eq!(ctx.lookup("input.tipo"), Some(json!("DNI")));
        assert_eq!(ctx.lookup("svc.response.Lista[0].Campo"), Some(json!("x")));
        assert_eq!(ctx.lookup("svc.body.Lista[0].Campo"), Some(json!("x")));
        assert_eq!(ctx.lookup("svc.status"), Some(json!(200)));
        assert_eq!(ctx.lookup("svc.headers.x-id"), Some(json!("h")));
        assert_eq!(ctx.lookup("svc.input.q"), Some(json!(1)));
        assert_eq!(ctx.lookup("t.body.ok"), Some(json!(true)));
        assert_eq!(ctx.lookup("token"), Some(json!("abc")));
        let none = |_: &str| None;
        assert_eq!(ctx.render("{{entrada.doc}}-{{nope}}", &none), "123-{{nope}}");
        assert_eq!(ctx.value("{{svc.status}}", &none, false), json!(200));
        assert_eq!(ctx.value("svc.status", &none, true), json!(200));
        assert_eq!(ctx.render_json(&json!({"a": "{{t.body}}", "b": "n={{svc.status}}"}), &none), json!({"a": {"ok": true}, "b": "n=200"}));
    }

    #[test]
    fn modernize_rewrites_old_syntax() {
        let mut flow = Flow::default();
        let mut inp = node("in1", "input");
        inp.key = "entrada".into();
        let mut svc = node("svc", "request");
        svc.body_overrides = vec![Pair::new("doc", "{{input.doc}}"), Pair::new("x", "{{nodes.svc.body.id}}")];
        svc.key = "svc".into();
        flow.nodes = vec![inp, svc];
        assert_eq!(modernize_refs(&mut flow), 2);
        assert_eq!(flow.nodes[1].body_overrides[0].value, "{{entrada.doc}}");
        assert_eq!(flow.nodes[1].body_overrides[1].value, "{{svc.response.id}}");
        assert_eq!(flow.nodes[1].id, "svc");
        assert_eq!(modernize_refs(&mut flow), 0);
    }

    #[test]
    fn ops() {
        assert!(check(Some(&json!(200)), "eq", "200"));
        assert!(check(Some(&json!("200")), "eq", "200"));
        assert!(!check(Some(&json!("abc")), "eq", "abd"));
        assert!(check(Some(&json!("abc")), "eq", "\"abc\""));
        assert!(check(Some(&json!(5)), "gt", "3") && check(Some(&json!(5)), "lte", "5"));
        assert!(check(Some(&json!([1, 2])), "contains", "2"));
        assert!(check(None, "isUndefined", "") && !check(None, "isDefined", ""));
        assert!(check(Some(&json!("DNI")), "in", "DNI,CE"));
        assert!(check(Some(&json!([])), "isEmpty", ""));
        assert_eq!(split_assert("eq 201"), ("eq".into(), "201".into()));
        assert_eq!(split_assert("201"), ("eq".into(), "201".into()));
    }

    #[test]
    fn compare_modes() {
        let flow_body = json!({"id": 1, "nombre": "Ana", "fecha": "hoy"});
        let real = json!({"id": 1, "nombre": "Ana", "fecha": "ayer", "extra": true});
        let mut cfg = Compare { ignore: vec!["fecha".into()], ..Default::default() };
        assert!(compare(&cfg, Some(200), &flow_body, Some(200), &real).ok);
        cfg.mode = "strict".into();
        let r = compare(&cfg, Some(200), &flow_body, Some(201), &real);
        assert_eq!(r.diffs.iter().map(|d| d.kind.as_str()).collect::<Vec<_>>(), ["status", "sobra"]);
        cfg.mode = "fields".into();
        cfg.fields = vec!["nombre".into()];
        cfg.ignore.clear();
        assert!(compare(&cfg, None, &flow_body, Some(500), &real).ok);
        let list = Compare { ignore: vec!["l[*].t".into()], ..Default::default() };
        assert!(compare(&list, None, &json!({"l": [{"t": 1, "v": 2}]}), None, &json!({"l": [{"t": 9, "v": 2}]})).ok);
    }

    #[test]
    fn scheduler_parallel_and_join() {
        // start → a, b (paralelo) ; a → j ; b → c → j (unión)
        let mut flow = Flow::default();
        flow.nodes = ["start", "a", "b", "c", "j"].iter().map(|i| node(i, if *i == "start" { "start" } else { "data" })).collect();
        flow.edges = vec![edge("start", "out", "a"), edge("start", "out", "b"), edge("a", "out", "j"), edge("b", "out", "c"), edge("c", "out", "j")];
        let mut s = Scheduler::new(&flow);
        assert_eq!(s.take_ready(), ["start"]);
        s.complete(&flow, "start", Some("out"));
        assert_eq!(s.take_ready(), ["a", "b"]);
        s.complete(&flow, "a", Some("out"));
        assert!(s.ready.is_empty() && s.waiting.contains("j"), "j espera a la rama de b");
        s.complete(&flow, "b", Some("out"));
        assert_eq!(s.take_ready(), ["c"]);
        s.complete(&flow, "c", Some("out"));
        assert_eq!(s.take_ready(), ["j"]);
        s.complete(&flow, "j", Some("out"));
        assert!(s.finished());
    }

    #[test]
    fn upstream_and_validation() {
        let mut flow = Flow::default();
        flow.nodes = vec![node("in", "input"), node("a", "request"), node("b", "extract"), node("c", "response")];
        flow.edges = vec![edge("a", "out", "b"), edge("b", "out", "c")];
        let up: Vec<_> = upstream_by_distance(&flow, "c").into_iter().map(|x| x.0).collect();
        assert_eq!(up, ["b", "a", "in"]);
        assert!(validate(&flow).is_empty());
        flow.edges.push(edge("b", "true", "x"));
        assert_eq!(validate(&flow).len(), 1);
        assert_eq!(unique_key("Consulta Cliente", &["consulta_cliente".into()]), "consulta_cliente_2");
        assert_eq!(unique_key("D0-003", &[]), "d0-003");
    }
}
