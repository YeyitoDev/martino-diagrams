//! Peticiones en YAML con el formato OpenCollection de Bruno v3 (`.yml`): es el formato de las
//! peticiones nuevas. Los `.bru` se siguen leyendo y, si ya existían, se guardan como `.bru`.
use crate::bru;
use crate::model::{Pair, RawBlock, Request};
use serde::{Deserialize, Deserializer, Serialize};
use serde_yaml::{Number, Value};

/// Extensión de las peticiones nuevas.
pub const EXT: &str = ".yml";
const EXTS: [&str; 3] = [".yml", ".yaml", ".bru"];

/// `Usuarios/crear.yml` → `Some("Usuarios/crear")`; `None` si no es un archivo de petición.
pub fn strip_ext(path: &str) -> Option<&str> {
    EXTS.iter().find_map(|e| path.strip_suffix(e))
}

pub fn is_request_file(path: &str) -> bool {
    strip_ext(path).is_some()
}

/// Lee una petición según la extensión de `path`.
pub fn parse_file(path: &str, src: &str) -> Result<Request, String> {
    if path.ends_with(".bru") { Ok(bru::parse_request(src)) } else { parse_request(src) }
}

/// Escribe una petición según la extensión de `path` (un `.bru` sigue siendo `.bru`).
pub fn serialize_file(path: &str, r: &Request) -> String {
    if path.ends_with(".bru") { bru::serialize_request(r) } else { serialize_request(r) }
}

/// Acepta cualquier escalar (`value: 10` escrito a mano) como texto.
fn text<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) => s,
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        v => serde_yaml::to_string(&v).unwrap_or_default().trim_end().to_string(),
    })
}

fn not(b: &bool) -> bool {
    !*b
}

#[derive(Serialize, Deserialize, Default)]
struct Doc {
    #[serde(default)]
    info: Info,
    #[serde(default)]
    http: Http,
    #[serde(default, skip_serializing_if = "Runtime::is_empty")]
    runtime: Runtime,
    #[serde(default, skip_serializing_if = "String::is_empty", deserialize_with = "text")]
    docs: String,
    /// Bloques `.bru` desconocidos (de un `.bru` importado): OpenCollection no tiene dónde ponerlos.
    #[serde(default, rename = "x-bru", skip_serializing_if = "Vec::is_empty")]
    extra: Vec<RawBlock>,
}

#[derive(Serialize, Deserialize, Default)]
struct Info {
    #[serde(default, deserialize_with = "text")]
    name: String,
    /// Martino: código de la petición (`USU-001`); OpenCollection no lo tiene.
    #[serde(default, skip_serializing_if = "String::is_empty", deserialize_with = "text")]
    id: String,
    #[serde(default, rename = "type", deserialize_with = "text")]
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    seq: Option<Number>,
}

#[derive(Serialize, Deserialize, Default)]
struct Http {
    #[serde(default, deserialize_with = "text")]
    method: String,
    #[serde(default, deserialize_with = "text")]
    url: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    params: Vec<Item>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    headers: Vec<Item>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    body: Option<Body>,
    /// `inherit` o `{type: bearer, token}` / `{type: basic, username, password}`; sin auth no va.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth: Option<Value>,
}

/// Fila nombre/valor (params, headers, variables, form). `type` sólo en params y multipart.
#[derive(Serialize, Deserialize, Default)]
struct Item {
    #[serde(default, deserialize_with = "text")]
    name: String,
    #[serde(default, deserialize_with = "text")]
    value: String,
    #[serde(default, rename = "type", skip_serializing_if = "String::is_empty", deserialize_with = "text")]
    kind: String,
    #[serde(default, skip_serializing_if = "not")]
    disabled: bool,
}

#[derive(Serialize, Deserialize)]
struct Body {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    data: Data,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum Data {
    Text(String),
    Rows(Vec<Item>),
}

impl Default for Data {
    fn default() -> Self {
        Data::Text(String::new())
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Runtime {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    variables: Vec<Item>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    scripts: Vec<Script>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    assertions: Vec<Assertion>,
    /// vars:post-response → acciones `set-variable` después de la respuesta.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    actions: Vec<Action>,
}

impl Runtime {
    fn is_empty(&self) -> bool {
        self.variables.is_empty() && self.scripts.is_empty() && self.assertions.is_empty() && self.actions.is_empty()
    }
}

#[derive(Serialize, Deserialize)]
struct Script {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default, deserialize_with = "text")]
    code: String,
}

#[derive(Serialize, Deserialize)]
struct Assertion {
    #[serde(default, deserialize_with = "text")]
    expression: String,
    #[serde(default, deserialize_with = "text")]
    operator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<Value>,
    #[serde(default, skip_serializing_if = "not")]
    disabled: bool,
}

#[derive(Serialize, Deserialize)]
struct Action {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    phase: String,
    #[serde(default)]
    selector: Selector,
    #[serde(default)]
    variable: Var,
    #[serde(default, skip_serializing_if = "not")]
    disabled: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct Selector {
    #[serde(default, deserialize_with = "text")]
    expression: String,
    #[serde(default)]
    method: String,
}

#[derive(Serialize, Deserialize, Default)]
struct Var {
    #[serde(default, deserialize_with = "text")]
    name: String,
    #[serde(default)]
    scope: String,
}

fn rows(pairs: &[Pair], kind: &str) -> Vec<Item> {
    pairs.iter().map(|p| Item { name: p.name.clone(), value: p.value.clone(), kind: kind.into(), disabled: !p.enabled }).collect()
}

fn pairs<'a>(items: impl IntoIterator<Item = &'a Item>) -> Vec<Pair> {
    items.into_iter().map(|i| Pair { name: i.name.clone(), value: i.value.clone(), enabled: !i.disabled }).collect()
}

fn auth_field(v: &Value, k: &str) -> String {
    v.get(k).map(|x| text(x.clone()).unwrap_or_default()).unwrap_or_default()
}

pub fn serialize_request(r: &Request) -> String {
    let seq = r.seq.map(|s| if s.fract() == 0.0 { Number::from(s as i64) } else { Number::from(s) });
    let info = Info { name: r.name.clone(), id: r.id.clone(), kind: r.kind.clone(), seq };
    let mut params = rows(&r.params, "query");
    params.extend(rows(&r.path_params, "path"));
    let body = match r.body_mode.as_str() {
        "json" => Some(Data::Text(r.body.json.clone())),
        "text" => Some(Data::Text(r.body.text.clone())),
        "xml" => Some(Data::Text(r.body.xml.clone())),
        "form-urlencoded" => Some(Data::Rows(rows(&r.body.form, ""))),
        "multipart-form" => Some(Data::Rows(rows(&r.body.multipart, "text"))),
        _ => None,
    }
    .map(|data| Body { kind: r.body_mode.clone(), data });
    let auth = match r.auth_mode.as_str() {
        "" | "none" => None,
        "bearer" => Some(serde_yaml::to_value(serde_json::json!({"type": "bearer", "token": r.auth.token})).unwrap()),
        "basic" => Some(serde_yaml::to_value(serde_json::json!({"type": "basic", "username": r.auth.username, "password": r.auth.password})).unwrap()),
        m => Some(Value::String(m.into())),
    };
    let http = Http { method: r.method.to_uppercase(), url: r.url.clone(), params, headers: rows(&r.headers, ""), body, auth };
    let scripts = [("before-request", &r.script_pre), ("after-response", &r.script_post), ("tests", &r.tests)]
        .into_iter()
        .filter(|x| !x.1.is_empty())
        .map(|(k, c)| Script { kind: k.into(), code: c.clone() })
        .collect();
    // Bruno guarda `res.status: eq 201` como operador + valor.
    let assertions = r
        .asserts
        .iter()
        .map(|p| {
            let (op, v) = p.value.split_once(' ').map_or((p.value.as_str(), None), |(o, v)| (o, Some(v)));
            Assertion { expression: p.name.clone(), operator: op.into(), value: v.map(|v| Value::String(v.into())), disabled: !p.enabled }
        })
        .collect();
    let actions = r
        .vars_post
        .iter()
        .map(|p| Action {
            kind: "set-variable".into(),
            phase: "after-response".into(),
            selector: Selector { expression: p.value.clone(), method: "jsonq".into() },
            variable: Var { name: p.name.clone(), scope: "runtime".into() },
            disabled: !p.enabled,
        })
        .collect();
    let runtime = Runtime { variables: rows(&r.vars_pre, ""), scripts, assertions, actions };
    let doc = Doc { info, http, runtime, docs: r.docs.clone(), extra: r.extra.clone() };
    serde_yaml::to_string(&doc).unwrap_or_default()
}

pub fn parse_request(src: &str) -> Result<Request, String> {
    let d: Doc = serde_yaml::from_str(src).map_err(|e| format!("YAML inválido: {e}"))?;
    let mut r = Request {
        name: d.info.name,
        id: d.info.id,
        seq: d.info.seq.and_then(|n| n.as_f64()),
        method: d.http.method.to_lowercase(),
        url: d.http.url,
        params: pairs(d.http.params.iter().filter(|p| p.kind != "path")),
        path_params: pairs(d.http.params.iter().filter(|p| p.kind == "path")),
        headers: pairs(&d.http.headers),
        vars_pre: pairs(&d.runtime.variables),
        docs: d.docs,
        extra: d.extra,
        ..Default::default()
    };
    if !d.info.kind.is_empty() {
        r.kind = d.info.kind;
    }
    if let Some(b) = d.http.body {
        match (b.kind.as_str(), b.data) {
            ("json", Data::Text(t)) => r.body.json = t,
            ("text", Data::Text(t)) => r.body.text = t,
            ("xml", Data::Text(t)) => r.body.xml = t,
            ("form-urlencoded", Data::Rows(x)) => r.body.form = pairs(&x),
            ("multipart-form", Data::Rows(x)) => r.body.multipart = pairs(&x),
            _ => {}
        }
        r.body_mode = b.kind;
    }
    match &d.http.auth {
        None => {}
        Some(Value::String(m)) => r.auth_mode = m.clone(),
        Some(a) => {
            r.auth_mode = auth_field(a, "type");
            r.auth.token = auth_field(a, "token");
            r.auth.username = auth_field(a, "username");
            r.auth.password = auth_field(a, "password");
        }
    }
    for s in d.runtime.scripts {
        match s.kind.as_str() {
            "before-request" => r.script_pre = s.code,
            "after-response" => r.script_post = s.code,
            "tests" => r.tests = s.code,
            _ => {}
        }
    }
    r.asserts = d
        .runtime
        .assertions
        .into_iter()
        .map(|a| {
            let v = a.value.map(|v| text(v).unwrap_or_default());
            Pair { name: a.expression, value: v.map_or(a.operator.clone(), |v| format!("{} {v}", a.operator)), enabled: !a.disabled }
        })
        .collect();
    r.vars_post = d
        .runtime
        .actions
        .into_iter()
        .filter(|a| a.kind == "set-variable" && a.phase == "after-response")
        .map(|a| Pair { name: a.variable.name, value: a.selector.expression, enabled: !a.disabled })
        .collect();
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trip() {
        let mut r = Request::new("post", "{{baseUrl}}/users/{{id}}");
        r.name = "Crear usuario".into();
        r.id = "USU-002".into();
        r.seq = Some(2.0);
        r.params = vec![Pair::new("page", "1"), Pair { name: "debug".into(), value: "true".into(), enabled: false }];
        r.path_params = vec![Pair::new("id", "7")];
        r.headers = vec![Pair::new("Content-Type", "application/json"), Pair::new("X-Multi", "a\nb")];
        r.body_mode = "json".into();
        r.body.json = "{\n  \"name\": \"{{nombre}}\",\n  \"edad\": {{edad}}\n}".into();
        r.auth_mode = "bearer".into();
        r.auth.token = "{{token}}".into();
        r.vars_pre = vec![Pair::new("nombre", "Ana")];
        r.vars_post = vec![Pair::new("userId", "res.body.id")];
        r.asserts = vec![Pair::new("res.status", "eq 201"), Pair::new("res.body", "isJson")];
        r.tests = "test(\"ok\", () => {});".into();
        r.docs = "Crea un usuario.\n\nSegunda línea.".into();
        let y = serialize_request(&r);
        assert!(y.contains("method: POST") && y.contains("id: USU-002") && y.contains("seq: 2\n"), "{y}");
        assert!(y.contains("operator: eq") && y.contains("type: set-variable"), "{y}");
        assert_eq!(parse_request(&y).unwrap(), r);
        assert_eq!(parse_file("a.yml", &serialize_file("a.yml", &r)).unwrap(), r);
    }

    #[test]
    fn reads_bruno_yaml_and_forms() {
        let src = "info:\n  name: Login\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: https://x/login\n  body:\n    type: form-urlencoded\n    data:\n      - name: user\n        value: ana\n      - name: n\n        value: 10\n        disabled: true\n  auth: inherit\n";
        let r = parse_request(src).unwrap();
        assert_eq!((r.method.as_str(), r.auth_mode.as_str(), r.body_mode.as_str()), ("post", "inherit", "form-urlencoded"));
        assert_eq!(r.body.form[1], Pair { name: "n".into(), value: "10".into(), enabled: false });
        assert_eq!(parse_request(&serialize_request(&r)).unwrap(), r);
        assert!(parse_request("meta {\n  name: x\n}\n\nget {\n  url: /a\n}\n").map_or(true, |r| r.method.is_empty()), "un .bru no pasa por YAML");
        assert_eq!(strip_ext("a/b.yaml"), Some("a/b"));
        assert!(!is_request_file("bruno.json"));
    }
}
