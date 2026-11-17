//! Modales: variables y entornos, cURL, importar, ajustes, ayuda y confirmación.
use crate::api;
use crate::state::{app, ConfirmAction, Modal, Panel};
use crate::ui::{copy, ModalFrame, VarLegend};
use leptos::prelude::*;
use leptos::task::spawn_local;
use martino_core::curl::{parse_curl, templatize_request};
use martino_core::model::{Environment, Pair};
use martino_core::vars::VarSource;
use serde_json::{json, Value};

#[component]
pub fn Modals() -> impl IntoView {
    let a = app();
    move || {
        a.modal.get().map(|m| match m {
            Modal::Env(focus) => view! { <EnvManager focus=focus /> }.into_any(),
            Modal::Curl { text, folder } => view! { <CurlImport text=text folder=folder /> }.into_any(),
            Modal::CurlOut(s) => {
                let s2 = s.clone();
                view! { <ModalFrame title="cURL" footer=view! { <button class="btn primary" on:click=move |_| copy(&s2)>"Copiar"</button> }.into_any()>
                    <pre class="code">{s}</pre></ModalFrame> }.into_any()
            }
            Modal::Import => view! { <ImportModal /> }.into_any(),
            Modal::Settings => view! { <SettingsModal /> }.into_any(),
            Modal::Folder => view! { <FolderModal /> }.into_any(),
            Modal::Help => view! { <Help /> }.into_any(),
            Modal::Confirm { text, action } => view! { <ConfirmModal text=text action=action /> }.into_any(),
        })
    }
}

// ---------- variables y entornos ----------

#[derive(Clone, PartialEq)]
struct Row {
    name: String,
    secret: bool,
}

fn rows_of(envs: &[Environment]) -> Vec<Row> {
    let mut out: Vec<Row> = vec![];
    for e in envs {
        for (p, secret) in e.vars.iter().map(|p| (p, false)).chain(e.secrets.iter().map(|p| (p, true))) {
            match out.iter_mut().find(|r| r.name == p.name) {
                Some(r) => r.secret |= secret,
                None => out.push(Row { name: p.name.clone(), secret }),
            }
        }
    }
    out
}

fn cell(e: &Environment, name: &str) -> Option<String> {
    e.vars.iter().chain(e.secrets.iter()).find(|p| p.name == name).map(|p| p.value.clone())
}

fn set_cell(e: &mut Environment, name: &str, value: String, secret: bool) {
    let (keep, other) = if secret { (&mut e.secrets, &mut e.vars) } else { (&mut e.vars, &mut e.secrets) };
    other.retain(|p| p.name != name);
    match keep.iter_mut().find(|p| p.name == name) {
        Some(p) => p.value = value,
        None => keep.push(Pair::new(name, value)),
    }
}

/// Matriz variable × entorno: se ve de un vistazo qué falta en cada entorno.
#[component]
fn EnvManager(focus: Option<String>) -> impl IntoView {
    let a = app();
    let col = a.collection.get_untracked();
    let envs: RwSignal<Vec<Environment>> = RwSignal::new(vec![]);
    let rows: RwSignal<Vec<Row>> = RwSignal::new(vec![]);
    let dirty = RwSignal::new(false);
    let filter = RwSignal::new(String::new());
    let show = RwSignal::new(false);
    let runtime: RwSignal<Vec<(String, String)>> = RwSignal::new(vec![]);
    let names = a.coll().map(|c| c.environments).unwrap_or_default();
    let col2 = col.clone();
    let focus2 = focus.clone();
    spawn_local(async move {
        let mut list = vec![];
        for n in &names {
            match api::get::<Environment>(&format!("/api/environment?collection={}&env={}", api::enc(&col2), api::enc(n))).await {
                Ok(e) => list.push(e),
                Err(e) => a.err(e),
            }
        }
        let mut r = rows_of(&list);
        if let Some(f) = &focus2 {
            if !r.iter().any(|x| &x.name == f) {
                r.push(Row { name: f.clone(), secret: false });
            }
        }
        rows.set(r);
        envs.set(list);
        if let Ok(v) = api::get::<serde_json::Map<String, Value>>("/api/runtime").await {
            runtime.set(v.into_iter().map(|(k, v)| (k, v.as_str().unwrap_or_default().to_string())).collect());
        }
    });
    let save = {
        let col = col.clone();
        move || {
            let col = col.clone();
            let list = envs.get_untracked();
            spawn_local(async move {
                for e in &list {
                    if let Err(err) = api::put::<Value>(&format!("/api/environment?collection={}&env={}", api::enc(&col), api::enc(&e.name)), e).await {
                        a.err(err);
                        return;
                    }
                }
                dirty.set(false);
                a.toast("ok", "Entornos guardados");
                a.reload_collections();
            });
        }
    };
    let new_env = {
        let col = col.clone();
        move || {
            let Some(name) = web_sys::window().unwrap().prompt_with_message("Nombre del entorno (dev, qa, prod…)").ok().flatten().filter(|s| !s.trim().is_empty()) else { return };
            let name = name.trim().to_string();
            // copia los nombres de las variables, vacías, para que se vea qué falta llenar
            let base = envs.get_untracked().first().cloned().unwrap_or_default();
            let e = Environment {
                name: name.clone(),
                vars: base.vars.iter().map(|p| Pair::new(&p.name, "")).collect(),
                secrets: base.secrets.iter().map(|p| Pair::new(&p.name, "")).collect(),
                extra: vec![],
            };
            let col = col.clone();
            spawn_local(async move {
                match api::put::<Value>(&format!("/api/environment?collection={}&env={}", api::enc(&col), api::enc(&name)), &e).await {
                    Ok(_) => {
                        envs.update(|l| l.push(e));
                        a.env.set(name);
                        a.reload_collections();
                    }
                    Err(err) => a.err(err),
                }
            });
        }
    };
    let footer = view! {
        <span class="muted grow">{move || if dirty.get() { "Cambios sin guardar" } else { "Los secretos se guardan en .workbench/secrets.json, nunca en el .bru" }}</span>
        <button class="btn" on:click=move |_| new_env()>"＋ Entorno"</button>
        <button class="btn primary" disabled=move || !dirty.get() on:click=move |_| save()>"Guardar"</button>
    }
    .into_any();
    let active = a.env;
    view! {
        <ModalFrame title=format!("Variables · {}", a.coll().map(|c| c.name).unwrap_or_default()) wide=true footer=footer>
            <div class="col">
                <div class="row">
                    <input class="in grow" data-noac placeholder="Filtrar variables…" prop:value=move || filter.get() on:input=move |e| filter.set(event_target_value(&e)) />
                    <label class="row"><input type="checkbox" prop:checked=move || show.get() on:change=move |e| show.set(event_target_checked(&e)) />"Mostrar secretos"</label>
                    <button class="btn" on:click=move |_| { rows.update(|r| r.push(Row { name: String::new(), secret: false })); }>"＋ Variable"</button>
                </div>
                <VarLegend />
                <div style="overflow:auto;max-height:52vh">
                <table class="matrix">
                    <thead><tr>
                        <th>"Variable"</th><th title="Secreta: el valor no va al .bru">"🔒"</th>
                        {move || envs.get().into_iter().map(|e| { let n = e.name.clone(); let n2 = n.clone(); view! {
                            <th class:act=move || active.get() == n title="Clic para activarlo" style="cursor:pointer" on:click=move |_| active.set(n2.clone())>
                                {e.name.clone()}{move || (active.get() == e.name).then_some(" ●")}
                            </th>
                        }}).collect_view()}
                        <th></th>
                    </tr></thead>
                    <tbody>
                        {move || {
                            let f = filter.get().to_lowercase();
                            let list = envs.get();
                            rows.get().into_iter().enumerate().filter(|(_, r)| f.is_empty() || r.name.to_lowercase().contains(&f)).map(|(ri, r)| {
                                let focused = focus.as_deref() == Some(r.name.as_str());
                                let name = r.name.clone();
                                let secret = r.secret;
                                let cells = list.iter().enumerate().map(|(ei, e)| {
                                    let v = cell(e, &name);
                                    let missing = v.as_deref().is_none_or(str::is_empty);
                                    let name = name.clone();
                                    view! {
                                        <td class:miss=missing>
                                            <input class="in mono" data-noac type={move || if secret && !show.get() { "password" } else { "text" }}
                                                placeholder=if v.is_none() { "falta" } else { "vacía" } prop:value=v.unwrap_or_default()
                                                on:input=move |ev| { let val = event_target_value(&ev); envs.update(|l| set_cell(&mut l[ei], &name, val, secret)); dirty.set(true) } />
                                        </td>
                                    }
                                }).collect_view();
                                let old = name.clone();
                                let n3 = name.clone();
                                view! {
                                    <tr class:focus=focused>
                                        <td><input class=format!("in mono {}", if secret { VarSource::Secret.css() } else { VarSource::Env.css() }) data-noac prop:value=name.clone()
                                            on:change=move |ev| {
                                                let new = event_target_value(&ev);
                                                envs.update(|l| for e in l.iter_mut() { for p in e.vars.iter_mut().chain(e.secrets.iter_mut()) { if p.name == old { p.name = new.clone() } } });
                                                rows.update(|r| r[ri].name = new.clone());
                                                dirty.set(true);
                                            } /></td>
                                        <td><input type="checkbox" title="Secreta" prop:checked=secret on:change=move |ev| {
                                            let s = event_target_checked(&ev);
                                            let n = n3.clone();
                                            envs.update(|l| for e in l.iter_mut() { if let Some(v) = cell(e, &n) { set_cell(e, &n, v, s) } });
                                            rows.update(|r| r[ri].secret = s);
                                            dirty.set(true);
                                        } /></td>
                                        {cells}
                                        <td><button class="btn ghost sm" title="Borrar en todos los entornos" on:click={let n = name.clone(); move |_| {
                                            envs.update(|l| for e in l.iter_mut() { e.vars.retain(|p| p.name != n); e.secrets.retain(|p| p.name != n) });
                                            rows.update(|r| { r.remove(ri); });
                                            dirty.set(true);
                                        }}>"×"</button></td>
                                    </tr>
                                }
                            }).collect_view()
                        }}
                    </tbody>
                </table>
                </div>
                {move || envs.with(|e| e.is_empty()).then(|| view! { <p class="muted">"Esta colección no tiene entornos. Crea uno con «＋ Entorno»."</p> })}
                <h4 style="margin:10px 0 0">"Runtime"<span class="muted" style="font-weight:400">" · las fijan vars:post-response; ganan a todo"</span></h4>
                {move || {
                    let rt = runtime.get();
                    if rt.is_empty() { return view! { <p class="muted">"Sin variables runtime."</p> }.into_any() }
                    view! {
                        <table class="matrix">{rt.into_iter().map(|(k, v)| { let k2 = k.clone(); view! {
                            <tr><td class="mono v-runtime">{k.clone()}</td><td class="mono">{v}</td><td><button class="btn ghost sm" on:click=move |_| {
                                let k = k2.clone();
                                spawn_local(async move { if api::put::<Value>("/api/runtime", &json!({ k.clone(): null })).await.is_ok() { runtime.update(|r| r.retain(|x| x.0 != k)) } });
                            }>"×"</button></td></tr>
                        }}).collect_view()}</table>
                        <button class="btn sm" on:click=move |_| spawn_local(async move { if api::del("/api/runtime").await.is_ok() { runtime.set(vec![]) } })>"Limpiar runtime"</button>
                    }.into_any()
                }}
            </div>
        </ModalFrame>
    }
}

// ---------- cURL ----------

#[component]
fn CurlImport(text: String, folder: Option<String>) -> impl IntoView {
    let a = app();
    let text = RwSignal::new(text);
    let folder = StoredValue::new(folder);
    let templ = RwSignal::new(true);
    let run_now = RwSignal::new(false);
    let parsed = Memo::new(move |_| {
        let mut r = parse_curl(&text.get())?;
        let mut done = vec![];
        if templ.get() {
            let env: Vec<Pair> = a.vars.with(|t| t.iter().filter(|v| matches!(v.source, VarSource::Env | VarSource::Secret)).map(|v| Pair::new(&v.name, &v.value)).collect());
            done = templatize_request(&mut r, &env);
        }
        Ok::<_, String>((r, done))
    });
    let go = move |target: &'static str| {
        let Ok((r, _)) = parsed.get_untracked() else { return };
        let col = a.collection.get_untracked();
        match target {
            "replace" => {
                if let Some(p) = a.active() {
                    p.req.set(r);
                    p.dirty.set(true);
                    if run_now.get_untracked() {
                        a.run_panels(vec![p]);
                    }
                } else {
                    a.add_panel(Panel::new(&col, None, r));
                }
            }
            "save" => {
                let folder = folder.get_value().unwrap_or_default();
                let run = run_now.get_untracked();
                spawn_local(async move {
                    match api::post::<Value>(&format!("/api/request?collection={}&folder={}", api::enc(&col), api::enc(&folder)), &r).await {
                        Ok(v) => {
                            let p = Panel::new(&col, v["path"].as_str().map(String::from), r);
                            a.add_panel(p.clone());
                            if run {
                                a.run_panels(vec![p]);
                            }
                        }
                        Err(e) => a.err(e),
                    }
                });
            }
            _ => {
                let p = Panel::new(&col, None, r);
                a.add_panel(p.clone());
                if run_now.get_untracked() {
                    a.run_panels(vec![p]);
                }
            }
        }
        a.modal.set(None);
    };
    let ok = move || parsed.with(|p| p.is_ok());
    let footer = view! {
        <label class="row grow"><input type="checkbox" prop:checked=move || run_now.get() on:change=move |e| run_now.set(event_target_checked(&e)) />"Ejecutar ahora"</label>
        <button class="btn" disabled=move || !ok() on:click=move |_| go("replace")>"Reemplazar panel"</button>
        <button class="btn" disabled=move || !ok() on:click=move |_| go("save")>"Guardar en la colección"</button>
        <button class="btn primary" disabled=move || !ok() title="Se abre como temporal: no se escribe en la colección hasta que lo guardes" on:click=move |_| go("new")>"Probar (temporal)"</button>
    }
    .into_any();
    view! {
        <ModalFrame title="Pegar cURL" footer=footer>
            <div class="col">
                <textarea class="in" rows=8 data-noac placeholder="curl 'https://…' -H '…' --data-raw '…'" prop:value=move || text.get() on:input=move |e| text.set(event_target_value(&e))></textarea>
                <label class="row"><input type="checkbox" prop:checked=move || templ.get() on:change=move |e| templ.set(event_target_checked(&e)) />
                    {move || format!("Usar variables del entorno «{}» (URL base, cabeceras y token)", a.env.get())}</label>
                {move || match parsed.get() {
                    Err(e) => view! { <span class="chip err">{e}</span> }.into_any(),
                    Ok((r, done)) => view! {
                        <div class="col">
                            <div class="row mono"><span class=format!("m m-{}", r.method)>{r.method.clone()}</span>{r.url.clone()}</div>
                            <div class="row" style="flex-wrap:wrap">{done.into_iter().map(|d| view! { <span class="chip ok">{d}</span> }).collect_view()}</div>
                        </div>
                    }.into_any(),
                }}
            </div>
        </ModalFrame>
    }
}

// ---------- importar ----------

fn pairs_from_lines(s: &str) -> Vec<Pair> {
    s.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| Pair::new(k.trim(), v.trim())).filter(|p| !p.name.is_empty()).collect()
}

#[component]
fn ImportModal() -> impl IntoView {
    let a = app();
    let text = RwSignal::new(String::new());
    let name = RwSignal::new(String::new());
    let target = RwSignal::new(String::new());
    let rename = RwSignal::new(String::new());
    let urls = RwSignal::new(String::new());
    let headers = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let on_file = move |ev: web_sys::Event| {
        let input: web_sys::HtmlInputElement = event_target(&ev);
        let Some(f) = input.files().and_then(|l| l.get(0)) else { return };
        if name.get_untracked().is_empty() {
            name.set(f.name().trim_end_matches(".json").trim_end_matches(".bru").replace(".postman_collection", ""));
        }
        spawn_local(async move {
            if let Ok(t) = wasm_bindgen_futures::JsFuture::from(f.text()).await {
                text.set(t.as_string().unwrap_or_default());
            }
        });
    };
    let go = move |_| {
        let t = text.get_untracked();
        let data: Value = serde_json::from_str(&t).unwrap_or(Value::String(t));
        let body = json!({
            "data": data,
            "name": Some(name.get_untracked()).filter(|s| !s.is_empty()),
            "collection": Some(target.get_untracked()).filter(|s| !s.is_empty()),
            "remap": {"rename": pairs_from_lines(&rename.get_untracked()), "urls": pairs_from_lines(&urls.get_untracked()), "headers": pairs_from_lines(&headers.get_untracked())},
        });
        busy.set(true);
        spawn_local(async move {
            match api::post::<Value>("/api/import", &body).await {
                Ok(v) => {
                    a.toast("ok", format!("Importado: {} peticiones, {} entornos", v["requests"].as_u64().unwrap_or(1), v["environments"].as_u64().unwrap_or(0)));
                    a.collection.set(v["collection"].as_str().unwrap_or_default().into());
                    a.reload_collections();
                    a.modal.set(None);
                }
                Err(e) => a.err(e),
            }
            busy.set(false);
        });
    };
    let footer = view! { <button class="btn primary" disabled=move || busy.get() || text.with(String::is_empty) on:click=go>"Importar"</button> }.into_any();
    view! {
        <ModalFrame title="Importar" footer=footer>
            <div class="col">
                <p class="muted">"Colección o entorno Postman v2.1 (.json) o una petición Bruno (.bru)."</p>
                <input type="file" accept=".json,.bru" on:change=on_file />
                <textarea class="in" rows=6 data-noac placeholder="…o pega aquí el contenido" prop:value=move || text.get() on:input=move |e| text.set(event_target_value(&e))></textarea>
                <div class="row"><span class="muted" style="width:150px">"Nombre"</span><input class="in grow" data-noac prop:value=move || name.get() on:input=move |e| name.set(event_target_value(&e)) /></div>
                <div class="row"><span class="muted" style="width:150px">"Dentro de"</span>
                    <select class="in grow" on:change=move |e| target.set(event_target_value(&e))>
                        <option value="">"Colección nueva"</option>
                        {a.collections.get_untracked().into_iter().map(|c| view! { <option value=c.id.clone()>{c.name}</option> }).collect_view()}
                    </select></div>
                <details>
                    <summary>"Normalizar variables (remapVariables)"</summary>
                    <p class="muted">"Una regla por línea. Convención: mismos nombres en todos los entornos (globalUrl, clientId, token sin «Bearer »)."</p>
                    <div class="row"><span class="muted" style="width:150px">"Renombrar"</span><textarea class="in grow" rows=2 data-noac placeholder="token_test=token" prop:value=move || rename.get() on:input=move |e| rename.set(event_target_value(&e))></textarea></div>
                    <div class="row"><span class="muted" style="width:150px">"URL → variable"</span><textarea class="in grow" rows=2 data-noac placeholder="https://dev.api.com=globalUrl" prop:value=move || urls.get() on:input=move |e| urls.set(event_target_value(&e))></textarea></div>
                    <div class="row"><span class="muted" style="width:150px">"Cabecera → variable"</span><textarea class="in grow" rows=2 data-noac placeholder="x-client-id=clientId" prop:value=move || headers.get() on:input=move |e| headers.set(event_target_value(&e))></textarea></div>
                </details>
            </div>
        </ModalFrame>
    }
}

// ---------- ajustes ----------

#[component]
fn SettingsModal() -> impl IntoView {
    let a = app();
    let s: RwSignal<Value> = RwSignal::new(json!({}));
    spawn_local(async move {
        match api::get::<Value>("/api/settings").await {
            Ok(v) => s.set(v),
            Err(e) => a.err(e),
        }
    });
    let num = move |k: &'static str| move || s.with(|v| v[k].as_u64().unwrap_or(0).to_string());
    let save = move |_| {
        let v = s.get_untracked();
        spawn_local(async move {
            match api::put::<Value>("/api/settings", &v).await {
                Ok(_) => {
                    a.toast("ok", "Ajustes guardados en workbench.local.json");
                    a.modal.set(None);
                }
                Err(e) => a.err(e),
            }
        });
    };
    let footer = view! { <button class="btn primary" on:click=save>"Guardar"</button> }.into_any();
    view! {
        <ModalFrame title="Ajustes" footer=footer>
            <div class="col" style="max-width:560px">
                <div class="row"><span style="width:220px">"Peticiones en paralelo"</span><input class="in" type="number" min="1" max="50" prop:value=num("concurrency")
                    on:input=move |e| s.update(|v| v["concurrency"] = json!(event_target_value(&e).parse::<u64>().unwrap_or(6))) /></div>
                <div class="row"><span style="width:220px">"Timeout (ms)"</span><input class="in" type="number" min="100" prop:value=num("requestTimeoutMs")
                    on:input=move |e| s.update(|v| v["requestTimeoutMs"] = json!(event_target_value(&e).parse::<u64>().unwrap_or(30000))) /></div>
                <div class="row"><span style="width:220px">"CA extra (PEM)"</span><input class="in grow" data-noac placeholder="/ruta/a/ca-corporativa.pem" prop:value=move || s.with(|v| v["tls"]["caFile"].as_str().unwrap_or("").to_string())
                    on:input=move |e| s.update(|v| v["tls"]["caFile"] = json!(event_target_value(&e))) /></div>
                <label class="row"><input type="checkbox" prop:checked=move || s.with(|v| v["tls"]["rejectUnauthorized"] != json!(false))
                    on:change=move |e| s.update(|v| v["tls"]["rejectUnauthorized"] = json!(event_target_checked(&e))) />"Verificar certificados TLS (desactívalo solo para entornos de prueba)"</label>
            </div>
        </ModalFrame>
    }
}

// ---------- carpeta de trabajo ----------

/// Cambiar de carpeta reemplaza todo el estado del servidor: recargar la página es lo más simple
/// y no deja paneles apuntando a colecciones de la carpeta anterior.
fn open_folder(path: Option<String>) {
    let a = app();
    spawn_local(async move {
        match api::post::<Value>("/api/root", &json!({ "path": path })).await {
            Ok(v) if v["cancelled"] == json!(true) => {}
            Ok(_) => { let _ = window().location().reload(); }
            Err(e) => a.err(e),
        }
    });
}

#[component]
fn FolderModal() -> impl IntoView {
    let a = app();
    let d: RwSignal<Value> = RwSignal::new(json!({}));
    spawn_local(async move {
        match api::get::<Value>("/api/root").await {
            Ok(v) => d.set(v),
            Err(e) => a.err(e),
        }
    });
    let typed = RwSignal::new(String::new());
    let root = move || d.with(|v| v["root"].as_str().unwrap_or("").to_string());
    view! {
        <ModalFrame title="Carpeta de trabajo">
            <div class="col" style="max-width:620px">
                <p class="muted">"Todo lo tuyo vive en esta carpeta: "<span class="mono">"collections/"</span>" (.bru), "<span class="mono">"flows/"</span>" y "<span class="mono">".workbench/"</span>" (secretos y diseño). Puedes versionarla con git; los secretos no se suben."</p>
                <div class="row"><span class="mono grow" title="Carpeta actual">{root}</span></div>
                <div class="row">
                    <button class="btn primary" on:click=move |_| open_folder(None)>"Elegir carpeta…"</button>
                    <input class="in grow" data-noac placeholder="…o pega la ruta: /Users/ana/apis" prop:value=move || typed.get()
                        on:input=move |e| typed.set(event_target_value(&e))
                        on:keydown=move |e: web_sys::KeyboardEvent| if e.key() == "Enter" { open_folder(Some(typed.get_untracked())) } />
                    <button class="btn" disabled=move || typed.with(|t| t.trim().is_empty()) on:click=move |_| open_folder(Some(typed.get_untracked()))>"Abrir"</button>
                </div>
                <h3>"Recientes"</h3>
                {move || {
                    let cur = root();
                    let list: Vec<String> = d.with(|v| v["recent"].as_array().into_iter().flatten().filter_map(|p| p.as_str().map(String::from)).filter(|p| *p != cur).collect());
                    if list.is_empty() {
                        view! { <p class="muted">"Aún no has abierto otras carpetas."</p> }.into_any()
                    } else {
                        list.into_iter().map(|p| { let go = p.clone(); view! { <button class="btn ghost mono" style="justify-content:flex-start" on:click=move |_| open_folder(Some(go.clone()))>{p}</button> } }).collect_view().into_any()
                    }
                }}
            </div>
        </ModalFrame>
    }
}

// ---------- ayuda ----------

#[component]
fn Help() -> impl IntoView {
    view! {
        <ModalFrame title="Ayuda">
            <div class="help">
                <h3>"Atajos"</h3>
                <ul>
                    <li><span class="kbd">"Ctrl+Enter"</span>" envía el panel activo · "<span class="kbd">"Ctrl+Shift+Enter"</span>" toda la pantalla en paralelo"</li>
                    <li><span class="kbd">"Ctrl+S"</span>" guarda · "<span class="kbd">"Ctrl+B"</span>" oculta la barra · "<span class="kbd">"Ctrl+1 / Ctrl+2"</span>" Peticiones / Flujos"</li>
                    <li><span class="kbd">"Ctrl+E"</span>" variables del entorno · "<span class="kbd">"Ctrl+V"</span>" fuera de un campo pega un cURL"</li>
                    <li>"En flujos: "<span class="kbd">"F10"</span>" paso a paso · "<span class="kbd">"Ctrl+Z / Ctrl+Y"</span>" deshacer/rehacer · "<span class="kbd">"Supr"</span>" borrar"</li>
                </ul>
                <h3>"Variables"</h3>
                <p>"Escribe "<span class="mono">"{"</span>" o "<span class="mono">"{{"</span>" en cualquier campo para ver la lista. El color dice de dónde sale cada una; pasa el cursor para ver su valor y Ctrl+clic para editarla."</p>
                <VarLegend />
                <p>"Precedencia: runtime > vars:pre-request > entorno (y secretos) > collection.bru > .env"</p>
                <h3>"Pantallas"</h3>
                <p>"Cada pantalla guarda sus paneles. Arrastra un panel por su "<span class="mono">"⋮⋮"</span>" a otra pestaña de pantalla para moverlo; doble clic en la pestaña la renombra."</p>
            </div>
        </ModalFrame>
    }
}

#[component]
fn ConfirmModal(text: String, action: ConfirmAction) -> impl IntoView {
    let a = app();
    let go = move |_| {
        let action = action.clone();
        spawn_local(async move {
            let r = match &action {
                ConfirmAction::DeleteCollection(id) => api::del(&format!("/api/collections?collection={}", api::enc(id))).await,
                ConfirmAction::DeleteRequest(c, p) => api::del(&format!("/api/request?collection={}&path={}", api::enc(c), api::enc(p))).await,
                ConfirmAction::DeleteFlow(id) => api::del(&format!("/api/flow?id={}", api::enc(id))).await,
            };
            match r {
                Ok(_) => {
                    a.modal.set(None);
                    if matches!(action, ConfirmAction::DeleteFlow(_)) {
                        a.flow_open.set(None);
                    }
                    a.reload_collections();
                }
                Err(e) => a.err(e),
            }
        });
    };
    let footer = view! { <button class="btn" on:click=move |_| a.modal.set(None)>"Cancelar"</button><button class="btn primary" on:click=go>"Sí, hacerlo"</button> }.into_any();
    view! { <ModalFrame title="Confirmar" footer=footer><p>{text}</p></ModalFrame> }
}
