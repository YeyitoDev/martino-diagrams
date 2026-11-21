//! Modo Flujos: lista de flujos, lienzo (nodos, conexiones, arrastre, zoom), ejecución e historial.
//! El panel derecho (inspector y código) vive en `flow_inspector.rs`.
use crate::api;
use crate::flow_inspector::Inspector;
use crate::requests::DND_REQ;
use crate::state::{app, uid, ConfirmAction, Modal, TreeItem};
use crate::ui::{copy, start_drag};
use leptos::prelude::*;
use leptos::task::spawn_local;
use martino_core::flow::{self, Edge, Flow, Node, NodeOut, Rule};
use martino_core::usage::{used_by_key, Used};
use serde_json::{json, Map, Value};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

pub const W: f64 = 200.0;
pub const H: f64 = 62.0;

pub fn label(kind: &str) -> &'static str {
    match kind {
        "start" => "Inicio",
        "input" => "Entrada",
        "request" => "Servicio",
        "extract" => "Extraer",
        "condition" => "Condición",
        "data" => "Datos fijos",
        "transform" => "Transformar",
        "crypto" => "Cifrado",
        "lookup" => "Equivalencia",
        "response" => "Respuesta",
        "end" => "Fin",
        "note" => "Nota",
        "image" => "Imagen",
        _ => "Nodo",
    }
}

fn port_label(p: &str) -> &'static str {
    match p {
        "error" => "error",
        "true" => "sí",
        "false" => "no",
        _ => "",
    }
}

/// Centro del puerto de salida. El `false` de una Condición sale por abajo; el resto, a la derecha.
/// Identificador visible del paso: un Servicio muestra el ID de su petición (PAG-001); el resto,
/// su código fijo (CND-01), que se asigna al crearlo.
pub fn node_code(f: &Flow, n: &Node) -> String {
    if n.kind == "request" && !n.req_ref.is_empty() {
        let id = app().collections.with(|cs| cs.iter().find(|c| c.id == f.collection).and_then(|c| find_id(&c.items, &n.req_ref)));
        if let Some(id) = id.filter(|i| !i.is_empty()) {
            return id;
        }
    }
    if n.sid.is_empty() { martino_core::layout::id_prefix(&n.kind).into() } else { n.sid.clone() }
}

fn find_id(items: &[TreeItem], path: &str) -> Option<String> {
    items.iter().find_map(|i| if i.folder { find_id(&i.items, path) } else { (i.path == path).then(|| i.id.clone()) })
}

/// Ancho del nodo: el que le diste al redimensionarlo o el de siempre.
pub fn node_w(n: &Node) -> f64 {
    n.width.unwrap_or(W)
}

pub fn port_pos(n: &Node, port: &str) -> (f64, f64) {
    if n.kind == "condition" && port == "false" {
        return (n.x + node_w(n) / 2.0, n.y + H);
    }
    let side: Vec<&str> = flow::ports(&n.kind).iter().copied().filter(|p| !(n.kind == "condition" && *p == "false")).collect();
    let i = side.iter().position(|p| *p == port).unwrap_or(0) as f64;
    (n.x + node_w(n), n.y + H * (i + 1.0) / (side.len() as f64 + 1.0))
}

fn has_input(kind: &str) -> bool {
    !matches!(kind, "start" | "note" | "image")
}

fn curve(x1: f64, y1: f64, x2: f64, y2: f64, down: bool) -> String {
    let d = ((x2 - x1).abs() / 2.0).max(50.0);
    let (c1x, c1y) = if down { (x1, y1 + d) } else { (x1 + d, y1) };
    format!("M{x1},{y1} C{c1x},{c1y} {},{y2} {x2},{y2}", x2 - d)
}

/// Peticiones (.yml o .bru) de una colección, aplanadas: (ruta, nombre).
pub fn bru_paths(items: &[TreeItem]) -> Vec<(String, String)> {
    items.iter().flat_map(|i| if i.folder { bru_paths(&i.items) } else { vec![(i.path.clone(), i.name.clone())] }).collect()
}

pub fn new_node(kind: &str, f: &Flow, x: f64, y: f64) -> Node {
    let base = match kind {
        "input" => "entrada",
        "request" => "servicio",
        "condition" => "condicion",
        "data" => "datos",
        "transform" => "transformar",
        "crypto" => "cifrado",
        "lookup" => "equivalencia",
        "response" => "respuesta",
        "extract" => "extraer",
        "end" => "fin",
        "start" => "inicio",
        _ => "nota",
    };
    let keys: Vec<String> = f.nodes.iter().map(|n| n.key.clone()).collect();
    let sid = martino_core::layout::next_code(f, kind);
    let mut n = Node { id: uid(), kind: kind.into(), key: flow::unique_key(base, &keys), name: label(kind).into(), sid, x, y, ..Default::default() };
    match kind {
        "input" | "data" => n.data = json!({}),
        "condition" => {
            n.mode = "all".into();
            n.rules = vec![Rule { left: String::new(), op: "eq".into(), right: String::new() }];
        }
        "response" => {
            n.status = "200".into();
            n.body = json!({});
        }
        "transform" => n.code = "// nodes, input y env están disponibles\nreturn { status: 200, body: input };".into(),
        "note" => n.text = "Escribe aquí".into(),
        "crypto" => n.crypto = Some(Default::default()),
        "lookup" => n.lookup = Some(Default::default()),
        _ => {}
    }
    n
}

#[derive(Clone, Copy, PartialEq)]
pub enum Pane {
    Canvas,
    /// El mismo flujo como pasos numerados por etapa.
    Story,
    Qa,
    Tables,
}

/// Estado del editor. Todo son señales, así que es `Copy` y va por contexto.
#[derive(Clone, Copy)]
pub struct Ed {
    pub id: RwSignal<Option<String>>,
    pub flow: RwSignal<Flow>,
    pub dirty: RwSignal<bool>,
    pub sel: RwSignal<Option<String>>,
    pub sel_edge: RwSignal<Option<usize>>,
    /// Salidas de la última ejecución, por clave (lo que leen las plantillas).
    pub outs: RwSignal<HashMap<String, NodeOut>>,
    /// Paso de la última ejecución, por id de nodo.
    pub steps: RwSignal<HashMap<String, Value>>,
    pub state: RwSignal<HashMap<String, &'static str>>,
    pub vars: RwSignal<Map<String, Value>>,
    pub result: RwSignal<Option<Value>>,
    pub running: RwSignal<bool>,
    pub compare: RwSignal<bool>,
    pub list: RwSignal<Vec<Value>>,
    pub code: RwSignal<bool>,
    /// Sube cuando el flujo cambia "desde fuera" (deshacer, código, abrir): el inspector se rehace.
    pub rev: RwSignal<u32>,
    pub view: RwSignal<(f64, f64, f64)>,
    /// Qué se ve en el cuerpo: el lienzo, los casos QA o las tablas.
    pub pane: RwSignal<Pane>,
    /// Último resultado de cada caso QA, por id.
    pub qa_res: RwSignal<HashMap<String, Value>>,
    /// Tablas de la colección del flujo (nombre, tabla) y de qué colección son.
    pub table_list: RwSignal<Vec<(String, martino_core::lookup::Table)>>,
    pub tables_of: RwSignal<String>,
    /// Qué campos de cada nodo usan los demás (para la respuesta recortada).
    pub used: Memo<HashMap<String, Vec<Used>>>,
    /// Qué respuesta muestran los nodos en el lienzo: usado | todo | nada.
    pub resp: RwSignal<&'static str>,
    /// Panel derecho (inspector) visible.
    pub insp: RwSignal<bool>,
    /// Columna de servicios (selección rápida) visible.
    pub svc: RwSignal<bool>,
    /// Texto del buscador del lienzo: resalta los pasos que coinciden.
    pub find: RwSignal<String>,
    /// Último paso que llegó de la ejecución (lo cuenta el narrador).
    pub last_step: RwSignal<Option<String>>,
    undo: StoredValue<Vec<Flow>>,
    redo: StoredValue<Vec<Flow>>,
    last: StoredValue<f64>,
}

pub fn ed() -> Ed {
    expect_context::<Ed>()
}

impl Ed {
    fn new() -> Ed {
        let flow = RwSignal::new(Flow::default());
        Ed {
            id: RwSignal::new(None),
            flow,
            dirty: RwSignal::new(false),
            sel: RwSignal::new(None),
            sel_edge: RwSignal::new(None),
            outs: RwSignal::new(HashMap::new()),
            steps: RwSignal::new(HashMap::new()),
            state: RwSignal::new(HashMap::new()),
            vars: RwSignal::new(Map::new()),
            result: RwSignal::new(None),
            running: RwSignal::new(false),
            compare: RwSignal::new(false),
            list: RwSignal::new(vec![]),
            code: RwSignal::new(false),
            rev: RwSignal::new(0),
            view: RwSignal::new((40.0, 40.0, 1.0)),
            pane: RwSignal::new(Pane::Canvas),
            used: Memo::new(move |_| flow.with(used_by_key)),
            qa_res: RwSignal::new(HashMap::new()),
            table_list: RwSignal::new(vec![]),
            tables_of: RwSignal::new(String::new()),
            resp: RwSignal::new("usado"),
            insp: RwSignal::new(true),
            svc: RwSignal::new(false),
            find: RwSignal::new(String::new()),
            last_step: RwSignal::new(None),
            undo: StoredValue::new(vec![]),
            redo: StoredValue::new(vec![]),
            last: StoredValue::new(0.0),
        }
    }

    /// Guarda una foto para deshacer; las ráfagas de menos de 300 ms van en una sola.
    pub fn snapshot(&self) {
        let now = js_sys::Date::now();
        if now - self.last.get_value() > 300.0 {
            let cur = self.flow.get_untracked();
            self.undo.update_value(|u| {
                u.push(cur);
                if u.len() > 100 {
                    u.remove(0);
                }
            });
            self.redo.update_value(|r| r.clear());
        }
        self.last.set_value(now);
    }

    pub fn edit(&self, f: impl FnOnce(&mut Flow)) {
        self.snapshot();
        self.flow.update(f);
        self.dirty.set(true);
    }

    pub fn edit_node(&self, id: &str, f: impl FnOnce(&mut Node)) {
        let id = id.to_string();
        self.edit(move |fl| {
            if let Some(n) = fl.nodes.iter_mut().find(|n| n.id == id) {
                f(n)
            }
        });
    }

    /// Reemplaza el flujo entero (código, deshacer) y rehace el inspector.
    pub fn replace(&self, f: Flow) {
        self.edit(|cur| *cur = f);
        self.rev.update(|r| *r += 1);
    }

    fn travel(&self, back: bool) {
        let (from, to) = if back { (self.undo, self.redo) } else { (self.redo, self.undo) };
        let mut prev = None;
        from.update_value(|v| prev = v.pop());
        let Some(prev) = prev else { return };
        to.update_value(|v| v.push(self.flow.get_untracked()));
        self.flow.set(prev);
        self.dirty.set(true);
        self.last.set_value(0.0);
        self.rev.update(|r| *r += 1);
    }

    pub fn delete_selected(&self) {
        if let Some(id) = self.sel.get_untracked() {
            self.edit(|f| {
                f.nodes.retain(|n| n.id != id);
                f.edges.retain(|e| e.from != id && e.to != id);
            });
            self.sel.set(None);
        } else if let Some(i) = self.sel_edge.get_untracked() {
            self.edit(|f| {
                if i < f.edges.len() {
                    f.edges.remove(i);
                }
            });
            self.sel_edge.set(None);
        }
    }

    fn clear_run(&self) {
        self.outs.set(HashMap::new());
        self.steps.set(HashMap::new());
        self.state.set(HashMap::new());
        self.vars.set(Map::new());
        self.result.set(None);
    }

    fn load(&self, id: Option<String>, mut f: Flow) {
        let changed = flow::modernize_refs(&mut f);
        // Flujos de antes: cada paso recibe su código fijo; se guarda con el próximo guardado.
        martino_core::layout::assign_codes(&mut f);
        // El flujo corre con las peticiones y entornos de su colección: se activa al abrirlo
        // (el entorno se ajusta solo si no existe en ella).
        if !f.collection.is_empty() && app().collection.get_untracked() != f.collection {
            app().collection.set(f.collection.clone());
        }
        self.id.set(id.clone());
        self.flow.set(f);
        self.dirty.set(changed > 0);
        self.sel.set(None);
        self.sel_edge.set(None);
        self.code.set(false);
        self.undo.set_value(vec![]);
        self.redo.set_value(vec![]);
        self.clear_run();
        self.qa_res.set(HashMap::new());
        self.rev.update(|r| *r += 1);
        self.fit();
        app().flow_open.set(id);
        if changed > 0 {
            app().toast("ok", format!("Actualicé {changed} referencias a la sintaxis nueva (Ctrl+S para guardar)"));
        }
    }

    /// Recarga la lista y abre `want`, el flujo recordado o el primero.
    pub fn refresh(&self, want: Option<String>) {
        let ed = *self;
        spawn_local(async move {
            let list = api::get::<Vec<Value>>("/api/flows").await.unwrap_or_default();
            let ids: Vec<String> = list.iter().filter_map(|f| f["id"].as_str().map(String::from)).collect();
            ed.list.set(list);
            let pick = want.or_else(|| app().flow_open.get_untracked()).filter(|id| ids.contains(id)).or_else(|| ids.first().cloned());
            match pick {
                Some(id) => ed.open(id).await,
                None => ed.load(None, Flow::default()),
            }
        });
    }

    async fn open(&self, id: String) {
        match api::get::<Flow>(&format!("/api/flow?id={}", api::enc(&id))).await {
            Ok(f) => self.load(Some(id), f),
            Err(e) => app().err(e),
        }
    }

    /// Guarda y devuelve si salió bien (el servidor valida claves y conexiones).
    pub async fn save(&self) -> bool {
        let Some(id) = self.id.get_untracked() else { return false };
        match api::put::<Value>(&format!("/api/flow?id={}", api::enc(&id)), &self.flow.get_untracked()).await {
            Ok(_) => {
                self.dirty.set(false);
                let name = self.flow.with_untracked(|f| f.name.clone());
                self.list.update(|l| l.iter_mut().filter(|f| f["id"] == json!(id)).for_each(|f| f["name"] = json!(name)));
                true
            }
            Err(e) => {
                app().err(format!("No se guardó: {e}"));
                false
            }
        }
    }

    async fn create(&self, f: Flow) {
        match api::post::<Value>("/api/flows", &f).await {
            Ok(v) => self.refresh(v["id"].as_str().map(String::from)),
            Err(e) => app().err(e),
        }
    }

    pub fn run(&self) {
        let (ed, a) = (*self, app());
        if ed.running.get_untracked() {
            return;
        }
        ed.clear_run();
        ed.running.set(true);
        let body = json!({"flow": ed.flow.get_untracked(), "env": a.env.get_untracked(), "compare": ed.compare.get_untracked()});
        spawn_local(async move {
            let r = api::stream("POST", "/api/flow/run", Some(&body), move |v| match v["type"].as_str() {
                Some("node-start") => {
                    let id = v["id"].as_str().unwrap_or("").to_string();
                    ed.state.update(|s| {
                        s.insert(id, "running");
                    });
                }
                Some("node") => ed.apply_step(&v["step"]),
                Some("done") => {
                    let r = &v["result"];
                    ed.vars.set(r["vars"].as_object().cloned().unwrap_or_default());
                    ed.result.set(Some(r.clone()));
                }
                _ => {}
            })
            .await;
            ed.running.set(false);
            if let Err(e) = r {
                a.err(e);
            }
        });
    }

    /// Pinta en el lienzo el recorrido de un resultado ya obtenido (p. ej. de un caso QA).
    pub fn show_result(&self, r: &Value) {
        self.clear_run();
        r["steps"].as_array().into_iter().flatten().for_each(|st| self.apply_step(st));
        self.vars.set(r["vars"].as_object().cloned().unwrap_or_default());
        self.result.set(Some(r.clone()));
    }

    pub fn apply_step(&self, st: &Value) {
        let id = st["id"].as_str().unwrap_or("").to_string();
        let key = st["key"].as_str().unwrap_or("").to_string();
        let ok = st["ok"] == json!(true);
        self.state.update(|s| {
            s.insert(id.clone(), if ok { "ok" } else { "err" });
        });
        if let Ok(out) = serde_json::from_value::<NodeOut>(st["out"].clone()) {
            self.outs.update(|o| {
                o.insert(key, out);
            });
        }
        self.steps.update(|s| {
            s.insert(id.clone(), st.clone());
        });
        self.last_step.set(Some(id));
    }

    /// Ejecuta un solo nodo con lo que dejó la última ejecución (`seed`).
    pub fn run_node(&self, id: String) {
        let (ed, a) = (*self, app());
        ed.state.update(|s| {
            s.insert(id.clone(), "running");
        });
        let body = json!({"flow": ed.flow.get_untracked(), "nodeId": id, "env": a.env.get_untracked(),
            "seed": {"outs": ed.outs.get_untracked(), "vars": ed.vars.get_untracked()}});
        spawn_local(async move {
            match api::post::<Value>("/api/flow/node-run", &body).await {
                Ok(v) => {
                    ed.apply_step(&v["step"]);
                    if let Some(m) = v["vars"].as_object() {
                        ed.vars.update(|x| x.extend(m.clone()));
                    }
                }
                Err(e) => {
                    ed.state.update(|s| {
                        s.insert(id, "err");
                    });
                    a.err(e);
                }
            }
        });
    }

    /// Nodo nuevo arriba a la izquierda de lo visible, bajando hasta un hueco libre.
    pub fn place(&self, kind: &str) -> Node {
        let (px, py, z) = self.view.get_untracked();
        self.flow.with_untracked(|f| {
            let x = ((80.0 - px) / z / 10.0).round() * 10.0;
            let busy = |y: f64| f.nodes.iter().any(|n| (n.x - x).abs() < W + 20.0 && (n.y - y).abs() < H + 20.0);
            let y = (0..60).map(|i| ((60.0 - py) / z / 10.0).round() * 10.0 + i as f64 * (H + 30.0)).find(|y| !busy(*y)).unwrap_or(0.0);
            new_node(kind, f, x, y)
        })
    }

    /// Abre `p`, o vuelve al lienzo si ya estaba abierta.
    pub fn toggle(&self, p: Pane) {
        self.pane.update(|x| *x = if *x == p { Pane::Canvas } else { p });
    }

    /// Acomoda los pasos en columnas (botón «Ordenar»); se puede deshacer.
    pub fn arrange(&self) {
        self.edit(|f| martino_core::layout::arrange(f, crate::flow_story::LANE_W, H + 80.0));
        self.rev.update(|r| *r += 1);
        self.fit();
    }

    /// Centra el lienzo en un paso.
    pub fn focus(&self, id: &str) {
        if let Some((x, y)) = self.flow.with_untracked(|f| f.node(id).map(|n| (n.x, n.y))) {
            let z = self.view.get_untracked().2;
            self.view.set((450.0 - x * z, 220.0 - y * z, z));
            self.sel.set(Some(id.to_string()));
        }
    }

    /// Encuadra todos los nodos en el lienzo.
    /// Encuadra todos los nodos: los mueve a la vista y, si no entran, aleja el zoom (hasta 40 %).
    pub fn fit(&self) {
        let (min_x, min_y, max_x, max_y) = self.flow.with_untracked(|f| f.nodes.iter().fold((f64::MAX, f64::MAX, f64::MIN, f64::MIN), |(a, b, c, d), n| {
            (a.min(n.x), b.min(n.y), c.max(n.x + node_w(n)), d.max(n.y + H + 60.0))
        }));
        if min_x == f64::MAX {
            self.view.set((40.0, 40.0, 1.0));
            return;
        }
        // Deja libre a la izquierda el panel de Entrada (≈ 270 px) y un margen abajo.
        let (cw, ch) = document().query_selector(".fcanvas").ok().flatten().map(|e| (e.client_width() as f64, e.client_height() as f64)).filter(|(w, h)| *w > 400.0 && *h > 200.0).unwrap_or((1200.0, 700.0));
        let z = ((cw - 310.0) / (max_x - min_x)).min((ch - 90.0) / (max_y - min_y)).clamp(0.4, 1.0);
        self.view.set((290.0 - min_x * z, 50.0 - min_y * z, z));
    }
}

#[component]
pub fn FlowsView() -> impl IntoView {
    let a = app();
    let ed = Ed::new();
    provide_context(ed);
    // Lo que ven los campos con `{{…}}` (colores y valores): el flujo y sus últimos resultados.
    Effect::new(move |_| a.flow_ctx.set(Some((ed.flow.get(), ed.outs.get()))));
    ed.refresh(None);
    // Tras borrar el flujo abierto (ConfirmModal deja `flow_open` en None) se vuelve a cargar la lista.
    Effect::new(move |_| {
        if a.flow_open.get().is_none() && ed.id.get_untracked().is_some() {
            ed.refresh(None);
        }
    });
    let keys = window_event_listener(leptos::ev::keydown, move |e: web_sys::KeyboardEvent| {
        if a.modal.get_untracked().is_some() {
            return;
        }
        let in_field = e.target().and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok()).is_some_and(|el| matches!(el.tag_name().as_str(), "INPUT" | "TEXTAREA" | "SELECT"));
        let ctrl = e.ctrl_key() || e.meta_key();
        match (ctrl, e.shift_key(), e.key().as_str()) {
            (true, _, "s") => {
                e.prevent_default();
                spawn_local(async move {
                    if ed.save().await {
                        a.toast("ok", "Flujo guardado");
                    }
                });
            }
            (true, _, "Enter") => {
                e.prevent_default();
                ed.run();
            }
            (true, false, "z") if !in_field => {
                e.prevent_default();
                ed.travel(true);
            }
            (true, true, "z") | (true, _, "y") if !in_field => {
                e.prevent_default();
                ed.travel(false);
            }
            (false, _, "Delete" | "Backspace") if !in_field && ed.pane.get_untracked() == Pane::Canvas => ed.delete_selected(),
            _ => {}
        }
    });
    on_cleanup(move || keys.remove());
    // Ctrl+V de un cURL sobre el lienzo (fuera de un campo): un paso Servicio con esa petición.
    let paste = window_event_listener(leptos::ev::paste, move |e: web_sys::ClipboardEvent| {
        let in_field = e.target().and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok()).is_some_and(|el| matches!(el.tag_name().as_str(), "INPUT" | "TEXTAREA"));
        let t = e.clipboard_data().and_then(|d| d.get_data("text").ok()).unwrap_or_default();
        if in_field || a.modal.get_untracked().is_some() || ed.pane.get_untracked() != Pane::Canvas || !martino_core::curl::is_curl(&t) {
            return;
        }
        e.prevent_default();
        if let Err(err) = crate::flow_panels::add_curl(ed, &t) {
            a.err(err);
        }
    });
    on_cleanup(move || paste.remove());
    // Clic en un flujo de la barra lateral con el editor ya abierto: se cambia (guardando antes si hay cambios).
    // Se recarga la lista porque el flujo puede ser nuevo (creado desde la barra lateral).
    Effect::new(move |_| {
        if let (Some(want), Some(cur)) = (a.flow_open.get(), ed.id.get_untracked()) {
            if want != cur {
                spawn_local(async move {
                    if !ed.dirty.get_untracked() || ed.save().await {
                        ed.refresh(Some(want));
                    }
                });
            }
        }
    });
    Effect::new(move |_| a.flow_dirty.set(ed.dirty.get()));
    Effect::new(move |prev: Option<u32>| {
        let n = a.save_all.get();
        if prev.is_some() && ed.dirty.get_untracked() {
            spawn_local(async move {
                if ed.save().await {
                    a.toast("ok", "Flujo guardado");
                }
            });
        }
        n
    });
    // Al salir del editor (p. ej. a Peticiones) no se pierden los cambios: se guardan.
    on_cleanup(move || {
        if let (Some(true), Some(Some(id)), Some(f)) = (ed.dirty.try_get_untracked(), ed.id.try_get_untracked(), ed.flow.try_get_untracked()) {
            spawn_local(async move {
                if let Err(e) = api::put::<Value>(&format!("/api/flow?id={}", api::enc(&id)), &f).await {
                    app().err(format!("No se guardó el flujo: {e}"));
                }
            });
        }
        a.flow_dirty.set(false);
    });
    view! {
        <div class="flows">
            <Toolbar />
            {move || if ed.id.get().is_none() {
                view! {
                    <div class="empty grow col" style="align-items:center;justify-content:center">
                        <p>"Aún no hay flujos en esta carpeta."</p>
                        <div class="row">
                            <button class="btn primary" on:click=move |_| spawn_local(async move { ed.create(crate::flow_panels::example(&a)).await })>"Crear ejemplo"</button>
                            <button class="btn" on:click=move |_| spawn_local(async move { ed.create(Flow { name: "Nuevo flujo".into(), collection: a.collection.get_untracked(), ..Default::default() }).await })>"Flujo vacío"</button>
                        </div>
                    </div>
                }.into_any()
            } else {
                match ed.pane.get() {
                    Pane::Qa => view! { <crate::cases::CasesView /> }.into_any(),
                    Pane::Tables => view! { <crate::tables::TablesView /> }.into_any(),
                    Pane::Story => view! { <div class="fbody"><crate::flow_story::StoryView />{move || ed.insp.get().then(|| view! { <Inspector /> })}</div> }.into_any(),
                    Pane::Canvas => view! { <div class="fbody"><Canvas />{move || ed.svc.get().then(|| view! { <crate::flow_panels::ServicePalette /> })}{move || ed.insp.get().then(|| view! { <Inspector /> })}</div> }.into_any(),
                }
            }}
        </div>
    }
}

#[component]
fn Toolbar() -> impl IntoView {
    let (a, ed) = (app(), ed());
    let del = move |_| {
        if let Some(id) = ed.id.get_untracked() {
            let name = ed.flow.with_untracked(|f| f.name.clone());
            a.modal.set(Some(Modal::Confirm { text: format!("¿Borrar el flujo «{name}»? Se elimina el archivo flows/{id}.flow.json."), action: ConfirmAction::DeleteFlow(id) }));
        }
    };
    view! {
        <div class="ftool">
            <button class="btn ghost sm" class:on=move || a.sidebar.get() title="Mostrar u ocultar la barra lateral (Ctrl+B)" on:click=move |_| a.sidebar.update(|s| *s = !*s)>"☰"</button>
            <div class="tgroup" title="Flujo abierto: cámbialo desde la barra lateral">
                <input class="in ftitle" data-noac title="Nombre del flujo" prop:value=move || ed.flow.with(|f| f.name.clone())
                    on:change=move |e| { let v = event_target_value(&e); ed.edit(|f| f.name = v) } />
                <span class="dirty-dot" class:on=move || ed.dirty.get() title="Cambios sin guardar"></span>
                <button class="btn ghost sm danger" title="Borrar este flujo" disabled=move || ed.id.get().is_none() on:click=del>"🗑"</button>
            </div>
            <div class="views" title="Ver el flujo como lienzo o como historia (pasos numerados por etapa)">
                <button class:on=move || ed.pane.get() != Pane::Story on:click=move |_| ed.pane.set(Pane::Canvas)>"Lienzo"</button>
                <button class:on=move || ed.pane.get() == Pane::Story on:click=move |_| ed.pane.set(Pane::Story)>"Historia"</button>
            </div>
            <button class="btn ghost sm" class:on=move || ed.svc.get() title="Buscar un servicio de la colección y añadirlo (tecla S)" on:click=move |_| ed.svc.update(|v| *v = !*v)>"▤ Servicios "<kbd>"S"</kbd></button>
            <crate::flow_panels::AddStep />
            <div class="tgroup" title="Editar">
                <button class="btn ghost sm" title="Deshacer (Ctrl+Z)" on:click=move |_| ed.travel(true)>"↶"</button>
                <button class="btn ghost sm" title="Rehacer (Ctrl+Y)" on:click=move |_| ed.travel(false)>"↷"</button>
                <button class="btn ghost sm" title="Encuadrar todo" on:click=move |_| ed.fit()>"⊡"</button>
                <button class="btn ghost sm" class:hot=move || !ed.flow.with(crate::flow_story::in_lanes)
                    title="Ordenar: acomoda cada paso en el carril de su etapa (Datos, Validar, Preparar, Enviar, Responder). Ctrl+Z deshace" on:click=move |_| ed.arrange()>"⇶ Ordenar"</button>
            </div>
            <span class="grow"></span>
            <div class="tgroup" title="Ver">
                <select class="in sm" title="Qué datos se ven debajo de cada nodo después de ejecutar" on:change=move |e| ed.resp.set(match event_target_value(&e).as_str() { "todo" => "todo", "nada" => "nada", _ => "usado" })>
                    {[("usado", "Datos: lo usado"), ("todo", "Datos: todo"), ("nada", "Datos: ocultos")].into_iter().map(|(k, l)| view! { <option value=k selected=move || ed.resp.get() == k>{l}</option> }).collect_view()}
                </select>
                <button class="btn ghost sm" class:on=move || ed.pane.get() == Pane::Tables title="Tablas de equivalencia de la colección (código → nombre)" on:click=move |_| ed.toggle(Pane::Tables)>"⇆ Tablas"</button>
                <button class="btn ghost sm" class:on=move || ed.pane.get() == Pane::Qa title="Casos de prueba de este flujo: importar CSV, qué esperas y ejecutar" on:click=move |_| ed.toggle(Pane::Qa)>
                    {move || format!("☰ Casos {}", ed.flow.with(|f| f.cases.len()))}
                </button>
                <button class="btn ghost sm" class:on=move || ed.code.get() title="Ver y editar el flujo como YAML o JSON" on:click=move |_| ed.code.update(|c| *c = !*c)>"</>"</button>
            </div>
            <div class="tgroup" title="Ejecutar">
                <label class="row muted sm" title="Compara la respuesta con el servicio global real"><input type="checkbox" prop:checked=move || ed.compare.get() on:change=move |e| ed.compare.set(event_target_checked(&e)) />"Comparar"</label>
                <button class="btn sm" title="Guardar (Ctrl+S)" disabled=move || !ed.dirty.get() on:click=move |_| spawn_local(async move { if ed.save().await { a.toast("ok", "Flujo guardado") } })>"Guardar"</button>
                <button class="btn primary sm" title="Ejecutar el flujo (Ctrl+Enter)" disabled=move || ed.running.get() on:click=move |_| ed.run()>
                    {move || if ed.running.get() { "Ejecutando…" } else { "▶ Ejecutar" }}
                </button>
            </div>
            <button class="btn ghost sm" class:on=move || ed.insp.get() title="Mostrar u ocultar el panel de propiedades" on:click=move |_| ed.insp.update(|v| *v = !*v)>"⚙ Panel"</button>
        </div>
    }
}

/// Conexión que se está dibujando: (desde, puerto, x1, y1, x2, y2) en coordenadas del lienzo.
type Link = (String, &'static str, f64, f64, f64, f64);

#[component]
fn Canvas() -> impl IntoView {
    let (a, ed) = (app(), ed());
    let wrap = NodeRef::<leptos::html::Div>::new();
    let link: RwSignal<Option<Link>> = RwSignal::new(None);
    let bg_down = move |e: web_sys::PointerEvent| {
        if e.button() != 0 {
            return;
        }
        ed.sel.set(None);
        ed.sel_edge.set(None);
        let (px, py, z) = ed.view.get_untracked();
        start_drag(&e, move |dx, dy| ed.view.set((px + dx, py + dy, z)), || {});
    };
    let wheel = move |e: web_sys::WheelEvent| {
        e.prevent_default();
        let (px, py, z) = ed.view.get_untracked();
        if e.ctrl_key() || e.meta_key() {
            let Some(el) = wrap.get_untracked() else { return };
            let r = el.get_bounding_client_rect();
            let (mx, my) = (e.client_x() as f64 - r.left(), e.client_y() as f64 - r.top());
            let nz = (z * (-e.delta_y() * 0.01).exp()).clamp(0.3, 2.0);
            ed.view.set((mx - (mx - px) * nz / z, my - (my - py) * nz / z, nz));
        } else {
            ed.view.set((px - e.delta_x(), py - e.delta_y(), z));
        }
    };
    let drop = move |e: web_sys::DragEvent| {
        e.prevent_default();
        if let (Some(kind), Some(el)) = (e.data_transfer().and_then(|d| d.get_data(crate::flow_panels::DND_NODE).ok()).filter(|s| !s.is_empty()), wrap.get_untracked()) {
            let ((px, py, z), r) = (ed.view.get_untracked(), el.get_bounding_client_rect());
            let (x, y) = (((e.client_x() as f64 - r.left() - px) / z - W / 2.0).round(), ((e.client_y() as f64 - r.top() - py) / z - H / 2.0).round());
            let n = ed.flow.with_untracked(|f| new_node(&kind, f, x, y));
            let id = n.id.clone();
            ed.edit(|f| f.nodes.push(n));
            ed.sel.set(Some(id));
            return;
        }
        let Some(data) = e.data_transfer().and_then(|d| d.get_data(DND_REQ).ok()).filter(|s| !s.is_empty()) else { return };
        let Ok(v) = serde_json::from_str::<Value>(&data) else { return };
        let (Some(el), (px, py, z)) = (wrap.get_untracked(), ed.view.get_untracked()) else { return };
        let r = el.get_bounding_client_rect();
        let (x, y) = ((e.client_x() as f64 - r.left() - px) / z - W / 2.0, (e.client_y() as f64 - r.top() - py) / z - H / 2.0);
        let col = v["collection"].as_str().unwrap_or("").to_string();
        let path = v["path"].as_str().unwrap_or("").to_string();
        if ed.flow.with_untracked(|f| !f.collection.is_empty() && f.collection != col) {
            a.err("Esa petición es de otra colección que la del flujo");
            return;
        }
        let mut n = ed.flow.with_untracked(|f| new_node("request", f, x, y));
        let file = path.rsplit('/').next().unwrap_or(&path);
        n.name = martino_core::yaml_req::strip_ext(file).unwrap_or(file).replace('-', " ");
        n.key = ed.flow.with_untracked(|f| flow::unique_key(&n.name, &f.nodes.iter().map(|n| n.key.clone()).collect::<Vec<_>>()));
        n.req_ref = path;
        let id = n.id.clone();
        ed.edit(|f| {
            if f.collection.is_empty() {
                f.collection = col;
            }
            f.nodes.push(n);
        });
        ed.sel.set(Some(id));
    };
    let edges = move || {
        let sel = ed.sel_edge.get();
        let st = ed.state.get();
        ed.flow.with(|f| {
            f.edges
                .iter()
                .enumerate()
                .filter_map(|(i, e)| {
                    let (from, to) = (f.node(&e.from)?, f.node(&e.to)?);
                    let (x1, y1) = port_pos(from, &e.port);
                    let d = curve(x1, y1, to.x, to.y + H / 2.0, from.kind == "condition" && e.port == "false");
                    // Camino recorrido: el origen terminó y el destino llegó a correr; «go» si el destino sigue corriendo.
                    let walk = match (st.get(&e.from).copied(), st.get(&e.to).copied()) {
                        (Some("ok" | "err"), Some("running")) => " lit go",
                        (Some("ok" | "err"), Some(_)) => " lit",
                        _ => "",
                    };
                    let cls = format!("fe p-{}{walk}{}", e.port, if sel == Some(i) { " sel" } else { "" });
                    Some(view! {
                        <g on:pointerdown=move |ev: web_sys::PointerEvent| { ev.stop_propagation(); ed.sel_edge.set(Some(i)); ed.sel.set(None) }>
                            <title>"Clic para seleccionar · Supr la borra"</title>
                            <path class="hit" d=d.clone() />
                            <path class=cls d=d />
                        </g>
                    })
                })
                .collect_view()
        })
    };
    view! {
        <div class="fcanvas" node_ref=wrap on:pointerdown=bg_down on:wheel=wheel
            // El lienzo se mueve con su propio arrastre/zoom; si el navegador lo desplaza solo (foco, clic), se devuelve.
            on:scroll=move |_| if let Some(el) = wrap.get_untracked() { el.set_scroll_left(0); el.set_scroll_top(0); } on:dragover=|e: web_sys::DragEvent| e.prevent_default() on:drop=drop>
            <div class="fworld" style=move || { let (x, y, z) = ed.view.get(); format!("transform:translate({x}px,{y}px) scale({z})") }>
                <crate::flow_story::Lanes />
                <svg class="fedges">
                    {edges}
                    {move || link.get().map(|(_, port, x1, y1, x2, y2)| view! { <path class="fe tmp" d=curve(x1, y1, x2, y2, port == "false") /> })}
                </svg>
                <For each=move || ed.flow.with(|f| f.nodes.iter().map(|n| n.id.clone()).collect::<Vec<_>>()) key=|id| id.clone() let:id>
                    <NodeView id=id link=link />
                </For>
            </div>
            <crate::flow_panels::InputDock />
            <crate::flow_panels::CanvasSearch />
            <crate::flow_story::Guide />
            <ResultBar />
            {move || ed.flow.with(|f| f.nodes.is_empty()).then(|| view! {
                <div class="fhint">"Arrastra peticiones desde la barra lateral o añade nodos con la barra de arriba. Une nodos arrastrando desde el punto de la derecha."</div>
            })}
        </div>
    }
}

#[component]
fn NodeView(id: String, link: RwSignal<Option<Link>>) -> impl IntoView {
    let ed = ed();
    let id_m = id.clone();
    let node = Memo::new(move |_| ed.flow.with(|f| f.node(&id_m).cloned()).unwrap_or_default());
    let id_c = id.clone();
    let joins = Memo::new(move |_| ed.flow.with(|f| f.edges.iter().filter(|e| e.to == id_c).count()));
    let id_s = id.clone();
    let state = move || ed.state.with(|s| s.get(&id_s).copied()).unwrap_or(if ed.result.with(Option::is_some) { "idle" } else { "" });
    let id_d = id.clone();
    let down = move |e: web_sys::PointerEvent| {
        e.stop_propagation();
        if e.button() != 0 {
            return;
        }
        ed.sel.set(Some(id_d.clone()));
        ed.sel_edge.set(None);
        let n = node.get_untracked();
        let (x0, y0, z) = (n.x, n.y, ed.view.get_untracked().2);
        let moved = Rc::new(Cell::new(false));
        let id = id_d.clone();
        start_drag(
            &e,
            move |dx, dy| {
                if !moved.replace(true) {
                    ed.snapshot();
                }
                let (x, y) = (((x0 + dx / z) / 10.0).round() * 10.0, ((y0 + dy / z) / 10.0).round() * 10.0);
                ed.flow.update(|f| {
                    if let Some(n) = f.nodes.iter_mut().find(|n| n.id == id) {
                        n.x = x;
                        n.y = y;
                    }
                });
                ed.dirty.set(true);
            },
            || {},
        );
    };
    let start_link = move |e: web_sys::PointerEvent, port: &'static str| {
        e.stop_propagation();
        let n = node.get_untracked();
        let (x1, y1) = port_pos(&n, port);
        let z = ed.view.get_untracked().2;
        let (cx0, cy0) = (e.client_x() as f64, e.client_y() as f64);
        let last = Rc::new(Cell::new((cx0, cy0)));
        let l2 = last.clone();
        link.set(Some((n.id.clone(), port, x1, y1, x1, y1)));
        start_drag(
            &e,
            move |dx, dy| {
                l2.set((cx0 + dx, cy0 + dy));
                link.update(|l| {
                    if let Some(l) = l {
                        l.4 = x1 + dx / z;
                        l.5 = y1 + dy / z;
                    }
                });
            },
            move || {
                let (cx, cy) = last.get();
                let to = document().element_from_point(cx as f32, cy as f32).and_then(|el| el.closest("[data-node]").ok().flatten()).and_then(|el| el.get_attribute("data-node"));
                if let (Some((from, port, ..)), Some(to)) = (link.get_untracked(), to) {
                    let ok = to != from && ed.flow.with_untracked(|f| f.node(&to).is_some_and(|n| has_input(&n.kind)));
                    if ok {
                        ed.edit(|f| {
                            if !f.edges.iter().any(|e| e.from == from && e.port == port && e.to == to) {
                                f.edges.push(Edge { from, port: port.into(), to });
                            }
                        });
                    }
                }
                link.set(None);
            },
        );
    };
    let id_k = id.clone();
    let ports = move || {
        let n = node.get();
        flow::ports(&n.kind)
            .iter()
            .map(|p| {
                let p: &'static str = p;
                let (_, py) = port_pos(&n, p);
                let bottom = n.kind == "condition" && p == "false";
                let style = if bottom { String::new() } else { format!("top:{}px", py - n.y - 6.0) };
                let (id, port) = (id_k.clone(), p.to_string());
                let fan = ed.flow.with(|f| f.edges.iter().filter(|e| e.from == id && e.port == port).count());
                let text = match (port_label(p), fan) {
                    ("", f) if f > 1 => format!("∥ {f} en paralelo"),
                    (l, f) if f > 1 => format!("{l} · ∥ {f}"),
                    (l, _) => l.to_string(),
                };
                view! {
                    <div class=format!("fport out p-{p}{}", if bottom { " bottom" } else { "" }) style=style title="Arrastra hasta otro nodo para conectar"
                        on:pointerdown=move |e| start_link(e, p)>
                        <span class="pl">{text}</span>
                    </div>
                }
            })
            .collect_view()
    };
    let sub = move || ed.flow.with(|f| node.with(|n| crate::flow_story::sentence(f, n)));
    let id_r = id.clone();
    let id_t = id.clone();
    let id_sel = id.clone();
    let (id_rules, id_g, id_g2) = (id.clone(), id.clone(), id.clone());
    // Esquina inferior derecha: ancho del nodo y alto de su respuesta.
    let resize = move |e: web_sys::PointerEvent| {
        e.stop_propagation();
        let n = node.get_untracked();
        let (w0, h0, z) = (node_w(&n), n.height.unwrap_or(120.0), ed.view.get_untracked().2);
        let id = id_g.clone();
        ed.snapshot();
        start_drag(&e, move |dx, dy| {
            let (w, h) = (((w0 + dx / z) / 10.0).round() * 10.0, ((h0 + dy / z) / 10.0).round() * 10.0);
            ed.flow.update(|f| if let Some(n) = f.nodes.iter_mut().find(|n| n.id == id) {
                n.width = Some(w.clamp(160.0, 720.0));
                n.height = Some(h.clamp(60.0, 800.0));
            });
            ed.dirty.set(true);
        }, || {});
    };
    let meta = move || {
        ed.steps.with(|s| s.get(&id_t).map(|st| {
            let status = st["out"]["status"].as_u64().map(|s| format!("{s} · ")).unwrap_or_default();
            format!("{status}{}ms", st["ms"])
        }))
    };
    view! {
        <div class=move || format!("fnode k-{} {} {}", node.with(|n| n.kind.clone()), state(), {
                let q = ed.find.get().trim().to_lowercase();
                if q.is_empty() { "" } else if ed.flow.with(|f| node.with(|n| crate::flow_panels::matches(f, n, &q))) { "found" } else { "faded" }
            }) class:sel=move || ed.sel.get().as_deref() == Some(id_sel.as_str())
            data-node=id.clone()
            style=move || node.with(|n| format!("left:{}px;top:{}px;width:{}px;{}{}", n.x, n.y, node_w(n),
                n.height.map(|h| format!("--rh:{h}px;")).unwrap_or_default(), if n.color.is_empty() { String::new() } else { format!("--nc:{}", n.color) }))
            on:pointerdown=down>
            {move || (joins.get() > 1).then(|| view! { <span class="fjoin">{format!("⇉ une {} ramas", joins.get())}</span> })}
            {move || has_input(&node.with(|n| n.kind.clone())).then(|| view! { <div class="fport in"></div> })}
            <div class="fn-top">
                <span class=move || format!("kshape k-{}", node.with(|n| n.kind.clone())) title=move || label(&node.with(|n| n.kind.clone()))></span>
                <span class="fn-id" title=move || format!("{} · identificador del paso", label(&node.with(|n| n.kind.clone())))>{move || ed.flow.with(|f| node_code(f, &node.get()))}</span>
                <span class="fn-ref" title="Clave del nodo · clic copia {{clave}}" on:pointerdown=|e| e.stop_propagation()
                    on:click=move |_| copy(&format!("{{{{{}}}}}", node.with_untracked(|n| n.key.clone())))>{move || node.with(|n| n.key.clone())}</span>
                <span class="grow"></span>
                <span class="fn-meta">{meta}</span>
                {move || (!flow::is_annotation(&node.with(|n| n.kind.clone()))).then(|| {
                    let id = id_r.clone();
                    view! { <button class="fn-play" title="Ejecutar solo este nodo (usa los datos de la última ejecución)" on:pointerdown=|e| e.stop_propagation() on:click=move |_| ed.run_node(id.clone())>"▶"</button> }
                })}
            </div>
            <div class="fn-name">{move || node.with(|n| if n.name.is_empty() { label(&n.kind).to_string() } else { n.name.clone() })}</div>
            <div class="fn-sub">{sub}</div>
            {ports}
            {move || (node.with(|n| n.kind == "condition" && n.rules.len() > 1)).then(|| view! { <crate::flow_steps::RuleList id=id_rules.clone() /> })}
            <crate::flow_steps::NodeResp id=id.clone() />
            <div class="fn-grip" title="Arrastra para cambiar el tamaño (doble clic: tamaño normal)" on:pointerdown=resize
                on:dblclick=move |e| { e.stop_propagation(); ed.edit_node(&id_g2, |n| { n.width = None; n.height = None }) }></div>
        </div>
    }
}

#[component]
fn ResultBar() -> impl IntoView {
    let ed = ed();
    move || {
        ed.result.get().map(|r| {
            let verdict = r["verdict"].as_str().unwrap_or("?").to_string();
            let cls = match verdict.as_str() {
                "OK" => "chip ok",
                "DIFERENTE" => "chip warn",
                _ => "chip err",
            };
            let diffs = r["compare"]["diffs"].as_array().map(|d| d.len()).unwrap_or(0);
            view! {
                <div class="fbar" on:pointerdown=|e| e.stop_propagation()>
                    <span class=cls>{verdict}</span>
                    {r["status"].as_u64().map(|s| view! { <span class=crate::ui::status_class(Some(s))>{s}</span> })}
                    <span class="muted">{format!("{} pasos", r["steps"].as_array().map(|s| s.len()).unwrap_or(0))}</span>
                    {(diffs > 0).then(|| view! { <span class="chip warn">{format!("{diffs} diferencias")}</span> })}
                    {r["error"].as_str().map(|e| view! { <span class="err-text">{e.to_string()}</span> })}
                    <button class="btn ghost sm" title="Ver la respuesta y el detalle" on:click=move |_| { ed.sel.set(None); ed.code.set(false) }>"Ver detalle"</button>
                    <button class="btn ghost sm" title="Cerrar" on:click=move |_| ed.result.set(None)>"✕"</button>
                </div>
            }
        })
    }
}
