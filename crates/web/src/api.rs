//! Cliente de la API: fetch con `X-Workbench: 1` y lectura de NDJSON en streaming.
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{RequestInit, Response};

fn js_err(e: JsValue) -> String {
    e.as_string().or_else(|| js_sys::Reflect::get(&e, &"message".into()).ok().and_then(|m| m.as_string())).unwrap_or_else(|| format!("{e:?}"))
}

async fn send(method: &str, url: &str, body: Option<String>) -> Result<Response, String> {
    let init = RequestInit::new();
    init.set_method(method);
    let h = web_sys::Headers::new().map_err(js_err)?;
    h.set("X-Workbench", "1").map_err(js_err)?;
    if let Some(b) = &body {
        h.set("Content-Type", "application/json").map_err(js_err)?;
        init.set_body(&JsValue::from_str(b));
    }
    init.set_headers(&h);
    let w = web_sys::window().unwrap();
    let r: Response = JsFuture::from(w.fetch_with_str_and_init(url, &init)).await.map_err(|e| format!("Sin conexión con Martino ({})", js_err(e)))?.unchecked_into();
    if !r.ok() {
        let t = JsFuture::from(r.text().map_err(js_err)?).await.map_err(js_err)?.as_string().unwrap_or_default();
        let msg = serde_json::from_str::<Value>(&t).ok().and_then(|v| v["error"].as_str().map(String::from)).unwrap_or(t);
        return Err(if msg.is_empty() { format!("HTTP {}", r.status()) } else { msg });
    }
    Ok(r)
}

pub async fn call<T: DeserializeOwned>(method: &str, url: &str, body: Option<&impl Serialize>) -> Result<T, String> {
    let r = send(method, url, body.map(|b| serde_json::to_string(b).unwrap())).await?;
    let t = JsFuture::from(r.text().map_err(js_err)?).await.map_err(js_err)?.as_string().unwrap_or_default();
    serde_json::from_str(&t).map_err(|e| format!("Respuesta inválida: {e}"))
}

pub async fn get<T: DeserializeOwned>(url: &str) -> Result<T, String> {
    call::<T>("GET", url, None::<&()>).await
}
pub async fn post<T: DeserializeOwned>(url: &str, body: &impl Serialize) -> Result<T, String> {
    call("POST", url, Some(body)).await
}
pub async fn put<T: DeserializeOwned>(url: &str, body: &impl Serialize) -> Result<T, String> {
    call("PUT", url, Some(body)).await
}
pub async fn del(url: &str) -> Result<Value, String> {
    call::<Value>("DELETE", url, None::<&()>).await
}

/// Lee una respuesta NDJSON línea por línea a medida que llega.
pub async fn stream(method: &str, url: &str, body: Option<&impl Serialize>, mut on: impl FnMut(Value)) -> Result<(), String> {
    let r = send(method, url, body.map(|b| serde_json::to_string(b).unwrap())).await?;
    let reader: web_sys::ReadableStreamDefaultReader = r.body().ok_or("Sin cuerpo")?.get_reader().unchecked_into();
    let mut buf: Vec<u8> = vec![];
    loop {
        let chunk = JsFuture::from(reader.read()).await.map_err(js_err)?;
        if js_sys::Reflect::get(&chunk, &"done".into()).map_err(js_err)?.as_bool().unwrap_or(true) {
            break;
        }
        let bytes = js_sys::Uint8Array::new(&js_sys::Reflect::get(&chunk, &"value".into()).map_err(js_err)?).to_vec();
        buf.extend(bytes);
        while let Some(i) = buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = buf.drain(..=i).collect();
            if let Ok(v) = serde_json::from_slice::<Value>(&line) {
                on(v);
            }
        }
    }
    Ok(())
}

pub fn enc(s: &str) -> String {
    String::from(js_sys::encode_uri_component(s))
}
