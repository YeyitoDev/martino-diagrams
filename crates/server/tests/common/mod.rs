//! Servidor Martino temporal + un upstream de prueba.
#![allow(dead_code)]
use axum::routing::{get, post};
use axum::{Json, Router};
use martino::config::Config;
use martino::state::Core;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct Env {
    pub _dir: tempfile::TempDir,
    pub core: Arc<Core>,
    pub base: String,
    pub upstream: String,
}

pub async fn upstream() -> String {
    let app = Router::new()
        .route("/users/{id}", get(|axum::extract::Path(id): axum::extract::Path<String>| async move { Json(json!({"id": id, "name": "Ana", "tags": ["a", "b"]})) }))
        .route("/echo", post(|h: axum::http::HeaderMap, body: String| async move {
            let auth = h.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
            Json(json!({"body": serde_json::from_str::<Value>(&body).unwrap_or(json!(body)), "auth": auth}))
        }))
        .route("/fail", get(|| async { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "boom"}))) }))
        .route("/slow", get(|| async { tokio::time::sleep(std::time::Duration::from_millis(300)).await; Json(json!({"slow": true})) }));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://127.0.0.1:{port}")
}

pub async fn start() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::new(Config::load(dir.path(), json!({"port": 0})));
    let (port, _) = martino::serve(core.clone()).await.unwrap();
    Env { _dir: dir, core, base: format!("http://127.0.0.1:{port}"), upstream: upstream().await }
}

impl Env {
    pub fn req(&self, m: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        reqwest::Client::new().request(m, format!("{}{path}", self.base)).header("X-Workbench", "1")
    }
    pub async fn json(&self, m: reqwest::Method, path: &str, body: Value) -> Value {
        let r = self.req(m, path).json(&body).send().await.unwrap();
        let st = r.status();
        let v: Value = r.json().await.unwrap();
        assert!(st.is_success(), "{path}: {st} {v}");
        v
    }
    pub async fn lines(&self, path: &str, body: Value) -> Vec<Value> {
        let t = self.req(reqwest::Method::POST, path).json(&body).send().await.unwrap().text().await.unwrap();
        t.lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }
}
