//! Panel derecho del editor de flujos: ajustes del flujo, edición del nodo seleccionado,
//! su último resultado y el código (YAML/JSON) del flujo entero.
use crate::flows::{bru_paths, ed, label, Ed};
use crate::state::app;
use crate::ui::{copy, JsonTree, KvTable, VarField};
use leptos::prelude::*;
use martino_core::flow::{self, Flow, Node, Rule, OPS};
use martino_core::flow_yaml::{flow_from_yaml, flow_to_yaml};
use martino_core::model::Pair;
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
            <p class="muted">"Cada fila guarda un valor en una variable del flujo. Vale una ruta sin llaves: "<span class="mono">"servicio.response.id"</span></p>
            {pairs_of(id.clone(), |n| n.vars.clone(), |n, v| n.vars = v, "variable", "servicio.response.id")}
        }.into_any(),
        "input" | "data" => {
            let i = id.clone();
            view! {
                <h4>{if kind == "input" { "Campos de la entrada (JSON)" } else { "Datos fijos (JSON)" }}</h4>
                {(kind == "input").then(|| view! { <p class="muted">"Es global: cualquier nodo lee "<span class="mono">{format!("{{{{{}.campo}}}}", n0.key)}</span>" sin conectarse."</p> })}
                <JsonField value=n0.data.clone() rows=10 on_change=Callback::new(move |v: Value| ed.edit_node(&i, move |n| n.data = v)) />
            }.into_any()
        }
        "transform" => {
            let i = id.clone();
            view! {
                <h4>"Código JavaScript"</h4>
                <p class="muted">"Recibe "<span class="mono">"nodes"</span>", "<span class="mono">"input"</span>" y "<span class="mono">"env"</span>"; devuelve "<span class="mono">"{ status, body }"</span>". Ayudantes: "<span class="mono">"jwt(token)"</span>", "<span class="mono">"paginate(lista, página, tamaño)"</span>". Límite: 1 s."</p>
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
        "end" => view! { <p class="muted">"Une las ramas que llegan y da el flujo por terminado aunque no haya Respuesta."</p> }.into_any(),
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
            <label class="f">"Clave (referencia en plantillas)"{node_text(id.clone(), |n| n.key.clone(), |n, v| n.key = v, "servicio")}</label>
            <label class="f">"Nombre"{node_text(id.clone(), |n| n.name.clone(), |n, v| n.name = v, "Nombre visible")}</label>
            <label class="f">"Descripción"{node_text(id.clone(), |n| n.desc.clone(), |n, v| n.desc = v, "Opcional")}</label>
            <div class="row">
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
                {st["port"].as_str().map(|p| view! { <span class="chip">{format!("sale por «{p}»")}</span> })}
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

/// Peticiones de la colección del flujo (para elegir el .bru de un Servicio o el global).
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
    let id_b = id.clone();
    view! {
        <h4>"Petición (.bru)"</h4>
        <div class="row">
            <select class="in grow" prop:value=move || ed.flow.with(|f| f.node(&id2).map(|n| n.req_ref.clone()).unwrap_or_default())
                on:change=move |e| { let v = event_target_value(&e); ed.edit_node(&id_r, move |n| n.req_ref = v) }>
                <option value="">"— elige una petición —"</option>
                {move || col_paths(ed).into_iter().map(|(p, name)| { let (on, i) = (p.clone(), id3.clone()); view! { <option value=p.clone() selected=move || ed.flow.with(|f| f.node(&i).is_some_and(|n| n.req_ref == on))>{format!("{name}  ·  {p}")}</option> } }).collect_view()}
            </select>
            <button class="btn primary" title="Enviar sólo este servicio con los datos de la última ejecución" on:click=move |_| { ed.run_node(id_b.clone()) }>"Enviar"</button>
        </div>
        <p class="muted">"Puedes arrastrar una petición de la barra lateral al lienzo. Usa "<span class="mono">"{{entrada.campo}}"</span>" o "<span class="mono">"{{otro_nodo.response.x}}"</span>" en los valores."</p>
        <div class="tabs">
            {tabs.into_iter().map(|(k, l)| view! { <button class:on=move || tab.get() == k on:click=move |_| tab.set(k)>{l}</button> }).collect_view()}
        </div>
        {move || {
            let id = id.clone();
            match tab.get() {
                "vars" => view! { <p class="muted">"Variables de la petición antes de enviarla (ganan sobre el entorno)."</p>{pairs_of(id, |n| n.vars.clone(), |n, v| n.vars = v, "variable", "{{entrada.userId}}")} }.into_any(),
                "query" => pairs_of(id, |n| n.query_overrides.clone(), |n, v| n.query_overrides = v, "parámetro", "valor").into_any(),
                "headers" => pairs_of(id, |n| n.header_overrides.clone(), |n, v| n.header_overrides = v, "header", "valor").into_any(),
                "body" => view! { <p class="muted">"Campos del body JSON a reemplazar (ruta → valor)."</p>{pairs_of(id, |n| n.body_overrides.clone(), |n, v| n.body_overrides = v, "cliente.id", "{{entrada.id}}")} }.into_any(),
                _ => view! { <p class="muted">"Guarda valores de la respuesta en variables del flujo: "<span class="mono">"response.x"</span>", "<span class="mono">"status"</span>", "<span class="mono">"headers.x"</span></p>{pairs_of(id, |n| n.save_vars.clone(), |n, v| n.save_vars = v, "variable", "response.id")} }.into_any(),
            }
        }}
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
            <span class="muted">"Sale por «sí» si se cumplen"</span>
            <select class="in" prop:value=move || ed.flow.with(|f| f.node(&id_mode).map(|n| n.mode.clone()).unwrap_or_default())
                on:change=move |e| { let v = event_target_value(&e); ed.edit_node(&id_mode2, move |n| n.mode = v) }>
                <option value="all">"todas"</option>
                <option value="any">"alguna"</option>
            </select>
        </div>
        <p class="muted">"A la izquierda, una ruta ("<span class="mono">"servicio.status"</span>", "<span class="mono">"entrada.tipo"</span>") o una plantilla."</p>
        {move || (0..count.get()).map(|i| {
            let (a1, a2, a3, a4) = (id.clone(), id.clone(), id.clone(), id.clone());
            let (r1, r2, r3, r4) = (id.clone(), id.clone(), id.clone(), id.clone());
            view! {
                <div class="rule">
                    <VarField value=Signal::derive(move || rule(&r1, i).left) placeholder="servicio.status"
                        on_input=Callback::new(move |v: String| set_rule(&a1, i, Box::new(move |r| r.left = v))) />
                    <div class="row">
                        <select class="in" prop:value=move || rule(&r2, i).op on:change=move |e| { let v = event_target_value(&e); set_rule(&a2, i, Box::new(move |r| r.op = v)) }>
                            {OPS.into_iter().map(|o| view! { <option value=o>{o}</option> }).collect_view()}
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
        <button class="btn sm" on:click=move |_| ed.edit_node(&id_add, |n| n.rules.push(Rule { left: String::new(), op: "eq".into(), right: String::new() }))>"+ Regla"</button>
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
            <label class="f">"Colección (de dónde salen los .bru)"
                <select class="in" prop:value=move || ed.flow.with(|f| f.collection.clone()) on:change=move |e| { let v = event_target_value(&e); ed.edit(|f| f.collection = v) }>
                    <option value="">"—"</option>
                    {move || a.collections.get().into_iter().map(|c| { let on = c.id.clone(); view! { <option value=c.id.clone() selected=move || ed.flow.with(|f| f.collection == on)>{c.name}</option> } }).collect_view()}
                </select>
            </label>
            <p class="muted">"Selecciona un nodo para editarlo. Arrastra desde el punto de la derecha de un nodo hasta otro para conectarlos; Supr borra lo seleccionado; Ctrl+rueda hace zoom."</p>
        </section>
        <section>
            <h4>"Comparación con el servicio global"</h4>
            <label class="f">"Petición global (.bru)"
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
