//! Panel derecho del editor de flujos: ajustes del flujo, edición del nodo seleccionado,
//! su último resultado y el código (YAML/JSON) del flujo entero.
use crate::flows::{bru_paths, ed, label, Ed};
use crate::state::app;
use crate::ui::{copy, JsonTree, KvTable, VarField};
use leptos::prelude::*;
use martino_core::flow::{self, Flow, Node, Rule, OPS};
use martino_core::flow_yaml::{flow_from_yaml, flow_to_yaml};
use martino_core::model::{Pair, Request, SentRequest};
use leptos::task::spawn_local;
use crate::api;
use serde_json::Value;

const COLORS: [&str; 7] = ["", "#f5c518", "#57c78a", "#6aa8f0", "#c792ea", "#ef6b6b", "#f39a6b"];

#[component]
pub fn Inspector() -> impl IntoView {
    let ed = ed();
    view! {
        <aside class="finsp">
            {move || {
                let _ = ed.rev.get();
                if ed.code.get() {
                    view! { <CodeView /> }.into_any()
                } else {
                    match ed.sel.get() {
                        Some(id) => view! { <NodeInspector id=id /> }.into_any(),
                        None => view! { <FlowInspector /> }.into_any(),
                    }
                }
            }}
        </aside>
    }
}

/// Campo de texto ligado a un campo del nodo.
fn node_text(id: String, get: fn(&Node) -> String, set: fn(&mut Node, String), ph: &'static str) -> impl IntoView {
    let ed = ed();
    let id2 = id.clone();
    view! {
        <input class="in" data-noac placeholder=ph prop:value=move || ed.flow.with(|f| f.node(&id2).map(get).unwrap_or_default())
            on:input=move |e| { let v = event_target_value(&e); ed.edit_node(&id, move |n| set(n, v)) } />
    }
}

/// JSON editable: se aplica sólo cuando es válido; si no, el borde se pone rojo.
#[component]
fn JsonField(value: Value, #[prop(into)] on_change: Callback<Value>, #[prop(optional)] rows: Option<u32>) -> impl IntoView {
    let text = RwSignal::new(serde_json::to_string_pretty(&value).unwrap_or_default());
    let bad = RwSignal::new(false);
    view! {
        <textarea class="in" class:bad=move || bad.get() rows=rows.unwrap_or(8) spellcheck="false" prop:value=move || text.get()
            on:input=move |e| {
                let t = event_target_value(&e);
                match serde_json::from_str::<Value>(if t.trim().is_empty() { "{}" } else { &t }) {
                    Ok(v) => { bad.set(false); on_change.run(v) }
                    Err(_) => bad.set(true),
                }
                text.set(t);
            }></textarea>
        {move || bad.get().then(|| view! { <span class="err-text">"JSON inválido: no se aplica hasta corregirlo"</span> })}
    }
}

fn pairs_of(id: String, get: fn(&Node) -> Vec<Pair>, set: fn(&mut Node, Vec<Pair>), key_ph: &'static str, val_ph: &'static str) -> impl IntoView {
    let ed = ed();
    let id2 = id.clone();
    view! {
        <KvTable pairs=Signal::derive(move || ed.flow.with(|f| f.node(&id2).map(get).unwrap_or_default()))
            on_change=Callback::new(move |v: Vec<Pair>| ed.edit_node(&id, move |n| set(n, v)))
            key_ph=key_ph val_ph=val_ph />
    }
}

#[component]
fn NodeInspector(id: String) -> impl IntoView {
    let ed = ed();
    let id_m = id.clone();
    let node = Memo::new(move |_| ed.flow.with(|f| f.node(&id_m).cloned()));
    let Some(n0) = node.get_untracked() else { return view! { <section><p class="muted">"El nodo ya no existe."</p></section> }.into_any() };
    let kind = n0.kind.clone();
    let tab = RwSignal::new(if ed.steps.with_untracked(|s| s.contains_key(&id)) { "res" } else { "edit" });
    let body = match kind.as_str() {
        "request" => view! { <RequestEditor id=id.clone() /> }.into_any(),
        "condition" => view! { <ConditionEditor id=id.clone() /> }.into_any(),
        "extract" => view! {
            <h4>"Guardar en variables"</h4>
            <crate::ui::Info><p>"Cada fila guarda un valor en una variable del flujo. Vale una ruta sin llaves: "<span class="mono">"servicio.response.id"</span></p></crate::ui::Info>
            {pairs_of(id.clone(), |n| n.vars.clone(), |n, v| n.vars = v, "variable", "servicio.response.id")}
        }.into_any(),
        "input" | "data" => {
            let i = id.clone();
            view! {
                <h4>{if kind == "input" { "Campos de la entrada (JSON)" } else { "Datos fijos (JSON)" }}</h4>
                {(kind == "input").then(|| view! { <crate::ui::Info><p>"Es global: cualquier nodo lee "<span class="mono">{format!("{{{{{}.campo}}}}", n0.key)}</span>" sin conectarse."</p></crate::ui::Info> })}
                <JsonField value=n0.data.clone() rows=10 on_change=Callback::new(move |v: Value| ed.edit_node(&i, move |n| n.data = v)) />
            }.into_any()
        }
        "transform" => {
            let i = id.clone();
            view! {
                <h4>"Código JavaScript"</h4>
                <crate::ui::Info><p>"Recibe "<span class="mono">"nodes"</span>", "<span class="mono">"input"</span>" y "<span class="mono">"env"</span>"; devuelve "<span class="mono">"{ status, body }"</span>". Ayudantes: "<span class="mono">"jwt(token)"</span>", "<span class="mono">"paginate(lista, página, tamaño)"</span>". Límite: 1 s."</p></crate::ui::Info>
                <textarea class="in" rows="14" spellcheck="false" prop:value=n0.code.clone()
                    on:input=move |e| { let v = event_target_value(&e); ed.edit_node(&i, move |n| n.code = v) }></textarea>
            }.into_any()
        }
        "crypto" => view! { <crate::flow_steps::CryptoEditor id=id.clone() /> }.into_any(),
        "lookup" => view! { <crate::flow_steps::LookupEditor id=id.clone() /> }.into_any(),
        "response" => {
            let (i, i2) = (id.clone(), id.clone());
            view! {
                <h4>"Status"</h4>
                <VarField value=Signal::derive(move || node.with(|n| n.as_ref().map(|n| n.status.clone()).unwrap_or_default()))
                    on_input=Callback::new(move |v: String| ed.edit_node(&i, move |n| n.status = v)) placeholder="200" />
                <h4>"Body (JSON con plantillas)"</h4>
                <JsonField value=n0.body.clone() rows=12 on_change=Callback::new(move |v: Value| ed.edit_node(&i2, move |n| n.body = v)) />
            }.into_any()
        }
        "note" => {
            let i = id.clone();
            view! {
                <textarea class="in" rows="6" prop:value=n0.text.clone()
                    on:input=move |e| { let v = event_target_value(&e); ed.edit_node(&i, move |n| n.text = v) }></textarea>
            }.into_any()
        }
        "start" => view! { <p class="muted">"El flujo empieza aquí. Conecta su salida al primer paso."</p> }.into_any(),
        "end" => view! { <crate::ui::Info><p>"Une las ramas que llegan y da el flujo por terminado aunque no haya Respuesta."</p></crate::ui::Info> }.into_any(),
        _ => view! { <p class="muted">"Sin opciones."</p> }.into_any(),
    };
    let id_c = id.clone();
    let key_ref = move || format!("{{{{{}}}}}", node.with(|n| n.as_ref().map(|n| n.key.clone()).unwrap_or_default()));
    view! {
        <section>
            <div class="row">
                <span class="chip">{label(&kind)}</span>
                <span class="fn-ref" title="Clic copia la referencia" on:click=move |_| copy(&key_ref())>{key_ref}</span>
                <span class="grow"></span>
                <button class="btn ghost sm danger" title="Borrar nodo (Supr)" on:click=move |_| ed.delete_selected()>"🗑"</button>
            </div>
            <div class="insp-2">
                <label class="f">"Nombre"{node_text(id.clone(), |n| n.name.clone(), |n, v| n.name = v, "Nombre visible")}</label>
                <label class="f" title="Con esta clave lo leen otros nodos: {{clave.campo}}">"Clave"{node_text(id.clone(), |n| n.key.clone(), |n, v| n.key = v, "servicio")}</label>
            </div>
            <details class="insp-more" open=node.with_untracked(|n| n.as_ref().is_some_and(|n| !n.desc.is_empty()))>
                <summary>"Descripción"</summary>
                {node_text(id.clone(), |n| n.desc.clone(), |n, v| n.desc = v, "Para qué sirve este paso (opcional)")}
            </details>
            <div class="row swatches" title="Color del nodo en el lienzo">
                {COLORS.into_iter().map(|c| {
                    let i = id_c.clone();
                    view! { <span class="swatch" class:on=move || node.with(|n| n.as_ref().is_some_and(|n| n.color == c)) title=if c.is_empty() { "Sin color" } else { c }
                        style=format!("background:{}", if c.is_empty() { "var(--bg-3)" } else { c }) on:click=move |_| ed.edit_node(&i, |n| n.color = c.into())></span> }
                }).collect_view()}
            </div>
        </section>
        <div class="tabs">
            <button class:on=move || tab.get() == "edit" on:click=move |_| tab.set("edit")>"Editar"</button>
            <button class:on=move || tab.get() == "res" on:click=move |_| tab.set("res")>"Resultado"</button>
        </div>
        <section style:display=move || if tab.get() == "edit" { "flex" } else { "none" }>{body}</section>
        <section style:display=move || if tab.get() == "res" { "flex" } else { "none" }><NodeResult id=id /></section>
    }
    .into_any()
}

#[component]
fn NodeResult(id: String) -> impl IntoView {
    let ed = ed();
    let id2 = id.clone();
    move || {
        let Some(st) = ed.steps.with(|s| s.get(&id2).cloned()) else {
            let id = id.clone();
            return view! {
                <p class="muted">"Todavía no se ha ejecutado. Ejecuta el flujo o sólo este nodo."</p>
                <button class="btn" on:click=move |_| ed.run_node(id.clone())>"▶ Ejecutar este nodo"</button>
            }
            .into_any();
        };
        let out = &st["out"];
        let key = st["key"].as_str().unwrap_or("").to_string();
        let pick = |k: &str| (!out[k].is_null()).then(|| out[k].clone());
        view! {
            <div class="row">
                <span class=if st["ok"] == Value::Bool(true) { "chip ok" } else { "chip err" }>{if st["ok"] == Value::Bool(true) { "OK" } else { "Error" }}</span>
                {out["status"].as_u64().map(|s| view! { <span class=crate::ui::status_class(Some(s))>{s}</span> })}
                <span class="muted">{format!("{} ms", st["ms"])}</span>
                {st["port"].as_str().map(|p| view! { <span class="chip" title="Por qué salida siguió el flujo">{match p { "out" => "siguió por la salida normal".into(), "error" => "siguió por la salida de error".into(), "true" => "salió por «sí»".into(), "false" => "salió por «no»".into(), p => format!("siguió por «{p}»") }}</span> })}
            </div>
            {st["error"].as_str().map(|e| view! { <pre class="code err-text">{e.to_string()}</pre> })}
            {st["sent"]["url"].as_str().map(|u| view! { <div class="mono muted" style="word-break:break-all">{format!("{} {u}", st["sent"]["method"].as_str().unwrap_or(""))}</div> })}
            {out["value"].as_str().map(|t| view! { <crate::flow_steps::TokenParts text=t.to_string() /> })}
            <crate::flow_steps::PinList id=id2.clone() />
            <p class="muted">"Clic en una clave copia su "<span class="mono">"{{ruta}}"</span>" para usarla en otro nodo."</p>
            {pick("response").map(|v| view! { <h4>"Respuesta"</h4><JsonTree value=v prefix=format!("{key}.response") /> })}
            {pick("value").map(|v| view! { <h4>"Valor"</h4><JsonTree value=v prefix=key.clone() /> })}
            {pick("input").map(|v| view! { <h4>"Entrada enviada"</h4><JsonTree value=v prefix=format!("{key}.input") /> })}
            {pick("headers").map(|v| view! { <h4>"Headers"</h4><JsonTree value=v prefix=format!("{key}.headers") /> })}
        }
        .into_any()
    }
}

/// Peticiones de la colección del flujo (para elegir la petición de un Servicio o el global).
fn col_paths(ed: Ed) -> Vec<(String, String)> {
    let a = app();
    let col = ed.flow.with(|f| f.collection.clone());
    a.collections.with(|cs| cs.iter().find(|c| c.id == col).map(|c| bru_paths(&c.items)).unwrap_or_default())
}

#[component]
fn RequestEditor(id: String) -> impl IntoView {
    let ed = ed();
    let id2 = id.clone();
    let id_r = id.clone();
    let id3 = id.clone();
    let tab = RwSignal::new("vars");
    let tabs: [(&str, &str); 5] = [("vars", "Vars"), ("query", "Query"), ("headers", "Headers"), ("body", "Body"), ("save", "Guardar")];
    let (id_b, id4, id_e) = (id.clone(), id.clone(), id.clone());
    view! {
        <h4>"Petición"</h4>
        <div class="row">
            <select class="in grow" prop:value=move || ed.flow.with(|f| f.node(&id2).map(|n| n.req_ref.clone()).unwrap_or_default())
                on:change=move |e| { let v = event_target_value(&e); ed.edit_node(&id_r, move |n| n.req_ref = v) }>
                <option value="">{move || if ed.flow.with(|f| f.node(&id4).is_some_and(|n| n.req_ref.is_empty() && n.request.is_some())) { "— petición propia (cURL) —" } else { "— elige una petición —" }}</option>
                {move || col_paths(ed).into_iter().map(|(p, name)| { let (on, i) = (p.clone(), id3.clone()); view! { <option value=p.clone() selected=move || ed.flow.with(|f| f.node(&i).is_some_and(|n| n.req_ref == on))>{format!("{name}  ·  {p}")}</option> } }).collect_view()}
            </select>
            <button class="btn primary" title="Enviar sólo este servicio con los datos de la última ejecución" on:click=move |_| { ed.run_node(id_b.clone()) }>"Enviar"</button>
        </div>
        <Endpoint id=id_e />
        <crate::ui::Info><p>"Puedes arrastrar una petición de la barra lateral al lienzo. Usa "<span class="mono">"{{entrada.campo}}"</span>" o "<span class="mono">"{{otro_nodo.response.x}}"</span>" en los valores."</p></crate::ui::Info>
        <div class="tabs">
            {tabs.into_iter().map(|(k, l)| view! { <button class:on=move || tab.get() == k on:click=move |_| tab.set(k)>{l}</button> }).collect_view()}
        </div>
        {move || {
            let id = id.clone();
            match tab.get() {
                "vars" => view! { <crate::ui::Info><p>"Variables de la petición antes de enviarla (ganan sobre el entorno)."</p></crate::ui::Info>{input_chips(ed, id.clone())}{pairs_of(id, |n| n.vars.clone(), |n, v| n.vars = v, "variable", "{{entrada.userId}}")} }.into_any(),
                "query" => pairs_of(id, |n| n.query_overrides.clone(), |n, v| n.query_overrides = v, "parámetro", "valor").into_any(),
                "headers" => pairs_of(id, |n| n.header_overrides.clone(), |n, v| n.header_overrides = v, "header", "valor").into_any(),
                "body" => view! { <p class="muted">"Campos del body JSON a reemplazar (ruta → valor)."</p>{pairs_of(id, |n| n.body_overrides.clone(), |n, v| n.body_overrides = v, "cliente.id", "{{entrada.id}}")} }.into_any(),
                _ => view! { <crate::ui::Info><p>"Guarda valores de la respuesta en variables del flujo: "<span class="mono">"response.x"</span>", "<span class="mono">"status"</span>", "<span class="mono">"headers.x"</span></p></crate::ui::Info>{pairs_of(id, |n| n.save_vars.clone(), |n, v| n.save_vars = v, "variable", "response.id")} }.into_any(),
            }
        }}
    }
}

/// Qué endpoint llama el servicio: método y URL con sus `{{variables}}` (y de dónde sale cada una),
/// su documentación y lo que se envió de verdad la última vez. También acepta pegar un cURL.
#[component]
fn Endpoint(id: String) -> impl IntoView {
    let (a, ed) = (app(), ed());
    let id = StoredValue::new(id);
    let node = Memo::new(move |_| ed.flow.with(|f| f.node(&id.get_value()).cloned()).unwrap_or_default());
    let saved: RwSignal<Option<Request>> = RwSignal::new(None);
    // Sólo cuando cambia la petición elegida (no con cada tecla en las vars del servicio).
    let target = Memo::new(move |_| (ed.flow.with(|f| f.collection.clone()), node.with(|n| n.req_ref.clone())));
    Effect::new(move |_| {
        let want = target.get();
        saved.set(None);
        if !want.1.is_empty() {
            spawn_local(async move {
                let r = api::get::<Request>(&format!("/api/request?collection={}&path={}", api::enc(&want.0), api::enc(&want.1))).await.ok();
                // Si eligieron otra petición mientras llegaba esta, se descarta.
                if target.get_untracked() == want {
                    saved.set(r);
                }
            });
        }
    });
    let curl = RwSignal::new(String::new());
    let use_curl = move |_| match martino_core::curl::parse_curl(&curl.get_untracked()) {
        Ok(r) => {
            ed.edit_node(&id.get_value(), move |n| { n.req_ref.clear(); n.request = Some(r) });
            curl.set(String::new());
            a.toast("ok", "El servicio ahora usa ese cURL");
        }
        Err(e) => a.err(e),
    };
    let detail = move || {
        let n = node.get();
        let inline = n.req_ref.is_empty();
        let r = if inline { n.request.clone() } else { saved.get() }?;
        // Variables que pide la petición, en orden de aparición, y quién les da valor.
        let mut names: Vec<String> = vec![];
        let texts = [r.url.clone(), r.body.json.clone(), r.body.text.clone()].into_iter().chain(r.headers.iter().map(|h| h.value.clone()));
        for t in texts {
            for seg in martino_core::interp::segments(&t) {
                if let martino_core::interp::Seg::Var(v, _) = seg {
                    if !names.iter().any(|x| x == v) {
                        names.push(v.to_string());
                    }
                }
            }
        }
        let rows = names.into_iter().map(|v| {
            let from = n.vars.iter().find(|p| p.name == v).map(|p| p.value.clone());
            let cls = if from.is_some() { "mono" } else { "muted" };
            view! { <span class="mono">{v}</span><span class=cls>{from.unwrap_or_else(|| "del entorno o de una variable del flujo".into())}</span> }
        }).collect_view();
        let m = r.method.to_lowercase();
        Some(view! {
            <div class="endpoint">
                <div class="row">
                    <span class=format!("m m-{m}")>{r.method.to_uppercase()}</span>
                    {(!r.id.is_empty()).then(|| view! { <span class="fn-id">{r.id.clone()}</span> })}
                    <b class="grow">{if r.name.is_empty() { n.name.clone() } else { r.name.clone() }}</b>
                </div>
                {if inline {
                    view! { <input class="in mono" title="URL de la petición propia" prop:value=r.url.clone()
                        on:change=move |e| { let v = event_target_value(&e); ed.edit_node(&id.get_value(), move |n| if let Some(r) = n.request.as_mut() { r.url = v }) } /> }.into_any()
                } else {
                    view! { <div class="mono ep-url">{r.url.clone()}</div> }.into_any()
                }}
                <div class="ep-vars">{rows}</div>
                {(!r.docs.trim().is_empty()).then(|| view! { <p class="ep-docs">{r.docs.trim().to_string()}</p> })}
            </div>
        })
    };
    let last = move || ed.steps.with(|s| s.get(&id.get_value()).cloned()).and_then(|st| {
        let sent: SentRequest = serde_json::from_value(st["sent"].clone()).ok().filter(|r: &SentRequest| !r.url.is_empty())?;
        let status = st["out"]["status"].as_u64().map(|s| format!(" → {s}")).unwrap_or_default();
        Some(view! {
            <details class="ep-sent" open>
                <summary>{format!("Última ejecución{status} · {} ms", st["ms"])}</summary>
                <pre class="code">{martino_core::model::raw_http(&sent, true)}</pre>
            </details>
        })
    });
    view! {
        {detail}
        {last}
        <details class="ep-curl">
            <summary>"Pegar un cURL en este servicio"</summary>
            <textarea class="in mono" rows="4" placeholder="curl -X POST https://… -H 'Content-Type: application/json' -d '{…}'" prop:value=move || curl.get() on:input=move |e| curl.set(event_target_value(&e))></textarea>
            <button class="btn sm" disabled=move || curl.with(|c| c.trim().is_empty()) on:click=use_curl>"Usar este cURL"</button>
        </details>
    }
}

/// Campos de la Entrada que el servicio aún no usa: un clic agrega la variable `campo = {{entrada.campo}}`.
fn input_chips(ed: Ed, id: String) -> impl IntoView {
    let id = StoredValue::new(id);
    let free = move || ed.flow.with(|f| {
        let (Some(inp), Some(n)) = (f.input_node(), f.node(&id.get_value())) else { return vec![] };
        let fields = inp.data.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()).unwrap_or_default();
        fields.into_iter().map(|k| (format!("{{{{{}.{k}}}}}", inp.key), k)).filter(|(r, _)| !n.vars.iter().any(|p| p.value.contains(r.as_str()))).collect::<Vec<_>>()
    });
    move || {
        let list = free();
        (!list.is_empty()).then(|| view! {
            <div class="in-chips"><span class="muted">"Insertar de la Entrada:"</span>
                {list.into_iter().map(|(r, k)| { let (r2, k2) = (r.clone(), k.clone()); view! {
                    <button class="chip v-flow" title=format!("Agrega la variable {k} = {r}")
                        on:click=move |_| { let (r, k) = (r2.clone(), k2.clone()); ed.edit_node(&id.get_value(), move |n| n.vars.push(Pair::new(&k, &r))) }>{format!("＋ {k}")}</button>
                } }).collect_view()}
            </div>
        })
    }
}

/// La comparación en palabras completas, con su código corto: «No es igual a (NEQ)».
pub fn op_label(op: &str) -> String {
    let (w, c) = match op {
        "eq" => ("Es igual a", "EQ"),
        "neq" => ("No es igual a", "NEQ"),
        "gt" => ("Mayor que", "GT"),
        "gte" => ("Mayor o igual que", "GTE"),
        "lt" => ("Menor que", "LT"),
        "lte" => ("Menor o igual que", "LTE"),
        "contains" => ("Contiene", "CONTAINS"),
        "notContains" => ("No contiene", "NOT CONTAINS"),
        "startsWith" => ("Empieza con", "STARTS WITH"),
        "endsWith" => ("Termina con", "ENDS WITH"),
        "in" => ("Está en la lista", "IN"),
        "notIn" => ("No está en la lista", "NOT IN"),
        "isDefined" => ("Existe", "DEFINED"),
        "isUndefined" => ("No existe", "UNDEFINED"),
        "isNull" => ("Es nulo", "NULL"),
        "isNotNull" => ("No es nulo", "NOT NULL"),
        "isEmpty" => ("Está vacío", "EMPTY"),
        "isNotEmpty" => ("No está vacío", "NOT EMPTY"),
        "isTruthy" => ("Es verdadero", "TRUE"),
        "isFalsy" => ("Es falso", "FALSE"),
        "isNumber" => ("Es un número", "NUMBER"),
        "isString" => ("Es un texto", "STRING"),
        _ => return op.to_string(),
    };
    format!("{w} ({c})")
}

/// Con la última ejecución: el valor actual del campo (texto) y si la regla se cumple.
pub fn rule_eval(r: &Rule) -> Option<(String, bool)> {
    if r.left.trim().is_empty() {
        return None;
    }
    app().flow_ctx.with(|f| {
        let (flow, outs) = f.as_ref()?;
        if outs.is_empty() {
            return None;
        }
        let vars = serde_json::Map::new();
        let ctx = martino_core::flow::RefCtx { flow, outs, vars: &vars };
        // Ruta sin llaves (lo normal) o plantilla con `{{…}}`.
        let actual = if r.left.contains("{{") { Some(ctx.value(&r.left, &|_| None, false)) } else { ctx.lookup(r.left.trim()) };
        let ok = martino_core::flow::check(actual.as_ref(), &r.op, &ctx.render(&r.right, &|_| None));
        Some((actual.map(|v| martino_core::interp::value_to_text(&v)).unwrap_or_else(|| "(no existe)".into()), ok))
    })
}

fn rule_preview(r: &Rule) -> Option<impl IntoView> {
    rule_eval(r).map(|(shown, ok)| {
        view! {
            <div class="rule-prev" class:ok=ok title="Según la última ejecución">
                <span>"vale "</span><b class="mono">{shown}</b>
                <span class="grow"></span><span>{if ok { "✓ se cumple" } else { "✗ no se cumple" }}</span>
            </div>
        }
    })
}

/// Símbolo corto para el lienzo: `≤`, `≠`… o la palabra completa si no hay símbolo.
pub fn op_short(op: &str) -> String {
    match op {
        "eq" => "=".into(),
        "neq" => "≠".into(),
        "gt" => ">".into(),
        "gte" => "≥".into(),
        "lt" => "<".into(),
        "lte" => "≤".into(),
        o => op_label(o).split(" (").next().unwrap_or(o).to_lowercase(),
    }
}

#[component]
fn ConditionEditor(id: String) -> impl IntoView {
    let ed = ed();
    let id_m = id.clone();
    // Las filas dependen sólo de cuántas reglas hay: así escribir no rehace los campos (ni pierde el foco).
    let rule = move |id: &str, i: usize| ed.flow.with(|f| f.node(id).and_then(|n| n.rules.get(i).cloned())).unwrap_or_default();
    let count = Memo::new(move |_| ed.flow.with(|f| f.node(&id_m).map(|n| n.rules.len()).unwrap_or(0)));
    let id_mode = id.clone();
    let id_mode2 = id.clone();
    let id_add = id.clone();
    let set_rule = move |id: &str, i: usize, f: Box<dyn FnOnce(&mut Rule)>| ed.edit_node(id, move |n| { if let Some(r) = n.rules.get_mut(i) { f(r) } });
    view! {
        <h4>"Reglas"</h4>
        <div class="row">
            <span class="muted">"Sale por «sí» cuando"</span>
            <span class="seg">
                {[("all", "se cumplen todas (Y)"), ("any", "se cumple alguna (O)")].into_iter().map(|(m, l)| {
                    let (a, b) = (id_mode.clone(), id_mode2.clone());
                    view! { <button class:on=move || ed.flow.with(|f| f.node(&a).map(|n| if n.mode == "any" { "any" } else { "all" } == m).unwrap_or(false))
                        on:click=move |_| ed.edit_node(&b, move |n| n.mode = m.into())>{l}</button> }
                }).collect_view()}
            </span>
        </div>
        <crate::ui::Info><p>"Cada regla se lee como una frase: "<b>"si"</b>" el campo "<b>"cumple"</b>" la comparación con el valor. Escribe la ruta sin llaves ("<span class="mono">"entrada.tipo"</span>", "<span class="mono">"servicio.status"</span>"): al poner el punto se sugieren los campos. Tras ejecutar, cada regla muestra su valor y si se cumple."</p></crate::ui::Info>
        {move || (0..count.get()).map(|i| {
            let (a1, a2, a3, a4) = (id.clone(), id.clone(), id.clone(), id.clone());
            let (r1, r2, r3, r4) = (id.clone(), id.clone(), id.clone(), id.clone());
            view! {
                <div class="rule">
                    <div class="rule-l"><span class="rule-w">{let r6 = id.clone(); move || if i == 0 { "Si".to_string() } else if ed.flow.with(|f| f.node(&r6).is_some_and(|n| n.mode == "any")) { "o si".into() } else { "y si".into() }}</span>
                        <span class="grow"><VarField value=Signal::derive(move || rule(&r1, i).left) placeholder="servicio.status"
                            on_input=Callback::new(move |v: String| set_rule(&a1, i, Box::new(move |r| r.left = v))) /></span>
                    </div>
                    {let r5 = id.clone(); move || rule_preview(&rule(&r5, i))}
                    <div class="row">
                        <select class="in" title="Comparación" prop:value=move || rule(&r2, i).op on:change=move |e| { let v = event_target_value(&e); set_rule(&a2, i, Box::new(move |r| r.op = v)) }>
                            {OPS.into_iter().map(|o| view! { <option value=o>{op_label(o)}</option> }).collect_view()}
                        </select>
                        <span class="grow" style:display=move || if rule(&r3, i).op.starts_with("is") { "none" } else { "block" }>
                            <VarField value=Signal::derive(move || rule(&r4, i).right) placeholder="200"
                                on_input=Callback::new(move |v: String| set_rule(&a3, i, Box::new(move |r| r.right = v))) />
                        </span>
                        <button class="btn ghost sm" title="Quitar regla" on:click=move |_| ed.edit_node(&a4, move |n| { if i < n.rules.len() { n.rules.remove(i); } })>"✕"</button>
                    </div>
                </div>
            }
        }).collect_view()}
        <button class="btn rule-add" on:click=move |_| ed.edit_node(&id_add, |n| n.rules.push(Rule { left: String::new(), op: "eq".into(), right: String::new() }))>"＋ Agregar otra regla"</button>
    }
}

#[component]
fn FlowInspector() -> impl IntoView {
    let (a, ed) = (app(), ed());
    let result = move || {
        ed.result.get().map(|r| {
            let body = r["body"].clone();
            let diffs: Vec<Value> = r["compare"]["diffs"].as_array().cloned().unwrap_or_default();
            view! {
                <section>
                    <h4>"Última ejecución"</h4>
                    <div class="row">
                        <span class="chip">{r["verdict"].as_str().unwrap_or("?").to_string()}</span>
                        {r["status"].as_u64().map(|s| view! { <span class=crate::ui::status_class(Some(s))>{s}</span> })}
                    </div>
                    {r["error"].as_str().map(|e| view! { <pre class="code err-text">{e.to_string()}</pre> })}
                    {(!body.is_null()).then(|| view! { <h4>"Respuesta del flujo"</h4><JsonTree value=body prefix="" /> })}
                    {(!diffs.is_empty()).then(|| view! {
                        <h4>"Diferencias con el servicio global"</h4>
                        {diffs.into_iter().map(|d| view! { <div class="mono" style="font-size:12px">{format!("≠ {} ({}): esperado {} · real {}", d["path"].as_str().unwrap_or(""), d["kind"].as_str().unwrap_or(""), d["expected"], d["actual"])}</div> }).collect_view()}
                    })}
                </section>
            }
        })
    };
    view! {
        {result}
        <section>
            <h4>"Flujo"</h4>
            <label class="f">"Nombre"<input class="in" data-noac prop:value=move || ed.flow.with(|f| f.name.clone()) on:input=move |e| { let v = event_target_value(&e); ed.edit(|f| f.name = v) } /></label>
            <label class="f">"Colección (de dónde salen las peticiones)"
                <select class="in" prop:value=move || ed.flow.with(|f| f.collection.clone()) on:change=move |e| { let v = event_target_value(&e); ed.edit(|f| f.collection = v) }>
                    <option value="">"—"</option>
                    {move || a.collections.get().into_iter().map(|c| { let on = c.id.clone(); view! { <option value=c.id.clone() selected=move || ed.flow.with(|f| f.collection == on)>{c.name}</option> } }).collect_view()}
                </select>
            </label>
            <crate::ui::Info><p>"Selecciona un nodo para editarlo. Arrastra desde el punto de la derecha de un nodo hasta otro para conectarlos; Supr borra lo seleccionado; Ctrl+rueda hace zoom."</p></crate::ui::Info>
        </section>
        <section>
            <h4>"Comparación con el servicio global"</h4>
            <label class="f">"Petición global"
                <select class="in" prop:value=move || ed.flow.with(|f| f.global.as_ref().map(|g| g.req_ref.clone()).unwrap_or_default())
                    on:change=move |e| { let v = event_target_value(&e); ed.edit(|f| f.global = (!v.is_empty()).then_some(flow::Global { req_ref: v })) }>
                    <option value="">"— sin comparación —"</option>
                    {move || col_paths(ed).into_iter().map(|(p, name)| { let on = p.clone(); view! { <option value=p.clone() selected=move || ed.flow.with(|f| f.global.as_ref().is_some_and(|g| g.req_ref == on))>{format!("{name}  ·  {p}")}</option> } }).collect_view()}
                </select>
            </label>
            <label class="f">"Modo"
                <select class="in" prop:value=move || ed.flow.with(|f| f.compare.mode.clone()) on:change=move |e| { let v = event_target_value(&e); ed.edit(|f| f.compare.mode = v) }>
                    <option value="expected">"expected: los campos del flujo deben existir e igualar"</option>
                    <option value="strict">"strict: todo igual"</option>
                    <option value="fields">"fields: sólo los campos listados"</option>
                </select>
            </label>
            <label class="f">"Ignorar (rutas separadas por coma)"
                <input class="in" data-noac placeholder="fecha, id" prop:value=move || ed.flow.with(|f| f.compare.ignore.join(", "))
                    on:change=move |e| { let v = event_target_value(&e); ed.edit(|f| f.compare.ignore = v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()) } />
            </label>
        </section>
        <section>
            <h4>"Variables del flujo"</h4>
            <KvTable pairs=Signal::derive(move || ed.flow.with(|f| f.vars.clone())) on_change=Callback::new(move |v: Vec<Pair>| ed.edit(|f| f.vars = v)) />
        </section>
    }
}

#[component]
fn CodeView() -> impl IntoView {
    let (a, ed) = (app(), ed());
    let fmt = RwSignal::new("yaml");
    let render = move |f: &str| ed.flow.with_untracked(|fl| if f == "yaml" { flow_to_yaml(fl) } else { serde_json::to_string_pretty(fl).unwrap_or_default() });
    let text = RwSignal::new(render("yaml"));
    let errs: RwSignal<Vec<String>> = RwSignal::new(vec![]);
    let switch = move |f: &'static str| {
        fmt.set(f);
        text.set(render(f));
        errs.set(vec![]);
    };
    let apply = move |_| {
        let t = text.get_untracked();
        let parsed: Result<Flow, String> = if fmt.get_untracked() == "yaml" { flow_from_yaml(&t) } else { serde_json::from_str(&t).map_err(|e| e.to_string()) };
        match parsed {
            Err(e) => errs.set(vec![e]),
            Ok(f) => {
                let v = flow::validate(&f);
                if v.is_empty() {
                    ed.replace(f);
                    errs.set(vec![]);
                    a.toast("ok", "Cambios aplicados (Ctrl+Z para deshacer)");
                } else {
                    errs.set(v);
                }
            }
        }
    };
    view! {
        <div class="tabs">
            <button class:on=move || fmt.get() == "yaml" on:click=move |_| switch("yaml")>"YAML"</button>
            <button class:on=move || fmt.get() == "json" on:click=move |_| switch("json")>"JSON"</button>
            <span class="grow"></span>
            <button class="btn ghost sm" on:click=move |_| copy(&text.get_untracked())>"Copiar"</button>
        </div>
        <section class="grow">
            <textarea class="in grow" spellcheck="false" style="min-height:50vh" prop:value=move || text.get() on:input=move |e| text.set(event_target_value(&e))></textarea>
            {move || errs.get().into_iter().map(|e| view! { <div class="err-text">{e}</div> }).collect_view()}
            <div class="row">
                <button class="btn primary" on:click=apply>"Aplicar"</button>
                <button class="btn" on:click=move |_| switch(fmt.get_untracked())>"Descartar"</button>
                <button class="btn ghost" on:click=move |_| ed.code.set(false)>"Cerrar"</button>
            </div>
        </section>
    }
}
