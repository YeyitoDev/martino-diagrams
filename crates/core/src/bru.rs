//! Parser/serializador del formato Bruno (.bru). Los bloques que no conocemos se guardan
//! tal cual en `extra`, así que leer y volver a escribir un archivo no pierde nada.
use crate::model::{Environment, Pair, RawBlock, Request};

pub const METHODS: [&str; 7] = ["get", "post", "put", "patch", "delete", "options", "head"];

/// Bloques en el orden en que se leen; el cuerpo ya viene sin la sangría de 2 espacios.
pub fn parse_blocks(src: &str) -> Vec<RawBlock> {
    let mut out = vec![];
    let mut lines = src.lines();
    while let Some(line) = lines.next() {
        let t = line.trim_end();
        let open = if t.ends_with(" {") { '{' } else if t.ends_with(" [") { '[' } else { continue };
        let name = t[..t.len() - 2].trim().to_string();
        let close = if open == '{' { "}" } else { "]" };
        let mut body = vec![];
        for l in lines.by_ref() {
            if l.trim_end() == close {
                break;
            }
            body.push(l.strip_prefix("  ").unwrap_or(l.trim_start_matches(' ')));
        }
        out.push(RawBlock { name, open, body: body.join("\n") });
    }
    out
}

pub fn parse_dict(body: &str) -> Vec<Pair> {
    let mut out = vec![];
    let mut lines = body.lines();
    while let Some(line) = lines.next() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        let (enabled, l) = match l.strip_prefix('~') {
            Some(r) => (false, r),
            None => (true, l),
        };
        let (k, v) = match l.split_once(':') {
            Some((k, v)) => (k.trim(), v.strip_prefix(' ').unwrap_or(v).trim_end()),
            None => (l, ""),
        };
        let value = if v == "'''" {
            let mut ml = vec![];
            for x in lines.by_ref() {
                if x.trim() == "'''" {
                    break;
                }
                ml.push(x.strip_prefix("  ").unwrap_or(x));
            }
            ml.join("\n")
        } else {
            v.to_string()
        };
        out.push(Pair { name: k.to_string(), value, enabled });
    }
    out
}

fn parse_list(body: &str) -> Vec<String> {
    body.split([',', '\n']).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

fn indent(s: &str) -> String {
    s.lines().map(|l| if l.is_empty() { String::new() } else { format!("  {l}") }).collect::<Vec<_>>().join("\n")
}

fn dict_block(name: &str, pairs: &[Pair]) -> String {
    let mut s = format!("{name} {{\n");
    for p in pairs {
        let pre = if p.enabled { "" } else { "~" };
        if p.value.contains('\n') {
            s.push_str(&format!("  {pre}{}: '''\n{}\n  '''\n", p.name, indent(&indent(&p.value))));
        } else {
            s.push_str(&format!("  {pre}{}: {}\n", p.name, p.value));
        }
    }
    s.push_str("}\n");
    s
}

fn text_block(name: &str, text: &str) -> String {
    format!("{name} {{\n{}\n}}\n", indent(text))
}

fn raw_block(b: &RawBlock) -> String {
    let close = if b.open == '[' { ']' } else { '}' };
    format!("{} {}\n{}\n{close}\n", b.name, b.open, indent(&b.body))
}

fn get<'a>(pairs: &'a [Pair], k: &str) -> &'a str {
    pairs.iter().find(|p| p.name == k).map(|p| p.value.as_str()).unwrap_or("")
}

pub fn parse_request(src: &str) -> Request {
    let mut r = Request::default();
    for b in parse_blocks(src) {
        let name = b.name.as_str();
        match name {
            "meta" => {
                let d = parse_dict(&b.body);
                r.name = get(&d, "name").into();
                r.id = get(&d, "id").into();
                let k = get(&d, "type");
                if !k.is_empty() {
                    r.kind = k.into();
                }
                r.seq = get(&d, "seq").parse().ok();
            }
            m if METHODS.contains(&m) => {
                let d = parse_dict(&b.body);
                r.method = m.into();
                r.url = get(&d, "url").into();
                r.body_mode = non_empty(get(&d, "body"), "none");
                r.auth_mode = non_empty(get(&d, "auth"), "none");
            }
            "auth" => r.auth_mode = non_empty(get(&parse_dict(&b.body), "mode"), "none"),
            "params:query" => r.params = parse_dict(&b.body),
            "params:path" => r.path_params = parse_dict(&b.body),
            "headers" => r.headers = parse_dict(&b.body),
            "auth:bearer" => r.auth.token = get(&parse_dict(&b.body), "token").into(),
            "auth:basic" => {
                let d = parse_dict(&b.body);
                r.auth.username = get(&d, "username").into();
                r.auth.password = get(&d, "password").into();
            }
            "body:json" => r.body.json = b.body,
            "body:text" => r.body.text = b.body,
            "body:xml" => r.body.xml = b.body,
            "body:form-urlencoded" => r.body.form = parse_dict(&b.body),
            "body:multipart-form" => r.body.multipart = parse_dict(&b.body),
            "vars:pre-request" => r.vars_pre = parse_dict(&b.body),
            "vars:post-response" => r.vars_post = parse_dict(&b.body),
            "assert" => r.asserts = parse_dict(&b.body),
            "script:pre-request" => r.script_pre = b.body,
            "script:post-response" => r.script_post = b.body,
            "tests" => r.tests = b.body,
            "docs" => r.docs = b.body,
            _ => r.extra.push(b),
        }
    }
    r
}

fn non_empty(v: &str, d: &str) -> String {
    if v.is_empty() { d.into() } else { v.into() }
}

pub fn serialize_request(r: &Request) -> String {
    let mut parts = vec![];
    let mut meta = vec![Pair::new("name", &r.name)];
    if !r.id.is_empty() {
        meta.push(Pair::new("id", &r.id));
    }
    if !r.method.is_empty() {
        meta.push(Pair::new("type", &r.kind));
    }
    if let Some(s) = r.seq {
        meta.push(Pair::new("seq", fmt_num(s)));
    }
    if !r.name.is_empty() || r.seq.is_some() {
        parts.push(dict_block("meta", &meta));
    }
    if !r.method.is_empty() {
        let m = vec![Pair::new("url", &r.url), Pair::new("body", &r.body_mode), Pair::new("auth", &r.auth_mode)];
        parts.push(dict_block(&r.method.to_lowercase(), &m));
    } else if r.auth_mode != "none" && !r.auth_mode.is_empty() {
        parts.push(dict_block("auth", &[Pair::new("mode", &r.auth_mode)]));
    }
    let dicts: [(&str, &Vec<Pair>); 2] = [("params:query", &r.params), ("params:path", &r.path_params)];
    for (n, d) in dicts {
        if !d.is_empty() {
            parts.push(dict_block(n, d));
        }
    }
    if !r.headers.is_empty() {
        parts.push(dict_block("headers", &r.headers));
    }
    if !r.auth.token.is_empty() || r.auth_mode == "bearer" {
        parts.push(dict_block("auth:bearer", &[Pair::new("token", &r.auth.token)]));
    }
    if !r.auth.username.is_empty() || !r.auth.password.is_empty() || r.auth_mode == "basic" {
        parts.push(dict_block("auth:basic", &[Pair::new("username", &r.auth.username), Pair::new("password", &r.auth.password)]));
    }
    for (n, t) in [("body:json", &r.body.json), ("body:text", &r.body.text), ("body:xml", &r.body.xml)] {
        if !t.is_empty() {
            parts.push(text_block(n, t));
        }
    }
    let dicts: [(&str, &Vec<Pair>); 5] = [
        ("body:form-urlencoded", &r.body.form),
        ("body:multipart-form", &r.body.multipart),
        ("vars:pre-request", &r.vars_pre),
        ("vars:post-response", &r.vars_post),
        ("assert", &r.asserts),
    ];
    for (n, d) in dicts {
        if !d.is_empty() {
            parts.push(dict_block(n, d));
        }
    }
    for (n, t) in [("script:pre-request", &r.script_pre), ("script:post-response", &r.script_post), ("tests", &r.tests), ("docs", &r.docs)] {
        if !t.is_empty() {
            parts.push(text_block(n, t));
        }
    }
    parts.extend(r.extra.iter().map(raw_block));
    parts.join("\n")
}

fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 { format!("{}", n as i64) } else { n.to_string() }
}

pub fn parse_environment(name: &str, src: &str) -> Environment {
    let mut e = Environment { name: name.into(), ..Default::default() };
    for b in parse_blocks(src) {
        match b.name.as_str() {
            "vars" => e.vars = parse_dict(&b.body),
            "vars:secret" => e.secrets = parse_list(&b.body).into_iter().map(|n| Pair::new(n, "")).collect(),
            _ => e.extra.push(b),
        }
    }
    e
}

/// Los valores secretos NO se escriben: solo sus nombres.
pub fn serialize_environment(e: &Environment) -> String {
    let mut parts = vec![dict_block("vars", &e.vars)];
    if !e.secrets.is_empty() {
        let names: Vec<_> = e.secrets.iter().map(|p| format!("  {}", p.name)).collect();
        parts.push(format!("vars:secret [\n{}\n]\n", names.join(",\n")));
    }
    parts.extend(e.extra.iter().map(raw_block));
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQ: &str = "meta {
  name: Crear usuario
  id: USR-002
  type: http
  seq: 2
}

post {
  url: {{baseUrl}}/users
  body: json
  auth: bearer
}

params:query {
  page: 1
  ~debug: true
}

headers {
  Content-Type: application/json
}

auth:bearer {
  token: {{token}}
}

body:json {
  {
    \"name\": \"{{nombre}}\",
    \"edad\": {{edad}}
  }
}

vars:post-response {
  userId: res.body.id
}

assert {
  res.status: eq 201
}

docs {
  Crea un usuario.

  Segunda línea.
}

x-custom:block {
  algo raro: 1
  que no entendemos
}
";

    #[test]
    fn request_round_trip_is_lossless() {
        let r = parse_request(REQ);
        assert_eq!(r.name, "Crear usuario");
        assert_eq!(r.id, "USR-002");
        assert_eq!(r.seq, Some(2.0));
        assert_eq!(r.method, "post");
        assert_eq!(r.url, "{{baseUrl}}/users");
        assert_eq!(r.params[1], Pair { name: "debug".into(), value: "true".into(), enabled: false });
        assert!(r.body.json.contains("\"edad\": {{edad}}"));
        assert_eq!(r.vars_post[0].value, "res.body.id");
        assert_eq!(r.extra.len(), 1);
        assert_eq!(serialize_request(&r), REQ);
    }

    #[test]
    fn multiline_values_and_root_auth() {
        let mut r = Request::default();
        r.auth_mode = "bearer".into();
        r.headers.push(Pair::new("X-Multi", "a\nb"));
        let s = serialize_request(&r);
        assert!(s.contains("auth {\n  mode: bearer\n}"));
        let back = parse_request(&s);
        assert_eq!(back.headers[0].value, "a\nb");
        assert_eq!(back.auth_mode, "bearer");
    }

    #[test]
    fn environment_keeps_secret_names_only() {
        let src = "vars {\n  baseUrl: http://localhost\n}\n\nvars:secret [\n  token,\n  apiKey\n]\n";
        let mut e = parse_environment("dev", src);
        assert_eq!(e.secrets.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["token", "apiKey"]);
        e.secrets[0].value = "SECRETO".into();
        let out = serialize_environment(&e);
        assert!(!out.contains("SECRETO"));
        assert_eq!(out, src);
    }
}
