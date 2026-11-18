//! Casos QA de un flujo: importar CSV viendo lo que trae, la lista numerada, qué espera cada
//! caso, ejecutarlos en paralelo y el resumen de por qué fallan.
use crate::api;
use crate::flows::ed;
use crate::state::app;
use crate::ui::download;
use leptos::prelude::*;
use leptos::task::spawn_local;
use martino_core::flow::{self, Case, Flow, Rule, OPS};
use martino_core::interp::value_to_text;
use serde_json::{json, Value};
use std::collections::BTreeMap;

const CONCURRENCY: usize = 6;

fn verdict(r: Option<&Value>) -> &'static str {
    match r.and_then(|r| r["verdict"].as_str()) {
        None => "none",
        Some("OK") => "ok",
        Some(_) => "fail",
    }
}

/// Primera razón legible de una falla (para agrupar el informe).
fn cause(r: &Value) -> String {
    if let Some(e) = r["error"].as_str() {
        return e.to_string();
    }
    let bad = r["checks"].as_array().into_iter().flatten().find(|k| k["ok"] != json!(true));
    match bad {
        Some(k) => format!("{} {} {} (llegó {})", k["left"].as_str().unwrap_or(""), k["op"].as_str().unwrap_or(""), k["right"].as_str().unwrap_or(""), value_to_text(&k["actual"])),
        None => r["verdict"].as_str().unwrap_or("?").to_string(),
    }
}

/// Ids nuevos `cN` que no choquen con los que ya hay.
pub fn renumber(existing: &[Case], mut new: Vec<Case>) -> Vec<Case> {
    let mut n = existing.iter().filter_map(|c| c.id.strip_prefix('c')?.parse::<usize>().ok()).max().unwrap_or(0);
    for c in &mut new {
        n += 1;
        c.id = format!("c{n}");
    }
    new
}

fn sample(f: &Flow) -> Value {
    f.input_node().map(|n| n.data.clone()).unwrap_or(json!({}))
}

fn edit_case(id: &str, f: impl FnOnce(&mut Case)) {
    let id = id.to_string();
    ed().edit(move |fl| {
        if let Some(c) = fl.cases.iter_mut().find(|c| c.id == id) {
            f(c)
        }
    });
}

#[component]
pub fn CasesView() -> impl IntoView {
    let (a, ed) = (app(), ed());
    let sel: RwSignal<Option<String>> = RwSignal::new(ed.flow.with_untracked(|f| f.cases.first().map(|c| c.id.clone())));
    let filter = RwSignal::new("all");
    let q = RwSignal::new(String::new());
    let running = RwSignal::new(false);
    let run_id: RwSignal<Option<String>> = RwSignal::new(None);
    let progress = RwSignal::new((0usize, 0usize));
    let import: RwSignal<Option<String>> = RwSignal::new(None);

    let counts = Memo::new(move |_| {
        let res = ed.qa_res.get();
        ed.flow.with(|f| {
            let mut c = (f.cases.len(), 0, 0, 0);
            for k in &f.cases {
                match verdict(res.get(&k.id)) {
                    "ok" => c.1 += 1,
                    "fail" => c.2 += 1,
                    _ => c.3 += 1,
                }
            }
            c
        })
    });
    // (número, id, nombre, estado) de los casos que pasan el filtro.
    let rows = Memo::new(move |_| {
        let (res, fl, text) = (ed.qa_res.get(), filter.get(), q.get().to_lowercase());
        ed.flow.with(|f| {
            f.cases.iter().enumerate()
                .map(|(i, c)| (i + 1, c.id.clone(), c.name.clone(), verdict(res.get(&c.id))))
                .filter(|r| fl == "all" || r.3 == fl)
                .filter(|r| text.is_empty() || r.2.to_lowercase().contains(&text) || format!("{:03}", r.0).contains(&text))
                .collect::<Vec<_>>()
        })
    });

    let run = move |ids: Vec<String>| {
        if running.get_untracked() || ids.is_empty() {
            return;
        }
        running.set(true);
        progress.set((0, ids.len()));
        let body = json!({"flow": ed.flow.get_untracked(), "env": a.env.get_untracked(), "cases": ids, "concurrency": CONCURRENCY});
        spawn_local(async move {
            let r = api::stream("POST", "/api/flow/run", Some(&body), move |v| match v["type"].as_str() {
                Some("start") => run_id.set(v["runId"].as_str().map(String::from)),
                Some("case") => {
                    let id = v["caseId"].as_str().unwrap_or("").to_string();
                    ed.qa_res.update(|m| {
                        m.insert(id, v["result"].clone());
                    });
                    progress.set((v["done"].as_u64().unwrap_or(0) as usize, v["total"].as_u64().unwrap_or(0) as usize));
                }
                _ => {}
            })
            .await;
            running.set(false);
            run_id.set(None);
            match r {
                Err(e) => a.err(e),
                Ok(()) => {
                    let (_, ok, ko, _) = counts.get_untracked();
                    a.toast(if ko > 0 { "err" } else { "ok" }, format!("Casos QA: {ok} pasan, {ko} fallan"));
                }
            }
        });
    };
    let stop = move |_| {
        if let Some(id) = run_id.get_untracked() {
            spawn_local(async move {
                let _ = api::post::<Value>("/api/cancel", &json!({"runId": id})).await;
            });
        }
    };
    let add = move |_| {
        let c = ed.flow.with_untracked(|f| renumber(&f.cases, vec![Case { name: format!("Caso {}", f.cases.len() + 1), input: sample(f), ..Default::default() }]));
        let id = c[0].id.clone();
        ed.edit(|f| f.cases.extend(c));
        sel.set(Some(id));
    };
    let on_file = move |ev: web_sys::Event| {
        let input: web_sys::HtmlInputElement = event_target(&ev);
        let Some(f) = input.files().and_then(|l| l.get(0)) else { return };
        input.set_value("");
        spawn_local(async move {
            if let Ok(t) = wasm_bindgen_futures::JsFuture::from(f.text()).await {
                import.set(Some(t.as_string().unwrap_or_default()));
            }
        });
    };
    let export = move |_| {
        let res = ed.qa_res.get_untracked();
        let mut out = vec![vec!["#".to_string(), "id".into(), "nombre".into(), "veredicto".into(), "status".into(), "ms".into(), "por qué".into()]];
        ed.flow.with_untracked(|f| {
            for (i, c) in f.cases.iter().enumerate() {
                let r = res.get(&c.id);
                out.push(vec![format!("{:03}", i + 1), c.id.clone(), c.name.clone(),
                    r.and_then(|r| r["verdict"].as_str()).unwrap_or("sin correr").into(),
                    r.map(|r| value_to_text(&r["status"])).unwrap_or_default(),
                    r.map(|r| value_to_text(&r["ms"])).unwrap_or_default(),
                    r.filter(|r| r["ok"] != json!(true)).map(cause).unwrap_or_default()]);
            }
        });
        download("resultados-casos.csv", "text/csv", &flow::to_csv(&out));
    };
    let template = move |_| download("plantilla-casos.csv", "text/csv", &ed.flow.with_untracked(|f| flow::csv_template(&sample(f))));
    let chip = move |key: &'static str, label: &'static str, n: fn((usize, usize, usize, usize)) -> usize| view! {
        <button class="chip qa-f" class:on=move || filter.get() == key class:ok=key == "ok" class:err=key == "fail" on:click=move |_| filter.set(key)>
            {label}" "{move || n(counts.get())}
        </button>
    };

    view! {
        <div class="qa">
            <div class="qa-head">
                {chip("all", "Todos", |c| c.0)}{chip("ok", "Pasan", |c| c.1)}{chip("fail", "Fallan", |c| c.2)}{chip("none", "Sin correr", |c| c.3)}
                <input class="in" placeholder="Buscar por nombre o número…" prop:value=move || q.get() on:input=move |e| q.set(event_target_value(&e)) />
                <span class="grow"></span>
                <label class="btn ghost" title="CSV (coma, punto y coma o tab). Columnas _nombre, _status, _espera.<ruta> y los campos de la entrada">
                    "Importar CSV"<input type="file" accept=".csv,.tsv,.txt" style="display:none" on:change=on_file />
                </label>
                <button class="btn ghost" title="Descarga un CSV con las columnas de la entrada" on:click=template>"Plantilla"</button>
                <button class="btn ghost" title="Resultados en CSV" disabled=move || ed.qa_res.with(|r| r.is_empty()) on:click=export>"Exportar resultados"</button>
                <button class="btn ghost" on:click=add>"+ Caso"</button>
                {move || if running.get() {
                    let (d, t) = progress.get();
                    view! { <span class="muted">{format!("{d} de {t}")}</span><button class="btn" on:click=stop>"Detener"</button> }.into_any()
                } else {
                    view! {
                        <button class="btn primary" disabled=move || rows.with(|r| r.is_empty()) on:click=move |_| run(rows.get_untracked().into_iter().map(|r| r.1).collect())>
                            {move || { let n = rows.with(|r| r.len()); if filter.get() == "all" { format!("▶ Ejecutar los {n}") } else { format!("▶ Ejecutar estos {n}") } }}
                        </button>
                    }.into_any()
                }}
            </div>
            {move || import.get().map(|text| view! { <ImportCsv text=text import=import sel=sel /> })}
            <div class="qa-body">
                <div class="qa-list">
                    {move || {
                        let r = rows.get();
                        if r.is_empty() {
                            let none = ed.flow.with(|f| f.cases.is_empty());
                            return view! { <p class="empty">{if none { "Aún no hay casos. Importa un CSV o crea uno con «+ Caso»." } else { "Ningún caso con ese filtro." }}</p> }.into_any();
                        }
                        r.into_iter().map(|(n, id, name, st)| {
                            let (pick, on) = (id.clone(), id.clone());
                            view! {
                                <button class="qa-row" class:on=move || sel.get().as_deref() == Some(on.as_str()) on:click=move |_| sel.set(Some(pick.clone()))>
                                    <span class="mono muted">{format!("#{n:03}")}</span>
                                    <span class="grow">{name}</span>
                                    <span class=match st { "ok" => "chip ok", "fail" => "chip err", _ => "chip" }>{match st { "ok" => "Pasa", "fail" => "Falla", _ => "Sin correr" }}</span>
                                </button>
                            }
                        }).collect_view().into_any()
                    }}
                </div>
                <div class="qa-detail">
                    <Report filter=filter />
                    {move || {
                        ed.rev.track();
                        let id = sel.get()?;
                        ed.flow.with_untracked(|f| f.cases.iter().any(|c| c.id == id)).then(|| view! { <CaseDetail id=id run=run /> })
                    }}
                </div>
            </div>
        </div>
    }
}

/// Fallas agrupadas por causa: dice qué arreglar, no sólo qué falló.
#[component]
fn Report(filter: RwSignal<&'static str>) -> impl IntoView {
    let ed = ed();
    move || {
        let res = ed.qa_res.get();
        let mut by: BTreeMap<String, usize> = BTreeMap::new();
        res.values().filter(|r| r["ok"] != json!(true)).for_each(|r| *by.entry(cause(r)).or_default() += 1);
        let ko: usize = by.values().sum();
        (!res.is_empty()).then(|| {
            let mut by: Vec<_> = by.into_iter().collect();
            by.sort_by(|a, b| b.1.cmp(&a.1));
            view! {
                <div class="sect qa-report">
                    <div class="sh">
                        {format!("Informe · {} de {} corridos fallan", ko, res.len())}
                        <span class="grow"></span>
                        {(ko > 0).then(|| view! { <button class="btn ghost sm" on:click=move |_| filter.set("fail")>"Ver los que fallan"</button> })}
                    </div>
                    {(ko > 0).then(|| view! {
                        <div class="sb">{by.into_iter().map(|(c, n)| view! { <div class="row"><span class="chip err">{n}</span><span class="mono">{c}</span></div> }).collect_view()}</div>
                    })}
                </div>
            }
        })
    }
}

#[component]
fn CaseDetail(id: String, run: impl Fn(Vec<String>) + Copy + Send + Sync + 'static) -> impl IntoView {
    let ed = ed();
    let cid = StoredValue::new(id);
    let get = move |f: &dyn Fn(&Case) -> String| ed.flow.with(|fl| fl.cases.iter().find(|c| c.id == cid.get_value()).map(|c| f(c)).unwrap_or_default());
    let pos = ed.flow.with_untracked(|f| f.cases.iter().position(|c| c.id == cid.get_value()).unwrap_or(0) + 1);
    let input_text = RwSignal::new(get(&|c| serde_json::to_string_pretty(&c.input).unwrap_or_default()));
    let input_err = RwSignal::new(String::new());
    let n_rules = Memo::new(move |_| ed.flow.with(|f| f.cases.iter().find(|c| c.id == cid.get_value()).map(|c| c.expect.len()).unwrap_or(0)));
    let result = Memo::new(move |_| ed.qa_res.with(|r| r.get(&cid.get_value()).cloned()));
    let rule_field = move |i: usize, f: fn(&Rule) -> String| get(&move |c: &Case| c.expect.get(i).map(f).unwrap_or_default());
    let set_rule = move |i: usize, f: fn(&mut Rule, String)| move |e: web_sys::Event| {
        let v = event_target_value(&e);
        edit_case(&cid.get_value(), move |c| if let Some(r) = c.expect.get_mut(i) { f(r, v) });
    };
    let del = move |_| {
        let id = cid.get_value();
        ed.qa_res.update(|m| {
            m.remove(&id);
        });
        ed.edit(|f| f.cases.retain(|c| c.id != id));
    };

    view! {
        <div class="qa-case">
            <div class="row">
                <span class="mono muted">{format!("#{pos:03}")}</span>
                <input class="in grow qa-name" aria-label="Nombre del caso" prop:value=move || get(&|c| c.name.clone())
                    on:input=move |e| { let v = event_target_value(&e); edit_case(&cid.get_value(), |c| c.name = v) } />
                <button class="btn" on:click=move |_| run(vec![cid.get_value()])>"▶ Sólo este caso"</button>
                <button class="btn ghost danger" title="Borrar este caso" on:click=del>"🗑"</button>
            </div>

            <div class="sect">
                <div class="sh">"1 · Lo que envías (entrada)"</div>
                <div class="sb">
                    <textarea class="in" rows="7" aria-label="Entrada en JSON" prop:value=move || input_text.get()
                        on:input=move |e| input_text.set(event_target_value(&e))
                        on:change=move |_| { match serde_json::from_str::<Value>(&input_text.get_untracked()) {
                            Ok(v) => { input_err.set(String::new()); edit_case(&cid.get_value(), |c| c.input = v) }
                            Err(e) => input_err.set(format!("JSON inválido: {e}")),
                        } }></textarea>
                    {move || (!input_err.get().is_empty()).then(|| view! { <span class="err-text">{input_err.get()}</span> })}
                </div>
            </div>

            <div class="sect">
                <div class="sh">"2 · Qué esperas obtener"</div>
                <div class="sb">
                    <div class="row">
                        <span class="qa-l">"Status"</span>
                        <input class="in mono" style="width:90px" placeholder="cualquiera" aria-label="Status esperado"
                            prop:value=move || get(&|c| c.expect_status.map(|s| s.to_string()).unwrap_or_default())
                            on:change=move |e| { let v = event_target_value(&e); edit_case(&cid.get_value(), |c| c.expect_status = v.trim().parse().ok()) } />
                    </div>
                    <datalist id="qa-left"><option value="ms">"tiempo en ms"</option><option value="nodo">"clave del nodo Respuesta"</option><option value="status"></option></datalist>
                    {move || (0..n_rules.get()).map(|i| view! {
                        <div class="row">
                            <input class="in mono grow" list="qa-left" placeholder="error.codigo · ms · nodo" aria-label="Qué se revisa" prop:value=move || rule_field(i, |r| r.left.clone()) on:input=set_rule(i, |r, v| r.left = v) />
                            <select class="in" aria-label="Operador" prop:value=move || rule_field(i, |r| r.op.clone()) on:change=set_rule(i, |r, v| r.op = v)>
                                {OPS.into_iter().map(|o| view! { <option value=o>{o}</option> }).collect_view()}
                            </select>
                            <input class="in mono grow" placeholder="valor" aria-label="Valor esperado" prop:value=move || rule_field(i, |r| r.right.clone()) on:input=set_rule(i, |r, v| r.right = v) />
                            <button class="btn ghost sm" title="Quitar" on:click=move |_| edit_case(&cid.get_value(), move |c| { if i < c.expect.len() { c.expect.remove(i); } })>"✕"</button>
                        </div>
                    }).collect_view()}
                    <button class="btn ghost sm" style="align-self:flex-start" on:click=move |_| edit_case(&cid.get_value(), |c| c.expect.push(Rule { left: String::new(), op: "eq".into(), right: String::new() }))>"+ Agregar lo que esperas"</button>
                    <textarea class="in" rows="2" placeholder="Por qué importa este caso (opcional)" aria-label="Nota" prop:value=move || get(&|c| c.note.clone())
                        on:input=move |e| { let v = event_target_value(&e); edit_case(&cid.get_value(), |c| c.note = v) }></textarea>
                </div>
            </div>

            <div class="sect">
                <div class="sh">"3 · Lo que salió"</div>
                <div class="sb">
                    {move || match result.get() {
                        None => view! { <span class="muted">"Todavía no se ejecuta. «▶ Sólo este caso» o «Ejecutar»."</span> }.into_any(),
                        Some(r) => {
                            let ok = r["ok"] == json!(true);
                            let shown = r.clone();
                            view! {
                                <div class="row">
                                    <span class=if ok { "chip ok" } else { "chip err" }>{r["verdict"].as_str().unwrap_or("?").to_string()}</span>
                                    {r["status"].as_u64().map(|s| view! { <span class=crate::ui::status_class(Some(s))>{s}</span> })}
                                    <span class="muted">{format!("{} ms", r["ms"].as_u64().unwrap_or(0))}</span>
                                    <span class="grow"></span>
                                    <button class="btn ghost sm" title="Pinta en el lienzo el camino que siguió este caso" on:click=move |_| { ed.show_result(&shown); ed.pane.set(crate::flows::Pane::Canvas) }>"Ver el recorrido en el lienzo"</button>
                                </div>
                                {r["error"].as_str().map(|e| view! { <span class="err-text">{e.to_string()}</span> })}
                                {r["checks"].as_array().cloned().unwrap_or_default().into_iter().map(|k| {
                                    let good = k["ok"] == json!(true);
                                    view! { <div class="row mono qa-check" class:bad=!good>
                                        {if good { "✓" } else { "✗" }}" "{format!("{} {} {}", k["left"].as_str().unwrap_or(""), k["op"].as_str().unwrap_or(""), k["right"].as_str().unwrap_or(""))}
                                        <span class="muted">{format!("→ llegó {}", value_to_text(&k["actual"]))}</span>
                                    </div> }
                                }).collect_view()}
                                <pre class="qa-body-out mono">{serde_json::to_string_pretty(&r["body"]).unwrap_or_default()}</pre>
                            }.into_any()
                        }
                    }}
                </div>
            </div>
        </div>
    }
}

/// Muestra lo que trae el archivo (como tabla o tal cual) antes de importar.
#[component]
fn ImportCsv(text: String, import: RwSignal<Option<String>>, sel: RwSignal<Option<String>>) -> impl IntoView {
    let ed = ed();
    let raw = RwSignal::new(false);
    let replace = RwSignal::new(false);
    let sample_v = ed.flow.with_untracked(sample);
    let rows = flow::parse_csv(&text);
    let parsed = flow::csv_to_cases(&text, &sample_v);
    let n = parsed.as_ref().map(|c| c.len()).unwrap_or(0);
    let head = rows.first().cloned().unwrap_or_default();
    let mapping: Vec<String> = head.iter().map(|h| {
        let h = h.trim();
        match h {
            "_nombre" | "_name" => "nombre del caso".into(),
            "_status" => "espero status".into(),
            _ if h.starts_with("_espera.") => format!("espero {}", &h["_espera.".len()..]),
            _ if h.starts_with('_') => "se ignora".into(),
            _ => match martino_core::interp::get_path(&sample_v, h) {
                Some(Value::Number(_)) => format!("entrada.{h} · número"),
                Some(Value::Bool(_)) => format!("entrada.{h} · sí/no"),
                Some(_) => format!("entrada.{h} · texto"),
                None => format!("entrada.{h} · nuevo"),
            },
        }
    }).collect();
    let body: Vec<Vec<String>> = rows.iter().skip(1).take(50).cloned().collect();
    let more = rows.len().saturating_sub(51);
    let text2 = text.clone();
    let err = parsed.as_ref().err().cloned();
    let go = move |_| {
        let Ok(cases) = parsed.clone() else { return };
        let rep = replace.get_untracked();
        let new = ed.flow.with_untracked(|f| renumber(if rep { &[] } else { &f.cases }, cases));
        let first = new.first().map(|c| c.id.clone());
        ed.edit(move |f| if rep { f.cases = new } else { f.cases.extend(new) });
        ed.qa_res.update(|m| if rep { m.clear() });
        sel.set(first);
        import.set(None);
        app().toast("ok", format!("Importé {n} casos"));
    };
    view! {
        <div class="sect qa-import">
            <div class="sh">
                {format!("Lo que trae el archivo · {} filas", rows.len().saturating_sub(1))}
                <span class="grow"></span>
                <div class="seg">
                    <button class:on=move || !raw.get() on:click=move |_| raw.set(false)>"Como tabla"</button>
                    <button class:on=move || raw.get() on:click=move |_| raw.set(true)>"Archivo original"</button>
                </div>
            </div>
            <div class="sb">
                {move || if raw.get() {
                    view! { <pre class="qa-raw mono">{text2.lines().take(200).enumerate().map(|(i, l)| format!("{:>4}  {l}\n", i + 1)).collect::<String>()}</pre> }.into_any()
                } else {
                    view! {
                        <div class="qa-table">
                            <table class="matrix">
                                <thead>
                                    <tr><th>"#"</th>{head.iter().map(|h| view! { <th class="mono">{h.clone()}</th> }).collect_view()}</tr>
                                    <tr><th></th>{mapping.iter().map(|m| view! { <th class="muted">{m.clone()}</th> }).collect_view()}</tr>
                                </thead>
                                <tbody>{body.iter().enumerate().map(|(i, r)| view! {
                                    <tr><td class="muted mono">{format!("{:03}", i + 1)}</td>{r.iter().map(|c| view! { <td class="mono">{c.clone()}</td> }).collect_view()}</tr>
                                }).collect_view()}</tbody>
                            </table>
                            {(more > 0).then(|| view! { <p class="muted">{format!("… y {more} filas más")}</p> })}
                        </div>
                    }.into_any()
                }}
                {err.map(|e| view! { <span class="err-text">{e}</span> })}
                <div class="row">
                    <label class="row"><input type="radio" name="qa-mode" prop:checked=move || !replace.get() on:change=move |_| replace.set(false) />"Agregar a los que hay"</label>
                    <label class="row"><input type="radio" name="qa-mode" prop:checked=move || replace.get() on:change=move |_| replace.set(true) />"Reemplazar todos"</label>
                    <span class="grow"></span>
                    <button class="btn ghost" on:click=move |_| import.set(None)>"Cancelar"</button>
                    <button class="btn primary" disabled=n == 0 on:click=go>{format!("Importar {n} casos")}</button>
                </div>
            </div>
        </div>
    }
}
