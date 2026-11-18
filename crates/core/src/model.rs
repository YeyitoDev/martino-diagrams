//! Modelo compartido entre servidor y navegador (se serializa igual en los dos lados).
use serde::{Deserialize, Serialize};

fn yes() -> bool {
    true
}
fn is_true(b: &bool) -> bool {
    *b
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Pair {
    pub name: String,
    #[serde(default)]
    pub value: String,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
}

impl Pair {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Pair { name: name.into(), value: value.into(), enabled: true }
    }
}

/// Bloque de un archivo .bru que no entendemos: se guarda tal cual para no perderlo.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RawBlock {
    pub name: String,
    /// `{` o `[`
    pub open: char,
    pub body: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Body {
    pub json: String,
    pub text: String,
    pub xml: String,
    pub form: Vec<Pair>,
    pub multipart: Vec<Pair>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Auth {
    pub token: String,
    pub username: String,
    pub password: String,
}

/// Una petición (`.yml` o `.bru`). También representa `collection.bru` y `folder.bru` (sin método).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Request {
    pub name: String,
    /// Código que identifica la petición (`USR-001`); se guarda en `meta.id`.
    pub id: String,
    pub kind: String,
    pub seq: Option<f64>,
    pub method: String,
    pub url: String,
    /// none | json | text | xml | form-urlencoded | multipart-form
    pub body_mode: String,
    /// none | inherit | bearer | basic
    pub auth_mode: String,
    pub body: Body,
    pub auth: Auth,
    pub params: Vec<Pair>,
    pub path_params: Vec<Pair>,
    pub headers: Vec<Pair>,
    pub vars_pre: Vec<Pair>,
    pub vars_post: Vec<Pair>,
    pub asserts: Vec<Pair>,
    pub script_pre: String,
    pub script_post: String,
    pub tests: String,
    pub docs: String,
    pub extra: Vec<RawBlock>,
}

impl Default for Request {
    fn default() -> Self {
        Request {
            name: String::new(),
            id: String::new(),
            kind: "http".into(),
            seq: None,
            method: String::new(),
            url: String::new(),
            body_mode: "none".into(),
            auth_mode: "none".into(),
            body: Body::default(),
            auth: Auth::default(),
            params: vec![],
            path_params: vec![],
            headers: vec![],
            vars_pre: vec![],
            vars_post: vec![],
            asserts: vec![],
            script_pre: String::new(),
            script_post: String::new(),
            tests: String::new(),
            docs: String::new(),
            extra: vec![],
        }
    }
}

impl Request {
    pub fn new(method: &str, url: &str) -> Self {
        Request { method: method.to_lowercase(), url: url.into(), ..Default::default() }
    }
    /// Texto del body según su modo (para enviar o mostrar).
    pub fn body_text(&self) -> Option<String> {
        match self.body_mode.as_str() {
            "json" => Some(self.body.json.clone()),
            "text" => Some(self.body.text.clone()),
            "xml" => Some(self.body.xml.clone()),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Environment {
    pub name: String,
    pub vars: Vec<Pair>,
    /// Nombres de las variables secretas; su valor vive en `.workbench/secrets.json`.
    pub secrets: Vec<Pair>,
    pub extra: Vec<RawBlock>,
}

/// Lo que se envió de verdad (pestaña "Enviada").
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SentRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<Pair>,
    pub body: Option<String>,
}

const SENSITIVE: [&str; 5] = ["authorization", "cookie", "x-api-key", "proxy-authorization", "set-cookie"];

pub fn is_sensitive_header(name: &str) -> bool {
    SENSITIVE.contains(&name.to_ascii_lowercase().as_str())
}

/// Petición en crudo, estilo HTTP/1.1. Con `mask` oculta las credenciales.
pub fn raw_http(req: &SentRequest, mask: bool) -> String {
    let (host, path) = split_url(&req.url);
    let mut out = format!("{} {} HTTP/1.1\nHost: {}\n", req.method.to_uppercase(), path, host);
    for h in &req.headers {
        let v = if mask && is_sensitive_header(&h.name) { mask_value(&h.value) } else { h.value.clone() };
        out.push_str(&format!("{}: {}\n", h.name, v));
    }
    if let Some(b) = &req.body {
        out.push('\n');
        out.push_str(b);
    }
    out
}

fn mask_value(v: &str) -> String {
    match v.split_once(' ') {
        Some((scheme, _)) if matches!(scheme, "Bearer" | "Basic" | "bearer" | "basic") => format!("{scheme} ••••••"),
        _ => "••••••".into(),
    }
}

/// `https://h:1/p?q` → (`h:1`, `/p?q`)
pub fn split_url(url: &str) -> (String, String) {
    let rest = url.split_once("://").map(|x| x.1).unwrap_or(url);
    match rest.find(['/', '?']) {
        Some(i) => {
            let p = &rest[i..];
            (rest[..i].into(), if p.starts_with('?') { format!("/{p}") } else { p.into() })
        }
        None => (rest.into(), "/".into()),
    }
}

/// Prefijo de ID para una carpeta: 3 letras en mayúscula (`Usuarios` → `USU`); sin carpeta, `REQ`.
pub fn id_prefix(folder: &str) -> String {
    let p: String = folder.rsplit('/').next().unwrap_or("").chars().filter_map(|c| match c {
        'á' | 'Á' => Some('A'), 'é' | 'É' => Some('E'), 'í' | 'Í' => Some('I'), 'ó' | 'Ó' => Some('O'), 'ú' | 'Ú' | 'ü' => Some('U'), 'ñ' | 'Ñ' => Some('N'),
        c if c.is_ascii_alphabetic() => Some(c.to_ascii_uppercase()),
        _ => None,
    }).take(3).collect();
    if p.is_empty() { "REQ".into() } else { p }
}

/// ID doble: respeta `wanted` (el que trae el agente o escribe la persona) si no está usado;
/// si no, el siguiente libre del prefijo (`USU-003`).
pub fn resolve_id(wanted: &str, prefix: &str, existing: &[String]) -> String {
    let w = wanted.trim();
    if !w.is_empty() && !existing.iter().any(|e| e.eq_ignore_ascii_case(w)) {
        return w.to_string();
    }
    let max = existing.iter().filter_map(|e| e.strip_prefix(prefix)?.strip_prefix('-')?.parse::<u32>().ok()).max().unwrap_or(0);
    format!("{prefix}-{:03}", max + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ids_are_dual() {
        let ex = vec!["USU-001".to_string(), "USU-004".into(), "PAG-014".into()];
        assert_eq!(id_prefix("Usuarios"), "USU");
        assert_eq!(id_prefix("demo/Pagos y cobros"), "PAG");
        assert_eq!(id_prefix("Órdenes"), "ORD");
        assert_eq!(id_prefix(""), "REQ");
        assert_eq!(resolve_id("PAG-020", "PAG", &ex), "PAG-020", "respeta el del agente");
        assert_eq!(resolve_id("pag-014", "PAG", &ex), "PAG-015", "ocupado: el siguiente libre");
        assert_eq!(resolve_id("", "USU", &ex), "USU-005");
        assert_eq!(resolve_id("", "ORD", &ex), "ORD-001");
    }

    #[test]
    fn raw_http_masks_credentials() {
        let r = SentRequest {
            method: "post".into(),
            url: "https://api.x.com/v1/a?b=1".into(),
            headers: vec![Pair::new("Authorization", "Bearer abc"), Pair::new("X-Api-Key", "k")],
            body: Some("{}".into()),
        };
        let m = raw_http(&r, true);
        assert!(m.starts_with("POST /v1/a?b=1 HTTP/1.1\nHost: api.x.com\n"));
        assert!(m.contains("Authorization: Bearer ••••••") && !m.contains("abc") && !m.contains(": k\n"));
        assert!(raw_http(&r, false).contains("Bearer abc"));
    }
}
