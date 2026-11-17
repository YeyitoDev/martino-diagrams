//! Modo Peticiones: barra lateral, pantallas, paneles, editor y respuesta.
use crate::api;
use crate::state::{app, Collection, ConfirmAction, Modal, Panel, Screen, TreeItem};
use crate::ui::{copy, fmt_size, status_class, Gutter, JsonTree, KvTable, VarField};
use leptos::prelude::*;
use leptos::task::spawn_local;
use martino_core::bru::METHODS;
use martino_core::interp::{beautify_json, looks_minified_json, var_names};
use martino_core::model::{raw_http, Pair, Request, SentRequest};
use martino_core::vars::VarSource;
use serde_json::{json, Value};

type Pairs = Vec<Pair>;

// ---------- barra lateral ----------

pub const DND_REQ: &str = "application/x-martino-request";

#[component]
pub fn Sidebar() -> impl IntoView {
    let a = app();
    let filter = RwSignal::new(String::new());
    view! {
        <aside class="side" style:width=move || format!("{}px", a.sidebar_w.get()) style:display={move || if a.sidebar.get() { "flex" } else { "none" }}>
            <div class="head">
                <input class="in grow" placeholder="Buscar petición…" data-noac prop:value=move || filter.get() on:input=move |e| filter.set(event_target_value(&e)) />
                <button class="btn ghost sm" title="Nueva colección" on:click=move |_| new_collection()>"＋"</button>
                <button class="btn ghost sm" title="Importar (Postman / .bru)" on:click=move |_| a.modal.set(Some(Modal::Import))>"⇣"</button>
            </div>
            <div class="scroll">
                <crate::temporales::TempList />
                <For each=move || a.collections.get() key=|c| (c.id.clone(), c.items.len(), format!("{:?}", c.items)) children=move |c: Collection| {
                    let id = c.id.clone();
                    let open = RwSignal::new(true);
                    let id2 = id.clone();
                    let id3 = id.clone();
                    let id4 = id.clone();
                    let items = c.items.clone();
                    view! {
                        <div>
                            <div class="tree-row col" class:on=move || a.collection.get() == id
                                on:click=move |_| { open.update(|o| *o = !*o); a.collection.set(id2.clone()); }
                                on:dragover=move |e| e.prevent_default()
                                on:drop=move |e| drop_on(e, id3.clone(), String::new(), None)>
                                <span class="caret">{move || if open.get() { "▾" } else { "▸" }}</span>
                                <span class="name">{c.name.clone()}</span>
                                <span class="acts">
                                    <button class="btn ghost sm" title="Nueva petición" on:click={let id = id4.clone(); move |e| { e.stop_propagation(); new_request(id.clone(), String::new()) }}>"＋"</button>
                                    <button class="btn ghost sm" title="Nueva carpeta" on:click={let id = id4.clone(); move |e| { e.stop_propagation(); new_folder(id.clone(), String::new()) }}>"▣"</button>
                                    <button class="btn ghost sm" title="Pegar cURL aquí" on:click={let id = id4.clone(); move |e| { e.stop_propagation(); a.collection.set(id.clone()); a.modal.set(Some(Modal::Curl { text: String::new(), folder: Some(String::new()) })) }}>"⌘V"</button>
                                    <button class="btn ghost sm danger" title="Eliminar (va a la papelera)" on:click={let id = id4.clone(); let name = c.name.clone(); move |e| { e.stop_propagation();
                                        a.modal.set(Some(Modal::Confirm { text: format!("¿Mover «{name}» a la papelera (.workbench/papelera)?"), action: ConfirmAction::DeleteCollection(id.clone()) })) }}>"🗑"</button>
                                </span>
                            </div>
                            <div style:display={move || if open.get() { "block" } else { "none" }}>
                                {tree(c.id.clone(), items.clone(), 1, filter)}
                            </div>
                        </div>
                    }
                } />
                {move || a.collections.with(|c| c.is_empty()).then(|| view! {
                    <div class="empty">"No hay colecciones."<br/><button class="btn" on:click=move |_| new_collection()>"Crear colección"</button></div>
                })}
            </div>
        </aside>
    }
}

fn matches(it: &TreeItem, f: &str) -> bool {
    f.is_empty() || it.name.to_lowercase().contains(f) || it.items.iter().any(|x| matches(x, f))
}

fn parent_of(path: &str) -> String {
    path.rsplit_once('/').map(|x| x.0.to_string()).unwrap_or_default()
}

/// Soltar una petición: mitad superior = antes, inferior = después, sobre carpeta/colección = al final.
fn drop_on(e: web_sys::DragEvent, col: String, folder: String, before: Option<String>) {
    e.prevent_default();
    e.stop_propagation();
    let Some(dt) = e.data_transfer() else { return };
    let Ok(raw) = dt.get_data(DND_REQ) else { return };
    let Ok(v) = serde_json::from_str::<Value>(&raw) else { return };
    if v["collection"].as_str() != Some(&col) {
        app().err("Por ahora solo se mueve dentro de la misma colección");
        return;
    }
    let from = v["path"].as_str().unwrap_or_default().to_string();
    if Some(&from) == before.as_ref() {
        return;
    }
    let a = app();
    spawn_local(async move {
        match api::post::<Value>("/api/request/move", &json!({"collection": col, "from": from, "toFolder": folder, "before": before})).await {
            Ok(r) => {
                let new = r["path"].as_str().unwrap_or_default().to_string();
                a.screens.with_untracked(|ss| {
                    for s in ss {
                        for p in s.panels.get_untracked() {
                            if p.path.get_untracked().as_deref() == Some(&from) {
                                p.path.set(Some(new.clone()));
                            }
                        }
                    }
                });
                let n = r["flowsUpdated"].as_u64().unwrap_or(0);
                if n > 0 {
                    a.toast("ok", format!("Movida; {n} flujo(s) actualizados"));
                }
            }
            Err(e) => a.err(e),
        }
    });
}

fn tree(col: String, items: Vec<TreeItem>, depth: usize, filter: RwSignal<String>) -> AnyView {
    let a = app();
    items
        .into_iter()
        .map(|it| {
            let pad = format!("padding-left:{}px", 8 + depth * 14);
            let (col1, it1) = (col.clone(), it.clone());
            let visible = move || matches(&it1, &filter.get().to_lowercase());
            let drop_cls = RwSignal::new("");
            if it.folder {
                let open = RwSignal::new(true);
                let (c2, p2, c3, p3, c4, p4) = (col.clone(), it.path.clone(), col.clone(), it.path.clone(), col.clone(), it.path.clone());
                view! {
                    <div style:display={move || if visible() { "block" } else { "none" }}>
                        <div class=move || format!("tree-row {}", drop_cls.get()) style=pad on:click=move |_| open.update(|o| *o = !*o)
                            on:dragover=move |e| { e.prevent_default(); drop_cls.set("drop-in") }
                            on:dragleave=move |_| drop_cls.set("")
                            on:drop=move |e| { drop_cls.set(""); drop_on(e, c2.clone(), p2.clone(), None) }>
                            <span class="caret">{move || if open.get() { "▾" } else { "▸" }}</span>
                            <span class="name">"📁 "{it.name.clone()}</span>
                            <span class="acts">
                                <button class="btn ghost sm" title="Nueva petición" on:click=move |e| { e.stop_propagation(); new_request(c3.clone(), p3.clone()) }>"＋"</button>
                                <button class="btn ghost sm" title="Pegar cURL aquí" on:click=move |e| { e.stop_propagation(); a.collection.set(c4.clone()); a.modal.set(Some(Modal::Curl { text: String::new(), folder: Some(p4.clone()) })) }>"⌘V"</button>
                            </span>
                        </div>
                        <div style:display={move || if open.get() { "block" } else { "none" }}>{tree(col1.clone(), it.items.clone(), depth + 1, filter)}</div>
                    </div>
                }
                .into_any()
            } else {
                let (c2, p2, c3, p3, c4, p4) = (col.clone(), it.path.clone(), col.clone(), it.path.clone(), col.clone(), it.path.clone());
                let (p5, c5) = (it.path.clone(), col.clone());
                let name = it.name.clone();
                let is_open = move || a.active().is_some_and(|p| p.path.get().as_deref() == Some(p5.as_str()) && p.collection.get() == c5);
                view! {
                    <div class=move || format!("tree-row {}", drop_cls.get()) class:on=is_open style=pad draggable="true"
                        style:display={move || if visible() { "flex" } else { "none" }}
                        on:click=move |_| a.open_request(c2.clone(), p2.clone())
                        on:dragstart=move |e: web_sys::DragEvent| {
                            if let Some(dt) = e.data_transfer() {
                                let _ = dt.set_data(DND_REQ, &json!({"collection": c3, "path": p3}).to_string());
                                let _ = dt.set_data("text/plain", &p3);
                            }
                        }
                        on:dragover=move |e: web_sys::DragEvent| {
                            e.prevent_default();
                            let el: web_sys::Element = event_target(&e);
                            let r = el.get_bounding_client_rect();
                            drop_cls.set(if (e.client_y() as f64) < r.top() + r.height() / 2.0 { "drop-before" } else { "drop-after" });
                        }
                        on:dragleave=move |_| drop_cls.set("")
                        on:drop={let path = it.path.clone(); move |e: web_sys::DragEvent| {
                            let after = drop_cls.get_untracked() == "drop-after";
                            drop_cls.set("");
                            let siblings = a.coll().map(|c| siblings_of(&c.items, &parent_of(&path))).unwrap_or_default();
                            let before = if after { siblings.iter().position(|s| *s == path).and_then(|i| siblings.get(i + 1).cloned()) } else { Some(path.clone()) };
                            drop_on(e, c4.clone(), parent_of(&p4), before)
                        }}>
                        <span class=format!("m m-{}", it.method)>{it.method.clone()}</span>
                        {(!it.id.is_empty()).then(|| view! { <span class="req-id">{it.id.clone()}</span> })}
                        <span class="name">{name.clone()}</span>
                        <span class="acts">
                            <button class="btn ghost sm danger" title="Eliminar" on:click={let (c, p) = (col.clone(), it.path.clone()); move |e| { e.stop_propagation();
                                a.modal.set(Some(Modal::Confirm { text: format!("¿Eliminar «{name}»?"), action: ConfirmAction::DeleteRequest(c.clone(), p.clone()) })) }}>"×"</button>
                        </span>
                    </div>
                }
                .into_any()
            }
        })
        .collect_view()
        .into_any()
}

fn siblings_of(items: &[TreeItem], folder: &str) -> Vec<String> {
    if folder.is_empty() {
        return items.iter().filter(|i| !i.folder).map(|i| i.path.clone()).collect();
    }
    for i in items.iter().filter(|i| i.folder) {
        if i.path == folder {
            return i.items.iter().filter(|x| !x.folder).map(|x| x.path.clone()).collect();
        }
        let s = siblings_of(&i.items, folder);
        if !s.is_empty() {
            return s;
        }
    }
    vec![]
}

fn prompt(msg: &str, def: &str) -> Option<String> {
    web_sys::window()?.prompt_with_message_and_default(msg, def).ok()?.filter(|s| !s.trim().is_empty())
}

pub fn new_collection() {
    let a = app();
    let Some(name) = prompt("Nombre de la colección", "Mi colección") else { return };
    spawn_local(async move {
        match api::post::<Value>("/api/collections", &json!({"name": name})).await {
            Ok(v) => {
                a.collection.set(v["id"].as_str().unwrap_or_default().into());
                a.reload_collections();
            }
            Err(e) => a.err(e),
        }
    });
}

fn new_folder(col: String, parent: String) {
    let a = app();
    let Some(name) = prompt("Nombre de la carpeta", "Nueva carpeta") else { return };
    spawn_local(async move {
        if let Err(e) = api::post::<Value>("/api/folder", &json!({"collection": col, "parent": parent, "name": name})).await {
            a.err(e)
        }
    });
}

pub fn new_request(col: String, folder: String) {
    let a = app();
    let Some(name) = prompt("Nombre de la petición", "Nueva petición") else { return };
    let req = Request { name, ..Request::new("get", "{{baseUrl}}/") };
    spawn_local(async move {
        match api::post::<Value>(&format!("/api/request?collection={}&folder={}", api::enc(&col), api::enc(&folder)), &req).await {
            Ok(v) => a.open_request(col, v["path"].as_str().unwrap_or_default().into()),
            Err(e) => a.err(e),
        }
    });
}

// ---------- pantallas ----------

#[component]
pub fn Screens() -> impl IntoView {
    let a = app();
    let editing = RwSignal::new(String::new());
    view! {
        <div class="screens">
            <For each=move || a.screens.get() key=|s| s.id.clone() children=move |s: Screen| {
                let (id, id2, id3, id4) = (s.id.clone(), s.id.clone(), s.id.clone(), s.id.clone());
                let drop = RwSignal::new(false);
                let close_id = s.id.clone();
                let count = move || s.panels.with(|p| p.len());
                view! {
                    <div class="st" class:on=move || a.screen.get() == id class:drop=move || drop.get()
                        on:click=move |_| a.screen.set(id2.clone())
                        on:dblclick=move |_| editing.set(id3.clone())
                        on:dragover=move |e: web_sys::DragEvent| { if is_panel_drag(&e) { e.prevent_default(); drop.set(true) } }
                        on:dragleave=move |_| drop.set(false)
                        on:drop=move |e: web_sys::DragEvent| { drop.set(false); move_panel_to_screen(&e, &id4) }>
                        {move || if editing.get() == s.id {
                            view! { <input class="in" data-noac prop:value=s.name.get_untracked()
                                on:change=move |e| { s.name.set(event_target_value(&e)); editing.set(String::new()) }
                                on:blur=move |_| editing.set(String::new()) /> }.into_any()
                        } else {
                            view! { <span title="Doble clic para renombrar · suelta aquí un panel para moverlo">{move || s.name.get()}</span> }.into_any()
                        }}
                        <span class="muted">{count}</span>
                        <button class="btn ghost sm" title="Cerrar pantalla" on:click=move |e| { e.stop_propagation(); close_screen(&close_id) }>"×"</button>
                    </div>
                }
            } />
            <button class="btn ghost sm" title="Nueva pantalla" on:click=move |_| {
                let n = a.screens.with(|s| s.len()) + 1;
                let s = Screen::new(&format!("Pantalla {n}"));
                a.screen.set(s.id.clone());
                a.screens.update(|ss| ss.push(s));
            }>"＋ pantalla"</button>
            <span class="grow"></span>
            <button class="btn sm" title="Ejecutar toda la pantalla en paralelo (Ctrl+Shift+Enter)" on:click=move |_| run_screen()>"▶▶ Pantalla"</button>
        </div>
    }
}

pub fn run_screen() {
    let a = app();
    if let Some(s) = a.current_screen() {
        a.run_panels(s.panels.get_untracked());
    }
}

fn close_screen(id: &str) {
    let a = app();
    a.screens.update(|ss| {
        if ss.len() > 1 {
            ss.retain(|s| s.id != id);
        }
    });
    if a.screen.get_untracked() == id {
        a.screen.set(a.screens.with_untracked(|s| s[0].id.clone()));
    }
}

const DND_PANEL: &str = "application/x-martino-panel";

fn is_panel_drag(e: &web_sys::DragEvent) -> bool {
    e.data_transfer().is_some_and(|d| d.types().includes(&DND_PANEL.into(), 0))
}

fn move_panel_to_screen(e: &web_sys::DragEvent, target: &str) {
    let Some(pid) = e.data_transfer().and_then(|d| d.get_data(DND_PANEL).ok()).filter(|s| !s.is_empty()) else { return };
    e.prevent_default();
    let a = app();
    let mut moved = None;
    a.screens.with_untracked(|ss| {
        for s in ss {
            s.panels.update(|ps| {
                if let Some(i) = ps.iter().position(|p| p.id == pid) {
                    moved = Some(ps.remove(i));
                }
            })
        }
    });
    if let Some(p) = moved {
        a.screens.with_untracked(|ss| ss.iter().find(|s| s.id == target).map(|s| s.panels.update(|ps| ps.push(p))));
        a.screen.set(target.into());
        a.active_panel.set(pid);
    }
}

// ---------- paneles ----------

#[component]
pub fn Panels() -> impl IntoView {
    let a = app();
    let screen = move || a.current_screen();
    view! {
        <div class="work">
            <Screens />
            <div class="panels">
                {move || screen().map(|s| {
                    let panels = s.panels;
                    view! {
                        <For each=move || panels.get() key=|p| p.id.clone() children=move |p: Panel| view! { <PanelView p=p panels=panels /> } />
                        {move || panels.with(|p| p.is_empty()).then(|| view! {
                            <div class="empty grow">
                                <p>"Abre una petición de la barra lateral, crea una nueva o pega un cURL (Ctrl+V fuera de un campo)."</p>
                                <div class="row" style="justify-content:center">
                                    <button class="btn" on:click=move |_| { let col = a.collection.get_untracked(); a.add_panel(Panel::new(&col, None, Request { name: "Sin título".into(), ..Request::new("get", "{{baseUrl}}/") })) }>"Panel vacío"</button>
                                    <button class="btn" on:click=move |_| a.modal.set(Some(Modal::Curl { text: String::new(), folder: None }))>"Pegar cURL"</button>
                                </div>
                            </div>
                        })}
                    }
                })}
            </div>
        </div>
    }
}

fn move_panel(panels: RwSignal<Vec<Panel>>, id: &str, delta: isize) {
    panels.update(|ps| {
        if let Some(i) = ps.iter().position(|p| p.id == id) {
            let j = (i as isize + delta).clamp(0, ps.len() as isize - 1) as usize;
            let p = ps.remove(i);
            ps.insert(j, p);
        }
    });
}

#[component]
fn PanelView(p: Panel, panels: RwSignal<Vec<Panel>>) -> impl IntoView {
    let a = app();
    let pid = p.id.clone();
    let el = NodeRef::<leptos::html::Div>::new();
    let drop = RwSignal::new(false);
    let start_w = RwSignal::new(0.0);
    let split_ref = NodeRef::<leptos::html::Div>::new();
    let start_split = RwSignal::new(0.0);
    let set_req = move |f: Box<dyn Fn(&mut Request)>| {
        p.req.update(|r| f(r));
        p.dirty.set(true);
    };
    // variables que faltan en el entorno activo (aviso antes de enviar)
    let missing = Memo::new(move |_| {
        let r = p.req.get();
        let mut names = vec![];
        let v = serde_json::to_value(&r).unwrap();
        collect_vars(&v, &mut names);
        names.sort();
        names.dedup();
        let _ = a.vars.get();
        names.into_iter().filter(|n| matches!(crate::ui::var_source(n), VarSource::Missing)).collect::<Vec<_>>()
    });
    let (id1, id2, id3, id4, id5) = (pid.clone(), pid.clone(), pid.clone(), pid.clone(), pid.clone());
    let pp = p.clone();
    let p_close = p.clone();
    view! {
        <div class="panel" node_ref=el class:active=move || a.active_panel.get() == id1 class:drop=move || drop.get() class:tmp=move || p.path.get().is_none()
            style:flex={move || if p.width.get() > 0.0 { format!("0 0 {}px", p.width.get()) } else { "1 1 0".into() }}
            on:mousedown=move |_| a.active_panel.set(id2.clone())
            on:dragover=move |e: web_sys::DragEvent| { if is_panel_drag(&e) { e.prevent_default(); drop.set(true) } }
            on:dragleave=move |_| drop.set(false)
            on:drop=move |e: web_sys::DragEvent| {
                drop.set(false);
                let Some(from) = e.data_transfer().and_then(|d| d.get_data(DND_PANEL).ok()) else { return };
                panels.update(|ps| {
                    if let (Some(i), Some(j)) = (ps.iter().position(|x| x.id == from), ps.iter().position(|x| x.id == id3)) {
                        let x = ps.remove(i); ps.insert(j, x);
                    }
                });
            }>
            <div class="ph">
                <span class="grip" draggable="true" title="Arrastra para reordenar o suéltalo en otra pantalla"
                    on:dragstart=move |e: web_sys::DragEvent| { if let Some(d) = e.data_transfer() { let _ = d.set_data(DND_PANEL, &id4); } }>"⋮⋮"</span>
                <span class=move || format!("m m-{}", p.req.with(|r| r.method.clone()))>{move || p.req.with(|r| r.method.clone())}</span>
                {move || if p.path.get().is_none() { Some(view! { <span class="tmp-tag" title="Temporal: no está en la colección">"TMP"</span> }.into_any()) }
                    else { p.req.with(|r| (!r.id.is_empty()).then(|| view! { <span class="req-id" title="ID de la petición (meta.id)">{r.id.clone()}</span> }.into_any())) }}
                <span class="title" class:dirty=move || p.dirty.get() title=move || p.path.get().unwrap_or_else(|| "sin guardar".into())>{move || p.req.with(|r| r.name.clone())}</span>
                <button class="btn ghost sm" title="Mover a la izquierda" on:click={let id = pid.clone(); move |_| move_panel(panels, &id, -1)}>"◀"</button>
                <button class="btn ghost sm" title="Mover a la derecha" on:click={let id = pid.clone(); move |_| move_panel(panels, &id, 1)}>"▶"</button>
                <button class="btn ghost sm" title="Generar cURL" on:click=move |_| a.modal.set(Some(Modal::CurlOut(martino_core::curl::to_curl(&p.req.get_untracked()))))>"cURL"</button>
                <button class="btn ghost sm" title="Guardar (Ctrl+S)" on:click={let p = pp.clone(); move |_| a.save_panel(&p)}>"💾"</button>
                <button class="btn ghost sm" title="Cerrar" on:click=move |_| {
                    if p_close.dirty.get_untracked() && !web_sys::window().unwrap().confirm_with_message("Hay cambios sin guardar. ¿Cerrar igual?").unwrap_or(false) { return }
                    panels.update(|ps| ps.retain(|x| x.id != id5));
                }>"✕"</button>
            </div>
            <crate::temporales::TempBanner p=p.clone() />
            <div class="urlbar">
                <select class="in" prop:value=move || p.req.with(|r| r.method.clone()) on:change=move |e| { let v = event_target_value(&e); set_req(Box::new(move |r| r.method = v.clone())) }>
                    {METHODS.iter().map(|m| view! { <option value=*m>{m.to_uppercase()}</option> }).collect_view()}
                </select>
                <VarField value=Signal::derive(move || p.req.with(|r| r.url.clone())) placeholder="{{baseUrl}}/ruta"
                    on_input=move |v: String| set_req(Box::new(move |r| r.url = v.clone()))
                    on_paste=move |e: web_sys::ClipboardEvent| {
                        let t = e.clipboard_data().and_then(|d| d.get_data("text").ok()).unwrap_or_default();
                        if martino_core::curl::is_curl(&t) { e.prevent_default(); a.modal.set(Some(Modal::Curl { text: t, folder: None })) }
                    } />
                <button class="btn primary" title="Ctrl+Enter" disabled=move || p.running.get() on:click={let p = p.clone(); move |_| a.run_panels(vec![p.clone()])}>
                    {move || if p.running.get() { "…" } else { "Enviar" }}
                </button>
            </div>
            {move || {
                let m = missing.get();
                (!m.is_empty()).then(|| view! {
                    <div class="missing-bar">
                        <span class="chip err">{format!("{} sin definir en «{}»", m.len(), a.env.get())}</span>
                        {m.into_iter().map(|n| { let n2 = n.clone(); view! {
                            <button class="btn sm v-missing" title="Crear en el entorno activo" on:click=move |_| a.modal.set(Some(Modal::Env(Some(n2.clone()))))>{format!("{{{{{n}}}}}")}</button>
                        }}).collect_view()}
                    </div>
                })
            }}
            <div class="split" node_ref=split_ref>
                <div class="pane" style:flex=move || format!("0 0 {}%", p.split.get() * 100.0)>
                    <RequestTabs p=p.clone() />
                </div>
                <Gutter horizontal=true
                    on_drag=move |(_, dy, start): (f64, f64, bool)| {
                        if start { start_split.set(p.split.get_untracked()); return }
                        let h = split_ref.get().map(|e| e.client_height() as f64).unwrap_or(500.0);
                        p.split.set((start_split.get_untracked() + dy / h).clamp(0.12, 0.88));
                    }
                    on_reset=move |_| p.split.set(0.45) />
                <div class="pane grow"><ResponseView p=p.clone() /></div>
            </div>
        </div>
        <Gutter on_drag=move |(dx, _, start): (f64, f64, bool)| {
                if start { start_w.set(el.get().map(|e| e.client_width() as f64).unwrap_or(400.0)); return }
                p.width.set((start_w.get_untracked() + dx).max(340.0));
            }
            on_reset=move |_| p.width.set(0.0) />
    }
}

fn collect_vars(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.extend(var_names(s)),
        Value::Array(a) => a.iter().for_each(|x| collect_vars(x, out)),
        Value::Object(m) => m.iter().filter(|(k, _)| !matches!(k.as_str(), "docs" | "tests" | "scriptPre" | "scriptPost")).for_each(|(_, x)| collect_vars(x, out)),
        _ => {}
    }
}

#[component]
fn RequestTabs(p: Panel) -> impl IntoView {
    let set = move |f: Box<dyn Fn(&mut Request)>| {
        p.req.update(|r| f(r));
        p.dirty.set(true);
    };
    let count = move |f: fn(&Request) -> usize| move || {
        let n = p.req.with(f);
        (n > 0).then(|| view! { <span class="n">{n}</span> })
    };
    let tab = move |id: &'static str, label: &'static str, n: Option<fn(&Request) -> usize>| {
        view! { <button class:on=move || p.tab.get() == id on:click=move |_| p.tab.set(id)>{label}{n.map(count)}</button> }
    };
    let pairs = move |get: fn(&Request) -> &Vec<Pair>, put: fn(&mut Request, Vec<Pair>)| {
        view! { <KvTable pairs=Signal::derive(move || p.req.with(|r| get(r).clone())) on_change=move |v: Pairs| set(Box::new(move |r| put(r, v.clone()))) /> }
    };
    view! {
        <div class="tabs">
            {tab("params", "Params", Some(|r| r.params.iter().filter(|x| x.enabled).count()))}
            {tab("headers", "Headers", Some(|r| r.headers.iter().filter(|x| x.enabled).count()))}
            {tab("body", "Body", None)}
            {tab("auth", "Auth", None)}
            {tab("vars", "Vars", Some(|r| r.vars_pre.len() + r.vars_post.len()))}
            {tab("asserts", "Asserts", Some(|r| r.asserts.len()))}
            {tab("docs", "Docs", None)}
        </div>
        <div class="pane-body">
            {move || match p.tab.get() {
                "params" => pairs(|r| &r.params, |r, v| r.params = v).into_any(),
                "headers" => pairs(|r| &r.headers, |r, v| r.headers = v).into_any(),
                "body" => view! { <BodyEditor p=p.clone() /> }.into_any(),
                "auth" => view! {
                    <div class="col">
                        <div class="row"><span class="muted">"Tipo"</span>
                            <select class="in" prop:value=move || p.req.with(|r| r.auth_mode.clone()) on:change=move |e| { let v = event_target_value(&e); set(Box::new(move |r| r.auth_mode = v.clone())) }>
                                <option value="none">"Sin auth"</option><option value="inherit">"Heredar de la colección"</option>
                                <option value="bearer">"Bearer"</option><option value="basic">"Basic"</option>
                            </select></div>
                        {move || match p.req.with(|r| r.auth_mode.clone()).as_str() {
                            "bearer" => view! { <div class="row"><span class="muted">"Token"</span>
                                <VarField value=Signal::derive(move || p.req.with(|r| r.auth.token.clone())) placeholder="{{token}}" on_input=move |v: String| set(Box::new(move |r| r.auth.token = v.clone())) /></div> }.into_any(),
                            "basic" => view! { <div class="col">
                                <div class="row"><span class="muted">"Usuario"</span><VarField value=Signal::derive(move || p.req.with(|r| r.auth.username.clone())) on_input=move |v: String| set(Box::new(move |r| r.auth.username = v.clone())) /></div>
                                <div class="row"><span class="muted">"Clave"</span><VarField value=Signal::derive(move || p.req.with(|r| r.auth.password.clone())) on_input=move |v: String| set(Box::new(move |r| r.auth.password = v.clone())) /></div>
                            </div> }.into_any(),
                            "inherit" => view! { <p class="muted">"Usa la auth de collection.bru."</p> }.into_any(),
                            _ => ().into_any(),
                        }}
                    </div>
                }.into_any(),
                "vars" => view! {
                    <div class="col">
                        <b>"vars:pre-request"</b><span class="muted">"Ganan al entorno; solo para esta petición."</span>
                        {pairs(|r| &r.vars_pre, |r, v| r.vars_pre = v)}
                        <b>"vars:post-response"</b><span class="muted">"res.body.x · res.status · res.headers.x → quedan como variables runtime."</span>
                        {pairs(|r| &r.vars_post, |r, v| r.vars_post = v)}
                    </div>
                }.into_any(),
                "asserts" => view! {
                    <div class="col"><span class="muted">"res.status → eq 200 · res.body.id → isDefined · res.body.lista → contains x"</span>
                    {pairs(|r| &r.asserts, |r, v| r.asserts = v)}</div>
                }.into_any(),
                _ => view! {
                    <textarea class="in" rows=10 style="width:100%" data-noac prop:value=move || p.req.with(|r| r.docs.clone())
                        on:input=move |e| { let v = event_target_value(&e); set(Box::new(move |r| r.docs = v.clone())) }></textarea>
                }.into_any(),
            }}
        </div>
    }
}

#[component]
fn BodyEditor(p: Panel) -> impl IntoView {
    let set = move |f: Box<dyn Fn(&mut Request)>| {
        p.req.update(|r| f(r));
        p.dirty.set(true);
    };
    let mode = move || p.req.with(|r| r.body_mode.clone());
    view! {
        <div class="col">
            <div class="row">
                <select class="in" prop:value=mode on:change=move |e| { let v = event_target_value(&e); set(Box::new(move |r| r.body_mode = v.clone())) }>
                    {["none", "json", "text", "xml", "form-urlencoded", "multipart-form"].map(|m| view! { <option value=m>{m}</option> }).collect_view()}
                </select>
                {move || (mode() == "json").then(|| view! {
                    <button class="btn sm" on:click=move |_| set(Box::new(|r| r.body.json = beautify_json(&r.body.json)))>"Formatear"</button>
                })}
            </div>
            {move || match mode().as_str() {
                "json" | "text" | "xml" => {
                    let m = mode();
                    let m2 = m.clone();
                    view! { <VarField multi=true rows=14
                        value=Signal::derive(move || p.req.with(|r| match m.as_str() { "json" => r.body.json.clone(), "xml" => r.body.xml.clone(), _ => r.body.text.clone() }))
                        on_input=move |v: String| { let m = m2.clone(); set(Box::new(move |r| match m.as_str() { "json" => r.body.json = v.clone(), "xml" => r.body.xml = v.clone(), _ => r.body.text = v.clone() })) }
                        on_paste=move |e: web_sys::ClipboardEvent| {
                            let t = e.clipboard_data().and_then(|d| d.get_data("text").ok()).unwrap_or_default();
                            if looks_minified_json(&t) {
                                e.prevent_default();
                                let pretty = beautify_json(&t);
                                set(Box::new(move |r| r.body.json = if r.body.json.trim().is_empty() { pretty.clone() } else { format!("{}{}", r.body.json, pretty) }));
                            }
                        } /> }.into_any()
                }
                "form-urlencoded" => view! { <KvTable pairs=Signal::derive(move || p.req.with(|r| r.body.form.clone())) on_change=move |v: Pairs| set(Box::new(move |r| r.body.form = v.clone())) /> }.into_any(),
                "multipart-form" => view! { <KvTable pairs=Signal::derive(move || p.req.with(|r| r.body.multipart.clone())) on_change=move |v: Pairs| set(Box::new(move |r| r.body.multipart = v.clone())) /> }.into_any(),
                _ => view! { <p class="muted">"Sin body. Ojo: GET con body también se puede."</p> }.into_any(),
            }}
        </div>
    }
}

#[component]
fn ResponseView(p: Panel) -> impl IntoView {
    let a = app();
    let tab = move |id: &'static str, label: String| view! { <button class:on=move || p.resp_tab.get() == id on:click=move |_| p.resp_tab.set(id)>{label}</button> };
    view! {
        {move || match p.result.get() {
            None => view! { <div class="empty">{move || if p.running.get() { "Enviando…" } else { "Ctrl+Enter para enviar" }}</div> }.into_any(),
            Some(r) => {
                let status = r["status"].as_u64();
                let asserts = r["asserts"].as_array().cloned().unwrap_or_default();
                let fails = asserts.iter().filter(|x| x["pass"] != json!(true)).count();
                let sent: SentRequest = serde_json::from_value(r["sent"].clone()).unwrap_or_default();
                let headers: Vec<Pair> = serde_json::from_value(r["headers"].clone()).unwrap_or_default();
                let r2 = r.clone();
                view! {
                    <div class="resp-meta">
                        <span class=status_class(status)>{status.map(|s| format!("{s} {}", r["statusText"].as_str().unwrap_or(""))).unwrap_or_else(|| "Error".into())}</span>
                        <span class="muted">{format!("{} ms", r["timeMs"])}</span>
                        <span class="muted">{fmt_size(r["size"].as_u64().unwrap_or(0))}</span>
                        {(!asserts.is_empty()).then(|| view! { <span class=if fails == 0 { "chip ok" } else { "chip err" }>{format!("asserts {}/{}", asserts.len() - fails, asserts.len())}</span> })}
                        {r["vars"].as_array().filter(|v| !v.is_empty()).map(|v| view! { <span class="chip v-runtime" title="vars:post-response guardadas como runtime">{format!("+{} runtime", v.len())}</span> })}
                        <span class="grow"></span>
                        <button class="btn ghost sm" on:click={let b = r["body"].as_str().unwrap_or("").to_string(); move |_| copy(&b)}>"Copiar"</button>
                    </div>
                    {r["error"].as_str().map(|e| view! { <div class="missing-bar"><span class="chip err">{e.to_string()}</span></div> })}
                    <div class="tabs">
                        {tab("body", "Body".into())}{tab("headers", format!("Headers {}", headers.len()))}
                        {tab("sent", "Enviada".into())}{(!asserts.is_empty()).then(|| tab("asserts", "Asserts".into()))}
                    </div>
                    {move || match p.resp_tab.get() {
                        "headers" => view! { <table class="kv mono">{headers.iter().map(|h| view! { <tr><td>{h.name.clone()}</td><td>{h.value.clone()}</td></tr> }).collect_view()}</table> }.into_any(),
                        "sent" => {
                            let sent = sent.clone();
                            view! {
                                <div class="row" style="padding:6px 8px"><label class="row"><input type="checkbox" prop:checked=move || a.reveal.get() on:change=move |e| a.reveal.set(event_target_checked(&e)) />"Mostrar credenciales"</label></div>
                                <pre class="code">{move || raw_http(&sent, !a.reveal.get())}</pre>
                            }.into_any()
                        }
                        "asserts" => view! { <table class="kv">{asserts.iter().map(|x| view! {
                            <tr><td>{if x["pass"] == json!(true) { "✅" } else { "❌" }}</td><td class="mono">{x["name"].as_str().unwrap_or("").to_string()}</td>
                            <td class="mono">{format!("{} {}", x["op"].as_str().unwrap_or(""), x["expected"].as_str().unwrap_or(""))}</td><td class="mono muted">{x["actual"].to_string()}</td></tr>
                        }).collect_view()}</table> }.into_any(),
                        _ => match r2.get("json").filter(|j| !j.is_null()) {
                            Some(j) => view! { <JsonTree value=j.clone() prefix="res.body" /> }.into_any(),
                            None => view! { <pre class="code">{r2["body"].as_str().unwrap_or("").to_string()}</pre> }.into_any(),
                        },
                    }}
                }.into_any()
            }
        }}
    }
}
