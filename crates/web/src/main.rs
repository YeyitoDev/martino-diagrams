mod api;
mod cases;
mod flow_panels;
mod flow_steps;
mod flow_story;
mod flow_inspector;
mod flows;
mod modals;
mod requests;
mod state;
mod tables;
mod temporales;
mod ui;

use leptos::prelude::*;
use state::{local_set, App, Mode, Modal};
use wasm_bindgen::JsCast;

fn main() {
    console_error_panic_hook::set_once();
    if let Some(sw) = web_sys::window().map(|w| w.navigator().service_worker()) {
        let _ = sw.register("/sw.js");
    }
    leptos::mount::mount_to_body(Shell);
}

fn in_field(e: &web_sys::KeyboardEvent) -> bool {
    e.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()).is_some_and(|el| matches!(el.tag_name().as_str(), "INPUT" | "TEXTAREA" | "SELECT"))
}

#[component]
fn Shell() -> impl IntoView {
    let a = App::new();
    // arranque: workspace → colecciones → eventos
    leptos::task::spawn_local(async move {
        // Si el servidor aún está arrancando se reintenta: el guardado automático no corre hasta restaurar.
        while !a.restore_workspace().await {
            let wait = js_sys::Promise::new(&mut |r, _| { let _ = leptos::prelude::window().set_timeout_with_callback_and_timeout_and_arguments_0(&r, 1000); });
            let _ = wasm_bindgen_futures::JsFuture::from(wait).await;
        }
        a.reload_collections();
        a.listen_events();
        a.loaded.set(true);
    });
    Effect::new(move |_| {
        let t = a.theme.get();
        let _ = ui::document().document_element().unwrap().set_attribute("data-theme", &t);
        local_set("martino.theme", &t);
    });
    // tabla de variables: cambia con la colección, el entorno o un evento runtime
    Effect::new(move |_| {
        let _ = (a.collection.get(), a.env.get(), a.vars_tick.get());
        a.refresh_vars();
    });
    // guardado del workspace (agrupado a 800 ms)
    let tick = RwSignal::new(0u32);
    Effect::new(move |_| {
        let _ = (a.mode.get(), a.screen.get(), a.collection.get(), a.env.get(), a.sidebar.get(), a.flow_open.get());
        a.screens.with(|ss| ss.iter().for_each(|s| { s.name.track(); s.panels.with(|ps| ps.iter().for_each(|p| { p.req.track(); p.path.track(); p.width.track(); p.split.track(); })) }));
        if !a.loaded.get_untracked() { return }
        let n = tick.get_untracked() + 1;
        tick.set(n);
        set_timeout(move || if tick.get_untracked() == n {
            let w = a.workspace_json();
            leptos::task::spawn_local(async move { let _ = api::put::<serde_json::Value>("/api/workspaces", &w).await; });
        }, std::time::Duration::from_millis(800));
    });
    let keys = window_event_listener(leptos::ev::keydown, move |e| {
        let ctrl = e.ctrl_key() || e.meta_key();
        match (ctrl, e.shift_key(), e.key().as_str()) {
            (true, true, "Enter") => { e.prevent_default(); requests::run_screen() }
            (true, false, "Enter") if a.mode.get_untracked() == Mode::Requests => { e.prevent_default(); if let Some(p) = a.active() { a.run_panels(vec![p]) } }
            (true, _, "s") if a.mode.get_untracked() == Mode::Requests => { e.prevent_default(); if let Some(p) = a.active() { a.save_panel(&p) } }
            (true, _, "b") => { e.prevent_default(); a.sidebar.update(|s| *s = !*s) }
            (true, _, "e") => { e.prevent_default(); a.modal.set(Some(Modal::Env(None))) }
            (true, _, "1") => { e.prevent_default(); a.mode.set(Mode::Requests) }
            (true, _, "2") => { e.prevent_default(); a.mode.set(Mode::Flows) }
            (_, _, "Escape") if a.modal.get_untracked().is_some() => a.modal.set(None),
            _ => {}
        }
    });
    let paste = window_event_listener(leptos::ev::paste, move |e: web_sys::ClipboardEvent| {
        let target_field = e.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()).is_some_and(|el| matches!(el.tag_name().as_str(), "INPUT" | "TEXTAREA"));
        if target_field || a.modal.get_untracked().is_some() || a.mode.get_untracked() != Mode::Requests { return }
        let t = e.clipboard_data().and_then(|d| d.get_data("text").ok()).unwrap_or_default();
        if martino_core::curl::is_curl(&t) { e.prevent_default(); a.modal.set(Some(Modal::Curl { text: t, folder: None })) }
    });
    on_cleanup(move || { keys.remove(); paste.remove(); });
    let _ = in_field;
    let start_w = RwSignal::new(0.0);
    let folder = RwSignal::new(String::new());
    leptos::task::spawn_local(async move {
        if let Ok(v) = api::get::<serde_json::Value>("/api/root").await {
            let r = v["root"].as_str().unwrap_or("");
            folder.set(r.rsplit(['/', '\\']).find(|s| !s.is_empty()).unwrap_or(r).to_string());
        }
    });
    let envs = move || a.coll().map(|c| c.environments).unwrap_or_default();
    // Sin selector de colección arriba (se elige en la barra lateral): si el entorno activo no existe
    // en la colección nueva, se pasa a su primer entorno.
    Effect::new(move |_| {
        let list = envs();
        if !list.is_empty() && !list.contains(&a.env.get_untracked()) {
            a.env.set(list[0].clone());
        }
    });
    view! {
        <div id="app">
            <header class="top">
                <div class="brand"><ui::Cat />"Martino"</div>
                <div class="seg modeseg" role="tablist">
                    // La «gota»: un fondo que se estira y cae sobre el modo elegido.
                    <span class=move || if a.mode.get() == Mode::Flows { "blob r" } else { "blob l" }></span>
                    <button class:on=move || a.mode.get() == Mode::Requests on:click=move |_| a.mode.set(Mode::Requests) title="Ctrl+1">"Peticiones"</button>
                    <button class:on=move || a.mode.get() == Mode::Flows on:click=move |_| a.mode.set(Mode::Flows) title="Ctrl+2">"Flujos"</button>
                </div>
                <div class="envsel" title="Entorno activo: de aquí salen los valores de {{variables}}">
                    <span class="envsel-l">"Entorno"</span>
                    <span class="dot" class:on=move || !a.env.get().is_empty()></span>
                    <select class="in" prop:value=move || a.env.get() on:change=move |e| a.env.set(event_target_value(&e))>
                        <option value="">"Sin entorno"</option>
                        {move || envs().into_iter().map(|e| { let on = e.clone(); view! { <option value=e.clone() selected=move || a.env.get() == on>{e.clone()}</option> } }).collect_view()}
                    </select>
                    <button class="btn varbtn" title="Ver y editar las variables de cada entorno (Ctrl+E)" on:click=move |_| a.modal.set(Some(Modal::Env(None)))>
                        "Variables"<span class="badge" title="Variables disponibles en este entorno">{move || a.vars.with(|v| v.len())}</span>
                    </button>
                </div>
                <span class="grow"></span>
                <button class="btn ghost" title="Carpeta de trabajo: cambia dónde se guardan tus datos" on:click=move |_| a.modal.set(Some(Modal::Folder))>"📁 "{move || folder.get()}</button>
                <button class="btn ghost" title="Pegar cURL" on:click=move |_| a.modal.set(Some(Modal::Curl { text: String::new(), folder: None }))>"cURL"</button>
                <button class="btn ghost" title="Ajustes" on:click=move |_| a.modal.set(Some(Modal::Settings))>"⚙"</button>
                <button class="btn ghost" title="Tema" on:click=move |_| a.theme.update(|t| *t = if t == "dark" { "light".into() } else { "dark".into() })>{move || if a.theme.get() == "dark" { "☀" } else { "☾" }}</button>
                <button class="btn ghost" title="Ayuda" on:click=move |_| a.modal.set(Some(Modal::Help))>"?"</button>
            </header>
            <div class="main">
                <requests::Sidebar />
                {move || a.sidebar.get().then(|| view! { <ui::Gutter
                    on_drag=move |(dx, _, start): (f64, f64, bool)| { if start { start_w.set(a.sidebar_w.get_untracked()); return } a.sidebar_w.set((start_w.get_untracked() + dx).clamp(180.0, 600.0)) }
                    on_reset=move |_| a.sidebar_w.set(270.0) /> })}
                {move || match a.mode.get() {
                    Mode::Requests => view! { <requests::Panels /> }.into_any(),
                    Mode::Flows => view! { <flows::FlowsView /> }.into_any(),
                }}
            </div>
            <footer class="status">
                <span>{move || format!("{} variables en «{}»", a.vars.with(|v| v.len()), a.env.get())}</span>
                <span class="grow"></span>
                <span>"Ctrl+Enter enviar · Ctrl+Shift+Enter pantalla · Ctrl+E variables"</span>
            </footer>
            <modals::Modals />
            <ui::VarTip />
            <ui::HoverTip />
            <ui::Autocomplete />
            <div class="toasts">{move || a.toasts.get().into_iter().map(|(_, k, m)| view! { <div class=format!("toast {k}")>{m}</div> }).collect_view()}</div>
        </div>
    }
}
