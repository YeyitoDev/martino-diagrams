mod common;
use reqwest::Method;
use serde_json::json;

#[tokio::test]
async fn guard_blocks_foreign_requests() {
    let e = common::start().await;
    let c = reqwest::Client::new();
    let url = format!("{}/api/collections", e.base);
    assert_eq!(c.get(&url).send().await.unwrap().status(), 403, "sin X-Workbench");
    assert_eq!(c.get(&url).header("X-Workbench", "1").header("Host", "evil.com").send().await.unwrap().status(), 403, "Host ajeno");
    assert_eq!(c.get(&url).header("X-Workbench", "1").header("Origin", "http://evil.com").send().await.unwrap().status(), 403, "Origin ajeno");
    assert_eq!(c.request(Method::OPTIONS, &url).header("X-Workbench", "1").send().await.unwrap().status(), 403, "preflight");
    let ok = c.get(&url).header("X-Workbench", "1").header("Origin", e.base.clone()).send().await.unwrap();
    assert_eq!(ok.status(), 200);
    assert_eq!(ok.headers()["cache-control"], "no-store");
}

#[tokio::test]
async fn crud_and_run_ndjson() {
    let e = common::start().await;
    let col = e.json(Method::POST, "/api/collections", json!({"name": "Demo"})).await["id"].as_str().unwrap().to_string();
    e.json(Method::PUT, &format!("/api/environment?collection={col}&env=dev"), json!({
        "name": "dev", "vars": [{"name": "base", "value": e.upstream}], "secrets": [{"name": "token", "value": "SEC"}]})).await;
    let req = json!({"name": "Usuario", "method": "get", "url": "{{base}}/users/{{uid}}",
        "varsPre": [{"name": "uid", "value": "7"}],
        "varsPost": [{"name": "userName", "value": "res.body.name"}],
        "asserts": [{"name": "res.status", "value": "eq 200"}, {"name": "res.body.tags", "value": "contains b"}]});
    let path = e.json(Method::POST, &format!("/api/request?collection={col}"), req).await["path"].as_str().unwrap().to_string();
    let echo = json!({"name": "Echo", "method": "post", "url": "{{base}}/echo", "bodyMode": "json", "authMode": "bearer",
        "auth": {"token": "{{token}}"}, "body": {"json": "{\"who\": \"{{userName}}\", \"id\": \"{{$guid}}\"}"}});
    let echo_path = e.json(Method::POST, &format!("/api/request?collection={col}"), echo).await["path"].as_str().unwrap().to_string();
    let got = e.json(Method::GET, &format!("/api/request?collection={col}&path={path}"), json!(null)).await;
    assert_eq!(got["seq"], 1.0);

    let lines = e.lines("/api/run", json!({"items": [{"id": "p1", "collection": col, "path": path, "env": "dev"}]})).await;
    assert_eq!(lines.first().unwrap()["type"], "start");
    let r = &lines.iter().find(|l| l["type"] == "result").unwrap()["result"];
    assert_eq!(r["status"], 200, "{r}");
    assert_eq!(r["json"]["id"], "7");
    assert!(r["asserts"].as_array().unwrap().iter().all(|a| a["pass"] == true), "{r}");
    assert_eq!(lines.last().unwrap()["type"], "done");
    assert_eq!(e.json(Method::GET, "/api/runtime", json!(null)).await["userName"], "Ana");

    // post-response → runtime → siguiente petición; secreto como bearer
    let lines = e.lines("/api/run", json!({"items": [{"id": "p2", "collection": col, "path": echo_path, "env": "dev"}]})).await;
    let r = &lines[1]["result"];
    assert_eq!(r["json"]["body"]["who"], "Ana");
    assert_eq!(r["json"]["auth"], "Bearer SEC");
    assert_eq!(r["json"]["body"]["id"].as_str().unwrap().len(), 36);

    // resolve: tabla con origen, lo que falta
    let res = e.json(Method::POST, "/api/resolve", json!({"collection": col, "env": "dev",
        "request": {"method": "get", "url": "{{base}}/{{nope}}"}})).await;
    let token = res["vars"].as_array().unwrap().iter().find(|v| v["name"] == "token").unwrap().clone();
    assert_eq!(token["source"], "secret");
    assert_eq!(res["missing"], json!(["nope"]));

    // mover y borrar
    let folder = e.json(Method::POST, "/api/folder", json!({"collection": col, "name": "Sub"})).await["path"].as_str().unwrap().to_string();
    let moved = e.json(Method::POST, "/api/request/move", json!({"collection": col, "from": path, "toFolder": folder})).await;
    assert_eq!(moved["path"], format!("{folder}/Usuario.yml"));
    let r = e.req(Method::DELETE, &format!("/api/collections?collection={col}")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(e.json(Method::GET, "/api/collections", json!(null)).await, json!([]));
    let bad = e.req(Method::GET, "/api/request?collection=x&path=../../etc/passwd.bru").send().await.unwrap();
    assert_eq!(bad.status(), 400);
}

#[tokio::test]
async fn import_postman_and_parallel_cancel() {
    let e = common::start().await;
    let pm = json!({"info": {"name": "PM"}, "item": [{"name": "Lento", "request": {"method": "GET", "url": format!("{}/slow", e.upstream)}}]});
    let r = e.json(Method::POST, "/api/import", json!({"data": pm})).await;
    assert_eq!(r["requests"], 1);
    let col = r["collection"].as_str().unwrap().to_string();
    let items: Vec<_> = (0..6).map(|i| json!({"id": format!("p{i}"), "collection": col, "path": "Lento.yml"})).collect();
    let t = std::time::Instant::now();
    let lines = e.lines("/api/run", json!({"items": items, "concurrency": 6})).await;
    assert_eq!(lines.iter().filter(|l| l["type"] == "result").count(), 6);
    assert!(t.elapsed().as_millis() < 1200, "en paralelo: {:?}", t.elapsed());
}

#[tokio::test]
async fn public_mode_requires_key() {
    let e = common::start().await;
    e.core.cfg.write().unwrap().public_key = Some("llave-secreta-de-prueba-123".into());
    let c = reqwest::Client::new();
    let url = format!("{}/api/collections", e.base);
    let tunnel = "abc.trycloudflare.com";
    let get = |extra: &str, host: &str| c.get(format!("{url}{extra}")).header("X-Workbench", "1").header("Host", host.to_string());
    assert_eq!(get("", tunnel).send().await.unwrap().status(), 403, "sin llave");
    assert_eq!(get("?key=otra", tunnel).send().await.unwrap().status(), 403, "llave mala");
    assert_eq!(get("?key=llave-secreta-de-prueba-123", "evil.com").send().await.unwrap().status(), 403, "otro host");
    let ok = get("?key=llave-secreta-de-prueba-123", tunnel).send().await.unwrap();
    assert_eq!(ok.status(), 200);
    assert!(ok.headers()["set-cookie"].to_str().unwrap().contains("HttpOnly"));
    let cookie = get("", tunnel).header("Cookie", "martino_key=llave-secreta-de-prueba-123").send().await.unwrap();
    assert_eq!(cookie.status(), 200);
    let ts = get("?key=llave-secreta-de-prueba-123", "mac.tail1.ts.net:4400").send().await.unwrap();
    assert_eq!(ts.status(), 200, "Tailscale serve con puerto");
    assert_eq!(get("?key=llave-secreta-de-prueba-123", "ts.net.evil.com").send().await.unwrap().status(), 403);
    let bad_origin = get("", tunnel).header("Cookie", "martino_key=llave-secreta-de-prueba-123").header("Origin", "https://evil.com").send().await.unwrap();
    assert_eq!(bad_origin.status(), 403);
    let switch = c.post(format!("{}/api/root", e.base)).header("X-Workbench", "1").header("Host", tunnel).header("Cookie", "martino_key=llave-secreta-de-prueba-123").json(&serde_json::json!({"path": "/"})).send().await.unwrap();
    assert_eq!(switch.status(), 403, "desde el túnel no se cambia de carpeta");
}

#[tokio::test]
async fn mcp_por_http_con_llave() {
    let e = common::start().await;
    e.core.cfg.write().unwrap().public_key = Some("llave-secreta-de-prueba-123".into());
    let c = reqwest::Client::new();
    let post = |q: &str| c.post(format!("{}/mcp{q}", e.base)).header("Host", "abc.trycloudflare.com").json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}));
    assert_eq!(post("").send().await.unwrap().status(), 403, "sin llave");
    let r: serde_json::Value = post("?key=llave-secreta-de-prueba-123").send().await.unwrap().json().await.unwrap();
    assert!(r["result"]["tools"].as_array().unwrap().iter().any(|t| t["name"] == "martino_casos_endpoint"), "{r}");
    let note = c.post(format!("{}/mcp?key=llave-secreta-de-prueba-123", e.base)).header("Host", "abc.trycloudflare.com").json(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    assert_eq!(note.send().await.unwrap().status(), 202, "las notificaciones no llevan respuesta");
}

#[tokio::test]
async fn switch_root_folder() {
    let home = tempfile::tempdir().unwrap();
    // Único test que toca recientes: así no escribe en el ~/.martino real.
    std::env::set_var("MARTINO_HOME", home.path());
    let e = common::start().await;
    e.json(reqwest::Method::POST, "/api/collections", json!({"name": "Mía"})).await;
    let other = tempfile::tempdir().unwrap();
    let bad = e.req(reqwest::Method::POST, "/api/root").json(&json!({"path": "/no/existe/martino"})).send().await.unwrap();
    assert_eq!(bad.status(), 400);
    let r = e.json(reqwest::Method::POST, "/api/root", json!({"path": other.path()})).await;
    let other_root = std::fs::canonicalize(other.path()).unwrap();
    assert_eq!(r["root"], json!(other_root));
    assert_eq!(e.json(reqwest::Method::GET, "/api/collections", json!(null)).await, json!([]), "la nueva carpeta empieza vacía");
    assert!(other_root.join("collections").is_dir());
    let d = e.json(reqwest::Method::GET, "/api/root", json!(null)).await;
    assert_eq!(d["recent"][0], json!(other_root));
    // De vuelta a la primera: sus datos siguen ahí.
    e.json(reqwest::Method::POST, "/api/root", json!({"path": e.core.cfg.read().unwrap().root})).await;
    assert_eq!(e.json(reqwest::Method::GET, "/api/collections", json!(null)).await.as_array().unwrap().len(), 1);
}
