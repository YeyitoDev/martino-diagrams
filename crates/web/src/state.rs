//! Estado global de la UI. Todo son señales `Copy`, así que se pasa por contexto.
use crate::api;
use leptos::prelude::*;
use leptos::task::spawn_local;
use martino_core::flow::{Flow, NodeOut};
use martino_core::model::Request;
use martino_core::vars::VarInfo;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TreeItem {
    pub name: String,
    pub path: String,
    pub folder: bool,
    pub method: String,
    pub id: String,
    pub items: Vec<TreeItem>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Collection {
    pub id: String,
    pub name: String,
    pub items: Vec<TreeItem>,
    pub environments: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Requests,
    Flows,
}

/// Un panel de petición dentro de una pantalla.
#[derive(Clone)]
pub struct Panel {
    pub id: String,
    pub collection: RwSignal<String>,
    pub path: RwSignal<Option<String>>,
    pub req: RwSignal<Request>,
    pub dirty: RwSignal<bool>,
    pub result: RwSignal<Option<Value>>,
    pub running: RwSignal<bool>,
    pub tab: RwSignal<&'static str>,
    pub resp_tab: RwSignal<&'static str>,
    /// fracción de alto para la petición (el resto es la respuesta)
    pub split: RwSignal<f64>,
    /// ancho en px (0 = reparte)
    pub width: RwSignal<f64>,
    /// variables usadas que no existen en el entorno activo
    pub missing: RwSignal<Vec<String>>,
    /// Temporal: formulario «Guardar en la colección» abierto.
    pub saving: RwSignal<bool>,
}

impl Panel {
    pub fn new(collection: &str, path: Option<String>, req: Request) -> Panel {
        in_root(|| Panel {
            id: uid(),
            collection: RwSignal::new(collection.to_string()),
            path: RwSignal::new(path),
            req: RwSignal::new(req),
            dirty: RwSignal::new(false),
            result: RwSignal::new(None),
            running: RwSignal::new(false),
            tab: RwSignal::new("params"),
            resp_tab: RwSignal::new("body"),
            split: RwSignal::new(0.45),
            width: RwSignal::new(0.0),
            missing: RwSignal::new(vec![]),
            saving: RwSignal::new(false),
        })
    }
}

#[derive(Clone)]
pub struct Screen {
    pub id: String,
    pub name: RwSignal<String>,
    pub panels: RwSignal<Vec<Panel>>,
}

impl Screen {
    pub fn new(name: &str) -> Screen {
        in_root(|| Screen { id: uid(), name: RwSignal::new(name.into()), panels: RwSignal::new(vec![]) })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Modal {
    /// Gestor de variables; `focus` resalta una variable (Ctrl+clic).
    Env(Option<String>),
    Settings,
    /// Carpeta de trabajo (la «base de datos» de cada persona).
    Folder,
    /// Pegar cURL; `Some(panel)` permite reemplazar ese panel. `folder` = destino al guardar.
    Curl { text: String, folder: Option<String> },
    CurlOut(String),
    Import,
    Help,
    Confirm { text: String, action: ConfirmAction },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConfirmAction {
    DeleteCollection(String),
    DeleteRequest(String, String),
    DeleteFlow(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tip {
    pub x: f64,
    pub y: f64,
    pub name: String,
}

#[derive(Clone, Copy)]
pub struct App {
    pub mode: RwSignal<Mode>,
    pub theme: RwSignal<String>,
    pub collections: RwSignal<Vec<Collection>>,
    pub collection: RwSignal<String>,
    pub env: RwSignal<String>,
    pub vars: RwSignal<Vec<VarInfo>>,
    /// Flujo abierto + últimos resultados (para colorear y mostrar `{{nodo.x}}`).
    pub flow_ctx: RwSignal<Option<(Flow, HashMap<String, NodeOut>)>>,
    pub screens: RwSignal<Vec<Screen>>,
    pub screen: RwSignal<String>,
    pub active_panel: RwSignal<String>,
    pub sidebar: RwSignal<bool>,
    pub sidebar_w: RwSignal<f64>,
    pub modal: RwSignal<Option<Modal>>,
    pub toasts: RwSignal<Vec<(String, &'static str, String)>>,
    pub tip: RwSignal<Option<Tip>>,
    pub reveal: RwSignal<bool>,
    pub flow_open: RwSignal<Option<String>>,
    /// Aumenta cuando cambia algo que obliga a recargar la tabla de variables.
    pub vars_tick: RwSignal<u32>,
    pub loaded: RwSignal<bool>,
    /// El flujo abierto tiene cambios sin guardar (lo publica el editor para la barra lateral).
    pub flow_dirty: RwSignal<bool>,
    /// Sube con «Guardar todo»: el editor de flujos guarda el suyo.
    pub save_all: RwSignal<u32>,
    /// Barra lateral: qué colecciones/carpetas están cerradas (sobrevive a las recargas) y qué tipo se ve.
    pub side_closed: RwSignal<HashSet<String>>,
    pub side_kind: RwSignal<&'static str>,
}

pub fn uid() -> String {
    format!("{:x}", (js_sys::Math::random() * 1e12) as u64)
}

thread_local! {
    static APP: std::cell::OnceCell<App> = const { std::cell::OnceCell::new() };
    static ROOT: std::cell::OnceCell<Owner> = const { std::cell::OnceCell::new() };
}

/// Crea señales que viven tanto como la app. Los eventos corren con el dueño del componente
/// que los dibujó (p. ej. un modal): lo creado ahí se destruiría al cerrarlo.
pub fn in_root<T>(f: impl FnOnce() -> T) -> T {
    ROOT.with(|r| match r.get() {
        Some(o) => o.with(f),
        None => f(),
    })
}

/// Global y no por contexto: los listeners del documento y los callbacks async corren sin
/// dueño reactivo, y `expect_context` ahí revienta.
pub fn app() -> App {
    APP.with(|a| *a.get().expect("App::new() todavía no se llamó"))
}

pub fn local_get(k: &str) -> Option<String> {
    web_sys::window()?.local_storage().ok()??.get_item(k).ok()?
}
pub fn local_set(k: &str, v: &str) {
    if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = s.set_item(k, v);
    }
}

impl App {
    pub fn new() -> App {
        ROOT.with(|r| { r.get_or_init(|| Owner::current().unwrap_or_default()); });
        let first = Screen::new("Pantalla 1");
        let app = App {
            mode: RwSignal::new(Mode::Requests),
            theme: RwSignal::new(local_get("martino.theme").unwrap_or_else(|| "dark".into())),
            collections: RwSignal::new(vec![]),
            collection: RwSignal::new(String::new()),
            env: RwSignal::new(String::new()),
            vars: RwSignal::new(vec![]),
            flow_ctx: RwSignal::new(None),
            screen: RwSignal::new(first.id.clone()),
            screens: RwSignal::new(vec![first]),
            active_panel: RwSignal::new(String::new()),
            sidebar: RwSignal::new(true),
            sidebar_w: RwSignal::new(270.0),
            modal: RwSignal::new(None),
            toasts: RwSignal::new(vec![]),
            tip: RwSignal::new(None),
            reveal: RwSignal::new(false),
            flow_open: RwSignal::new(None),
            vars_tick: RwSignal::new(0),
            loaded: RwSignal::new(false),
            flow_dirty: RwSignal::new(false),
            save_all: RwSignal::new(0),
            side_closed: RwSignal::new(HashSet::new()),
            side_kind: RwSignal::new("todo"),
        };
        APP.with(|a| a.set(app).ok());
        app
    }

    pub fn toast(&self, kind: &'static str, msg: impl Into<String>) {
        let id = uid();
        let toasts = self.toasts;
        toasts.update(|t| t.push((id.clone(), kind, msg.into())));
        set_timeout(move || toasts.update(|t| t.retain(|x| x.0 != id)), std::time::Duration::from_millis(if kind == "err" { 6000 } else { 3000 }));
    }

    pub fn err(&self, e: impl Into<String>) {
        self.toast("err", e)
    }

    pub fn current_screen(&self) -> Option<Screen> {
        let id = self.screen.get();
        self.screens.with(|s| s.iter().find(|x| x.id == id).cloned())
    }

    pub fn active(&self) -> Option<Panel> {
        let id = self.active_panel.get_untracked();
        self.screens.with_untracked(|ss| ss.iter().flat_map(|s| s.panels.get_untracked()).find(|p| p.id == id))
    }

    pub fn coll(&self) -> Option<Collection> {
        let id = self.collection.get();
        self.collections.with(|c| c.iter().find(|x| x.id == id).cloned())
    }

    pub fn reload_collections(&self) {
        let a = *self;
        spawn_local(async move {
            match api::get::<Vec<Collection>>("/api/collections").await {
                Ok(c) => {
                    if a.collection.get_untracked().is_empty() || !c.iter().any(|x| x.id == a.collection.get_untracked()) {
                        a.collection.set(c.first().map(|x| x.id.clone()).unwrap_or_default());
                    }
                    let envs = c.iter().find(|x| x.id == a.collection.get_untracked()).map(|x| x.environments.clone()).unwrap_or_default();
                    if !envs.contains(&a.env.get_untracked()) {
                        a.env.set(envs.first().cloned().unwrap_or_default());
                    }
                    a.collections.set(c);
                    a.vars_tick.update(|t| *t += 1);
                }
                Err(e) => a.err(e),
            }
        });
    }

    /// Tabla de variables del entorno activo (colorea los `{{…}}` y alimenta el autocompletado).
    pub fn refresh_vars(&self) {
        let a = *self;
        let (col, env) = (a.collection.get_untracked(), a.env.get_untracked());
        if col.is_empty() {
            a.vars.set(vec![]);
            return;
        }
        spawn_local(async move {
            if let Ok(v) = api::post::<Value>("/api/resolve", &json!({"collection": col, "env": env})).await {
                a.vars.set(serde_json::from_value(v["vars"].clone()).unwrap_or_default());
            }
        });
    }

    /// Abre una petición guardada en la pantalla actual: si ya está en ella la enfoca, si no crea un panel
    /// (aunque esté abierta en otra pantalla, para poder armar pantallas con los mismos endpoints).
    pub fn open_request(&self, col: String, path: String) {
        let a = *self;
        let existing = a.current_screen().and_then(|s| {
            s.panels.get_untracked().into_iter().find(|p| p.path.get_untracked().as_deref() == Some(&path) && p.collection.get_untracked() == col).map(|p| p.id)
        });
        a.mode.set(Mode::Requests);
        if let Some(pid) = existing {
            a.active_panel.set(pid);
            return;
        }
        spawn_local(async move {
            match api::get::<Request>(&format!("/api/request?collection={}&path={}", api::enc(&col), api::enc(&path))).await {
                Ok(mut r) => {
                    if martino_core::interp::looks_minified_json(&r.body.json) {
                        r.body.json = martino_core::interp::beautify_json(&r.body.json);
                    }
                    a.add_panel(Panel::new(&col, Some(path), r));
                }
                Err(e) => a.err(e),
            }
        });
    }

    pub fn add_panel(&self, p: Panel) {
        let id = p.id.clone();
        if let Some(s) = self.current_screen() {
            s.panels.update(|ps| ps.push(p));
        }
        self.active_panel.set(id);
    }

    /// Peticiones abiertas con cambios sin guardar (sólo las que ya están en la colección).
    pub fn dirty_panels(&self) -> Vec<Panel> {
        self.screens.with(|ss| ss.iter().flat_map(|s| s.panels.get()).filter(|p| p.dirty.get() && p.path.get().is_some()).collect())
    }

    /// Guarda todas las peticiones con cambios y el flujo abierto.
    pub fn save_everything(&self) {
        self.dirty_panels().iter().for_each(|p| self.save_panel(p));
        if self.flow_dirty.get_untracked() {
            self.save_all.update(|n| *n += 1);
        }
    }

    pub fn toggle_side(&self, key: &str) {
        self.side_closed.update(|s| if !s.remove(key) { s.insert(key.to_string()); });
    }

    pub fn save_panel(&self, p: &Panel) {
        let a = *self;
        let p = p.clone();
        let col = p.collection.get_untracked();
        let req = p.req.get_untracked();
        // Una temporal se guarda eligiendo carpeta e ID (temporales.rs).
        let Some(path) = p.path.get_untracked() else {
            p.saving.set(true);
            return;
        };
        spawn_local(async move {
            let res = api::put::<Value>(&format!("/api/request?collection={}&path={}", api::enc(&col), api::enc(&path)), &req).await.map(|_| ());
            match res {
                Ok(_) => {
                    p.dirty.set(false);
                    a.toast("ok", "Guardado");
                }
                Err(e) => a.err(e),
            }
        });
    }

    /// Ejecuta varios paneles en paralelo por /api/run (NDJSON) y pinta cada resultado al llegar.
    pub fn run_panels(&self, panels: Vec<Panel>) {
        if panels.is_empty() {
            return;
        }
        let a = *self;
        let env = a.env.get_untracked();
        let items: Vec<Value> = panels
            .iter()
            .map(|p| {
                p.running.set(true);
                json!({"id": p.id, "collection": p.collection.get_untracked(), "request": p.req.get_untracked(), "env": env})
            })
            .collect();
        spawn_local(async move {
            let ps = panels.clone();
            let r = api::stream("POST", "/api/run", Some(&json!({"items": items})), |line| {
                if line["type"] == "result" {
                    if let Some(p) = ps.iter().find(|p| line["id"] == json!(p.id)) {
                        p.running.set(false);
                        p.result.set(Some(line["result"].clone()));
                        if p.resp_tab.get_untracked() == "asserts" && line["result"]["asserts"].as_array().is_none_or(|x| x.is_empty()) {
                            p.resp_tab.set("body");
                        }
                    }
                }
            })
            .await;
            for p in &panels {
                p.running.set(false);
            }
            if let Err(e) = r {
                a.err(e);
            }
        });
    }

    /// Lo que se guarda en `.workbench/workspaces.json`.
    pub fn workspace_json(&self) -> Value {
        let screens: Vec<Value> = self.screens.with_untracked(|ss| {
            ss.iter()
                .map(|s| {
                    let panels: Vec<Value> = s
                        .panels
                        .get_untracked()
                        .iter()
                        .map(|p| {
                            let path = p.path.get_untracked();
                            let mut v = json!({"collection": p.collection.get_untracked(), "path": path, "width": p.width.get_untracked(), "split": p.split.get_untracked()});
                            if path.is_none() || p.dirty.get_untracked() {
                                v["request"] = serde_json::to_value(p.req.get_untracked()).unwrap();
                            }
                            v
                        })
                        .collect();
                    json!({"name": s.name.get_untracked(), "panels": panels})
                })
                .collect()
        });
        let active = self.screens.with_untracked(|ss| ss.iter().position(|s| s.id == self.screen.get_untracked()).unwrap_or(0));
        json!({"screens": screens, "active": active, "collection": self.collection.get_untracked(), "env": self.env.get_untracked(),
               "mode": if self.mode.get_untracked() == Mode::Flows { "flows" } else { "requests" }, "flow": self.flow_open.get_untracked(),
               "sidebar": self.sidebar.get_untracked(), "sidebarW": self.sidebar_w.get_untracked()})
    }

    /// Devuelve false si el servidor no respondió: entonces no hay que guardar encima (se perdería el workspace).
    pub async fn restore_workspace(&self) -> bool {
        let Ok(w) = api::get::<Value>("/api/workspaces").await else { return false };
        if let Some(c) = w["collection"].as_str() {
            self.collection.set(c.into());
        }
        if let Some(e) = w["env"].as_str() {
            self.env.set(e.into());
        }
        if w["mode"] == "flows" {
            self.mode.set(Mode::Flows);
        }
        self.flow_open.set(w["flow"].as_str().map(String::from));
        self.sidebar.set(w["sidebar"].as_bool().unwrap_or(true));
        if let Some(x) = w["sidebarW"].as_f64() {
            self.sidebar_w.set(x);
        }
        let mut screens = vec![];
        for s in w["screens"].as_array().cloned().unwrap_or_default() {
            let sc = Screen::new(s["name"].as_str().unwrap_or("Pantalla"));
            let mut panels = vec![];
            for p in s["panels"].as_array().cloned().unwrap_or_default() {
                let col = p["collection"].as_str().unwrap_or_default().to_string();
                let path = p["path"].as_str().map(String::from);
                let (req, dirty) = match (serde_json::from_value::<Request>(p["request"].clone()), &path) {
                    (Ok(r), _) if !p["request"].is_null() => (r, path.is_some()),
                    (_, Some(path)) => match api::get::<Request>(&format!("/api/request?collection={}&path={}", api::enc(&col), api::enc(path))).await {
                        Ok(r) => (r, false),
                        Err(_) => continue,
                    },
                    _ => continue,
                };
                let panel = Panel::new(&col, path, req);
                panel.dirty.set(dirty);
                panel.width.set(p["width"].as_f64().unwrap_or(0.0));
                panel.split.set(p["split"].as_f64().unwrap_or(0.45));
                panels.push(panel);
            }
            sc.panels.set(panels);
            screens.push(sc);
        }
        if !screens.is_empty() {
            let idx = w["active"].as_u64().unwrap_or(0) as usize;
            self.screen.set(screens.get(idx).unwrap_or(&screens[0]).id.clone());
            if let Some(p) = screens.get(idx).and_then(|s| s.panels.get_untracked().first().cloned()) {
                self.active_panel.set(p.id);
            }
            self.screens.set(screens);
        }
        true
    }

    /// Escucha `/api/events` y se reconecta si se corta.
    pub fn listen_events(&self) {
        let a = *self;
        spawn_local(async move {
            let _ = api::stream("GET", "/api/events", None::<&()>, |v| match v["type"].as_str() {
                Some("collections") => a.reload_collections(),
                Some("runtime") => a.vars_tick.update(|t| *t += 1),
                _ => {}
            })
            .await;
            set_timeout(move || a.listen_events(), std::time::Duration::from_secs(2));
        });
    }
}
