//! Versión YAML legible de un flujo (claves y tipos en español, conexiones como `siguiente: [a, b]`).
use crate::flow::{Edge, Flow, Node};
use serde_json::{json, Map, Value};

const TYPES: [(&str, &str); 13] = [
    ("start", "inicio"),
    ("input", "entrada"),
    ("request", "servicio"),
    ("extract", "extraer"),
    ("condition", "condicion"),
    ("data", "datos"),
    ("transform", "transformar"),
    ("crypto", "cifrado"),
    ("lookup", "equivalencia"),
    ("response", "respuesta"),
    ("end", "fin"),
    ("note", "nota"),
    ("image", "imagen"),
];
const PORTS: [(&str, &str); 4] = [("out", "siguiente"), ("error", "error"), ("true", "si"), ("false", "no")];
const FIELDS: [(&str, &str); 4] = [("key", "clave"), ("type", "tipo"), ("name", "nombre"), ("collection", "coleccion")];

fn es<'a>(table: &'a [(&'a str, &'a str)], en: &'a str) -> &'a str {
    table.iter().find(|x| x.0 == en).map(|x| x.1).unwrap_or(en)
}
fn en<'a>(table: &'a [(&'a str, &'a str)], es: &'a str) -> &'a str {
    table.iter().find(|x| x.1 == es).map(|x| x.0).unwrap_or(es)
}

pub fn flow_to_yaml(flow: &Flow) -> String {
    let mut root = Map::new();
    root.insert("nombre".into(), json!(flow.name));
    root.insert("coleccion".into(), json!(flow.collection));
    if let Some(s) = flow.nodes.iter().find(|n| n.kind == "start") {
        root.insert("inicio".into(), json!(flow.outgoing(&s.id).map(|e| e.to.clone()).collect::<Vec<_>>()));
    }
    let mut nodes = vec![];
    for n in &flow.nodes {
        let Value::Object(src) = serde_json::to_value(n).unwrap() else { unreachable!() };
        let mut m = Map::new();
        if n.id != n.key {
            m.insert("id".into(), json!(n.id));
        }
        for (k, v) in src {
            match k.as_str() {
                "id" => {}
                "type" => {
                    m.insert("tipo".into(), json!(es(&TYPES, &n.kind)));
                }
                "x" | "y" => {}
                _ => {
                    m.insert(es(&FIELDS, &k).into(), v);
                }
            }
        }
        m.insert("pos".into(), json!([n.x, n.y]));
        if n.kind != "start" {
            for (p, name) in PORTS {
                let to: Vec<_> = flow.outgoing(&n.id).filter(|e| e.port == p).map(|e| e.to.clone()).collect();
                if !to.is_empty() {
                    m.insert(name.into(), json!(to));
                }
            }
        }
        nodes.push(Value::Object(m));
    }
    root.insert("nodos".into(), Value::Array(nodes));
    let rest = serde_json::to_value(flow).unwrap();
    for (k, name) in [("global", "global"), ("compare", "comparar"), ("cases", "casos"), ("vars", "variables")] {
        if let Some(v) = rest.get(k).filter(|v| !v.is_null() && v.as_array().map_or(true, |a| !a.is_empty())) {
            root.insert(name.into(), v.clone());
        }
    }
    serde_yaml::to_string(&Value::Object(root)).unwrap_or_default()
}

pub fn flow_from_yaml(src: &str) -> Result<Flow, String> {
    let root: Value = serde_yaml::from_str(src).map_err(|e| format!("YAML inválido: {e}"))?;
    let root = root.as_object().ok_or("El YAML debe ser un objeto")?;
    let mut flow = Flow {
        name: root.get("nombre").and_then(Value::as_str).unwrap_or_default().into(),
        collection: root.get("coleccion").and_then(Value::as_str).unwrap_or_default().into(),
        ..Default::default()
    };
    let as_list = |v: Option<&Value>| -> Vec<String> {
        match v {
            Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
            Some(Value::String(s)) => vec![s.clone()],
            _ => vec![],
        }
    };
    let mut edges = vec![];
    for (i, raw) in root.get("nodos").and_then(Value::as_array).cloned().unwrap_or_default().into_iter().enumerate() {
        let src = raw.as_object().ok_or_else(|| format!("nodos[{i}] no es un objeto"))?;
        let mut m = Map::new();
        for (k, v) in src {
            match k.as_str() {
                "tipo" => {
                    m.insert("type".into(), json!(en(&TYPES, v.as_str().unwrap_or(""))));
                }
                "pos" => {
                    m.insert("x".into(), v.get(0).cloned().unwrap_or(json!(0)));
                    m.insert("y".into(), v.get(1).cloned().unwrap_or(json!(0)));
                }
                k if PORTS.iter().any(|p| p.1 == k) => {}
                _ => {
                    m.insert(en(&FIELDS, k).into(), v.clone());
                }
            }
        }
        if !m.contains_key("id") {
            let key = m.get("key").cloned().unwrap_or(json!(format!("n{i}")));
            m.insert("id".into(), key);
        }
        let node: Node = serde_json::from_value(Value::Object(m)).map_err(|e| format!("nodos[{i}]: {e}"))?;
        for (p, name) in PORTS {
            for to in as_list(src.get(name)) {
                edges.push(Edge { from: node.id.clone(), port: p.into(), to });
            }
        }
        flow.nodes.push(node);
    }
    let inicio = as_list(root.get("inicio"));
    let mut start_edges = vec![];
    if !inicio.is_empty() {
        let start = match flow.nodes.iter().find(|n| n.kind == "start") {
            Some(s) => s.id.clone(),
            None => {
                flow.nodes.insert(0, Node { id: "start".into(), kind: "start".into(), key: "inicio".into(), name: "Inicio".into(), ..Default::default() });
                "start".into()
            }
        };
        start_edges = inicio.into_iter().map(|to| Edge { from: start.clone(), port: "out".into(), to }).collect();
    }
    flow.edges = start_edges.into_iter().chain(edges).collect();
    for (k, name) in [("global", "global"), ("compare", "comparar"), ("cases", "casos"), ("vars", "variables")] {
        if let Some(v) = root.get(name) {
            let mut tmp = serde_json::to_value(&flow).unwrap();
            tmp[k] = v.clone();
            flow = serde_json::from_value(tmp).map_err(|e| format!("{name}: {e}"))?;
        }
    }
    // `orden: carriles` acomoda los pasos por sistema; sin él, el import los ordena por etapa.
    if root.get("orden").and_then(Value::as_str) == Some("carriles") {
        crate::layout::arrange_systems(&mut flow);
    }
    Ok(flow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{Case, Global, Rule};
    use crate::model::Pair;

    #[test]
    fn yaml_round_trip() {
        let mut flow = Flow { name: "Consulta".into(), collection: "demo".into(), ..Default::default() };
        let mk = |id: &str, kind: &str, x: f64| Node { id: id.into(), key: id.into(), kind: kind.into(), name: id.to_uppercase(), x, y: 10.0, ..Default::default() };
        let mut cond = mk("es_dni", "condition", 3.0);
        cond.rules = vec![Rule { left: "entrada.tipo".into(), op: "eq".into(), right: "DNI".into() }];
        let mut tr = mk("t", "transform", 4.0);
        tr.code = "const a = 1;\nreturn { status: 200, body: a };".into();
        let mut img = mk("img1", "image", 5.0);
        img.key = String::new();
        img.src = "data:image/png;base64,AAA".into();
        let mut inp = mk("entrada", "input", 1.0);
        inp.data = serde_json::json!({"tipo": "DNI"});
        let mut svc = mk("svc", "request", 2.0);
        svc.id = "n-svc".into();
        svc.req_ref = "clientes/consulta.bru".into();
        svc.save_vars = vec![Pair::new("id", "response.id")];
        flow.nodes = vec![mk("start", "start", 0.0), inp, svc, cond, tr, mk("fin", "end", 6.0), img];
        flow.edges = vec![
            Edge { from: "start".into(), port: "out".into(), to: "n-svc".into() },
            Edge { from: "start".into(), port: "out".into(), to: "es_dni".into() },
            Edge { from: "n-svc".into(), port: "out".into(), to: "fin".into() },
            Edge { from: "n-svc".into(), port: "error".into(), to: "fin".into() },
            Edge { from: "es_dni".into(), port: "true".into(), to: "t".into() },
            Edge { from: "es_dni".into(), port: "false".into(), to: "fin".into() },
        ];
        flow.global = Some(Global { req_ref: "global.bru".into() });
        flow.cases = vec![Case { id: "c1".into(), name: "uno".into(), input: serde_json::json!({"tipo": "CE"}), expect_status: Some(200), ..Default::default() }];
        let y = flow_to_yaml(&flow);
        assert!(y.contains("inicio:\n- n-svc\n- es_dni") && y.contains("tipo: imagen") && y.contains("si:\n  - t"), "{y}");
        let back = flow_from_yaml(&y).unwrap();
        assert_eq!(back, flow);
    }

    #[test]
    fn short_form_creates_start() {
        let f = flow_from_yaml("nombre: x\ninicio: [a]\nnodos:\n  - clave: a\n    tipo: respuesta\n    status: '200'\n").unwrap();
        assert_eq!(f.nodes[0].kind, "start");
        assert_eq!(f.edges[0].to, "a");
        assert!(flow_from_yaml("- a").is_err());
    }
}
