//! Mix B: frase de cada paso, carriles por etapa en el lienzo, narrador de la ejecución y la vista
//! Historia (el mismo flujo como pasos numerados, agrupados por etapa).
use crate::flows::{ed, label, node_code, Pane};
use leptos::prelude::*;
use martino_core::flow::{Flow, Node};
use martino_core::layout::{reading_order, stage, STAGES};
use serde_json::Value;

/// Ancho de cada carril de etapa (lo usa también «Ordenar»).
pub const LANE_W: f64 = crate::flows::W + 60.0;

fn quoted(v: &str) -> String {
    let t = v.trim().trim_start_matches("{{").trim_end_matches("}}");
    if t.is_empty() { "…".into() } else { t.into() }
}

/// Qué hace el paso, en una frase («Busca entrada.banco en la tabla bancos»).
pub fn sentence(f: &Flow, n: &Node) -> String {
    match n.kind.as_str() {
        "start" => "Arranca el flujo.".into(),
        "input" => match n.data.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>().join(", ")) {
            Some(k) if !k.is_empty() => format!("Recibe {k}."),
            _ => "Recibe los datos de entrada.".into(),
        },
        "data" => "Valores fijos para reusar.".into(),
        "lookup" => match &n.lookup {
            Some(l) if !l.table.is_empty() => format!("Busca {} en la tabla {}.", quoted(&l.value), l.table),
            _ => "Elige una tabla de equivalencia.".into(),
        },
        "condition" => {
            let join = if n.mode == "any" { " o " } else { " y " };
            let rules: Vec<String> = n.rules.iter().filter(|r| !r.left.is_empty()).map(|r| format!("{} {} {}", r.left, crate::flow_inspector::op_short(&r.op), r.right).trim().to_string()).collect();
            if rules.is_empty() { "Define la regla.".into() } else { format!("Sigue si {}.", rules.join(join)) }
        }
        "extract" => format!("Guarda {}.", n.vars.iter().map(|p| p.name.clone()).collect::<Vec<_>>().join(", ")),
        "transform" => "Calcula con JavaScript.".into(),
        "crypto" => n.crypto.as_ref().map(|c| format!("{} ({}).", martino_core::crypto::op_label(&c.op, &c.format), martino_core::crypto::format(&c.format).map(|f| f.name).unwrap_or(""))).unwrap_or_else(|| "Elige qué cifrar.".into()),
        "request" => {
            if n.req_ref.is_empty() {
                return match &n.request {
                    Some(r) => format!("Llama a {} {} (desde cURL).", r.method.to_uppercase(), r.url.split('?').next().unwrap_or(&r.url).trim_start_matches("https://").trim_start_matches("http://")),
                    None => "Llama a una petición propia.".into(),
                };
            }
            let code = node_code(f, n);
            let file = n.req_ref.rsplit('/').next().unwrap_or(&n.req_ref);
            let name = martino_core::yaml_req::strip_ext(file).unwrap_or(file).replace('-', " ");
            format!("Llama a «{name}» ({code}).")
        }
        "response" => format!("Responde {}.", if n.status.is_empty() { "200" } else { &n.status }),
        "end" => "Termina sin responder.".into(),
        "note" => n.text.clone(),
        _ => n.desc.clone(),
    }
}

/// Lo que dejó un paso al ejecutarse, corto: «200 · 184 ms», «falló: …».
fn brief(st: &Value) -> String {
    let ms = st["ms"].as_u64().unwrap_or(0);
    if st["ok"] != Value::Bool(true) {
        return format!("falló: {}", st["error"].as_str().unwrap_or("error"));
    }
    match st["out"]["status"].as_u64() {
        Some(s) => format!("{s} · {ms} ms"),
        None => format!("{ms} ms"),
    }
}

/// ¿Cada paso está en el carril de su etapa? (si no, los carriles confundirían: se pide «Ordenar»)
pub fn in_lanes(f: &Flow) -> bool {
    f.nodes.iter().filter(|n| !martino_core::flow::is_annotation(&n.kind)).all(|n| ((n.x + crate::flows::W / 2.0) / LANE_W).floor() as i64 == stage(&n.kind) as i64)
}

/// Carriles de fondo del lienzo, uno por etapa; sólo cuando el flujo está ordenado por etapas.
#[component]
pub fn Lanes() -> impl IntoView {
    let ed = ed();
    move || ed.flow.with(in_lanes).then(|| view! {
        <div class="flanes" aria-hidden="true">
            {STAGES.iter().enumerate().map(|(i, s)| view! {
                <div class="flane" style=format!("left:{}px;width:{}px", i as f64 * LANE_W, LANE_W)><span>{format!("{:02} {s}", i + 1)}</span></div>
            }).collect_view()}
        </div>
    })
}

/// El gato cuenta qué pasó: mientras corre, el último paso; al terminar, el resultado.
#[component]
pub fn Guide() -> impl IntoView {
    let ed = ed();
    move || {
        let running = ed.running.get();
        let n = ed.steps.with(|s| s.len());
        let text = if let Some(r) = ed.result.get().filter(|_| !running) {
            match (r["status"].as_u64(), r["error"].as_str()) {
                (_, Some(e)) => format!("Se detuvo en el paso {n}: {e}"),
                (Some(s), _) => format!("Listo: respondió {s} en {n} pasos."),
                _ => format!("Terminó en {n} pasos sin respuesta."),
            }
        } else if running {
            let last = ed.last_step.get().and_then(|id| ed.flow.with(|f| f.node(&id).map(|nd| (nd.name.clone(), node_code(f, nd)))));
            let st = ed.last_step.get().and_then(|id| ed.steps.with(|s| s.get(&id).cloned()));
            match (last, st) {
                (Some((name, code)), Some(st)) => format!("Paso {n} · {code} {name}: {}", brief(&st)),
                _ => "Arrancando…".into(),
            }
        } else {
            return None;
        };
        Some(view! {
            <div class="fguide" on:pointerdown=|e| e.stop_propagation()>
                <crate::ui::Cat /><p>{text}</p>
            </div>
        })
    }
}

/// Vista Historia: los pasos numerados, agrupados por etapa, con sus salidas de error al costado.
#[component]
pub fn StoryView() -> impl IntoView {
    let ed = ed();
    move || {
        let f = ed.flow.get();
        let order = reading_order(&f);
        let steps = ed.steps.get();
        let mut last_stage = usize::MAX;
        let total = order.len();
        let items = order.iter().enumerate().filter_map(|(i, id)| {
            let n = f.node(id)?.clone();
            let st = stage(&n.kind);
            let head = (st != last_stage).then(|| { last_stage = st; view! { <div class="sh">{format!("{:02} {}", st + 1, STAGES[st])}</div> } });
            let step = steps.get(&n.id).cloned();
            let state = match &step { Some(s) if s["ok"] == Value::Bool(true) => "done", Some(_) => "fail", None if ed.result.with(Option::is_some) && n.kind != "input" => "skip", None => "" };
            let branch = f.edges.iter().find(|e| e.from == n.id && (e.port == "false" || e.port == "error")).and_then(|e| {
                let t = f.node(&e.to)?;
                Some(format!("{} {} · {}", if e.port == "error" { "Si falla:" } else { "Si no:" }, node_code(&f, t), t.name))
            });
            let (id_sel, kind) = (n.id.clone(), n.kind.clone());
            Some(view! {
                {head}
                <div class=format!("chap {state}{}", if i + 1 == total { " last" } else { "" }) on:click=move |_| { ed.sel.set(Some(id_sel.clone())) }>
                    <div class="num">{i + 1}</div>
                    <div class=format!("card k-{kind}") class:sel=move || ed.sel.get().as_deref() == Some(n.id.as_str())>
                        <div class="hd"><span class="fn-id">{node_code(&f, &n)}</span><span class=format!("kshape k-{kind}")></span><span class="muted">{label(&kind)}</span></div>
                        <div class="nm">{if n.name.is_empty() { label(&n.kind).to_string() } else { n.name.clone() }}</div>
                        <div class="say">{sentence(&f, &n)}</div>
                        {step.map(|s| view! { <div class="res">{format!("→ {}", brief(&s))}</div> })}
                    </div>
                    {branch.map(|b| view! { <div class="branch">{b}</div> })}
                </div>
            })
        }).collect::<Vec<_>>();
        view! {
            <div class="fstory">
                {if total == 0 { Some(view! { <p class="muted">"Este flujo aún no tiene pasos. Añádelos con «＋ Añadir paso»."</p> }) } else { None }}
                {items}
                <button class="btn ghost sm" on:click=move |_| ed.pane.set(Pane::Canvas)>"Ver en el lienzo"</button>
            </div>
        }
    }
}
