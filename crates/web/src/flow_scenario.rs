//! Vista Escenario (mockups B + D): el camino de un caso en línea recta, un carril por sistema, los
//! valores que viajan escritos sobre la flecha y las salidas no tomadas en un riel abajo, sin cruces.
use crate::flow_story::{brief, sentence};
use crate::flows::{ed, label, node_code};
use leptos::prelude::*;
use martino_core::flow::Node;
use serde_json::Value;
use std::collections::HashMap;

const LABEL_W: f64 = 120.0;
const CARD_W: f64 = 210.0;
const CARD_H: f64 = 124.0;
const COL_W: f64 = 330.0;
const ROW_H: f64 = 150.0;
const EXIT_H: f64 = 26.0;

fn name(n: &Node) -> String {
    if n.name.is_empty() { label(&n.kind).to_string() } else { n.name.clone() }
}

/// Lo que el paso `a` deja y usa alguien más adelante en el camino (va escrito sobre su flecha).
fn travels(used: &HashMap<String, Vec<martino_core::usage::Used>>, a: &Node, later: &[&Node]) -> String {
    let later: Vec<String> = later.iter().map(|n| name(n)).collect();
    let mut paths: Vec<String> = vec![];
    for u in used.get(&a.key).into_iter().flatten().filter(|u| later.contains(&u.by)) {
        let p = u.path.rsplit('.').next().unwrap_or(&u.path).to_string();
        if !paths.contains(&p) {
            paths.push(p);
        }
    }
    match paths.len() {
        0..=2 => paths.join(", "),
        n => format!("{}, {} +{}", paths[0], paths[1], n - 2),
    }
}

#[component]
pub fn ScenarioView() -> impl IntoView {
    let ed = ed();
    // "" = camino principal; si no, el id del caso QA.
    let pick = RwSignal::new(String::new());
    let chips = move || {
        let res = ed.qa_res.get();
        let main = view! { <button class="chip qa-f" class:on=move || pick.get().is_empty() on:click=move |_| pick.set(String::new())>"Camino principal"</button> };
        let cases = ed.flow.with(|f| f.cases.clone()).into_iter().map(|c| {
            let dot = match res.get(&c.id) { Some(r) if r["ok"] == Value::Bool(true) => "ok", Some(_) => "ko", None => "" };
            let (id, id2) = (c.id.clone(), c.id.clone());
            view! {
                <button class="chip qa-f" class:on=move || pick.get() == id on:click=move |_| pick.set(id2.clone()) title=c.note.clone()>
                    <span class=format!("sdot {dot}")></span>{format!("{} · {}", c.id, c.name)}
                </button>
            }
        }).collect_view();
        view! { {main}{cases} }
    };
    let body = move || {
        let f = ed.flow.get();
        let used = ed.used.get();
        let case = f.cases.iter().find(|c| c.id == pick.get()).cloned();
        let res = case.as_ref().and_then(|c| ed.qa_res.with(|r| r.get(&c.id).cloned()));
        // Pasos que dejó la corrida del caso (en orden), o los de la última ejecución si es el camino principal.
        let steps: HashMap<String, Value> = match &res {
            Some(r) => r["steps"].as_array().into_iter().flatten().map(|s| (s["id"].as_str().unwrap_or("").to_string(), s.clone())).collect(),
            None if case.is_none() => ed.steps.get(),
            None => HashMap::new(),
        };
        let mut path: Vec<String> = match &res {
            Some(r) => r["steps"].as_array().into_iter().flatten().filter_map(|s| s["id"].as_str().map(String::from)).collect(),
            None => martino_core::layout::main_path(&f),
        };
        let mut seen = std::collections::HashSet::new();
        path.retain(|id| seen.insert(id.clone()) && f.node(id).is_some_and(|n| n.kind != "start" && !martino_core::flow::is_annotation(&n.kind)));
        let nodes: Vec<&Node> = path.iter().filter_map(|id| f.node(id)).collect();
        let mut lanes = vec!["Martino".to_string()];
        for n in &nodes {
            let l = martino_core::layout::system_lane(n);
            if l != "Respuesta" && !lanes.contains(&l) {
                lanes.push(l);
            }
        }
        lanes.push("Respuesta".into());
        let row = |n: &Node| lanes.iter().position(|l| *l == martino_core::layout::system_lane(n)).unwrap_or(0) as f64;
        let x = |i: usize| LABEL_W + 20.0 + i as f64 * COL_W;
        let y = |n: &Node| row(n) * ROW_H + 13.0;
        // Salidas no tomadas: conexiones que dejan el camino, debajo de su paso.
        let exits: Vec<Vec<(String, String, String)>> = nodes.iter().map(|n| {
            f.edges.iter().filter(|e| e.from == n.id && !path.contains(&e.to)).filter_map(|e| {
                let t = f.node(&e.to)?;
                let why = match e.port.as_str() { "error" => "✕ Si falla", "false" => "✕ Si no", _ => "→" };
                Some((t.id.clone(), format!("{why} → {} {}", node_code(&f, t), name(t)), e.port.clone()))
            }).collect()
        }).collect();
        let rail_y = lanes.len() as f64 * ROW_H + 10.0;
        let rail_h = exits.iter().map(Vec::len).max().unwrap_or(0) as f64 * EXIT_H + 34.0;
        let (w, h) = (x(nodes.len()) + 20.0, rail_y + rail_h);
        let mut lines = vec![];
        let mut tags = vec![];
        for (i, pair) in nodes.windows(2).enumerate() {
            let (a, b) = (pair[0], pair[1]);
            let (x1, y1, x2, y2) = (x(i) + CARD_W, y(a) + CARD_H / 2.0, x(i + 1), y(b) + CARD_H / 2.0);
            let mx = x1 + (x2 - x1) / 2.0;
            lines.push(view! { <path class="se" d=format!("M{x1} {y1} H{mx} V{y2} H{}", x2 - 4.0) marker-end="url(#sarr)" /> }.into_any());
            let t = travels(&used, a, &nodes[i + 1..]);
            if !t.is_empty() {
                tags.push(view! { <div class="stag" style=format!("left:{}px;top:{}px;width:{}px", x1 + 6.0, y1 - 20.0, COL_W - CARD_W - 12.0) title=t.clone()>{t.clone()}</div> }.into_any());
            }
        }
        for (i, (n, ex)) in nodes.iter().zip(&exits).enumerate().filter(|(_, (_, ex))| !ex.is_empty()) {
            let cx = x(i) + CARD_W / 2.0;
            lines.push(view! { <path class="sx" d=format!("M{cx} {} V{}", y(n) + CARD_H, rail_y + 28.0) /> }.into_any());
            for (j, (to, text, port)) in ex.iter().cloned().enumerate() {
                tags.push(view! {
                    <button class=format!("sexit p-{port}") style=format!("left:{}px;top:{}px;width:{CARD_W}px", x(i), rail_y + 28.0 + j as f64 * EXIT_H) title=text.clone()
                        on:click=move |_| { ed.sel.set(Some(to.clone())); ed.insp.set(true) }>{text.clone()}</button>
                }.into_any());
            }
        }
        let cards = nodes.iter().enumerate().map(|(i, n)| {
            let st = steps.get(&n.id).cloned();
            let state = match &st { Some(s) if s["ok"] == Value::Bool(true) => "done", Some(_) => "fail", None => "" };
            let (id, id_sel) = (n.id.clone(), n.id.clone());
            view! {
                <div class=format!("scard card k-{} {state}", n.kind) class:sel=move || ed.sel.get().as_deref() == Some(id_sel.as_str())
                    style=format!("left:{}px;top:{}px;width:{CARD_W}px;height:{CARD_H}px", x(i), y(n)) title=n.desc.clone()
                    on:click=move |_| { ed.sel.set(Some(id.clone())); ed.insp.set(true) }>
                    <div class="hd"><span class="snum">{i + 1}</span><span class="fn-id">{node_code(&f, n)}</span><span class=format!("kshape k-{}", n.kind)></span><span class="muted">{label(&n.kind)}</span></div>
                    <div class="nm">{name(n)}</div>
                    <div class="say">{if n.desc.is_empty() { sentence(&f, n) } else { n.desc.clone() }}</div>
                    {st.map(|s| view! { <div class="res">{format!("→ {}", brief(&s))}</div> })}
                </div>
            }
        }).collect_view();
        let lane_bands = lanes.iter().enumerate().map(|(i, l)| view! {
            <div class="slane" style=format!("top:{}px;height:{ROW_H}px", i as f64 * ROW_H)><span>{l.clone()}</span></div>
        }).collect_view();
        let head = match (&case, &res) {
            (None, _) => "Camino principal: desde el Inicio por las salidas buenas. Elige un caso para ver el camino que siguió de verdad.".to_string(),
            (Some(_), None) => "Este caso aún no se ejecuta: abre «☰ Casos» y ejecútalo para ver su camino. Mientras, se muestra el camino principal.".to_string(),
            (Some(c), Some(r)) => {
                let expect: Vec<String> = c.expect_status.map(|s| format!("status = {s}")).into_iter()
                    .chain(c.expect.iter().map(|e| format!("{} {} {}", e.left, crate::flow_inspector::op_short(&e.op), e.right).trim().to_string())).collect();
                let failed = r["checks"].as_array().into_iter().flatten().filter(|k| k["ok"] != Value::Bool(true)).count();
                format!("{} · respondió {} en {} ms · espera {}{}", r["verdict"].as_str().unwrap_or("?"), r["status"].as_u64().map(|s| s.to_string()).unwrap_or("—".into()), r["ms"],
                    if expect.is_empty() { "—".into() } else { expect.join(" · ") }, if failed > 0 { format!(" · {failed} no se cumplen") } else { String::new() })
            }
        };
        let verdict = res.as_ref().map(|r| if r["ok"] == Value::Bool(true) { "ok" } else { "ko" }).unwrap_or("");
        view! {
            <p class=format!("shead {verdict}")>{head}</p>
            <div class="sboard">
                <div class="scanvas" style=format!("width:{w}px;height:{h}px")>
                    {lane_bands}
                    <div class="srail" style=format!("top:{rail_y}px;height:{rail_h}px")><span>"Salidas"</span></div>
                    <svg class="slines" width=w height=h>
                        <defs><marker id="sarr" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto"><path d="M0 0 L8 4 L0 8 z" /></marker></defs>
                        {lines}
                    </svg>
                    {cards}
                    {tags}
                </div>
            </div>
        }
    };
    view! {
        <div class="fscen">
            <div class="qa-head">{chips}</div>
            {body}
        </div>
    }
}
