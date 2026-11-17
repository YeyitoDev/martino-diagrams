//! Precedencia de variables y de dónde sale cada una (para resolver y para colorear en la UI).
//! runtime > vars:pre-request > entorno (secretos incluidos) > collection.bru > .env
use crate::interp::DYNAMIC_VARS;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VarSource {
    Runtime,
    Request,
    Env,
    Secret,
    Collection,
    Dotenv,
    Dynamic,
    Flow,
    Missing,
}

impl VarSource {
    /// Clase CSS que pinta la variable (`.v-env`, `.v-missing`…).
    pub fn css(self) -> &'static str {
        match self {
            VarSource::Runtime => "v-runtime",
            VarSource::Request => "v-request",
            VarSource::Env => "v-env",
            VarSource::Secret => "v-secret",
            VarSource::Collection => "v-collection",
            VarSource::Dotenv => "v-dotenv",
            VarSource::Dynamic => "v-dynamic",
            VarSource::Flow => "v-flow",
            VarSource::Missing => "v-missing",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            VarSource::Runtime => "runtime",
            VarSource::Request => "petición",
            VarSource::Env => "entorno",
            VarSource::Secret => "secreta",
            VarSource::Collection => "colección",
            VarSource::Dotenv => ".env",
            VarSource::Dynamic => "dinámica",
            VarSource::Flow => "flujo",
            VarSource::Missing => "sin definir",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VarInfo {
    pub name: String,
    pub value: String,
    pub source: VarSource,
    /// Capas de menor prioridad que también la definen (y quedan tapadas).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shadowed: Vec<VarSource>,
}

#[derive(Clone, Debug, Default)]
pub struct Scope {
    /// En orden de precedencia (la primera gana).
    pub layers: Vec<(VarSource, Vec<(String, String)>)>,
}

impl Scope {
    pub fn push(&mut self, src: VarSource, vars: impl IntoIterator<Item = (String, String)>) {
        self.layers.push((src, vars.into_iter().collect()));
    }

    pub fn lookup(&self, name: &str) -> Option<VarInfo> {
        let (name, only_dotenv) = match name.strip_prefix("process.env.") {
            Some(n) => (n, true),
            None => (name, false),
        };
        let mut hits = self
            .layers
            .iter()
            .filter(|(s, _)| !only_dotenv || *s == VarSource::Dotenv)
            .filter_map(|(s, vs)| vs.iter().rev().find(|(k, _)| k == name).map(|(_, v)| (*s, v)));
        let (source, value) = hits.next()?;
        Some(VarInfo { name: name.into(), value: value.clone(), source, shadowed: hits.map(|h| h.0).collect() })
    }

    pub fn get(&self, name: &str) -> Option<String> {
        self.lookup(name).map(|v| v.value)
    }

    /// Como `get`, pero si el valor usa `{{otra}}` también la resuelve (hasta 5 niveles).
    pub fn resolve(&self, name: &str) -> Option<String> {
        self.resolve_depth(name, 0)
    }

    fn resolve_depth(&self, name: &str, depth: usize) -> Option<String> {
        let v = self.get(name)?;
        if depth >= 5 || !v.contains("{{") {
            return Some(v);
        }
        Some(crate::interp::interpolate(&v, &|n| self.resolve_depth(n, depth + 1)))
    }

    /// Tabla completa ya resuelta (una fila por nombre).
    pub fn table(&self) -> Vec<VarInfo> {
        let mut seen = std::collections::HashSet::new();
        let mut out = vec![];
        for (_, vs) in &self.layers {
            for (k, _) in vs {
                if seen.insert(k.clone()) {
                    out.extend(self.lookup(k));
                }
            }
        }
        out
    }
}

/// De dónde sale `name` según la tabla de la UI. `flow_keys`: claves de nodos visibles.
pub fn classify(name: &str, table: &[VarInfo], flow_keys: &[String]) -> VarSource {
    if name.starts_with('$') {
        return if DYNAMIC_VARS.contains(&name) { VarSource::Dynamic } else { VarSource::Missing };
    }
    let bare = name.strip_prefix("process.env.").unwrap_or(name);
    if let Some(v) = table.iter().find(|v| v.name == bare) {
        return v.source;
    }
    let head = name.split(['.', '[']).next().unwrap_or("");
    if flow_keys.iter().any(|k| k == head) || (head == "input" && name.contains('.')) {
        return VarSource::Flow;
    }
    VarSource::Missing
}

#[cfg(test)]
mod tests {
    use super::*;
    fn kv(k: &str, v: &str) -> (String, String) {
        (k.into(), v.into())
    }
    #[test]
    fn precedence_and_shadowing() {
        let mut s = Scope::default();
        s.push(VarSource::Runtime, [kv("token", "rt")]);
        s.push(VarSource::Env, [kv("token", "env"), kv("url", "u")]);
        s.push(VarSource::Dotenv, [kv("url", "dot"), kv("KEY", "k")]);
        let t = s.lookup("token").unwrap();
        assert_eq!((t.value.as_str(), t.source, t.shadowed.clone()), ("rt", VarSource::Runtime, vec![VarSource::Env]));
        assert_eq!(s.get("process.env.url").as_deref(), Some("dot"));
        assert_eq!(s.get("KEY").as_deref(), Some("k"));
        assert_eq!(s.table().len(), 3);
        let keys = vec!["svc".to_string()];
        assert_eq!(classify("svc.response.id", &s.table(), &keys), VarSource::Flow);
        assert_eq!(classify("$guid", &[], &keys), VarSource::Dynamic);
        assert_eq!(classify("nada", &s.table(), &keys), VarSource::Missing);
    }
}
