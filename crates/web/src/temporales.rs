//! Temporales: peticiones abiertas que no están en la colección (pegadas de un cURL, nuevas o de un
//! agente). Viven sólo en tu espacio de trabajo (`.workbench`), no en el .bru ni en git, hasta que
//! las guardas; al guardar reciben su ID (el que traían si está libre, o el siguiente de la carpeta).
use crate::api;
use crate::state::{app, Panel, TreeItem};
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde_json::Value;

/// Todas las temporales abiertas, con la pantalla donde están.
fn temporales() -> Vec<(String, Panel)> {
    app().screens.with(|ss| ss.iter().flat_map(|s| s.panels.get().into_iter().filter(|p| p.path.get().is_none()).map(|p| (s.id.clone(), p))).collect())
}

fn folders(items: &[TreeItem], out: &mut Vec<(String, String)>, depth: usize) {
    for i in items.iter().filter(|i| i.folder) {
        out.push((i.path.clone(), format!("{}{}", "  ".repeat(depth), i.name)));
        folders(&i.items, out, depth + 1);
    }
}

/// Sección «Temporales · N» arriba de la barra lateral.
#[component]
pub fn TempList() -> impl IntoView {
    let a = app();
    move || {
        let list = temporales();
        (!list.is_empty()).then(|| {
            view! {
                <div class="tmp-list">
                    <div class="tree-row tmp-head" title="Peticiones para probar: no están en la colección ni en git hasta que las guardes">
                        <span class="tmp-tag">"TMP"</span><span class="name">{format!("Temporales · {}", list.len())}</span>
                    </div>
                    {list.into_iter().map(|(screen, p)| {
                        let (pid, pid2) = (p.id.clone(), p.id.clone());
                        view! {
                            <div class="tree-row tmp-row" class:on=move || a.active_panel.get() == pid style="padding-left:22px"
                                on:click=move |_| { a.screen.set(screen.clone()); a.active_panel.set(pid2.clone()) }>
                                <span class=move || format!("m m-{}", p.req.with(|r| r.method.clone()))>{move || p.req.with(|r| r.method.clone())}</span>
                                <span class="name">{move || p.req.with(|r| r.name.clone())}</span>
                            </div>
                        }
                    }).collect_view()}
                </div>
            }
        })
    }
}

/// Aviso dentro de un panel temporal, con el formulario para guardarlo.
#[component]
pub fn TempBanner(p: Panel) -> impl IntoView {
    let a = app();
    let name = RwSignal::new(String::new());
    let folder = RwSignal::new(String::new());
    let id = RwSignal::new(String::new());
    let brought = RwSignal::new(String::new());
    // Las señales del panel son `Copy`: los cierres las capturan sin clonar el panel.
    let Panel { id: pid, collection, path, req, dirty, saving, .. } = p;
    let suggest = move || {
        let (col, f, w) = (collection.get_untracked(), folder.get_untracked(), brought.get_untracked());
        spawn_local(async move {
            if let Ok(v) = api::get::<Value>(&format!("/api/request/next-id?collection={}&folder={}&wanted={}", api::enc(&col), api::enc(&f), api::enc(&w))).await {
                id.set(v["id"].as_str().unwrap_or("").into());
            }
        });
    };
    // Al abrir el formulario: nombre y ID que traía la petición, y la sugerencia para la carpeta.
    Effect::new(move |_| {
        if saving.get() {
            name.set(req.with_untracked(|r| r.name.clone()));
            brought.set(req.with_untracked(|r| r.id.clone()));
            suggest();
        }
    });
    let save = move || {
        let (col, f) = (collection.get_untracked(), folder.get_untracked());
        let mut r = req.get_untracked();
        r.name = name.get_untracked().trim().to_string();
        r.id = id.get_untracked().trim().to_string();
        spawn_local(async move {
            match api::post::<Value>(&format!("/api/request?collection={}&folder={}", api::enc(&col), api::enc(&f)), &r).await {
                Ok(v) => {
                    r.id = v["id"].as_str().unwrap_or("").into();
                    let p = v["path"].as_str().unwrap_or("").to_string();
                    a.toast("ok", format!("Guardada como {} en {p}", r.id));
                    req.set(r);
                    path.set(Some(p));
                    dirty.set(false);
                    saving.set(false);
                }
                Err(e) => a.err(e),
            }
        });
    };
    let close = move |_| {
        if !web_sys::window().unwrap().confirm_with_message("¿Descartar esta petición temporal? No está guardada en ningún lado.").unwrap_or(false) {
            return;
        }
        a.screens.with_untracked(|ss| ss.iter().for_each(|s| s.panels.update(|ps| ps.retain(|x| x.id != pid))));
    };
    let folder_opts = move || {
        let mut out = vec![(String::new(), "(raíz de la colección)".to_string())];
        if let Some(c) = a.collections.with(|cs| cs.iter().find(|c| c.id == collection.get()).cloned()) {
            folders(&c.items, &mut out, 0);
        }
        out
    };
    let close = StoredValue::new(close);
    move || {
        path.get().is_none().then(|| view! {
            <div class="tmp-bar">
                <span class="tmp-tag">"TEMPORAL"</span>
                <span>"Sólo para probar: "<b>"no se escribe en la colección"</b>" ni aparece en git."</span>
                <span class="grow"></span>
                <button class="btn sm danger" on:click=move |e| close.with_value(|c| c(e))>"Descartar"</button>
                <button class="btn sm primary" on:click=move |_| saving.update(|s| *s = !*s)>"Guardar en la colección"</button>
            </div>
            {move || saving.get().then(|| view! {
                <form class="tmp-save" on:submit=move |e| { e.prevent_default(); save() }>
                    <label class="f">"Nombre"<input class="in" data-noac prop:value=move || name.get() on:input=move |e| name.set(event_target_value(&e)) /></label>
                    <label class="f">"Carpeta"
                        <select class="in" prop:value=move || folder.get() on:change=move |e| { folder.set(event_target_value(&e)); suggest() }>
                            {move || folder_opts().into_iter().map(|(v, l)| view! { <option value=v>{l}</option> }).collect_view()}
                        </select>
                    </label>
                    <label class="f">"ID"
                        <input class="in mono" data-noac prop:value=move || id.get() on:input=move |e| id.set(event_target_value(&e)) />
                    </label>
                    <p class="muted">{move || {
                        let b = brought.get();
                        if b.is_empty() { "El siguiente libre de la carpeta. Puedes escribir otro.".to_string() }
                        else if b == id.get() { format!("Es el ID que trajo ({b}).") }
                        else { format!("Trajo {b}, pero ya está usado: te propongo {}.", id.get()) }
                    }}</p>
                    <div class="row"><button class="btn primary" type="submit">"Guardar"</button><button class="btn ghost" type="button" on:click=move |_| saving.set(false)>"Cancelar"</button></div>
                </form>
            })}
        })
    }
}
