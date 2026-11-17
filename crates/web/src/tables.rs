//! Tablas de equivalencia de la colección del flujo (`tablas/*.csv`): crear, editar como
//! cuadrícula (se guarda sola al salir de cada celda), importar CSV, probar y usar en el flujo.
use crate::api;
use crate::flows::{ed, Ed, Pane};
use crate::state::app;
use leptos::prelude::*;
use leptos::task::spawn_local;
use martino_core::lookup::{parse_table, LookupSpec, Table};
use serde_json::{json, Value};

fn col_of(ed: Ed) -> String {
    ed.flow.with_untracked(|f| f.collection.clone())
}

/// Carga las tablas de la colección del flujo (una vez por colección).
pub fn load(ed: Ed) {
    let col = col_of(ed);
    if col.is_empty() || ed.tables_of.get_untracked() == col {
        return;
    }
    ed.tables_of.set(col.clone());
    spawn_local(async move {
        match api::get::<Vec<Value>>(&format!("/api/tables?collection={}", api::enc(&col))).await {
            Ok(l) => ed.table_list.set(l.iter().map(|t| (t["name"].as_str().unwrap_or("").to_string(), parse_table(t["csv"].as_str().unwrap_or("")))).collect()),
            Err(e) => app().err(e),
        }
    });
}

fn save(ed: Ed, name: String, t: Table) {
    // En memoria va la tabla tal cual (con sus filas vacías recién agregadas); al disco, el CSV.
    let csv = t.to_csv();
    ed.table_list.update(|l| match l.iter_mut().find(|x| x.0 == name) {
        Some(x) => x.1 = t,
        None => l.push((name.clone(), t)),
    });
    let col = col_of(ed);
    spawn_local(async move {
        if let Err(e) = api::put::<Value>(&format!("/api/tables?collection={}&name={}", api::enc(&col), api::enc(&name)), &json!({"csv": csv})).await {
            app().err(format!("No se guardó la tabla: {e}"));
        }
    });
}

/// Nombre de archivo válido a partir de lo que escribe la persona.
fn table_name(s: &str) -> String {
    s.trim().to_lowercase().chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' }).collect::<String>().trim_matches('_').to_string()
}

#[component]
pub fn TablesView() -> impl IntoView {
    let (a, ed) = (app(), ed());
    load(ed);
    let cur = RwSignal::new(None::<String>);
    Effect::new(move |_| {
        if cur.get_untracked().is_none() {
            cur.set(ed.table_list.with(|l| l.first().map(|t| t.0.clone())));
        }
    });
    let new_name = RwSignal::new(String::new());
    let create = move |csv: String, name: String| {
        let name = table_name(&name);
        if name.is_empty() {
            a.err("Ponle un nombre a la tabla");
            return;
        }
        if ed.table_list.with_untracked(|l| l.iter().any(|t| t.0 == name)) {
            a.err(format!("Ya existe la tabla «{name}»"));
            return;
        }
        let mut t = parse_table(&csv);
        if t.rows.is_empty() {
            t.rows.push(vec![String::new(); t.cols.len().max(1)]);
        }
        save(ed, name.clone(), t);
        cur.set(Some(name));
        new_name.set(String::new());
    };
    let on_file = move |ev: web_sys::Event| {
        let input: web_sys::HtmlInputElement = event_target(&ev);
        let Some(f) = input.files().and_then(|l| l.get(0)) else {
            return;
        };
        input.set_value("");
        let name = f.name().rsplit_once('.').map(|x| x.0.to_string()).unwrap_or(f.name());
        spawn_local(async move {
            if let Ok(t) = wasm_bindgen_futures::JsFuture::from(f.text()).await {
                create(t.as_string().unwrap_or_default(), name);
            }
        });
    };
    let no_col = move || ed.flow.with(|f| f.collection.is_empty());
    view! {
        <div class="tables">
            <div class="qa-head">
                <h2>"Tablas de equivalencia"</h2>
                <span class="muted">"Traducen un valor a otro (código de banco → nombre). Se guardan en "<span class="mono">"tablas/*.csv"</span>" de la colección: entran a git y las comparte el equipo."</span>
                <span class="grow"></span>
                <button class="btn ghost" on:click=move |_| ed.pane.set(Pane::Canvas)>"← Volver al lienzo"</button>
            </div>
            {move || if no_col() {
                view! { <p class="empty">"Elige la colección del flujo (panel derecho del lienzo) para tener tablas."</p> }.into_any()
            } else {
                view! {
                    <div class="tb-body">
                        <nav class="tb-list">
                            {move || ed.table_list.get().into_iter().map(|(name, t)| {
                                let rows = t.rows.len();
                                let (n1, n2) = (name.clone(), name.clone());
                                view! { <button class="tb-item" class:on=move || cur.get().as_deref() == Some(n1.as_str()) on:click=move |_| cur.set(Some(n2.clone()))><b>{name}</b><span class="muted">{format!("{rows} filas")}</span></button> }
                            }).collect_view()}
                            <div class="row">
                                <input class="in grow" data-noac placeholder="nombre de la tabla nueva" prop:value=move || new_name.get() on:input=move |e| new_name.set(event_target_value(&e))
                                    on:keydown=move |e: web_sys::KeyboardEvent| if e.key() == "Enter" { create("valor,equivale_a\n".into(), new_name.get_untracked()) } />
                                <button class="btn" on:click=move |_| create("valor,equivale_a\n".into(), new_name.get_untracked())>"+ Crear"</button>
                            </div>
                            <label class="btn ghost" title="La primera fila son los nombres de las columnas">
                                "Importar CSV"<input type="file" accept=".csv,.tsv,.txt" style="display:none" on:change=on_file />
                            </label>
                        </nav>
                        {move || match cur.get() {
                            Some(name) => view! { <TableEditor name=name /> }.into_any(),
                            None => view! { <p class="empty grow">"Crea una tabla o importa un CSV."</p> }.into_any(),
                        }}
                    </div>
                }.into_any()
            }}
        </div>
    }
}

#[component]
fn TableEditor(name: String) -> impl IntoView {
    let (a, ed) = (app(), ed());
    let n = name.clone();
    let t = Memo::new(move |_| crate::flow_steps::table(ed, &n).unwrap_or_default());
    // La cuadrícula se rehace sólo si cambia la forma (filas × columnas): así escribir no pierde el foco.
    let shape = Memo::new(move |_| t.with(|t| (t.cols.len(), t.rows.len())));
    let n2 = name.clone();
    let change = move |f: Box<dyn FnOnce(&mut Table)>| {
        let mut x = t.get_untracked();
        f(&mut x);
        save(ed, n2.clone(), x);
    };
    let probe = RwSignal::new(String::new());
    let by = RwSignal::new(String::new());
    let (n3, n4) = (name.clone(), name.clone());
    let use_in_flow = move |_| {
        let mut node = ed.place("lookup");
        node.name = format!("Equivalencia {n3}");
        node.lookup = Some(LookupSpec { table: n3.clone(), ..Default::default() });
        let id = node.id.clone();
        ed.edit(|f| f.nodes.push(node));
        ed.pane.set(Pane::Canvas);
        ed.sel.set(Some(id));
        a.toast("ok", "Nodo Equivalencia agregado: conéctalo y dile qué valor buscar");
    };
    let del = move |_| {
        if !web_sys::window().unwrap().confirm_with_message(&format!("¿Borrar la tabla «{n4}»? Se elimina tablas/{n4}.csv.")).unwrap_or(false) {
            return;
        }
        let (col, nm) = (col_of(ed), n4.clone());
        ed.table_list.update(|l| l.retain(|x| x.0 != nm));
        spawn_local(async move {
            if let Err(e) = api::del(&format!("/api/tables?collection={}&name={}", api::enc(&col), api::enc(&nm))).await {
                app().err(e);
            }
        });
    };
    view! {
        <section class="tb-edit">
            <div class="row">
                <h3>{name.clone()}</h3>
                <span class="mono muted">{format!("tablas/{name}.csv")}</span>
                <span class="grow"></span>
                <button class="btn primary" on:click=use_in_flow>"Usar en el flujo"</button>
                <button class="btn ghost danger" title="Borrar la tabla" on:click=del>"🗑"</button>
            </div>
            <div class="tb-grid-wrap">
                {move || {
                    let (cols, rows) = shape.get();
                    let ch = change.clone();
                    view! {
                        <table class="tb-grid">
                            <thead><tr>
                                {(0..cols).map(|c| { let ch = ch.clone(); view! {
                                    <th><input class="in" data-noac prop:value=move || t.with(|t| t.cols.get(c).cloned().unwrap_or_default())
                                        on:change=move |e| { let v = event_target_value(&e).trim().to_string(); ch(Box::new(move |t| t.cols[c] = v)) } /></th>
                                }}).collect_view()}
                                <th><button class="btn ghost sm" title="Agregar columna" on:click={let ch = ch.clone(); move |_| ch(Box::new(|t| { let n = t.cols.len() + 1; t.cols.push(format!("columna_{n}")); t.rows.iter_mut().for_each(|r| r.push(String::new())) }))}>"+ Columna"</button></th>
                            </tr></thead>
                            <tbody>
                                {(0..rows).map(|r| { let ch = ch.clone(); view! {
                                    <tr>
                                        {(0..cols).map(|c| { let ch = ch.clone(); view! {
                                            <td><input class="in mono" data-noac prop:value=move || t.with(|t| t.rows.get(r).and_then(|x| x.get(c)).cloned().unwrap_or_default())
                                                on:change={let ch = ch.clone(); move |e| { let v = event_target_value(&e); ch(Box::new(move |t| { if let Some(row) = t.rows.get_mut(r) { row.resize(row.len().max(c + 1), String::new()); row[c] = v } })) }}
                                                on:keydown=move |e: web_sys::KeyboardEvent| if e.key() == "Enter" && r + 1 == rows {
                                                    // Enter en la última fila: guarda la celda y abre otra fila.
                                                    let v = event_target_value(&e);
                                                    ch(Box::new(move |t| { if let Some(row) = t.rows.get_mut(r) { row.resize(row.len().max(c + 1), String::new()); row[c] = v } let n = t.cols.len().max(1); t.rows.push(vec![String::new(); n]) }));
                                                    // La cuadrícula se rehízo: el foco va a la primera celda de la fila nueva.
                                                    request_animation_frame(|| { if let Ok(Some(el)) = crate::ui::document().query_selector(".tb-grid tbody tr:last-child input") { let _ = wasm_bindgen::JsCast::unchecked_into::<web_sys::HtmlElement>(el).focus(); } });
                                                } /></td>
                                        }}).collect_view()}
                                        <td><button class="btn ghost sm" title="Quitar fila" on:click={let ch = ch.clone(); move |_| ch(Box::new(move |t| { if r < t.rows.len() { t.rows.remove(r); } }))}>"✕"</button></td>
                                    </tr>
                                }}).collect_view()}
                            </tbody>
                        </table>
                        <button class="btn sm" on:click=move |_| ch(Box::new(|t| { let n = t.cols.len().max(1); t.rows.push(vec![String::new(); n]) }))>"+ Fila"</button>
                    }
                }}
            </div>
            <p class="muted">"Cada celda se guarda al salir de ella. La búsqueda no distingue mayúsculas ni espacios."</p>
            <h4>"Probar"</h4>
            <div class="row">
                <input class="in grow" data-noac placeholder="escribe un valor" prop:value=move || probe.get() on:input=move |e| probe.set(event_target_value(&e)) />
                <select class="in" title="Columna donde buscar" prop:value=move || by.get() on:change=move |e| by.set(event_target_value(&e))>
                    {move || t.get().cols.into_iter().enumerate().map(|(i, c)| { let v = if i == 0 { String::new() } else { c.clone() }; view! { <option value=v>{format!("buscar en {c}")}</option> } }).collect_view()}
                </select>
            </div>
            {move || {
                let q = probe.get();
                (!q.trim().is_empty()).then(|| match t.get().lookup(&LookupSpec { column: by.get(), ..Default::default() }, &q, &name) {
                    Ok(v) => view! { <pre class="code ok-text">{serde_json::to_string_pretty(&v).unwrap_or_default()}</pre> }.into_any(),
                    Err(e) => view! { <pre class="code err-text">{e}</pre> }.into_any(),
                })
            }}
        </section>
    }
}
