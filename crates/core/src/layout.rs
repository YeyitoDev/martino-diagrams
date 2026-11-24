//! Orden del lienzo: identificadores fijos de cada paso y acomodo automático en columnas.
use crate::flow::{is_annotation, Flow, Node};
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

/// Camino principal: desde el Inicio por las salidas buenas (no `error` ni `false`), en orden de
/// llegada. Es lo que recorre el happy path cuando aún no hay una corrida que diga el real.
pub fn main_path(flow: &Flow) -> Vec<String> {
    let mut queue: std::collections::VecDeque<&str> = flow.nodes.iter().filter(|n| n.kind == "start").map(|n| n.id.as_str()).collect();
    let mut seen: std::collections::HashSet<&str> = queue.iter().copied().collect();
    let mut out = vec![];
    while let Some(id) = queue.pop_front() {
        if flow.node(id).is_some_and(|n| n.kind != "start") {
            out.push(id.to_string());
        }
        for e in flow.edges.iter().filter(|e| e.from == id && e.port != "error" && e.port != "false") {
            if seen.insert(e.to.as_str()) {
                queue.push_back(&e.to);
            }
        }
    }
    out
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

/// Carril por sistema de un paso: «Martino» (lo que pasa dentro del flujo), el sistema al que
/// llama (carpeta de la petición o host de la URL) o «Respuesta».
pub fn system_lane(n: &Node) -> String {
    match n.kind.as_str() {
        "response" | "end" => "Respuesta".into(),
        "request" if n.req_ref.contains('/') => n.req_ref.split('/').next().unwrap_or_default().replace('-', " "),
        "request" => n.request.as_ref().map(|r| r.url.split("://").last().unwrap_or(&r.url).split(['/', '?']).next().unwrap_or_default().to_string()).filter(|h| !h.is_empty()).unwrap_or_else(|| "Servicio".into()),
        _ => "Martino".into(),
    }
}

/// Ancho de columna, alto de fila (paso + su respuesta) y margen izquierdo (nombre del carril) de los carriles por sistema.
const SYS_COL: f64 = 270.0;
const SYS_ROW: f64 = 160.0;
const SYS_X0: f64 = 180.0;

/// Carriles por sistema: una fila por sistema (Martino, los sistemas en el orden en que se llaman,
/// Respuesta) y una columna por distancia al inicio; dos pasos en la misma celda se apilan.
/// Devuelve (nombre, y, alto) de cada carril y dónde va cada paso.
pub fn system_layout(flow: &Flow) -> (Vec<(String, f64, f64)>, HashMap<String, (f64, f64)>) {
    let d = depths(flow);
    let mut order: Vec<&Node> = flow.nodes.iter().filter(|n| !is_annotation(&n.kind)).collect();
    order.sort_by_key(|n| d[&n.id]);
    let mut names = vec!["Martino".to_string()];
    for n in &order {
        let l = system_lane(n);
        if l != "Respuesta" && !names.contains(&l) {
            names.push(l);
        }
    }
    names.push("Respuesta".into());
    let mut cells: HashMap<(usize, usize), usize> = HashMap::new();
    let mut rows = vec![1usize; names.len()];
    let slots: Vec<(String, usize, usize, usize)> = order.iter().map(|n| {
        let lane = names.iter().position(|l| *l == system_lane(n)).unwrap_or(0);
        let k = cells.entry((lane, d[&n.id])).or_insert(0);
        *k += 1;
        rows[lane] = rows[lane].max(*k);
        (n.id.clone(), lane, d[&n.id], *k - 1)
    }).collect();
    let mut top = 0.0;
    let lanes: Vec<(String, f64, f64)> = names.into_iter().zip(&rows).map(|(name, r)| {
        let lane = (name, top, *r as f64 * SYS_ROW);
        top += lane.2;
        lane
    }).collect();
    let place = slots.into_iter().map(|(id, lane, col, k)| (id, (SYS_X0 + col as f64 * SYS_COL, lanes[lane].1 + 30.0 + k as f64 * SYS_ROW))).collect();
    (lanes, place)
}

/// Acomoda los pasos en carriles por sistema (botón «Carriles» y `orden: carriles` en el YAML).
pub fn arrange_systems(flow: &mut Flow) {
    let (_, place) = system_layout(flow);
    for n in flow.nodes.iter_mut() {
        if let Some(&(x, y)) = place.get(&n.id) {
            n.x = x;
            n.y = y;
        }
    }
}

/// ¿Cada paso está dentro del carril de su sistema? (si no, el lienzo no dibuja los carriles)
pub fn in_system_lanes(flow: &Flow) -> bool {
    let (lanes, _) = system_layout(flow);
    flow.nodes.iter().filter(|n| !is_annotation(&n.kind)).all(|n| {
        lanes.iter().find(|l| l.0 == system_lane(n)).is_some_and(|(_, top, h)| n.y >= *top && n.y < top + h)
    })
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

    #[test]
    fn camino_principal_evita_error_y_false() {
        let mut f = Flow { nodes: vec![node("s", "start"), node("a", "request"), node("b", "condition"), node("ok", "response"), node("no", "response"), node("x", "response")], ..Default::default() };
        let port = |a: &str, p: &str, b: &str| Edge { from: a.into(), port: p.into(), to: b.into() };
        f.edges = vec![edge("s", "a"), port("a", "error", "x"), edge("a", "b"), port("b", "true", "ok"), port("b", "false", "no"), port("ok", "out", "a")];
        assert_eq!(main_path(&f), ["a", "b", "ok"], "sin Inicio, sin ramas de error y sin repetir en un ciclo");
    }

    #[test]
    fn carriles_por_sistema() {
        let mut f = Flow { nodes: vec![node("s", "start"), node("a", "request"), node("b", "condition"), node("ok", "response"), node("x", "response")], ..Default::default() };
        f.nodes[1].req_ref = "Cuentas/Consultar.yml".into();
        f.edges = vec![edge("s", "a"), edge("a", "b"), edge("b", "ok"), edge("a", "x")];
        let (lanes, _) = system_layout(&f);
        assert_eq!(lanes.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(), ["Martino", "Cuentas", "Respuesta"]);
        assert!(!in_system_lanes(&f), "todo en 0,0: fuera de sus carriles");
        arrange_systems(&mut f);
        assert!(in_system_lanes(&f));
        let p = |id: &str| { let n = f.node(id).unwrap(); (n.x, n.y) };
        assert_eq!((p("a"), p("x"), p("ok")), ((450.0, 190.0), (720.0, 350.0), (990.0, 350.0)), "una columna por distancia, una fila por sistema");
        f.nodes[1].y = 0.0;
        assert!(!in_system_lanes(&f), "mover un paso fuera de su carril apaga los carriles");
    }
}
