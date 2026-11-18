mod common;
use martino::flows::{run_flow, RunOpts};
use martino_core::flow::{Compare, Edge, Flow, Global, Node, Rule};
use martino_core::model::{Pair, Request};
use serde_json::json;

fn node(id: &str, kind: &str) -> Node {
    Node { id: id.into(), key: id.into(), kind: kind.into(), name: id.into(), ..Default::default() }
}
fn edge(f: &str, p: &str, t: &str) -> Edge {
    Edge { from: f.into(), port: p.into(), to: t.into() }
}
fn svc(id: &str, method: &str, url: String) -> Node {
    Node { request: Some(Request::new(method, &url)), ..node(id, "request") }
}

async fn run(e: &common::Env, flow: &Flow, compare: bool) -> martino::flows::FlowResult {
    run_flow(&e.core, flow, &RunOpts { compare, ..Default::default() }, &Default::default(), &|_| {}).await
}

#[tokio::test]
async fn parallel_join_condition_transform_response() {
    let e = common::start().await;
    let u = &e.upstream;
    let mut entrada = node("entrada", "input");
    entrada.data = json!({"uid": "5", "tipo": "DNI"});
    let a = svc("lento_a", "get", format!("{u}/slow"));
    let b = svc("lento_b", "get", format!("{u}/slow"));
    let mut user = svc("usuario", "get", format!("{u}/users/{{{{entrada.uid}}}}"));
    user.save_vars = vec![Pair::new("nombre", "response.name")];
    let mut cond = node("es_dni", "condition");
    cond.rules = vec![Rule { left: "entrada.tipo".into(), op: "eq".into(), right: "DNI".into() }];
    let mut tr = node("armar", "transform");
    tr.code = "return { status: 200, body: { nombre: env.nombre, id: nodes.usuario.response.id, ok: nodes.lento_a.response.slow } }".into();
    let mut ok = node("ok", "response");
    ok.status = "{{armar.status}}".into();
    ok.body = json!({"cliente": "{{armar.body}}", "tipo": "{{entrada.tipo}}"});
    let mut no = node("no_dni", "response");
    no.status = "400".into();
    let mut flow = Flow { nodes: vec![node("start", "start"), entrada, a, b, user, cond, tr, ok, no], ..Default::default() };
    flow.edges = vec![
        edge("start", "out", "lento_a"), edge("start", "out", "lento_b"), edge("start", "out", "usuario"),
        edge("lento_a", "out", "es_dni"), edge("lento_b", "out", "es_dni"), edge("usuario", "out", "es_dni"),
        edge("es_dni", "true", "armar"), edge("es_dni", "false", "no_dni"), edge("armar", "out", "ok"),
    ];
    let t = std::time::Instant::now();
    let r = run(&e, &flow, false).await;
    assert_eq!(r.verdict, "OK", "{:?}", r.error);
    assert!(t.elapsed().as_millis() < 550, "las dos llamadas lentas van en paralelo: {:?}", t.elapsed());
    assert_eq!(r.steps.iter().filter(|s| s.key == "es_dni").count(), 1, "la unión se ejecuta una sola vez");
    assert_eq!((r.status, r.body.clone()), (Some(200), json!({"cliente": {"nombre": "Ana", "id": "5", "ok": true}, "tipo": "DNI"})));

    // rama false
    flow.nodes[1].data = json!({"uid": "5", "tipo": "CE"});
    let r = run(&e, &flow, false).await;
    assert_eq!(r.status, Some(400));
    assert_eq!(r.response_node.as_deref(), Some("no_dni"));
}

#[tokio::test]
async fn error_port_end_and_compare() {
    let e = common::start().await;
    let u = &e.upstream;
    let col = e.core.store.create_collection("C").unwrap();
    e.core.store.create_request(&col, "", &Request { name: "global".into(), ..Request::new("get", &format!("{u}/users/{{{{entrada.uid}}}}")) }).unwrap();

    // error → rama de error → fin (sin respuesta)
    let mut flow = Flow { collection: col.clone(), ..Default::default() };
    flow.nodes = vec![svc("falla", "get", format!("{u}/fail")), node("fin", "end"), node("start", "start")];
    let mut x = node("guardar", "extract");
    x.vars = vec![Pair::new("err", "falla.response.error")];
    flow.nodes.push(x);
    flow.edges = vec![edge("start", "out", "falla"), edge("falla", "error", "guardar"), edge("guardar", "out", "fin")];
    let r = run(&e, &flow, false).await;
    assert!(r.ended && r.verdict == "OK", "{r:?}");
    assert_eq!(r.vars["err"], json!("boom"));

    // comparación con el global real: expected + ignore
    let mut entrada = node("entrada", "input");
    entrada.data = json!({"uid": "9"});
    let mut resp = node("r", "response");
    resp.body = json!({"id": "{{entrada.uid}}", "name": "Ana", "tags": ["x"]});
    let mut f = Flow { collection: col, nodes: vec![node("start", "start"), entrada, resp], ..Default::default() };
    f.edges = vec![edge("start", "out", "r")];
    f.global = Some(Global { req_ref: "global.yml".into() });
    let r = run(&e, &f, true).await;
    assert_eq!(r.verdict, "DIFERENTE");
    assert_eq!(r.compare.as_ref().unwrap().diffs[0].path, "tags[0]");
    f.compare = Compare { ignore: vec!["tags".into()], ..Default::default() };
    assert_eq!(run(&e, &f, true).await.verdict, "OK");
}

#[tokio::test]
async fn lookup_and_crypto_steps() {
    use martino_core::crypto::CryptoSpec;
    use martino_core::lookup::LookupSpec;
    let e = common::start().await;
    let col = e.core.store.create_collection("Demo").unwrap();
    e.core.store.put_table(&col, "bancos", "codigo,nombre\n002,BCP\n003,Interbank\n").unwrap();
    let mut entrada = node("entrada", "input");
    entrada.data = json!({"banco": "003"});
    let mut banco = node("banco", "lookup");
    banco.lookup = Some(LookupSpec { table: "bancos".into(), value: "{{entrada.banco}}".into(), ..Default::default() });
    let key = "k".repeat(32);
    let cripto = |id: &str, op: &str, input: &str| Node {
        crypto: Some(CryptoSpec { format: "jwe".into(), op: op.into(), alg: "dir".into(), key: key.clone(), input: input.into(), ..Default::default() }),
        ..node(id, "crypto")
    };
    let mut ok = node("ok", "response");
    ok.body = json!({"banco": "{{banco.nombre}}", "claro": "{{abrir.nombre}}", "token_partes": "{{cerrar}}"});
    let mut flow = Flow {
        collection: col,
        nodes: vec![node("start", "start"), entrada, banco, cripto("cerrar", "encrypt", "{{banco}}"), cripto("abrir", "decrypt", "{{cerrar}}"), ok],
        ..Default::default()
    };
    flow.edges = vec![edge("start", "out", "banco"), edge("banco", "out", "cerrar"), edge("cerrar", "out", "abrir"), edge("abrir", "out", "ok")];
    let r = run(&e, &flow, false).await;
    assert_eq!(r.verdict, "OK", "{:?}", r.error);
    assert_eq!(r.body["banco"], json!("Interbank"));
    assert_eq!(r.body["claro"], json!("Interbank"), "ida y vuelta del JWE");
    assert_eq!(r.body["token_partes"].as_str().unwrap().split('.').count(), 5);

    flow.nodes[1].data = json!({"banco": "999"});
    let r = run(&e, &flow, false).await;
    assert!(r.error.unwrap_or_default().contains("no está en la tabla bancos"), "sin equivalencia se detiene con un mensaje claro");
}
