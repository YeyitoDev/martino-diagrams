//! Qué campos del resultado de un nodo usan los demás (para mostrar en el lienzo sólo lo que importa).
use crate::flow::{map_node_strings, ref_key, Flow};
use crate::interp::{segments, Seg};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq)]
pub struct Used {
    /// Ruta dentro del nodo, tal como se escribió (`response.email`, `status`, `id`).
    pub path: String,
    /// Nombre del nodo que lo usa.
    pub by: String,
    /// Lo usa una condición (si no, lo recibe otro nodo).
    pub cond: bool,
}

/// Para cada nodo, qué campos suyos usan los demás, en una sola pasada por el flujo: en plantillas
/// `{{key.x}}` y en rutas sin llaves (izquierda de una condición, valores de Extraer).
pub fn used_by_key(flow: &Flow) -> HashMap<String, Vec<Used>> {
    let keys: HashSet<&str> = flow.nodes.iter().map(|n| n.key.as_str()).collect();
    let mut out: HashMap<String, Vec<Used>> = HashMap::new();
    for n in &flow.nodes {
        let by = if n.name.is_empty() { n.key.clone() } else { n.name.clone() };
        let cond = n.kind == "condition";
        let mut refs: Vec<String> = vec![];
        map_node_strings(&mut [n.clone()], &mut |s| {
            refs.extend(segments(s).into_iter().filter_map(|seg| if let Seg::Var(r, _) = seg { Some(r.to_string()) } else { None }));
            s.to_string()
        });
        let bare = n.rules.iter().map(|r| &r.left).chain(n.vars.iter().filter(|_| n.kind == "extract").map(|p| &p.value));
        refs.extend(bare.filter(|s| !s.contains("{{")).cloned());
        for r in &refs {
            let r = r.trim().strip_prefix("nodes.").unwrap_or(r.trim());
            let key = ref_key(r);
            if key == n.key || !keys.contains(key) {
                continue;
            }
            // Ruta vacía = usa el resultado entero (`{{clave}}`).
            let path = r[key.len()..].trim_start_matches('.').to_string();
            let list = out.entry(key.to_string()).or_default();
            if !list.iter().any(|u| u.path == path) {
                list.push(Used { path, by: by.clone(), cond });
            }
        }
    }
    out
}

pub fn used_fields(flow: &Flow, key: &str) -> Vec<Used> {
    used_by_key(flow).remove(key).unwrap_or_default()
}

/// Cómo se muestra una ruta de servicio: sin el `response.` del principio; vacía = «(todo)».
pub fn short_path(path: &str) -> &str {
    match path {
        "" | "response" | "body" => "(todo)",
        p => p.strip_prefix("response.").or_else(|| p.strip_prefix("body.")).unwrap_or(p),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{Node, Rule};
    use crate::model::Pair;

    #[test]
    fn finds_templates_and_bare_paths() {
        let n = |key: &str, kind: &str| Node { id: key.into(), key: key.into(), name: key.into(), kind: kind.into(), ..Default::default() };
        let mut cond = n("existe", "condition");
        cond.rules = vec![Rule { left: "usuario.response.email".into(), op: "isDefined".into(), right: String::new() }];
        let mut pagos = n("pagos", "request");
        pagos.vars = vec![Pair::new("userId", "{{usuario.response.id}}"), Pair::new("x", "{{nodes.usuario.status}}")];
        let mut otro = n("otro", "extract");
        otro.vars = vec![Pair::new("v", "usuario.response.id")];
        let mut ok = n("ok", "response");
        ok.body = serde_json::json!({"id": "{{usuario.response.id}}", "nombre": "{{usuario.response.name}}"});
        let flow = Flow { nodes: vec![n("usuario", "request"), cond, pagos, otro, ok], ..Default::default() };
        let u = used_fields(&flow, "usuario");
        let paths: Vec<_> = u.iter().map(|u| (u.path.as_str(), u.cond)).collect();
        assert_eq!(paths, [("response.email", true), ("response.id", false), ("status", false), ("response.name", false)], "un `id` dentro del body también cuenta");
        assert_eq!(short_path("response.email"), "email");
        assert!(used_fields(&flow, "pagos").is_empty());
        assert_eq!(short_path(""), "(todo)");
    }
}
