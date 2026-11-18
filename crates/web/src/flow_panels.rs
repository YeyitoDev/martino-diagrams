//! Paneles fijos del editor de flujos: «＋ Añadir paso» y el buscador «/» (barra de arriba), la columna
//! de servicios, el buscador del lienzo y la Entrada.
use crate::flows::{ed, label};
use crate::state::{app, TreeItem};
use leptos::prelude::*;
use martino_core::flow::{Case, Edge, Flow, Node, Rule};
use martino_core::model::Pair;
use crate::state::App;
use serde_json::json;
use serde_json::Value;

pub const DND_NODE: &str = "application/x-martino-node";

/// Herramientas agrupadas por para qué sirven, en el orden en que se suele armar un flujo.
const GROUPS: [(&str, &[&str]); 5] = [
    ("Datos", &["input", "data", "lookup"]),
    ("Llamar", &["request"]),
    ("Procesar", &["extract", "transform", "crypto"]),
    ("Decidir", &["condition"]),
    ("Cerrar", &["response", "end", "note"]),
];

fn hint(kind: &str) -> &'static str {
    match kind {
        "input" => "Campos que recibe el flujo",
        "data" => "Valores fijos para reusar",
        "lookup" => "Traduce un código con una tabla",
        "request" => "Llama a una petición guardada",
        "extract" => "Guarda campos en variables",
        "transform" => "JavaScript que arma datos",
        "crypto" => "Firma, cifra o descifra",
        "condition" => "Sí / no según reglas",
        "response" => "Lo que devuelve el flujo",
        "end" => "Termina sin responder",
        _ => "Comentario en el lienzo",
    }
}

/// Palabras con las que también se encuentra cada paso en el buscador «/».
fn words(kind: &str) -> &'static str {
    match kind {
        "input" => "entrada campos recibir input parametros",
        "data" => "datos fijos constantes valores",
        "lookup" => "equivalencia tabla traducir codigo buscar mapear",
        "request" => "servicio llamar peticion api http endpoint",
        "extract" => "extraer guardar variable sacar",
        "transform" => "transformar javascript calcular codigo armar",
        "crypto" => "cifrado cifrar firma firmar jwe jws hmac descifrar verificar",
        "condition" => "condicion si comparar regla decidir if validar",
        "response" => "respuesta devolver status salida",
        "end" => "fin terminar",
        _ => "nota comentario",
    }
}

fn fold(t: &str) -> String {
    t.to_lowercase().chars().map(|c| match c { 'á' => 'a', 'é' => 'e', 'í' => 'i', 'ó' => 'o', 'ú' => 'u', c => c }).collect()
}

fn kinds() -> impl Iterator<Item = (&'static str, &'static str)> {
    GROUPS.into_iter().flat_map(|(g, ks)| ks.iter().map(move |k| (g, *k)))
}

fn add_kind(ed: crate::flows::Ed, kind: &str) {
    let n = ed.place(kind);
    let id = n.id.clone();
    ed.edit(|f| f.nodes.push(n));
    ed.sel.set(Some(id));
}

/// Crea un paso Servicio con la petición de un cURL (queda dentro del flujo, sin archivo).
pub fn add_curl(ed: crate::flows::Ed, text: &str) -> Result<(), String> {
    let r = martino_core::curl::parse_curl(text)?;
    let mut n = ed.place("request");
    let path = r.url.split(['?', '#']).next().unwrap_or("").trim_end_matches('/').rsplit('/').next().unwrap_or("").to_string();
    n.name = if r.name.is_empty() { format!("{} {}", r.method.to_uppercase(), if path.is_empty() { "petición" } else { &path }) } else { r.name.clone() };
    n.key = ed.flow.with_untracked(|f| martino_core::flow::unique_key(&n.name, &f.nodes.iter().map(|n| n.key.clone()).collect::<Vec<_>>()));
    n.request = Some(r);
    let id = n.id.clone();
    ed.edit(|f| f.nodes.push(n));
    ed.sel.set(Some(id));
    app().toast("ok", "Servicio creado desde el cURL");
    Ok(())
}

fn step_item(ed: crate::flows::Ed, k: &'static str, on: bool, close: impl Fn() + 'static) -> impl IntoView {
    view! {
        <button class="step-i" class:on=on draggable="true" title="Clic para añadir · o arrástralo al lienzo"
            on:mousedown=|e| e.prevent_default()
            on:click=move |_| { add_kind(ed, k); close() }
            on:dragstart=move |e: web_sys::DragEvent| { if let Some(d) = e.data_transfer() { let _ = d.set_data(DND_NODE, k); } }>
            <span class=format!("kshape k-{k}")></span><b>{label(k)}</b><small>{hint(k)}</small>
        </button>
    }
}

/// Barra de arriba: botón «＋ Añadir paso» (menú por grupos) y buscador «/» de pasos.
#[component]
pub fn AddStep() -> impl IntoView {
    let ed = ed();
    let open = RwSignal::new(false);
    let curl = RwSignal::new(String::new());
    let q = RwSignal::new(String::new());
    let sel = RwSignal::new(0usize);
    let focused = RwSignal::new(false);
    let input = NodeRef::<leptos::html::Input>::new();
    let found = move || {
        let t = fold(q.get().trim());
        kinds().filter(|(_, k)| t.is_empty() || fold(&format!("{} {} {}", label(k), words(k), hint(k))).contains(&t)).map(|x| x.1).collect::<Vec<_>>()
    };
    // «/» fuera de un campo enfoca el buscador de pasos.
    let keys = window_event_listener(leptos::ev::keydown, move |e: web_sys::KeyboardEvent| {
        let in_field = e.target().and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok()).is_some_and(|el| matches!(el.tag_name().as_str(), "INPUT" | "TEXTAREA" | "SELECT"));
        if e.key() == "/" && !in_field && app().modal.get_untracked().is_none() {
            e.prevent_default();
            if let Some(i) = input.get_untracked() { let _ = i.focus(); }
        }
    });
    on_cleanup(move || keys.remove());
    let key = move |e: web_sys::KeyboardEvent| {
        let list = found();
        match e.key().as_str() {
            "ArrowDown" => { e.prevent_default(); sel.update(|s| *s = (*s + 1) % list.len().max(1)) }
            "ArrowUp" => { e.prevent_default(); sel.update(|s| *s = (*s + list.len().max(1) - 1) % list.len().max(1)) }
            "Enter" => if let Some(k) = list.get(sel.get_untracked()) { add_kind(ed, k); q.set(String::new()); if let Some(i) = input.get_untracked() { let _ = i.blur(); } },
            "Escape" => { q.set(String::new()); if let Some(i) = input.get_untracked() { let _ = i.blur(); } }
            _ => {}
        }
    };
    view! {
        <div class="addstep">
            <button class="btn sm add-b" class:on=move || open.get() title="Añadir un paso al flujo" on:click=move |_| open.update(|o| *o = !*o)>"＋ Añadir paso"</button>
            {move || open.get().then(|| view! {
                <div class="step-menu" on:pointerdown=|e| e.stop_propagation()>
                    {GROUPS.into_iter().map(|(g, ks)| view! {
                        <div><h5>{g}</h5>{ks.iter().map(|&k| step_item(ed, k, false, move || open.set(false))).collect_view()}</div>
                    }).collect_view()}
                    <div class="step-curl"><h5>"Desde un cURL"</h5>
                        <textarea class="in" data-noac rows="3" placeholder="Pega aquí un cURL (también vale Ctrl+V sobre el lienzo)" prop:value=move || curl.get() on:input=move |e| curl.set(event_target_value(&e))></textarea>
                        <button class="btn sm" disabled=move || curl.with(|c| c.trim().is_empty()) on:click=move |_| match add_curl(ed, &curl.get_untracked()) {
                            Ok(()) => { curl.set(String::new()); open.set(false) }
                            Err(e) => app().err(e),
                        }>"Crear servicio"</button>
                    </div>
                </div>
            })}
            <label class="step-q" title="Buscar un paso por lo que quieres hacer: cifrar, comparar, llamar…">
                "⌕"<input node_ref=input data-noac placeholder="Buscar paso…" prop:value=move || q.get()
                    on:input=move |e| { q.set(event_target_value(&e)); sel.set(0) } on:keydown=key
                    on:focus=move |_| { focused.set(true); open.set(false) } on:blur=move |_| focused.set(false) /><kbd>"/"</kbd>
                {move || focused.get().then(|| {
                    let list = found();
                    view! {
                        <div class="step-res">
                            {if list.is_empty() { view! { <div class="muted step-none">"Ningún paso coincide. Prueba «llamar», «cifrar» o «si»."</div> }.into_any() }
                             else { list.into_iter().enumerate().map(|(i, k)| step_item(ed, k, i == sel.get(), || ())).collect_view().into_any() }}
                        </div>
                    }
                })}
            </label>
        </div>
    }
}

/// ¿El paso coincide con el buscador del lienzo? (nombre, clave, ID o petición)
pub fn matches(f: &Flow, n: &Node, q: &str) -> bool {
    let code = crate::flows::node_code(f, n);
    [n.name.as_str(), n.key.as_str(), code.as_str(), n.req_ref.as_str(), label(&n.kind)].iter().any(|t| fold(t).contains(&fold(q)))
}

/// Buscador del lienzo: resalta los pasos que coinciden; Enter centra el siguiente.
#[component]
pub fn CanvasSearch() -> impl IntoView {
    let ed = ed();
    let at = RwSignal::new(0usize);
    let hits = move || {
        let q = ed.find.get();
        if q.trim().is_empty() { return vec![]; }
        ed.flow.with(|f| f.nodes.iter().filter(|n| matches(f, n, q.trim())).map(|n| n.id.clone()).collect::<Vec<_>>())
    };
    let next = move || {
        let h = hits();
        if h.is_empty() { return; }
        let i = at.get_untracked() % h.len();
        ed.focus(&h[i]);
        at.set(i + 1);
    };
    view! {
        <div class="fsearch" on:pointerdown=|e| e.stop_propagation() on:wheel=|e| e.stop_propagation()>
            "⌕"<input data-noac placeholder="Buscar en el lienzo: nombre, ID…" prop:value=move || ed.find.get()
                on:input=move |e| { ed.find.set(event_target_value(&e)); at.set(0) }
                on:keydown=move |e: web_sys::KeyboardEvent| match e.key().as_str() { "Enter" => next(), "Escape" => ed.find.set(String::new()), _ => {} } />
            {move || (!ed.find.get().trim().is_empty()).then(|| view! { <span class="muted">{format!("{}", hits().len())}</span> })}
        </div>
    }
}

/// Peticiones aplanadas: (ruta, nombre, método, ID).
fn requests(items: &[TreeItem], out: &mut Vec<(String, String, String, String)>) {
    for i in items {
        if i.folder { requests(&i.items, out) } else { out.push((i.path.clone(), i.name.clone(), i.method.clone(), i.id.clone())) }
    }
}

/// Columna de selección rápida: los servicios de la colección del flujo; clic o arrastrar añade el paso.
#[component]
pub fn ServiceColumn() -> impl IntoView {
    let (a, ed) = (app(), ed());
    let q = RwSignal::new(String::new());
    let col = move || { let c = ed.flow.with(|f| f.collection.clone()); if c.is_empty() { a.collection.get() } else { c } };
    let list = move || {
        let mut out = vec![];
        a.collections.with(|cs| if let Some(c) = cs.iter().find(|c| c.id == col()) { requests(&c.items, &mut out) });
        let t = fold(q.get().trim());
        out.into_iter().filter(|r| t.is_empty() || fold(&format!("{} {} {}", r.1, r.3, r.0)).contains(&t)).collect::<Vec<_>>()
    };
    let add = move |path: String, name: String| {
        let mut n = ed.place("request");
        n.name = name;
        n.key = ed.flow.with_untracked(|f| martino_core::flow::unique_key(&n.name, &f.nodes.iter().map(|n| n.key.clone()).collect::<Vec<_>>()));
        n.req_ref = path;
        let (id, c) = (n.id.clone(), col());
        ed.edit(|f| { if f.collection.is_empty() { f.collection = c; } f.nodes.push(n) });
        ed.sel.set(Some(id));
    };
    view! {
        <aside class="fsvc">
            <div class="fsvc-h"><span class="kshape k-request"></span>"Servicios"<span class="grow"></span>
                <button class="btn ghost sm" title="Ocultar columna" on:click=move |_| ed.svc.set(false)>"✕"</button></div>
            <input class="in" data-noac placeholder="Filtrar por nombre o ID…" prop:value=move || q.get() on:input=move |e| q.set(event_target_value(&e)) />
            <div class="fsvc-list">
                {move || {
                    let l = list();
                    if l.is_empty() { return view! { <p class="muted">"No hay peticiones en esta colección. Créalas en Peticiones."</p> }.into_any(); }
                    l.into_iter().map(|(path, name, method, id)| {
                        let (p2, n2, c) = (path.clone(), name.clone(), col());
                        view! {
                            <button class="fsvc-i" draggable="true" title=format!("{path} · clic o arrástralo al lienzo") on:click=move |_| add(p2.clone(), n2.clone())
                                on:dragstart=move |e: web_sys::DragEvent| { if let Some(d) = e.data_transfer() {
                                    let _ = d.set_data(crate::requests::DND_REQ, &serde_json::json!({"collection": c, "path": path}).to_string()); } }>
                                <span class=format!("m m-{}", method.to_lowercase())>{method.to_uppercase()}</span>
                                {(!id.is_empty()).then(|| view! { <span class="req-id">{id}</span> })}
                                <span class="fsvc-n">{name}</span>
                            </button>
                        }
                    }).collect_view().into_any()
                }}
            </div>
        </aside>
    }
}

/// Entrada siempre a la vista (se puede ocultar). Sus campos se editan aquí, y las «entradas guardadas»
/// son los casos QA del flujo: elegir una la carga en los campos; los CSV de Casos QA llegan aquí solos.
#[component]
pub fn InputDock() -> impl IntoView {
    let ed = ed();
    let open = RwSignal::new(true);
    let inp = Memo::new(move |_| ed.flow.with(|f| f.input_node().map(|n| (n.id.clone(), n.key.clone(), n.data.clone()))));
    let set = move |id: String, k: String, raw: String| {
        // Número, booleano o JSON si se lee como tal; si no, texto ("001" queda como texto).
        let v = serde_json::from_str::<Value>(&raw).ok().filter(|v| !v.is_string()).unwrap_or(Value::String(raw));
        ed.edit_node(&id, move |n| n.data[k] = v);
        ed.rev.update(|r| *r += 1);
    };
    let load = move |id: String, v: Value| {
        ed.edit_node(&id, move |n| n.data = v);
        ed.rev.update(|r| *r += 1);
    };
    let save_as = move |data: Value| {
        ed.edit(|f| {
            let name = format!("Entrada {}", f.cases.len() + 1);
            let new = crate::cases::renumber(&f.cases, vec![Case { name, input: data, ..Default::default() }]);
            f.cases.extend(new);
        });
    };
    view! {
        <div class="fdock" class:closed=move || !open.get() on:pointerdown=|e| e.stop_propagation() on:wheel=|e| e.stop_propagation()>
            <div class="fdock-h">
                <button class="fdock-tg" title=move || if open.get() { "Ocultar la entrada" } else { "Mostrar la entrada" } on:click=move |_| open.update(|o| *o = !*o)>
                    {move || if open.get() { "▾" } else { "▸" }}<span class="kshape k-input"></span>"Entrada"
                    <span class="muted">{move || ed.flow.with(|f| (!f.cases.is_empty()).then(|| format!(" · {} guardadas", f.cases.len())))}</span>
                </button>
                <span class="grow"></span>
                {move || inp.with(|i| i.as_ref().map(|(id, ..)| id.clone())).map(|id| view! {
                    <button class="btn ghost sm" title="Abrir en el inspector (agregar o quitar campos)" on:click=move |_| ed.sel.set(Some(id.clone()))>"✎"</button>
                })}
            </div>
            <Show when=move || open.get()>
                {move || match inp.get() {
                    None => view! {
                        <button class="btn ghost sm" on:click=move |_| { let n = ed.place("input"); let id = n.id.clone(); ed.edit(|f| f.nodes.push(n)); ed.sel.set(Some(id)) }>"+ Añadir entrada"</button>
                    }.into_any(),
                    Some((id, key, data)) => {
                        let fields = data.as_object().cloned().unwrap_or_default();
                        let empty = fields.is_empty();
                        let (id_l, data_s, data_c) = (id.clone(), data.clone(), data.clone());
                        let cases: Vec<(String, String, Value)> = ed.flow.with_untracked(|f| f.cases.iter().map(|c| (c.id.clone(), c.name.clone(), c.input.clone())).collect());
                        let current = cases.iter().find(|c| c.2 == data_c).map(|c| c.0.clone()).unwrap_or_default();
                        view! {
                            {(!cases.is_empty()).then(|| {
                                let byid = cases.clone();
                                view! {
                                    <label class="fdock-sc" title="Escenario: elige uno y los campos cambian a sus valores">
                                        <span class="muted">"Escenario"</span>
                                        <select class="in" prop:value=current.clone() on:change=move |e| {
                                            let v = event_target_value(&e);
                                            if let Some(c) = byid.iter().find(|c| c.0 == v) { load(id_l.clone(), c.2.clone()) }
                                        }>
                                            <option value="" disabled=true>"— valores editados —"</option>
                                            {cases.into_iter().map(|(cid, name, _)| { let on = cid == current; view! { <option value=cid.clone() selected=on>{format!("{cid} · {name}")}</option> } }).collect_view()}
                                        </select>
                                    </label>
                                }
                            })}
                            {fields.into_iter().map(|(k, v)| {
                                let (id, k2, tip) = (id.clone(), k.clone(), format!("{{{{{key}.{k}}}}}"));
                                let shown = v.as_str().map(String::from).unwrap_or_else(|| v.to_string());
                                view! {
                                    <label class="fdock-f" title=tip>
                                        <span class="mono">{k}</span>
                                        <input class="in" data-noac prop:value=shown
                                            on:change=move |e| set(id.clone(), k2.clone(), event_target_value(&e))
                                            on:keydown=move |e: web_sys::KeyboardEvent| if e.key() == "Enter" && !(e.ctrl_key() || e.meta_key()) {
                                                if let Some(t) = e.target().and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::HtmlElement>(t).ok()) { let _ = t.blur(); }
                                                ed.run();
                                            } />
                                    </label>
                                }
                            }).collect_view()}
                            {empty.then(|| view! { <p class="muted">"Sin campos: agrégalos con ✎"</p> })}
                            <div class="row">
                                <button class="btn ghost sm" title="Guarda estos valores como otra entrada (un caso QA)" on:click=move |_| save_as(data_s.clone())>"+ Guardar entrada"</button>
                                <button class="btn ghost sm" title="Importa entradas desde CSV o ejecútalas todas" on:click=move |_| ed.pane.set(crate::flows::Pane::Qa)>"CSV / todas"</button>
                            </div>
                        }.into_any()
                    }
                }}
                <button class="btn primary sm" disabled=move || ed.running.get() on:click=move |_| ed.run()>
                    {move || if ed.running.get() { "Ejecutando…" } else { "▶ Ejecutar" }}
                </button>
            </Show>
        </div>
    }
}

/// Flujo de ejemplo con la colección activa (botón «Crear ejemplo» de la pantalla vacía).
pub fn example(a: &App) -> Flow {
    let col = a.collection.get_untracked();
    let paths = a.coll().map(|c| crate::flows::bru_paths(&c.items)).unwrap_or_default();
    let req = paths.iter().map(|p| p.0.clone()).find(|p| martino_core::yaml_req::strip_ext(p) == Some("Usuarios/Obtener-usuario")).or_else(|| paths.first().map(|p| p.0.clone())).unwrap_or_default();
    let n = |id: &str, kind: &str, key: &str, name: &str, x: f64, y: f64| Node { id: id.into(), kind: kind.into(), key: key.into(), name: name.into(), x, y, ..Default::default() };
    let pair = |k: &str, v: &str| Pair { name: k.into(), value: v.into(), enabled: true };
    let mut entrada = n("e1", "input", "entrada", "Entrada", 40.0, 260.0);
    entrada.data = json!({"userId": 1});
    let mut svc = n("e2", "request", "usuario", "Obtener usuario", 320.0, 120.0);
    svc.req_ref = req;
    svc.vars = vec![pair("userId", "{{entrada.userId}}")];
    let mut cond = n("e3", "condition", "existe", "¿Tiene correo?", 600.0, 120.0);
    cond.mode = "all".into();
    cond.rules = vec![Rule { left: "usuario.response.email".into(), op: "isDefined".into(), right: String::new() }];
    let mut ok = n("e4", "response", "ok", "Respuesta 200", 900.0, 60.0);
    ok.status = "200".into();
    ok.body = json!({"id": "{{usuario.response.id}}", "nombre": "{{usuario.response.name}}", "correo": "{{usuario.response.email}}"});
    let mut no = n("e5", "response", "no_existe", "Respuesta 404", 900.0, 300.0);
    no.status = "404".into();
    no.body = json!({"error": "Usuario {{entrada.userId}} no encontrado"});
    let mut note = n("e6", "note", "nota", "Nota", 320.0, 300.0);
    note.text = "Cambia userId a 999 en la Entrada y ejecuta: el servicio da 404 y el flujo sale por la rama de error.".into();
    let e = |from: &str, port: &str, to: &str| Edge { from: from.into(), port: port.into(), to: to.into() };
    Flow {
        name: "Ejemplo: consultar usuario".into(),
        collection: col,
        nodes: vec![n("e0", "start", "inicio", "Inicio", 40.0, 120.0), entrada, svc, cond, ok, no, note],
        edges: vec![e("e0", "out", "e2"), e("e2", "out", "e3"), e("e2", "error", "e5"), e("e3", "true", "e4"), e("e3", "false", "e5")],
        ..Default::default()
    }
}

