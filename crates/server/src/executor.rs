//! Envío HTTP(S): interpolación, herencia de la colección, asserts y vars:post-response.
use crate::config::Settings;
use base64::Engine;
use martino_core::flow::{check, split_assert};
use martino_core::interp::{get_path, interpolate, set_path, var_names, DYNAMIC_VARS};
use martino_core::model::{Pair, Request, SentRequest};
use rand::Rng;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::RwLock;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

pub type Resolver<'a> = &'a (dyn Fn(&str) -> Option<String> + Sync);

#[derive(Serialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct AssertResult {
    pub name: String,
    pub op: String,
    pub expected: String,
    pub actual: Value,
    pub pass: bool,
}

#[derive(Serialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct RunResult {
    pub ok: bool,
    pub status: Option<u16>,
    pub status_text: String,
    pub headers: Vec<Pair>,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json: Option<Value>,
    pub time_ms: u64,
    pub size: usize,
    pub sent: SentRequest,
    pub asserts: Vec<AssertResult>,
    /// vars:post-response ya evaluadas (pasan a runtime).
    pub vars: Vec<Pair>,
    /// Variables que quedaron sin resolver.
    pub missing: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl RunResult {
    pub fn headers_json(&self) -> Value {
        Value::Object(self.headers.iter().map(|h| (h.name.clone(), json!(h.value))).collect())
    }
    pub fn body_value(&self) -> Value {
        self.json.clone().unwrap_or_else(|| if self.body.is_empty() { Value::Null } else { json!(self.body) })
    }
}

/// Cambios que aplica un nodo de flujo encima de la petición.
#[derive(Default, Clone)]
pub struct Overrides {
    pub body: Vec<(String, Value)>,
    pub headers: Vec<Pair>,
    pub query: Vec<Pair>,
}

pub fn dynamic(name: &str) -> Option<String> {
    let mut r = rand::thread_rng();
    let alnum = |n: usize, r: &mut rand::rngs::ThreadRng| -> String {
        (0..n).map(|_| r.sample(rand::distributions::Alphanumeric) as char).collect()
    };
    Some(match name {
        "$guid" | "$randomUUID" => uuid::Uuid::new_v4().to_string(),
        "$timestamp" => chrono::Utc::now().timestamp().to_string(),
        "$isoTimestamp" => chrono::Utc::now().to_rfc3339(),
        "$randomInt" => r.gen_range(0..1000).to_string(),
        "$randomEmail" => format!("{}@example.com", alnum(8, &mut r).to_lowercase()),
        "$randomFirstName" => ["Ana", "Luis", "María", "Jorge", "Rosa", "Diego"][r.gen_range(0..6)].into(),
        "$randomLastName" => ["Quispe", "García", "Rojas", "Flores", "Torres", "Díaz"][r.gen_range(0..6)].into(),
        "$randomBoolean" => r.gen_bool(0.5).to_string(),
        "$randomAlphaNumeric" => alnum(1, &mut r),
        _ => return None,
    })
}

fn pct(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn form_encode(pairs: &[(String, String)]) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", pct(k), pct(v))).collect::<Vec<_>>().join("&")
}

/// Valor de `res.status`, `res.body.x`, `res.headers.x`, `res.responseTime`.
pub fn res_value(r: &RunResult, path: &str) -> Option<Value> {
    let p = path.trim().strip_prefix("res.").unwrap_or(path.trim());
    let (head, rest) = p.split_once('.').unwrap_or((p, ""));
    match head {
        "status" => r.status.map(|s| json!(s)),
        "responseTime" => Some(json!(r.time_ms)),
        "headers" => r.headers.iter().find(|h| h.name.eq_ignore_ascii_case(rest)).map(|h| json!(h.value)),
        _ => {
            let body = r.body_value();
            let path = p.strip_prefix("body").unwrap_or(p).trim_start_matches('.');
            if path.is_empty() { Some(body) } else { get_path(&body, path).cloned() }
        }
    }
}

pub struct Executor {
    client: RwLock<reqwest::Client>,
}

fn build_client(s: &Settings) -> Result<reqwest::Client, String> {
    let mut b = reqwest::Client::builder()
        .use_rustls_tls()
        .timeout(Duration::from_millis(s.request_timeout_ms.max(1)))
        .danger_accept_invalid_certs(!s.tls.reject_unauthorized)
        .redirect(reqwest::redirect::Policy::limited(10));
    if !s.tls.ca_file.trim().is_empty() {
        let pem = std::fs::read(s.tls.ca_file.trim()).map_err(|e| format!("No se pudo leer la CA {}: {e}", s.tls.ca_file))?;
        for c in reqwest::Certificate::from_pem_bundle(&pem).map_err(|e| format!("CA inválida: {e}"))? {
            b = b.add_root_certificate(c);
        }
    }
    b.build().map_err(|e| e.to_string())
}

impl Executor {
    pub fn new(s: &Settings) -> Executor {
        let client = build_client(s).unwrap_or_else(|e| {
            eprintln!("⚠ {e}; se usa TLS por defecto");
            build_client(&Settings { tls: Default::default(), ..s.clone() }).unwrap()
        });
        Executor { client: RwLock::new(client) }
    }

    pub fn reconfigure(&self, s: &Settings) -> Result<(), String> {
        *self.client.write().unwrap() = build_client(s)?;
        Ok(())
    }

    /// Envía `req`. `root` es collection.bru (cabeceras y auth heredadas).
    pub async fn execute(&self, req: &Request, root: Option<&Request>, resolve: Resolver<'_>, ov: &Overrides, cancel: &CancellationToken) -> RunResult {
        let missing = std::sync::Mutex::new(Vec::<String>::new());
        let full = |n: &str| -> Option<String> {
            let v = resolve(n).or_else(|| dynamic(n));
            if v.is_none() && !missing.lock().unwrap().iter().any(|m| m == n) {
                missing.lock().unwrap().push(n.to_string());
            }
            v
        };
        let it = |s: &str| interpolate(s, &full);

        // URL + path params + query
        let mut url = it(&req.url);
        for p in req.path_params.iter().filter(|p| p.enabled) {
            url = url.replace(&format!(":{}", p.name), &it(&p.value));
        }
        let mut query: Vec<(String, String)> = req.params.iter().filter(|p| p.enabled).map(|p| (it(&p.name), it(&p.value))).collect();
        for q in &ov.query {
            query.retain(|x| x.0 != q.name);
            query.push((q.name.clone(), it(&q.value)));
        }
        if !url.contains("://") && !url.is_empty() {
            url = format!("http://{url}");
        }
        if !query.is_empty() {
            url = format!("{url}{}{}", if url.contains('?') { "&" } else { "?" }, form_encode(&query));
        }

        // cabeceras: colección + petición + overrides
        let mut headers: Vec<Pair> = vec![];
        let mut put = |name: String, value: String| {
            headers.retain(|h| !h.name.eq_ignore_ascii_case(&name));
            headers.push(Pair::new(name, value));
        };
        for h in root.map(|r| r.headers.as_slice()).unwrap_or(&[]).iter().chain(&req.headers).filter(|h| h.enabled) {
            put(it(&h.name), it(&h.value));
        }
        let auth_src = if req.auth_mode == "inherit" { root.unwrap_or(req) } else { req };
        match auth_src.auth_mode.as_str() {
            "bearer" => {
                let t = it(&auth_src.auth.token);
                let t = t.strip_prefix("Bearer ").unwrap_or(&t).to_string();
                put("Authorization".into(), format!("Bearer {t}"));
            }
            "basic" => {
                let raw = format!("{}:{}", it(&auth_src.auth.username), it(&auth_src.auth.password));
                put("Authorization".into(), format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(raw)));
            }
            _ => {}
        }
        for h in &ov.headers {
            put(h.name.clone(), it(&h.value));
        }

        // body
        let mut body: Option<String> = match req.body_mode.as_str() {
            "json" | "text" | "xml" => req.body_text().map(|b| it(&b)),
            "form-urlencoded" => Some(form_encode(&req.body.form.iter().filter(|p| p.enabled).map(|p| (it(&p.name), it(&p.value))).collect::<Vec<_>>())),
            // shortcut: multipart se envía como urlencoded y sin archivos; usar reqwest::multipart cuando haya que subir archivos.
            "multipart-form" => Some(form_encode(&req.body.multipart.iter().filter(|p| p.enabled).map(|p| (it(&p.name), it(&p.value))).collect::<Vec<_>>())),
            _ => None,
        };
        let mut warn = None;
        if !ov.body.is_empty() {
            let mut v: Value = match body.as_deref().map(serde_json::from_str::<Value>) {
                Some(Ok(v)) => v,
                None => json!({}),
                Some(Err(e)) => {
                    warn = Some(format!("El body no es JSON válido; no se aplicó el mapeo ({e})"));
                    Value::Null
                }
            };
            if warn.is_none() {
                for (p, val) in &ov.body {
                    set_path(&mut v, p, val.clone());
                }
                body = Some(serde_json::to_string_pretty(&v).unwrap());
            }
        }
        let has_ct = headers.iter().any(|h| h.name.eq_ignore_ascii_case("content-type"));
        if !has_ct && body.is_some() {
            let ct = match req.body_mode.as_str() {
                "xml" => "application/xml",
                "text" => "text/plain",
                "form-urlencoded" | "multipart-form" => "application/x-www-form-urlencoded",
                _ => "application/json",
            };
            headers.push(Pair::new("Content-Type", ct));
        }

        let method = if req.method.is_empty() { "get".to_string() } else { req.method.to_lowercase() };
        let sent = SentRequest { method: method.to_uppercase(), url: url.clone(), headers: headers.clone(), body: body.clone() };
        let mut res = RunResult { sent, ..Default::default() };
        let missing_now = missing.lock().unwrap().clone();
        res.missing = missing_now;

        let m = reqwest::Method::from_bytes(method.to_uppercase().as_bytes()).unwrap_or(reqwest::Method::GET);
        let client = self.client.read().unwrap().clone();
        let mut rb = client.request(m, &url);
        for h in &headers {
            rb = rb.header(&h.name, &h.value);
        }
        if let Some(b) = body {
            rb = rb.body(b); // también en GET
        }
        let t0 = Instant::now();
        let out = tokio::select! {
            _ = cancel.cancelled() => Err("Cancelada".to_string()),
            r = async {
                let resp = rb.send().await.map_err(describe)?;
                let status = resp.status();
                let hs: Vec<Pair> = resp.headers().iter().map(|(k, v)| Pair::new(k.as_str(), v.to_str().unwrap_or(""))).collect();
                let bytes = resp.bytes().await.map_err(describe)?;
                Ok::<_, String>((status, hs, bytes))
            } => r,
        };
        res.time_ms = t0.elapsed().as_millis() as u64;
        match out {
            Err(e) => res.error = Some(e),
            Ok((status, hs, bytes)) => {
                res.status = Some(status.as_u16());
                res.status_text = status.canonical_reason().unwrap_or("").into();
                res.headers = hs;
                res.size = bytes.len();
                res.body = String::from_utf8_lossy(&bytes).into();
                res.json = serde_json::from_slice(&bytes).ok();
                res.ok = status.as_u16() < 400;
            }
        }
        if res.error.is_none() {
            res.error = warn;
        }
        if res.status.is_some() {
            for v in req.vars_post.iter().filter(|p| p.enabled) {
                if let Some(x) = res_value(&res, &v.value) {
                    res.vars.push(Pair::new(&v.name, martino_core::interp::value_to_text(&x)));
                }
            }
            for a in req.asserts.iter().filter(|p| p.enabled) {
                let (op, expected) = split_assert(&a.value);
                let expected = it(&expected);
                let actual = res_value(&res, &a.name);
                let pass = check(actual.as_ref(), &op, &expected);
                res.asserts.push(AssertResult { name: a.name.clone(), op, expected, actual: actual.unwrap_or(Value::Null), pass });
            }
            if res.asserts.iter().any(|a| !a.pass) {
                res.ok = false;
            }
        }
        res
    }
}

fn describe(e: reqwest::Error) -> String {
    let mut msg = e.to_string();
    let mut src = std::error::Error::source(&e);
    while let Some(s) = src {
        msg = format!("{msg}: {s}");
        src = s.source();
    }
    if e.is_timeout() {
        format!("Tiempo de espera agotado ({msg})")
    } else if msg.contains("certificate") || msg.contains("UnknownIssuer") {
        format!("{msg}. Revisa Ajustes → TLS (CA extra o desactivar la verificación)")
    } else {
        msg
    }
}

/// Nombres `{{x}}` usados por la petición (para avisar de las que faltan antes de enviar).
pub fn used_vars(r: &Request) -> Vec<String> {
    let v = serde_json::to_value(r).unwrap();
    let mut out = vec![];
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) => out.extend(var_names(s)),
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            Value::Object(m) => m.values().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    walk(&v, &mut out);
    out.sort();
    out.dedup();
    out.retain(|n| !DYNAMIC_VARS.contains(&n.as_str()));
    out
}
