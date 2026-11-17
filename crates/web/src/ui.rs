//! Componentes compartidos: VarField (resaltado por origen), tooltip, autocompletado,
//! tabla clave/valor, árbol JSON, arrastre de bordes y modal.
use crate::state::{app, Modal, Tip};
use leptos::prelude::*;
use martino_core::flow::{is_annotation, RefCtx};
use martino_core::interp::{leaf_paths, segments, value_to_text, Seg, DYNAMIC_VARS};
use martino_core::model::Pair;
use martino_core::vars::{classify, VarSource};
use serde_json::{Map, Value};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement, HtmlInputElement, HtmlTextAreaElement};

pub fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

pub fn copy(text: &str) {
    let _ = web_sys::window().unwrap().navigator().clipboard().write_text(text);
    app().toast("ok", format!("Copiado: {}", if text.len() > 60 { &text[..60] } else { text }));
}

/// Claves de los nodos del flujo abierto (para colorear `{{clave.x}}`).
fn flow_keys() -> Vec<String> {
    // Claves de nodos y Variables del flujo: las dos se resuelven dentro del flujo.
    app().flow_ctx.with(|f| f.as_ref().map(|(f, _)| f.keys().into_iter().chain(f.vars.iter().filter(|p| p.enabled).map(|p| p.name.clone())).collect()).unwrap_or_default())
}

pub fn var_source(name: &str) -> VarSource {
    let a = app();
    a.vars.with(|t| classify(name, t, &flow_keys()))
}

/// Valor que se mostraría para `name` (entorno o resultado del flujo).
pub fn var_value(name: &str) -> Option<(String, VarSource, Vec<VarSource>)> {
    let a = app();
    let src = var_source(name);
    if src == VarSource::Flow {
        return a.flow_ctx.with(|f| {
            let (flow, outs) = f.as_ref()?;
            let vars = Map::new();
            let v = RefCtx { flow, outs, vars: &vars }.lookup(name).map(|v| value_to_text(&v)).or_else(|| flow.vars.iter().find(|p| p.name == name).map(|p| p.value.clone()));
            Some((v.unwrap_or_else(|| "(sin ejecutar todavía)".into()), src, vec![]))
        });
    }
    let bare = name.strip_prefix("process.env.").unwrap_or(name);
    a.vars.with(|t| t.iter().find(|v| v.name == bare).map(|v| (v.value.clone(), v.source, v.shadowed.clone())))
}

/// Input o textarea con los `{{…}}` pintados según su origen (capa `.hl-back` detrás).
#[component]
pub fn VarField(
    #[prop(into)] value: Signal<String>,
    #[prop(into)] on_input: Callback<String>,
    #[prop(optional)] multi: bool,
    #[prop(optional, into)] placeholder: String,
    #[prop(optional)] rows: Option<u32>,
    #[prop(optional, into)] on_paste: Option<Callback<web_sys::ClipboardEvent>>,
    #[prop(optional, into)] on_keydown: Option<Callback<web_sys::KeyboardEvent>>,
) -> impl IntoView {
    let a = app();
    let back = NodeRef::<leptos::html::Div>::new();
    let segs = move || {
        let text = value.get();
        let _ = a.vars.get();
        let _ = a.flow_ctx.with(|f| f.is_some());
        let mut out: Vec<AnyView> = segments(&text)
            .into_iter()
            .map(|s| match s {
                Seg::Text(t) => t.to_string().into_any(),
                Seg::Var(n, raw) => view! { <span class=format!("v {}", var_source(n).css())>{raw.to_string()}</span> }.into_any(),
            })
            .collect();
        out.push(" ".into_any());
        out
    };
    let sync = move |ev: web_sys::Event| {
        let t: HtmlElement = event_target(&ev);
        if let Some(b) = back.get() {
            b.set_scroll_left(t.scroll_left());
            b.set_scroll_top(t.scroll_top());
        }
    };
    let hover = move |ev: web_sys::MouseEvent| {
        let Some(b) = back.get() else { return };
        let (x, y) = (ev.client_x() as f64, ev.client_y() as f64);
        let list = b.query_selector_all(".v").unwrap();
        for i in 0..list.length() {
            let el: Element = list.item(i).unwrap().unchecked_into();
            let rects = el.get_client_rects();
            for j in 0..rects.length() {
                let r = rects.item(j).unwrap();
                if x >= r.left() && x <= r.right() && y >= r.top() && y <= r.bottom() {
                    let name = el.text_content().unwrap_or_default();
                    let name = name.trim_start_matches('{').trim_end_matches('}').trim().to_string();
                    a.tip.set(Some(Tip { x: r.left(), y: r.bottom() + 6.0, name }));
                    return;
                }
            }
        }
        if a.tip.get_untracked().is_some() {
            a.tip.set(None);
        }
    };
    let click = move |ev: web_sys::MouseEvent| {
        if !(ev.ctrl_key() || ev.meta_key()) {
            return;
        }
        if let Some(t) = a.tip.get_untracked() {
            ev.prevent_default();
            a.tip.set(None);
            if var_source(&t.name) != VarSource::Flow && !t.name.starts_with('$') {
                a.modal.set(Some(Modal::Env(Some(t.name))));
            }
        }
    };
    let input = move |ev: web_sys::Event| on_input.run(event_target_value(&ev));
    let paste = move |ev: web_sys::ClipboardEvent| {
        if let Some(cb) = on_paste {
            cb.run(ev)
        }
    };
    let keydown = move |ev: web_sys::KeyboardEvent| {
        if let Some(cb) = on_keydown {
            cb.run(ev)
        }
    };
    let field = if multi {
        view! {
            <textarea class="in" rows=rows.unwrap_or(8) spellcheck="false" placeholder=placeholder
                prop:value=move || value.get() on:input=input on:scroll=sync on:mousemove=hover
                on:mouseleave=move |_| a.tip.set(None) on:mousedown=click on:paste=paste on:keydown=keydown></textarea>
        }
        .into_any()
    } else {
        view! {
            <input class="in" spellcheck="false" placeholder=placeholder
                prop:value=move || value.get() on:input=input on:scroll=sync on:mousemove=hover
                on:mouseleave=move |_| a.tip.set(None) on:mousedown=click on:paste=paste on:keydown=keydown />
        }
        .into_any()
    };
    view! {
        <div class="vf" class:multi=multi>
            <div class="hl-back" node_ref=back aria-hidden="true">{segs}</div>
            {field}
        </div>
    }
}

/// Tooltip del `{{variable}}` bajo el cursor.
#[component]
pub fn VarTip() -> impl IntoView {
    let a = app();
    move || {
        a.tip.get().map(|t| {
            let info = var_value(&t.name);
            let src = var_source(&t.name);
            let secret = src == VarSource::Secret;
            let val = match &info {
                Some((v, _, _)) if secret && !a.reveal.get() => "•".repeat(v.chars().count().clamp(6, 16)),
                Some((v, _, _)) => v.clone(),
                None if src == VarSource::Dynamic => "se genera al enviar".into(),
                None => format!("no existe en «{}»", a.env.get()),
            };
            let shadowed = info.map(|i| i.2).unwrap_or_default();
            view! {
                <div class="tip" style=format!("left:{}px;top:{}px", t.x, t.y)>
                    <div class="row"><b class="mono">{t.name.clone()}</b><span class=format!("chip {}", src.css())>{src.label()}</span></div>
                    <div class="val">{val}</div>
                    {(!shadowed.is_empty()).then(|| view! { <div class="hint">{format!("Tapa a: {}", shadowed.iter().map(|s| s.label()).collect::<Vec<_>>().join(", "))}</div> })}
                    {(src != VarSource::Flow && src != VarSource::Dynamic).then(|| view! { <div class="hint">"Ctrl+clic para editar"</div> })}
                </div>
            }
        })
    }
}

/// Leyenda de colores (también explica la precedencia).
#[component]
pub fn VarLegend() -> impl IntoView {
    let items = [VarSource::Runtime, VarSource::Request, VarSource::Env, VarSource::Secret, VarSource::Collection, VarSource::Dotenv, VarSource::Dynamic, VarSource::Flow, VarSource::Missing];
    view! { <div class="legend">{items.into_iter().map(|s| view! { <span class=s.css()>{s.label()}</span> }).collect_view()}</div> }
}

// ---------- autocompletado ----------

#[derive(Clone, PartialEq)]
struct Ac {
    x: f64,
    y: f64,
    items: Vec<(String, String, &'static str)>,
    sel: Option<usize>,
    start: usize,
    caret: usize,
    double: bool,
}

thread_local! {
    static AC_TARGET: RefCell<Option<Element>> = const { RefCell::new(None) };
}

fn field_value(el: &Element) -> Option<(String, usize)> {
    if let Some(i) = el.dyn_ref::<HtmlInputElement>() {
        let t = i.type_();
        if !matches!(t.as_str(), "text" | "search" | "url" | "") {
            return None;
        }
        return Some((i.value(), i.selection_start().ok().flatten().unwrap_or(0) as usize));
    }
    el.dyn_ref::<HtmlTextAreaElement>().map(|t| (t.value(), t.selection_start().ok().flatten().unwrap_or(0) as usize))
}

fn set_field(el: &Element, v: &str, caret: usize) {
    if let Some(i) = el.dyn_ref::<HtmlInputElement>() {
        i.set_value(v);
        let _ = i.set_selection_range(caret as u32, caret as u32);
    } else if let Some(t) = el.dyn_ref::<HtmlTextAreaElement>() {
        t.set_value(v);
        let _ = t.set_selection_range(caret as u32, caret as u32);
    }
    let ev = web_sys::Event::new_with_event_init_dict("input", &{
        let i = web_sys::EventInit::new();
        i.set_bubbles(true);
        i
    })
    .unwrap();
    let _ = el.dispatch_event(&ev);
}

/// Todo lo que se puede insertar: variables, dinámicas y referencias del flujo.
fn candidates() -> Vec<(String, String, &'static str)> {
    let a = app();
    let mut out: Vec<(String, String, &'static str)> = a.vars.with(|t| t.iter().map(|v| (v.name.clone(), if v.source == VarSource::Secret { "••••••".into() } else { v.value.clone() }, v.source.css())).collect());
    out.extend(DYNAMIC_VARS.iter().map(|d| (d.to_string(), "dinámica".into(), "v-dynamic")));
    a.flow_ctx.with(|f| {
        if let Some((flow, outs)) = f {
            for n in flow.nodes.iter().filter(|n| !is_annotation(&n.kind) && !matches!(n.kind.as_str(), "start" | "end")) {
                let base: Value = match (n.kind.as_str(), outs.get(&n.key)) {
                    ("request", Some(o)) => serde_json::json!({"input": o.input, "response": o.response, "status": o.status}),
                    ("request", None) => serde_json::json!({"response": {}, "status": 200}),
                    ("input" | "data", None) => n.data.clone(),
                    (_, Some(o)) => o.value.clone(),
                    _ => Value::Null,
                };
                out.push((n.key.clone(), n.name.clone(), "v-flow"));
                for (p, v) in leaf_paths(&base).into_iter().take(60) {
                    if !p.is_empty() {
                        out.push((format!("{}.{p}", n.key), value_to_text(&v), "v-flow"));
                    }
                }
            }
        }
    });
    out
}

/// Escucha en fase de captura en `document`: funciona en cualquier input/textarea (salvo `data-noac`).
#[component]
pub fn Autocomplete() -> impl IntoView {
    let st: RwSignal<Option<Ac>> = RwSignal::new(None);
    let insert = move |name: String| {
        let Some(s) = st.get_untracked() else { return };
        st.set(None);
        AC_TARGET.with(|t| {
            if let Some(el) = t.borrow().as_ref() {
                let Some((v, _)) = field_value(el) else { return };
                let u: Vec<u16> = v.encode_utf16().collect();
                let mut after = String::from_utf16_lossy(&u[s.caret.min(u.len())..]);
                if after.starts_with("}}") {
                    after = after[2..].to_string();
                }
                let before = String::from_utf16_lossy(&u[..s.start.min(u.len())]);
                let ins = format!("{{{{{name}}}}}");
                let caret = before.encode_utf16().count() + ins.encode_utf16().count();
                set_field(el, &format!("{before}{ins}{after}"), caret);
                if let Some(h) = el.dyn_ref::<HtmlElement>() {
                    let _ = h.focus();
                }
            }
        });
    };
    let on_input = move |ev: &web_sys::Event| {
        let Some(el) = ev.target().and_then(|t| t.dyn_into::<Element>().ok()) else { return };
        if el.closest("[data-noac]").ok().flatten().is_some() {
            return;
        }
        let Some((v, caret)) = field_value(&el) else { return };
        let u: Vec<u16> = v.encode_utf16().collect();
        let before = String::from_utf16_lossy(&u[..caret.min(u.len())]);
        let valid = |s: &str| s.chars().all(|c| c.is_alphanumeric() || "_.-$[]".contains(c));
        let found = match before.rfind("{{") {
            Some(i) if valid(&before[i + 2..]) => Some((i, true, before[i + 2..].to_string())),
            _ => match before.rfind('{') {
                Some(i) if valid(&before[i + 1..]) && !before[..i].ends_with('{') => Some((i, false, before[i + 1..].to_string())),
                _ => None,
            },
        };
        let Some((byte_i, double, partial)) = found else {
            st.set(None);
            return;
        };
        let p = partial.to_lowercase();
        let mut items: Vec<_> = candidates().into_iter().filter(|c| p.is_empty() || c.0.to_lowercase().contains(&p)).collect();
        items.sort_by_key(|c| (!c.0.to_lowercase().starts_with(&p), c.0.matches('.').count(), c.0.clone()));
        items.truncate(50);
        if items.is_empty() {
            st.set(None);
            return;
        }
        let r = el.get_bounding_client_rect();
        AC_TARGET.with(|t| *t.borrow_mut() = Some(el));
        // shortcut: el menú sale bajo el campo, no bajo el cursor; medir el caret si molesta en textareas grandes.
        st.set(Some(Ac {
            x: r.left(),
            y: r.bottom().min(web_sys::window().unwrap().inner_height().unwrap().as_f64().unwrap() - 270.0),
            items,
            sel: double.then_some(0),
            start: before[..byte_i].encode_utf16().count(),
            caret,
            double,
        }));
    };
    let on_key = move |ev: &web_sys::Event| {
        let Some(s) = st.get_untracked() else { return };
        let k: &web_sys::KeyboardEvent = ev.unchecked_ref();
        let n = s.items.len();
        let stop = || {
            ev.prevent_default();
            ev.stop_immediate_propagation();
        };
        match k.key().as_str() {
            "ArrowDown" => {
                stop();
                st.update(|x| if let Some(x) = x { x.sel = Some(x.sel.map(|i| (i + 1) % n).unwrap_or(0)) });
            }
            "ArrowUp" => {
                stop();
                st.update(|x| if let Some(x) = x { x.sel = Some(x.sel.map(|i| (i + n - 1) % n).unwrap_or(n - 1)) });
            }
            "Enter" | "Tab" => match s.sel {
                Some(i) => {
                    stop();
                    insert(s.items[i].0.clone());
                }
                None => st.set(None),
            },
            "Escape" => {
                stop();
                st.set(None);
            }
            _ => {}
        }
    };
    let doc = document();
    let opts = gloo_events::EventListenerOptions::run_in_capture_phase();
    gloo_events::EventListener::new_with_options(&doc, "input", opts, on_input).forget();
    gloo_events::EventListener::new_with_options(&doc, "keydown", gloo_events::EventListenerOptions { phase: gloo_events::EventListenerPhase::Capture, passive: false }, on_key).forget();
    gloo_events::EventListener::new_with_options(&doc, "focusout", opts, move |_| set_timeout(move || st.set(None), std::time::Duration::from_millis(150))).forget();
    move || {
        st.get().map(|s| {
            let sel = s.sel;
            let rows = s
                .items
                .iter()
                .enumerate()
                .map(|(i, (name, detail, css))| {
                    let n = name.clone();
                    view! {
                        <div class:on=sel == Some(i) on:mousedown=move |e| { e.prevent_default(); insert(n.clone()); }>
                            <span class=format!("mono {css}")>{name.clone()}</span><small>{detail.clone()}</small>
                        </div>
                    }
                })
                .collect_view();
            view! {
                <div class="ac" style=format!("left:{}px;top:{}px", s.x, s.y)>
                    {rows}
                    <div class="muted" style="cursor:default;font-size:11px">{if s.double { "↑↓ elegir · Enter/Tab insertar · Esc cerrar" } else { "↑↓ para elegir (Enter sin elegir = salto de línea)" }}</div>
                </div>
            }
        })
    }
}

// ---------- tabla clave/valor ----------

#[component]
pub fn KvTable(
    #[prop(into)] pairs: Signal<Vec<Pair>>,
    #[prop(into)] on_change: Callback<Vec<Pair>>,
    #[prop(optional, into)] key_ph: Option<String>,
    #[prop(optional, into)] val_ph: Option<String>,
    #[prop(optional)] no_toggle: bool,
) -> impl IntoView {
    let key_ph = key_ph.unwrap_or_else(|| "nombre".into());
    let val_ph = val_ph.unwrap_or_else(|| "valor".into());
    let edit = move |i: usize, f: Box<dyn Fn(&mut Pair)>| {
        let mut v = pairs.get_untracked();
        if i >= v.len() {
            v.push(Pair::new("", ""));
        }
        f(&mut v[i]);
        on_change.run(v);
    };
    view! {
        <table class="kv">
            <For each=move || 0..pairs.with(|p| p.len()) + 1 key=|i| *i children=move |i| {
                let (kp, vp) = (key_ph.clone(), val_ph.clone());
                let get = move || pairs.with(|p| p.get(i).cloned());
                view! {
                    <tr>
                        <td class="ck">{(!no_toggle).then(|| view! {
                            <input type="checkbox" prop:checked=move || get().map(|p| p.enabled).unwrap_or(true)
                                on:change=move |e| { let c = event_target_checked(&e); edit(i, Box::new(move |p| p.enabled = c)) } />
                        })}</td>
                        <td><input class="in" placeholder=kp prop:value=move || get().map(|p| p.name).unwrap_or_default()
                            on:input=move |e| { let v = event_target_value(&e); edit(i, Box::new(move |p| p.name = v.clone())) } /></td>
                        <td><VarField value=Signal::derive(move || get().map(|p| p.value).unwrap_or_default()) placeholder=vp
                            on_input=move |v: String| edit(i, Box::new(move |p| p.value = v.clone())) /></td>
                        <td class="x">{move || get().is_some().then(|| view! {
                            <button class="btn ghost sm" title="Quitar" on:click=move |_| {
                                let mut v = pairs.get_untracked(); if i < v.len() { v.remove(i); } on_change.run(v);
                            }>"×"</button>
                        })}</td>
                    </tr>
                }
            } />
        </table>
    }
}

// ---------- árbol JSON ----------

/// Árbol de datos: el cursor muestra la ruta, clic copia `{{prefijo.ruta}}`, Alt+clic llama a `on_alt`.
#[component]
pub fn JsonTree(value: Value, #[prop(into)] prefix: String, #[prop(optional, into)] on_alt: Option<Callback<(String, Value, f64, f64)>>) -> impl IntoView {
    view! { <div class="jt">{json_node(value, String::new(), prefix, on_alt, 0)}</div> }
}

fn join_path(base: &str, key: &str) -> String {
    let simple = key.chars().all(|c| c.is_alphanumeric() || c == '_');
    match (base.is_empty(), simple) {
        (true, true) => key.into(),
        (true, false) => format!("[\"{key}\"]"),
        (false, true) => format!("{base}.{key}"),
        (false, false) => format!("{base}[\"{key}\"]"),
    }
}

fn leaf(v: &Value) -> AnyView {
    match v {
        Value::String(s) => view! { <span class="s">{format!("\"{s}\"")}</span> }.into_any(),
        Value::Number(n) => view! { <span class="n">{n.to_string()}</span> }.into_any(),
        Value::Bool(b) => view! { <span class="b">{b.to_string()}</span> }.into_any(),
        Value::Null => view! { <span class="z">"null"</span> }.into_any(),
        Value::Array(a) => view! { <span class="z">{format!("[{}]", a.len())}</span> }.into_any(),
        Value::Object(_) => view! { <span class="z">"{}"</span> }.into_any(),
    }
}

fn json_node(v: Value, path: String, prefix: String, on_alt: Option<Callback<(String, Value, f64, f64)>>, depth: usize) -> AnyView {
    let entries: Vec<(String, String, Value)> = match &v {
        Value::Object(m) if !m.is_empty() => m.iter().map(|(k, x)| (k.clone(), join_path(&path, k), x.clone())).collect(),
        Value::Array(a) if !a.is_empty() => a.iter().enumerate().map(|(i, x)| (format!("[{i}]"), format!("{path}[{i}]"), x.clone())).collect(),
        _ => return leaf(&v),
    };
    let open = RwSignal::new(depth < 2 || entries.len() < 6);
    let rows = entries
        .into_iter()
        .map(|(label, p, x)| {
            let full = if p.is_empty() { prefix.clone() } else if prefix.is_empty() { p.clone() } else if p.starts_with('[') { format!("{prefix}{p}") } else { format!("{prefix}.{p}") };
            let (f2, x2) = (full.clone(), x.clone());
            let click = move |e: web_sys::MouseEvent| {
                if e.alt_key() {
                    if let Some(cb) = on_alt {
                        cb.run((f2.clone(), x2.clone(), e.client_x() as f64, e.client_y() as f64));
                    }
                } else {
                    copy(&format!("{{{{{f2}}}}}"));
                }
            };
            let nested = x.is_object() || x.is_array();
            view! {
                <div>
                    <span class="k" title=format!("{{{{{full}}}}} · clic copia · Alt+clic acciones") on:click=click>{label}</span>": "
                    {if nested { json_node(x, p, prefix.clone(), on_alt, depth + 1) } else { leaf(&x) }}
                </div>
            }
        })
        .collect_view();
    let bracket = if v.is_array() { ("[", "]") } else { ("{", "}") };
    view! {
        <span>
            <span class="tog" on:click=move |_| open.update(|o| *o = !*o)>{move || if open.get() { bracket.0.to_string() } else { format!("{}…{}", bracket.0, bracket.1) }}</span>
            <div class="kids" style:display={move || if open.get() { "block" } else { "none" }}>{rows}</div>
            {move || open.get().then_some(bracket.1)}
        </span>
    }
    .into_any()
}

// ---------- arrastre ----------

/// Arrastre con el puntero: llama a `on_move(dx, dy)` desde el punto inicial hasta soltar.
pub fn start_drag(ev: &web_sys::PointerEvent, on_move: impl Fn(f64, f64) + 'static, on_end: impl Fn() + 'static) {
    ev.prevent_default();
    let (x0, y0) = (ev.client_x() as f64, ev.client_y() as f64);
    let w = web_sys::window().unwrap();
    let holder: Rc<RefCell<Vec<gloo_events::EventListener>>> = Rc::default();
    let h2 = holder.clone();
    let mv = gloo_events::EventListener::new(&w, "pointermove", move |e| {
        let e: &web_sys::PointerEvent = e.unchecked_ref();
        on_move(e.client_x() as f64 - x0, e.client_y() as f64 - y0);
    });
    let up = gloo_events::EventListener::new(&w, "pointerup", move |_| {
        on_end();
        h2.borrow_mut().clear();
    });
    holder.borrow_mut().extend([mv, up]);
}

/// Borde arrastrable; doble clic restablece.
#[component]
pub fn Gutter(#[prop(into)] on_drag: Callback<(f64, f64, bool)>, #[prop(into)] on_reset: Callback<()>, #[prop(optional)] horizontal: bool) -> impl IntoView {
    let dragging = RwSignal::new(false);
    view! {
        <div class="gutter" class:h=horizontal class:drag=move || dragging.get()
            on:dblclick=move |_| on_reset.run(())
            on:pointerdown=move |e| {
                dragging.set(true);
                on_drag.run((0.0, 0.0, true));
                start_drag(&e, move |dx, dy| on_drag.run((dx, dy, false)), move || dragging.set(false));
            }></div>
    }
}

// ---------- modal ----------

#[component]
pub fn ModalFrame(#[prop(into)] title: String, #[prop(optional)] wide: bool, children: Children, #[prop(optional)] footer: Option<AnyView>) -> impl IntoView {
    let a = app();
    let label = title.clone();
    view! {
        <div class="overlay" on:mousedown=move |e| { if e.target() == e.current_target() { a.modal.set(None) } }>
            <div class="modal" class:wide=wide role="dialog" aria-label=label>
                <header><span class="grow">{title}</span><button class="btn ghost" aria-label="Cerrar" on:click=move |_| a.modal.set(None)>"✕"</button></header>
                <div class="body">{children()}</div>
                {footer.map(|f| view! { <footer>{f}</footer> })}
            </div>
        </div>
    }
}

pub fn status_class(s: Option<u64>) -> String {
    format!("st-{}", s.map(|s| s / 100).unwrap_or(0))
}

pub fn fmt_size(n: u64) -> String {
    if n < 1024 { format!("{n} B") } else if n < 1024 * 1024 { format!("{:.1} KB", n as f64 / 1024.0) } else { format!("{:.1} MB", n as f64 / 1048576.0) }
}

pub fn download(name: &str, mime: &str, content: &str) {
    let parts = js_sys::Array::of1(&wasm_bindgen::JsValue::from_str(content));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type(mime);
    let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &opts).unwrap();
    let url = web_sys::Url::create_object_url_with_blob(&blob).unwrap();
    let a: web_sys::HtmlAnchorElement = document().create_element("a").unwrap().unchecked_into();
    a.set_href(&url);
    a.set_download(name);
    a.click();
    let _ = web_sys::Url::revoke_object_url(&url);
}
