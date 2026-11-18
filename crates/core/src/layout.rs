//! Orden del lienzo: identificadores fijos de cada paso y acomodo automático en columnas.
use crate::flow::{is_annotation, Flow};
use std::collections::HashMap;

/// Prefijo del identificador visible de cada tipo de paso.
pub fn id_prefix(kind: &str) -> &'static str {
    match kind {
        "start" => "INI",
        "input" => "ENT",
        "request" => "SRV",
        "extract" => "EXT",
        "condition" => "CND",
        "data" => "DAT",
        "transform" => "TRF",
        "crypto" => "CIF",
        "lookup" => "EQV",
        "response" => "RES",
        "end" => "FIN",
        "image" => "IMG",
        _ => "NOT",
    }
}

/// El siguiente código libre para `kind` (no reusa números de pasos borrados que sigan en uso).
pub fn next_code(flow: &Flow, kind: &str) -> String {
    let p = id_prefix(kind);
    let n = flow.nodes.iter().filter_map(|x| x.sid.strip_prefix(p)?.strip_prefix('-')?.parse::<u32>().ok()).max().unwrap_or(0);
    format!("{p}-{:02}", n + 1)
}

/// Da código a los pasos que no lo tienen (flujos de antes). Devuelve cuántos asignó.
pub fn assign_codes(flow: &mut Flow) -> usize {
    let mut n = 0;
    for i in 0..flow.nodes.len() {
        if flow.nodes[i].sid.is_empty() {
            let c = next_code(flow, &flow.nodes[i].kind.clone());
            flow.nodes[i].sid = c;
            n += 1;
        }
    }
    n
}

/// Etapas del flujo, en orden; cada tipo de paso cae en una (carriles del lienzo y capítulos de la Historia).
pub const STAGES: [&str; 5] = ["Datos", "Validar", "Preparar", "Enviar", "Responder"];

pub fn stage(kind: &str) -> usize {
    match kind {
        "start" | "input" | "data" | "lookup" => 0,
        "condition" => 1,
        "extract" | "transform" | "crypto" => 2,
        "request" => 3,
        _ => 4,
    }
}

/// Distancia de cada paso al inicio (camino más largo); el tope de vueltas evita ciclos infinitos.
pub fn depths(flow: &Flow) -> HashMap<String, usize> {
    let ids: Vec<String> = flow.nodes.iter().filter(|n| !is_annotation(&n.kind)).map(|n| n.id.clone()).collect();
    let mut depth: HashMap<String, usize> = ids.iter().map(|i| (i.clone(), 0)).collect();
    for _ in 0..ids.len() {
        let mut changed = false;
        for e in &flow.edges {
            if let (Some(&a), Some(&b)) = (depth.get(&e.from), depth.get(&e.to)) {
                if b < a + 1 {
                    depth.insert(e.to.clone(), a + 1);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    depth
}

/// Orden de lectura: por etapa y, dentro de ella, por distancia al inicio y posición actual.
pub fn reading_order(flow: &Flow) -> Vec<String> {
    let d = depths(flow);
    let mut ids: Vec<&crate::flow::Node> = flow.nodes.iter().filter(|n| !is_annotation(&n.kind)).collect();
    ids.sort_by(|a, b| (stage(&a.kind), d[&a.id], a.y as i64, a.x as i64).cmp(&(stage(&b.kind), d[&b.id], b.y as i64, b.x as i64)));
    ids.into_iter().map(|n| n.id.clone()).collect()
}

/// Acomoda cada paso en el carril de su etapa (`col_w` de ancho) y, dentro del carril, de arriba
/// abajo según su distancia al inicio. Las notas e imágenes no se mueven.
pub fn arrange(flow: &mut Flow, col_w: f64, row_h: f64) {
    let order = reading_order(flow);
    let mut rows = [0usize; 5];
    let mut place: HashMap<String, (f64, f64)> = HashMap::new();
    for id in &order {
        let st = stage(&flow.node(id).map(|n| n.kind.clone()).unwrap_or_default());
        place.insert(id.clone(), (30.0 + st as f64 * col_w, 60.0 + rows[st] as f64 * row_h));
        rows[st] += 1;
    }
    for n in flow.nodes.iter_mut() {
        if let Some(&(x, y)) = place.get(&n.id) {
            n.x = x;
            n.y = y;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{Edge, Node};

    fn node(id: &str, kind: &str) -> Node {
        Node { id: id.into(), kind: kind.into(), key: id.into(), ..Default::default() }
    }
    fn edge(a: &str, b: &str) -> Edge {
        Edge { from: a.into(), port: "out".into(), to: b.into() }
    }

    #[test]
    fn carriles_por_etapa_y_codigos_fijos() {
        let mut f = Flow { nodes: vec![node("s", "start"), node("a", "request"), node("b", "condition"), node("c", "response"), node("d", "response"), node("n", "note")], ..Default::default() };
        f.edges = vec![edge("s", "b"), edge("b", "a"), edge("a", "c"), edge("b", "d")];
        f.nodes[5].x = 999.0;
        arrange(&mut f, 260.0, 130.0);
        let p = |id: &str| { let n = f.node(id).unwrap(); (n.x, n.y) };
        assert_eq!((p("s"), p("b"), p("a")), ((30.0, 60.0), (290.0, 60.0), (810.0, 60.0)), "cada uno en el carril de su etapa");
        assert_eq!(p("d"), (1070.0, 60.0), "d está más cerca del inicio que c: va primero en Responder");
        assert_eq!(p("c"), (1070.0, 190.0));
        assert_eq!(f.node("n").unwrap().x, 999.0, "las notas no se mueven");
        assert_eq!(reading_order(&f), ["s", "b", "a", "d", "c"]);
        assert_eq!(assign_codes(&mut f), 6);
        assert_eq!(f.node("c").unwrap().sid, "RES-01");
        assert_eq!(next_code(&f, "response"), "RES-03");
        f.nodes.retain(|n| n.id != "c");
        assert_eq!(next_code(&f, "response"), "RES-03", "no reusa el número de uno que sigue existiendo");
    }
}
