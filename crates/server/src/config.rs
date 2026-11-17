//! `workbench.config.json` (versionado) + `workbench.local.json` (no versionado, sobrescribe).
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Tls {
    /// PEM con CA extra (p. ej. la del proxy corporativo).
    pub ca_file: String,
    pub reject_unauthorized: bool,
}
impl Default for Tls {
    fn default() -> Self {
        Tls { ca_file: String::new(), reject_unauthorized: true }
    }
}

/// Ajustes editables desde la UI.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub concurrency: usize,
    pub request_timeout_ms: u64,
    pub tls: Tls,
}
impl Default for Settings {
    fn default() -> Self {
        Settings { concurrency: 6, request_timeout_ms: 30_000, tls: Tls::default() }
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub root: PathBuf,
    pub port: u16,
    pub collections_dir: PathBuf,
    pub state_dir: PathBuf,
    pub flows_dir: PathBuf,
    pub settings: Settings,
    /// Modo `--public`: llave para entrar desde un túnel (*.trycloudflare.com o *.ts.net). Nunca se guarda en disco.
    pub public_key: Option<String>,
}

fn merge(a: &mut Value, b: Value) {
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                merge(a.entry(k).or_insert(Value::Null), v)
            }
        }
        (a, b) => *a = b,
    }
}

fn read_json(p: &Path) -> Value {
    std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!({}))
}

impl Config {
    /// Lee los dos archivos de `root`; `overrides` gana sobre ambos (tests y CLI).
    pub fn load(root: &Path, overrides: Value) -> Config {
        let mut v = json!({
            "port": 4400, "collectionsDir": "collections", "stateDir": ".workbench",
            "flowsDir": "flows", "settings": {}
        });
        merge(&mut v, read_json(&root.join("workbench.config.json")));
        merge(&mut v, read_json(&root.join("workbench.local.json")));
        if let Some(p) = std::env::var("WORKBENCH_PORT").ok().and_then(|p| p.parse::<u16>().ok()) {
            v["port"] = json!(p);
        }
        merge(&mut v, overrides);
        let dir = |k: &str| root.join(v[k].as_str().unwrap_or(k));
        Config {
            root: root.into(),
            port: v["port"].as_u64().unwrap_or(4400) as u16,
            collections_dir: dir("collectionsDir"),
            state_dir: dir("stateDir"),
            flows_dir: dir("flowsDir"),
            settings: serde_json::from_value(v["settings"].clone()).unwrap_or_default(),
            public_key: None,
        }
    }

    /// Guarda los ajustes en `workbench.local.json` (nunca en el archivo versionado).
    pub fn save_settings(&mut self, s: Settings) -> std::io::Result<()> {
        let path = self.root.join("workbench.local.json");
        let mut local = read_json(&path);
        local["settings"] = serde_json::to_value(&s).unwrap();
        std::fs::write(&path, serde_json::to_string_pretty(&local).unwrap())?;
        self.settings = s;
        Ok(())
    }
}

/// `~/.martino/recientes.json`: carpetas de trabajo abiertas, la última primero.
/// `MARTINO_HOME` la reubica (tests).
fn recent_file() -> Option<PathBuf> {
    let home = std::env::var_os("MARTINO_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(|h| PathBuf::from(h).join(".martino")))?;
    Some(home.join("recientes.json"))
}

pub fn recent() -> Vec<PathBuf> {
    let v: Vec<PathBuf> = recent_file().and_then(|f| serde_json::from_value(read_json(&f)).ok()).unwrap_or_default();
    v.into_iter().filter(|p| p.is_dir()).collect()
}

pub fn remember(dir: &Path) {
    let Some(f) = recent_file() else { return };
    let mut v = recent();
    v.retain(|p| p != dir);
    v.insert(0, dir.into());
    v.truncate(10);
    let _ = std::fs::create_dir_all(f.parent().unwrap());
    let _ = std::fs::write(f, serde_json::to_string_pretty(&v).unwrap());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_overrides_versioned() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("workbench.config.json"), r#"{"flowsDir":"f","settings":{"concurrency":3}}"#).unwrap();
        std::fs::write(d.path().join("workbench.local.json"), r#"{"settings":{"requestTimeoutMs":5}}"#).unwrap();
        let mut c = Config::load(d.path(), json!({"port": 1}));
        assert_eq!((c.port, c.settings.concurrency, c.settings.request_timeout_ms), (1, 3, 5));
        assert!(c.flows_dir.ends_with("f"));
        c.save_settings(Settings { concurrency: 9, ..c.settings.clone() }).unwrap();
        assert_eq!(Config::load(d.path(), json!({})).settings.concurrency, 9);
        assert!(!std::fs::read_to_string(d.path().join("workbench.config.json")).unwrap().contains('9'));
    }
}
