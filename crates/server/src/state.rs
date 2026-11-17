//! Estado compartido del servidor (y de la CLI de flujos).
use crate::config::Config;
use crate::executor::Executor;
use crate::flows::FlowStore;
use crate::store::{enabled, env_pairs, secret_pairs, Store};
use martino_core::model::Request;
use martino_core::vars::{Scope, VarSource};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

/// La carpeta de trabajo activa. Abrir otra carpeta reemplaza el `Core` entero; los handlers
/// siguen pidiendo `State<Arc<Core>>` y reciben siempre el actual.
#[derive(Clone)]
pub struct Hub(pub Arc<RwLock<Arc<Core>>>);
impl axum::extract::FromRef<Hub> for Arc<Core> {
    fn from_ref(h: &Hub) -> Arc<Core> {
        h.0.read().unwrap().clone()
    }
}

pub struct Core {
    pub cfg: RwLock<Config>,
    pub store: Store,
    pub flows: FlowStore,
    pub exec: Executor,
    /// Variables runtime (las fijan vars:post-response y el usuario). Viven en memoria.
    pub runtime: RwLock<Vec<(String, String)>>,
    pub events: broadcast::Sender<String>,
    pub runs: Mutex<HashMap<String, CancellationToken>>,
}

impl Core {
    pub fn new(cfg: Config) -> Arc<Core> {
        let store = Store::new(cfg.collections_dir.clone(), cfg.state_dir.clone());
        let flows = FlowStore::new(cfg.flows_dir.clone());
        let exec = Executor::new(&cfg.settings);
        Arc::new(Core {
            cfg: RwLock::new(cfg),
            store,
            flows,
            exec,
            runtime: Default::default(),
            events: broadcast::channel(256).0,
            runs: Default::default(),
        })
    }

    /// Capas de variables en orden de precedencia.
    pub fn scope(&self, col: &str, env: Option<&str>, req: Option<&Request>) -> Scope {
        let mut s = Scope::default();
        s.push(VarSource::Runtime, self.runtime.read().unwrap().clone());
        if let Some(r) = req {
            s.push(VarSource::Request, enabled(&r.vars_pre));
        }
        if let Some(e) = env.filter(|e| !e.is_empty()).and_then(|e| self.store.environment(col, e).ok()) {
            s.push(VarSource::Env, env_pairs(&e));
            s.push(VarSource::Secret, secret_pairs(&e));
        }
        if let Ok(root) = self.store.root(col) {
            s.push(VarSource::Collection, enabled(&root.vars_pre));
        }
        s.push(VarSource::Dotenv, self.store.dotenv(col));
        s
    }

    pub fn set_runtime(&self, k: &str, v: Option<String>) {
        let mut rt = self.runtime.write().unwrap();
        rt.retain(|x| x.0 != k);
        if let Some(v) = v {
            rt.push((k.into(), v));
        }
    }

    /// Publica un evento en `/api/events`.
    pub fn emit(&self, v: Value) {
        let _ = self.events.send(v.to_string());
    }

    pub fn start_run(&self) -> (String, CancellationToken) {
        let id = uuid::Uuid::new_v4().to_string();
        let t = CancellationToken::new();
        self.runs.lock().unwrap().insert(id.clone(), t.clone());
        (id, t)
    }

    pub fn end_run(&self, id: &str) {
        self.runs.lock().unwrap().remove(id);
    }
}
