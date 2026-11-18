//! Lo nuevo del lienzo: la respuesta recortada en cada nodo (sólo lo que se usa o se fijó),
//! y los editores de los pasos Cifrado y Equivalencia.
use crate::api;
use crate::flows::{ed, Ed};
use crate::state::app;
use crate::ui::VarField;
use leptos::prelude::*;
use leptos::task::spawn_local;
use martino_core::crypto::{self, CryptoSpec, FORMATS};
use martino_core::flow::{self, Node, NodeOut};
use martino_core::interp::value_to_text;
use martino_core::lookup::{LookupSpec, Table};
use martino_core::model::Pair;
use martino_core::usage::short_path;
use serde_json::{json, Value};

/// Lo que se puede leer de un nodo ejecutado por una ruta relativa.
fn read(n: &Node, out: &NodeOut, path: &str) -> Option<Value> {
    if n.kind == "request" {
        flow::service_value(out, path)
    } else {
        flow::data_value(out, path)
    }
}

/// Lo que ve el «Todo» del nodo: la respuesta de un servicio o el valor de los demás.
fn base(n: &Node, out: &NodeOut) -> Value {
    if n.kind == "request" {
        out.response.clone()
    } else {
        out.value.clone()
    }
}

/// Valor corto para el lienzo.
fn brief(v: &Value) -> String {
    let t = match v {
        Value::Object(m) => format!("{{ {} campos }}", m.len()),
        Value::Array(a) => format!("[ {} ]", a.len()),
        v => value_to_text(v),
    };
    if t.chars().count() > 34 {
        format!("{}…", t.chars().take(33).collect::<String>())
    } else {
        t
    }
}

fn shows_result(kind: &str) -> bool {
    matches!(kind, "request" | "transform" | "crypto" | "lookup" | "data" | "extract")
}

/// Debajo del nodo, tras ejecutar: status/tiempo ya van arriba; aquí los campos que usan otros
/// nodos (azul), los que usa una condición (naranja) y los fijados (amarillo).
#[component]
pub fn NodeResp(id: String) -> impl IntoView {
    let ed = ed();
    move || {
        let mode = ed.resp.get();
        let n = ed.flow.with(|f| f.node(&id).cloned())?;
        let out = ed.outs.with(|o| o.get(&n.key).cloned())?;
        if mode == "nada" || !shows_result(&n.kind) {
            return None;
        }
        let used = ed.used.with(|u| u.get(&n.key).cloned().unwrap_or_default());
        // (campo visible, valor, clase, por qué se muestra, ruta completa que se copia)
        let mut rows: Vec<(String, String, &'static str, String, String)> = used
            .iter()
            .map(|u| {
                (
                    short_path(&u.path).to_string(),
                    read(&n, &out, &u.path).map(|v| brief(&v)).unwrap_or_else(|| "—".into()),
                    if u.cond { "cond" } else { "ref" },
                    format!("Lo usa «{}»", u.by),
                    u.path.clone(),
                )
            })
            .collect();
        for p in n.pins.iter().filter(|p| !used.iter().any(|u| &u.path == *p)) {
            rows.push((short_path(p).to_string(), read(&n, &out, p).map(|v| brief(&v)).unwrap_or_else(|| "—".into()), "pin", "Fijado por ti".into(), p.clone()));
        }
        let b = base(&n, &out);
        let keys: Vec<String> = b.as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
        let shown = |k: &str| rows.iter().any(|r| r.0 == k || r.0.starts_with(&format!("{k}.")) || r.0.starts_with(&format!("{k}[")));
        let rest: Vec<String> = keys.iter().filter(|k| !shown(k)).cloned().collect();
        // Nadie lo usa todavía: una vista previa de los primeros campos en vez de sólo «+ N».
        let preview = mode == "usado" && rows.is_empty();
        if mode == "todo" || preview {
            if b.is_object() {
                let take = if preview { 3 } else { rest.len() };
                let pre = if n.kind == "request" { "response." } else { "" };
                rows.extend(rest.iter().take(take).map(|k| (k.clone(), brief(&b[k.as_str()]), "", String::new(), format!("{pre}{k}"))));
            } else if rows.is_empty() && !b.is_null() {
                rows.push(("(todo)".into(), brief(&b), "", String::new(), String::new()));
            }
        }
        let hidden = if mode == "todo" { 0 } else { rest.len().saturating_sub(if preview { 3 } else { 0 }) };
        if rows.is_empty() && hidden == 0 {
            return None;
        }
        // Redimensionado: se ven todas las filas (con barra si no entran); si no, las 6 primeras.
        let max = if n.height.is_some() { usize::MAX } else { 6 };
        let extra = rows.len().saturating_sub(max);
        Some(view! {
            <div class="fn-resp" title="Respuesta recortada: lo que usan otros nodos y lo que fijaste. Cambia con «Respuesta en nodos» arriba.">
                {rows.into_iter().take(max).map(|(k, v, cls, why, path)| {
                    let full = if path.is_empty() { n.key.clone() } else { format!("{}.{path}", n.key) };
                    let tip = format!("{why}{}Clic copia {{{{{full}}}}} para pegarlo en otro nodo", if why.is_empty() { "" } else { " · " });
                    view! {
                        <div class=format!("fr-row {cls}") title=tip on:pointerdown=|e| e.stop_propagation()
                            on:click=move |_| crate::ui::copy(&format!("{{{{{full}}}}}"))><span class="fr-k">{k}</span><span class="fr-v">{v}</span></div>
                    }
                }).collect_view()}
                {(extra + hidden > 0).then(|| view! { <div class="fr-more">{format!("+ {} campos más", extra + hidden)}</div> })}
            </div>
        })
    }
}

/// En el Resultado del inspector: qué campos se ven siempre en el nodo.
#[component]
pub fn PinList(id: String) -> impl IntoView {
    let ed = ed();
    move || {
        let n = ed.flow.with(|f| f.node(&id).cloned())?;
        let out = ed.outs.with(|o| o.get(&n.key).cloned())?;
        let keys: Vec<String> = base(&n, &out).as_object()?.keys().cloned().collect();
        if !shows_result(&n.kind) || keys.is_empty() {
            return None;
        }
        let used = ed.used.with(|u| u.get(&n.key).cloned().unwrap_or_default());
        let prefix = if n.kind == "request" { "response." } else { "" };
        Some(view! {
            <h4>"Mostrar en el nodo"</h4>
            <crate::ui::Info><p>"Lo que usa otro nodo se muestra solo. Marca lo que quieras ver además."</p></crate::ui::Info>
            <div class="pins">
                {keys.into_iter().map(|k| {
                    let path = format!("{prefix}{k}");
                    let by = used.iter().find(|u| u.path == path || short_path(&u.path) == k).map(|u| u.by.clone());
                    let (id, p2) = (id.clone(), path.clone());
                    let on = n.pins.contains(&path) || by.is_some();
                    let (title, locked) = (by.clone().map(|b| format!("Lo usa «{b}»")).unwrap_or_default(), by.is_some());
                    view! {
                        <label class="pin" title=title>
                            <input type="checkbox" prop:checked=on disabled=locked
                                on:change=move |e| { let v = event_target_checked(&e); let p = p2.clone(); ed.edit_node(&id, move |n| { n.pins.retain(|x| *x != p); if v { n.pins.push(p) } }) } />
                            <span class="mono">{k}</span>
                            {by.map(|b| view! { <span class="muted">{format!("· lo usa {b}")}</span> })}
                        </label>
                    }
                }).collect_view()}
            </div>
        })
    }
}

// ---------- Cifrado ----------

fn spec_of(ed: Ed, id: &str) -> CryptoSpec {
    ed.flow.with(|f| f.node(id).and_then(|n| n.crypto.clone())).unwrap_or_default()
}

fn set_spec(ed: Ed, id: &str, f: impl FnOnce(&mut CryptoSpec) + 'static) {
    ed.edit_node(id, move |n| f(n.crypto.get_or_insert_with(CryptoSpec::default)));
}

/// Si la operación usa la parte pública de un par RSA (cifrar / verificar).
fn wants_public(s: &CryptoSpec) -> bool {
    matches!(s.op.as_str(), "encrypt" | "verify")
}

#[component]
pub fn CryptoEditor(id: String) -> impl IntoView {
    let ed = ed();
    let id_s = id.clone();
    let spec = Memo::new(move |_| spec_of(ed, &id_s));
    let fmt = move || crypto::format(&spec.get().format).unwrap_or(&FORMATS[0]);
    // Descifrar un JWE o verificar un JWS: alg/enc vienen en la cabecera del token.
    let from_token = move || spec.with(|s| matches!((s.format.as_str(), s.op.as_str()), ("jwe", "decrypt") | ("jws", "verify")));
    let rsa = move || {
        let s = spec.get();
        s.alg.starts_with("RSA") || s.alg.starts_with("RS") || s.alg.starts_with("PS") || s.format == "rsa-oaep"
    };
    // Si el flujo ya tiene la llave de prueba se reutiliza: generar otra rompería los demás nodos.
    let has_test_key = move || ed.flow.with(|f| f.vars.iter().any(|p| p.name == if rsa() { "privada_prueba" } else { "llave_prueba" }));
    let gen = {
        let id = id.clone();
        move |_| {
            let (id, s, rsa) = (id.clone(), spec.get_untracked(), untrack(rsa));
            let pick_for = move |s: &CryptoSpec| if !rsa { "llave_prueba" } else if wants_public(s) { "publica_prueba" } else { "privada_prueba" };
            if untrack(has_test_key) {
                let pick = pick_for(&s);
                set_spec(ed, &id, move |s| s.key = format!("{{{{{pick}}}}}"));
                return;
            }
            spawn_local(async move {
                match api::post::<Value>("/api/crypto/keygen", &json!({"kind": if rsa { "rsa" } else { "aes" }})).await {
                    Ok(v) => {
                        // Una línea con `\n`: así cabe en una variable y el servidor la vuelve PEM.
                        let one = |k: &str| v[k].as_str().unwrap_or("").trim().replace('\n', "\\n");
                        let mut add: Vec<(String, String)> =
                            if rsa { vec![("publica_prueba".into(), one("public")), ("privada_prueba".into(), one("private"))] } else { vec![("llave_prueba".into(), one("key"))] };
                        let pick = pick_for(&s);
                        ed.edit(|f| {
                            f.vars.retain(|p| !add.iter().any(|a| a.0 == p.name));
                            f.vars.extend(add.drain(..).map(|(k, v)| Pair::new(k, v)));
                        });
                        set_spec(ed, &id, move |s| s.key = format!("{{{{{pick}}}}}"));
                        app().toast(
                            "ok",
                            if rsa {
                                "Par RSA de prueba en las Variables del flujo: publica_prueba y privada_prueba"
                            } else {
                                "Llave AES de prueba en las Variables del flujo: llave_prueba"
                            },
                        );
                    }
                    Err(e) => app().err(e),
                }
            });
        }
    };
    let (i_op, i_fmt, i_alg, i_enc, i_out, i_kid, i_key, i_in, i_sig) =
        (id.clone(), id.clone(), id.clone(), id.clone(), id.clone(), id.clone(), id.clone(), id.clone(), id.clone());
    let key_ref = move || ed.flow.with(|f| f.node(&id).map(|n| format!("{{{{{}}}}}", n.key)).unwrap_or_default());
    view! {
        <h4>"Qué hace"</h4>
        <div class="seg">
            {move || fmt().ops.iter().map(|op| {
                let (op, i) = (*op, i_op.clone());
                view! { <button class:on=move || spec.get().op == op on:click=move |_| set_spec(ed, &i, move |s| {
                    s.op = op.into();
                    // Llave de prueba: la pública cifra/verifica, la privada descifra/firma.
                    s.key = if wants_public(s) { s.key.replace("{{privada_prueba}}", "{{publica_prueba}}") } else { s.key.replace("{{publica_prueba}}", "{{privada_prueba}}") };
                })>{crypto::op_label(op, &spec.get_untracked().format)}</button> }
            }).collect_view()}
        </div>
        <h4>"Formato"</h4>
        <div class="fmt-grid">
            {FORMATS.iter().map(|f| {
                let (fid, i) = (f.id, i_fmt.clone());
                view! {
                    <button class="fmt" class:on=move || spec.get().format == fid on:click=move |_| set_spec(ed, &i, move |s| s.switch_format(fid))>
                        <b>{f.name}</b><span class="muted">{f.hint}</span>
                    </button>
                }
            }).collect_view()}
        </div>
        {move || from_token().then(|| view! { <p class="muted">"El algoritmo se lee de la cabecera del token."</p> })}
        {move || (!fmt().algs.is_empty() && !from_token()).then(|| { let i = i_alg.clone(); view! {
            <label class="f">{if spec.get().format == "jwe" { "alg · cómo se envuelve la llave" } else { "Algoritmo" }}
                <select class="in" prop:value=move || spec.get().alg on:change=move |e| { let v = event_target_value(&e); set_spec(ed, &i, move |s| s.alg = v) }>
                    {fmt().algs.iter().map(|a| view! { <option value=*a>{*a}</option> }).collect_view()}
                </select>
            </label>
        }})}
        {move || (!fmt().encs.is_empty() && !from_token()).then(|| { let i = i_enc.clone(); view! {
            <label class="f">"enc · cómo se cifra el contenido"
                <select class="in" prop:value=move || spec.get().enc on:change=move |e| { let v = event_target_value(&e); set_spec(ed, &i, move |s| s.enc = v) }>
                    {fmt().encs.iter().map(|a| view! { <option value=*a>{*a}</option> }).collect_view()}
                </select>
            </label>
        }})}
        {move || (!fmt().outputs.is_empty()).then(|| { let i = i_out.clone(); view! {
            <label class="f">"Formato del texto cifrado"
                <select class="in" prop:value=move || spec.get().output on:change=move |e| { let v = event_target_value(&e); set_spec(ed, &i, move |s| s.output = v) }>
                    {fmt().outputs.iter().map(|a| view! { <option value=*a>{*a}</option> }).collect_view()}
                </select>
            </label>
        }})}
        {move || (matches!(spec.get().format.as_str(), "jwe" | "jws") && !from_token()).then(|| { let i = i_kid.clone(); view! {
            <label class="f">"kid (opcional)"
                <input class="in" data-noac placeholder="banco-2026" prop:value=move || spec.get().kid on:input=move |e| { let v = event_target_value(&e); set_spec(ed, &i, move |s| s.kid = v) } />
            </label>
        }})}
        {move || (!fmt().key.is_empty()).then(|| { let (i, g) = (i_key.clone(), gen.clone()); view! {
            <label class="f">"Llave"
                <VarField value=Signal::derive(move || spec.get().key) placeholder="{{publica_banco}}" on_input=Callback::new(move |v: String| set_spec(ed, &i, move |s| s.key = v)) />
            </label>
            <p class="muted">{fmt().key}". Guárdala como secreto del entorno (no va al archivo de la petición ni a git)."</p>
            <button class="btn sm" on:click=g>{move || if has_test_key() { "Usar la llave de prueba del flujo" } else { "Generar llave de prueba" }}</button>
        }})}
        <label class="f">{move || if spec.get().op == "encrypt" || spec.get().op == "sign" { "Qué procesa" } else { "Qué descifra o verifica" }}
            <VarField value=Signal::derive(move || spec.get().input) placeholder="{{armar_pago.response}}" on_input=Callback::new(move |v: String| set_spec(ed, &i_in, move |s| s.input = v)) />
        </label>
        {move || (spec.get().format == "hmac" && spec.get().op == "verify").then(|| { let i = i_sig.clone(); view! {
            <label class="f">"Firma recibida"
                <VarField value=Signal::derive(move || spec.get().signature) placeholder="{{webhook.headers.x-signature}}" on_input=Callback::new(move |v: String| set_spec(ed, &i, move |s| s.signature = v)) />
            </label>
        }})}
        <p class="muted">"El resultado queda en "<span class="mono">{key_ref}</span>". Si es JSON, también en "<span class="mono">"{{clave.campo}}"</span>"."</p>
    }
}

/// JWE/JWS compactos: cada parte con su color, para ver qué es qué.
#[component]
pub fn TokenParts(text: String) -> impl IntoView {
    let parts: Vec<String> = text.split('.').map(String::from).collect();
    let names: &[&str] = match parts.len() {
        5 => &["cabecera", "llave cifrada", "IV", "contenido", "etiqueta"],
        3 => &["cabecera", "contenido", "firma"],
        _ => return None,
    };
    let n = parts.len();
    Some(view! {
        <h4>"Partes del token"</h4>
        <div class="tok mono">
            {parts.into_iter().enumerate().map(|(i, p)| view! { <span class=format!("tk{i}") title=names[i]>{p}</span>{(i + 1 < n).then_some(".")} }).collect_view()}
        </div>
        <div class="row tok-legend">{names.iter().enumerate().map(|(i, l)| view! { <span><i class=format!("tk{i}")></i>{*l}</span> }).collect_view()}</div>
    })
}

// ---------- Equivalencia ----------

fn lookup_of(ed: Ed, id: &str) -> LookupSpec {
    ed.flow.with(|f| f.node(id).and_then(|n| n.lookup.clone())).unwrap_or_default()
}

fn set_lookup(ed: Ed, id: &str, f: impl FnOnce(&mut LookupSpec) + 'static) {
    ed.edit_node(id, move |n| f(n.lookup.get_or_insert_with(LookupSpec::default)));
}

/// Tabla ya cargada por nombre.
pub fn table(ed: Ed, name: &str) -> Option<Table> {
    ed.table_list.with(|l| l.iter().find(|t| t.0 == name).map(|t| t.1.clone()))
}

#[component]
pub fn LookupEditor(id: String) -> impl IntoView {
    let ed = ed();
    crate::tables::load(ed);
    let id_s = id.clone();
    let spec = Memo::new(move |_| lookup_of(ed, &id_s));
    let t = Memo::new(move |_| table(ed, &spec.get().table));
    let probe = RwSignal::new(String::new());
    let (i_t, i_v, i_c, i_m, i_d) = (id.clone(), id.clone(), id.clone(), id.clone(), id.clone());
    let key = Memo::new(move |_| ed.flow.with(|f| f.node(&id).map(|n| n.key.clone()).unwrap_or_default()));
    // Valor que se busca: lo que escribes en «Probar» o, si está vacío, el de la plantilla con los datos actuales.
    let current = move || {
        let p = probe.get();
        if !p.trim().is_empty() {
            return p;
        }
        let v = spec.get().value;
        app().flow_ctx.with(|f| f.as_ref().map(|(flow, outs)| {
            let vars = serde_json::Map::new();
            martino_core::flow::RefCtx { flow, outs, vars: &vars }.render(&v, &|_| None)
        })).unwrap_or(v)
    };
    let in_col = move || t.with(|t| t.as_ref().map(|t| t.cols.iter().position(|c| *c == spec.with(|s| s.column.clone())).unwrap_or(0)).unwrap_or(0));
    view! {
        <div class="eqv">
            <div class="eqv-s">
                <span>"Busca"</span>
                <span class="grow"><VarField value=Signal::derive(move || spec.get().value) placeholder="{{entrada.banco}}" on_input=Callback::new(move |v: String| set_lookup(ed, &i_v, move |s| s.value = v)) /></span>
            </div>
            <div class="eqv-s">
                <span>"en la tabla"</span>
                <select class="in grow" prop:value=move || spec.get().table on:change=move |e| { let v = event_target_value(&e); set_lookup(ed, &i_t, move |s| { s.table = v; s.column.clear() }) }>
                    <option value="">"— elige una tabla —"</option>
                    {move || ed.table_list.get().into_iter().map(|(n, tb)| { let on = spec.with(|s| s.table == n); view! { <option value=n.clone() selected=on>{format!("{n} · {} filas", tb.rows.len())}</option> } }).collect_view()}
                </select>
                <button class="btn sm" title="Crear, editar o importar tablas (CSV)" on:click=move |_| ed.pane.set(crate::flows::Pane::Tables)>"Editar tablas"</button>
            </div>
            {move || {
                let tb = t.get()?;
                let ic = in_col();
                let q = current();
                let qn = q.trim().to_lowercase();
                let hit = tb.rows.iter().position(|r| r.get(ic).is_some_and(|c| c.trim().to_lowercase() == qn));
                let shown = 8usize;
                let more = tb.rows.len().saturating_sub(shown);
                let i_c = i_c.clone();
                Some(view! {
                    <div class="eqv-t">
                        <table>
                            <thead><tr>
                                {tb.cols.iter().enumerate().map(|(i, c)| {
                                    let (name, i_c) = (if i == 0 { String::new() } else { c.clone() }, i_c.clone());
                                    view! {
                                        <th class=if i == ic { "in" } else { "out" } title=if i == ic { "Se busca en esta columna".to_string() } else { format!("Clic: buscar por «{c}» en vez de esta (al revés)") }
                                            on:click=move |_| { let n = name.clone(); set_lookup(ed, &i_c, move |s| s.column = n) }>
                                            <small>{if i == ic { "ENTRA" } else { "SALE" }}</small>{c.clone()}
                                        </th>
                                    }
                                }).collect_view()}
                            </tr></thead>
                            <tbody>
                                {tb.rows.iter().enumerate().filter(|(i, _)| *i < shown || Some(*i) == hit).map(|(i, r)| view! {
                                    <tr class:hit=Some(i) == hit>{r.iter().map(|c| view! { <td>{c.clone()}</td> }).collect_view()}</tr>
                                }).collect_view()}
                            </tbody>
                        </table>
                        {(more > 0).then(|| view! { <div class="muted eqv-more">{format!("+ {more} filas más (en Editar tablas)")}</div> })}
                    </div>
                })
            }}
            {move || t.get().map(|tb| { let ic = in_col(); view! {
                <div class="eqv-out"><span class="muted">"Sale como"</span>
                    {tb.cols.iter().enumerate().filter(|(i, _)| *i != ic).map(|(_, c)| { let r = format!("{{{{{}.{c}}}}}", key.get()); let r2 = r.clone(); view! {
                        <button class="eqv-chip" title="Clic copia para usarlo en otro paso" on:click=move |_| crate::ui::copy(&r2)>{r}</button>
                    }}).collect_view()}
                </div>
            }})}
            <div class="eqv-s">
                <span>"Si no está"</span>
                <select class="in grow" prop:value=move || spec.get().on_miss on:change=move |e| { let v = event_target_value(&e); set_lookup(ed, &i_m, move |s| s.on_miss = v) }>
                    <option value="">"Detener con error"</option>
                    <option value="keep">"Dejar el valor igual"</option>
                    <option value="default">"Usar un valor fijo"</option>
                </select>
                {move || (spec.get().on_miss == "default").then(|| { let i = i_d.clone(); view! {
                    <input class="in eqv-def" data-noac placeholder="valor fijo" prop:value=move || spec.get().default on:input=move |e| { let v = event_target_value(&e); set_lookup(ed, &i, move |s| s.default = v) } />
                }})}
            </div>
            <div class="eqv-probe">
                <input class="in" data-noac placeholder="Probar otro valor (p. ej. 003)" prop:value=move || probe.get() on:input=move |e| probe.set(event_target_value(&e)) />
                {move || {
                    let tb = t.get()?;
                    let q = current();
                    if q.trim().is_empty() || q.contains("{{") { return None; }
                    let sp = spec.get();
                    let ic = in_col();
                    // No está, pero la regla «si no está» deja seguir: aviso, no éxito.
                    if tb.find(&sp.column, &q).is_none() && tb.lookup(&sp, &q, &sp.table).is_ok() {
                        let what = if sp.on_miss == "keep" { "deja el valor igual".to_string() } else { format!("usa el valor fijo «{}»", sp.default) };
                        return Some(view! { <div class="eqv-res miss">{format!("{q} no está en la tabla → {what}")}</div> }.into_any());
                    }
                    Some(match tb.lookup(&sp, &q, &sp.table) {
                        Ok(v) => {
                            let outs = tb.cols.iter().enumerate().filter(|(i, _)| *i != ic).map(|(_, c)| v[c.as_str()].as_str().unwrap_or("").to_string()).collect::<Vec<_>>().join(" · ");
                            view! { <div class="eqv-res ok">{format!("{q} → {outs}")}</div> }.into_any()
                        }
                        Err(e) => view! { <div class="eqv-res err">{e}</div> }.into_any(),
                    })
                }}
            </div>
            <p class="muted eqv-note">"Busca sin distinguir mayúsculas ni espacios. Clic en un encabezado para buscar por otra columna."</p>
            {move || t.get().map(|tb| {
                let name = spec.get().table;
                let (c0, c1) = (tb.cols.first().cloned().unwrap_or_default(), tb.cols.get(1).cloned().unwrap_or_default());
                let v = spec.get().value.trim().trim_start_matches("{{").trim_end_matches("}}").to_string();
                let col = spec.get().column;
                let a = if col.is_empty() || col == c0 { format!("{{{{tabla.{name}[{v}].{c1}}}}}") } else { format!("{{{{tabla.{name}[{col}={v}].{c0}}}}}") };
                let b = format!("{{{{tabla.{name}[{c1}=…].{c0}}}}}");
                let (a2, b2) = (a.clone(), b.clone());
                view! {
                    <div class="eqv-inline"><span class="muted">"Sin este paso, directo en cualquier campo:"</span>
                        <button class="eqv-chip" title="Clic copia" on:click=move |_| crate::ui::copy(&a2)>{a}</button>
                        <button class="eqv-chip" title="Al revés · clic copia" on:click=move |_| crate::ui::copy(&b2)>{b}</button>
                    </div>
                }
            })}
        </div>
    }
}

/// En el lienzo, una Condición con varias reglas las muestra todas (con ✓/✗ tras ejecutar).
#[component]
pub fn RuleList(id: String) -> impl IntoView {
    let ed = ed();
    move || {
        let n = ed.flow.with(|f| f.node(&id).cloned())?;
        let join = if n.mode == "any" { "O" } else { "Y" };
        Some(view! {
            <div class="fn-rules">
                {n.rules.iter().enumerate().map(|(i, r)| {
                    let ev = crate::flow_inspector::rule_eval(r);
                    let cls = match ev { Some((_, true)) => "ok", Some((_, false)) => "ko", None => "" };
                    view! {
                        <div class=format!("fn-rule {cls}") title=format!("{} {} {}", r.left, crate::flow_inspector::op_label(&r.op), r.right)>
                            <span class="fn-rj">{if i == 0 { "" } else { join }}</span>
                            <span class="fn-rt">{format!("{} {} {}", r.left, crate::flow_inspector::op_short(&r.op), r.right)}</span>
                            <span>{match ev { Some((_, true)) => "✓", Some((_, false)) => "✗", None => "" }}</span>
                        </div>
                    }
                }).collect_view()}
            </div>
        })
    }
}
