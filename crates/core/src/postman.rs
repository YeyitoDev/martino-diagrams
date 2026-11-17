//! Colecciones y entornos Postman v2.1 → modelo Bruno, y `remap_variables`.
use crate::interp::{segments, Seg};
use crate::model::{Environment, Pair, Request};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Item {
    pub name: String,
    pub folder: bool,
    /// Petición (o folder.bru si es carpeta).
    pub request: Request,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Converted {
    pub name: String,
    /// collection.bru (auth, cabeceras y variables comunes)
    pub root: Request,
    pub items: Vec<Item>,
    pub environments: Vec<Environment>,
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn kv_list(v: Option<&Value>) -> Vec<Pair> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|x| x.get("key").is_some())
                .map(|x| Pair {
                    name: s(x, "key"),
                    value: x.get("value").map(|v| v.as_str().map(String::from).unwrap_or_else(|| v.to_string())).unwrap_or_default(),
                    enabled: !x.get("disabled").and_then(Value::as_bool).unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn apply_auth(r: &mut Request, auth: Option<&Value>) {
    let Some(a) = auth else { return };
    let kind = s(a, "type");
    let field = |k: &str| kv_list(a.get(&kind)).into_iter().find(|p| p.name == k).map(|p| p.value).unwrap_or_default();
    match kind.as_str() {
        "bearer" => {
            r.auth_mode = "bearer".into();
            r.auth.token = field("token");
        }
        "basic" => {
            r.auth_mode = "basic".into();
            r.auth.username = field("username");
            r.auth.password = field("password");
        }
        "noauth" => r.auth_mode = "none".into(),
        _ => {}
    }
}

fn convert_request(name: &str, req: &Value, seq: usize) -> Request {
    let mut r = Request { name: name.into(), seq: Some(seq as f64), method: s(req, "method").to_lowercase(), ..Default::default() };
    if r.method.is_empty() {
        r.method = "get".into();
    }
    match req.get("url") {
        Some(Value::String(u)) => r.url = u.clone(),
        Some(u) => {
            r.url = s(u, "raw").split('?').next().unwrap_or_default().to_string();
            r.params = kv_list(u.get("query"));
        }
        None => {}
    }
    if r.params.is_empty() {
        crate::curl::split_query(&mut r);
    }
    r.headers = kv_list(req.get("header"));
    r.auth_mode = if req.get("auth").is_some() { "none".into() } else { "inherit".into() };
    apply_auth(&mut r, req.get("auth"));
    if let Some(b) = req.get("body") {
        match s(b, "mode").as_str() {
            "raw" => {
                let raw = s(b, "raw");
                let lang = b.pointer("/options/raw/language").and_then(Value::as_str).unwrap_or("");
                let t = raw.trim_start();
                if lang == "json" || t.starts_with('{') || t.starts_with('[') {
                    r.body_mode = "json".into();
                    r.body.json = raw;
                } else if lang == "xml" {
                    r.body_mode = "xml".into();
                    r.body.xml = raw;
                } else {
                    r.body_mode = "text".into();
                    r.body.text = raw;
                }
            }
            "urlencoded" => {
                r.body_mode = "form-urlencoded".into();
                r.body.form = kv_list(b.get("urlencoded"));
            }
            "formdata" => {
                r.body_mode = "multipart-form".into();
                r.body.multipart = kv_list(b.get("formdata"));
            }
            _ => {}
        }
    }
    if let Some(d) = req.get("description").and_then(Value::as_str) {
        r.docs = d.into();
    }
    r
}

fn convert_items(items: &[Value]) -> Vec<Item> {
    items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            let name = s(it, "name");
            match it.get("item").and_then(Value::as_array) {
                Some(children) => {
                    let mut root = Request { name: name.clone(), seq: Some(i as f64 + 1.0), ..Default::default() };
                    apply_auth(&mut root, it.get("auth"));
                    Item { name, folder: true, request: root, items: convert_items(children) }
                }
                None => Item { request: convert_request(&name, it.get("request").unwrap_or(&Value::Null), i + 1), name, ..Default::default() },
            }
        })
        .collect()
}

pub fn convert_environment(v: &Value) -> Environment {
    let mut e = Environment { name: s(v, "name"), ..Default::default() };
    for x in v.get("values").and_then(Value::as_array).cloned().unwrap_or_default() {
        let p = Pair { name: s(&x, "key"), value: s(&x, "value"), enabled: x.get("enabled").and_then(Value::as_bool).unwrap_or(true) };
        if s(&x, "type") == "secret" {
            e.secrets.push(p)
        } else {
            e.vars.push(p)
        }
    }
    e
}

/// Acepta una colección o un entorno Postman (detecta cuál es).
pub fn convert(v: &Value) -> Result<Converted, String> {
    if v.get("values").is_some() && v.get("item").is_none() {
        let env = convert_environment(v);
        return Ok(Converted { name: env.name.clone(), environments: vec![env], ..Default::default() });
    }
    let items = v.get("item").and_then(Value::as_array).ok_or("No parece una colección Postman v2.1 (falta `item`)")?;
    let name = v.pointer("/info/name").and_then(Value::as_str).unwrap_or("Importada").to_string();
    let mut root = Request { name: name.clone(), ..Default::default() };
    apply_auth(&mut root, v.get("auth"));
    root.vars_pre = kv_list(v.get("variable"));
    Ok(Converted { name, root, items: convert_items(items), environments: vec![] })
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Remap {
    /// `x_test` → `x`
    pub rename: Vec<Pair>,
    /// URL literal → variable (`https://dev.api.com` → `globalUrl`)
    pub urls: Vec<Pair>,
    /// Cabecera → variable (`x-client-id` → `clientId`)
    pub headers: Vec<Pair>,
}

fn rename_in(s: &str, rename: &[Pair]) -> String {
    segments(s)
        .into_iter()
        .map(|seg| match seg {
            Seg::Text(t) => t.to_string(),
            Seg::Var(n, raw) => match rename.iter().find(|p| p.name == n) {
                Some(p) => format!("{{{{{}}}}}", p.value),
                None => raw.to_string(),
            },
        })
        .collect()
}

fn remap_request(r: &mut Request, opts: &Remap) {
    let mut v = serde_json::to_value(&*r).unwrap();
    fn walk(v: &mut Value, f: &dyn Fn(&str) -> String) {
        match v {
            Value::String(s) => *s = f(s),
            Value::Array(a) => a.iter_mut().for_each(|x| walk(x, f)),
            Value::Object(m) => m.values_mut().for_each(|x| walk(x, f)),
            _ => {}
        }
    }
    walk(&mut v, &|s| rename_in(s, &opts.rename));
    *r = serde_json::from_value(v).unwrap();
    for u in &opts.urls {
        let lit = u.name.trim_end_matches('/');
        if !lit.is_empty() && r.url.starts_with(lit) {
            r.url = format!("{{{{{}}}}}{}", u.value, &r.url[lit.len()..]);
        }
    }
    for h in r.headers.iter_mut() {
        if let Some(m) = opts.headers.iter().find(|m| m.name.eq_ignore_ascii_case(&h.name)) {
            if !h.value.contains("{{") {
                h.value = if h.value.starts_with("Bearer ") { format!("Bearer {{{{{}}}}}", m.value) } else { format!("{{{{{}}}}}", m.value) };
            }
        }
    }
}

/// Renombra variables y sustituye URLs/cabeceras escritas a mano por variables.
pub fn remap_variables(c: &mut Converted, opts: &Remap) {
    fn items(list: &mut [Item], opts: &Remap) {
        for it in list {
            remap_request(&mut it.request, opts);
            items(&mut it.items, opts);
        }
    }
    remap_request(&mut c.root, opts);
    items(&mut c.items, opts);
    for e in &mut c.environments {
        for p in e.vars.iter_mut().chain(e.secrets.iter_mut()) {
            if let Some(r) = opts.rename.iter().find(|r| r.name == p.name) {
                p.name = r.value.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
          "info": {"name": "Clientes", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},
          "auth": {"type": "bearer", "bearer": [{"key": "token", "value": "{{token_test}}"}]},
          "variable": [{"key": "version", "value": "v1"}],
          "item": [
            {"name": "Consultas", "item": [
              {"name": "Buscar", "request": {"method": "GET",
                "header": [{"key": "x-client-id", "value": "ABC"}, {"key": "x-off", "value": "1", "disabled": true}],
                "url": {"raw": "https://dev.api.com/v1/clientes?doc={{doc_test}}", "query": [{"key": "doc", "value": "{{doc_test}}"}]}}}
            ]},
            {"name": "Crear", "request": {"method": "POST", "url": "https://dev.api.com/v1/clientes",
              "body": {"mode": "raw", "raw": "{\"a\":1}", "options": {"raw": {"language": "json"}}}}},
            {"name": "Form", "request": {"method": "POST", "url": "{{base}}/f",
              "body": {"mode": "urlencoded", "urlencoded": [{"key": "k", "value": "v"}]}, "auth": {"type": "noauth"}}}
          ]
        })
    }

    #[test]
    fn converts_collection() {
        let c = convert(&sample()).unwrap();
        assert_eq!(c.name, "Clientes");
        assert_eq!((c.root.auth_mode.as_str(), c.root.auth.token.as_str()), ("bearer", "{{token_test}}"));
        assert_eq!(c.root.vars_pre[0], Pair::new("version", "v1"));
        let f = &c.items[0];
        assert!(f.folder && f.items.len() == 1);
        let b = &f.items[0].request;
        assert_eq!((b.method.as_str(), b.url.as_str(), b.auth_mode.as_str()), ("get", "https://dev.api.com/v1/clientes", "inherit"));
        assert_eq!(b.params[0].value, "{{doc_test}}");
        assert!(!b.headers[1].enabled);
        assert_eq!(c.items[1].request.body_mode, "json");
        assert_eq!(c.items[2].request.body.form[0], Pair::new("k", "v"));
        assert_eq!(c.items[2].request.auth_mode, "none");
    }

    #[test]
    fn converts_environment_and_remaps() {
        let env = json!({"name": "dev", "values": [
            {"key": "token_test", "value": "T", "type": "secret", "enabled": true},
            {"key": "globalUrl", "value": "https://dev.api.com", "enabled": true}]});
        let mut c = convert(&sample()).unwrap();
        c.environments.push(convert(&env).unwrap().environments.remove(0));
        let opts = Remap {
            rename: vec![Pair::new("token_test", "token"), Pair::new("doc_test", "doc")],
            urls: vec![Pair::new("https://dev.api.com/", "globalUrl")],
            headers: vec![Pair::new("X-Client-Id", "clientId")],
        };
        remap_variables(&mut c, &opts);
        assert_eq!(c.root.auth.token, "{{token}}");
        let b = &c.items[0].items[0].request;
        assert_eq!(b.url, "{{globalUrl}}/v1/clientes");
        assert_eq!(b.params[0].value, "{{doc}}");
        assert_eq!(b.headers[0].value, "{{clientId}}");
        assert_eq!(c.environments[0].secrets[0].name, "token");
        assert_eq!(c.items[1].request.url, "{{globalUrl}}/v1/clientes");
    }
}
