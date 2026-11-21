//! Inicio: presenta Martino y sus funciones con una animación del recorrido de un flujo, y guía un
//! tutorial de 4 pasos con los flujos «Tutorial 1–3» de la colección de ejemplo.
use crate::state::{app, local_get, local_set, Mode};
use leptos::prelude::*;

/// Pasos del tutorial: (clave, título, qué hacer, flujo que abre).
const STEPS: [(&str, &str, &str, Option<&str>); 4] = [
    ("t1", "Tu primera llamada", "Abre el flujo, dale ▶ Ejecutar y mira cómo se enciende el camino. Clic en un dato copia su ruta.", Some("gratis--tutorial-1")),
    ("t2", "Decidir con una condición", "Cambia el escenario de la Entrada a «Menor de edad» y vuelve a ejecutar: el flujo toma el otro camino.", Some("gratis--tutorial-2")),
    ("t3", "Traducir con una tabla", "Mira cómo «Buscar el país» convierte JP en Japón, y prueba el escenario «Código inventado».", Some("gratis--tutorial-3")),
    ("t4", "Pegar un cURL", "Copia un cURL de tu navegador y pégalo (Ctrl+V) sobre el lienzo: se vuelve un paso Servicio.", None),
];

/// Funciones: (forma, título, qué hace, a dónde lleva).
const FEATURES: [(&str, &str, &str, &str); 6] = [
    ("request", "Peticiones", "Prueba APIs con variables de colores y guarda todo en archivos que van a git.", "req"),
    ("condition", "Flujos", "Encadena servicios, decide con reglas y mira el recorrido de cada ejecución.", "flow"),
    ("input", "Casos QA", "Define qué esperas de cada entrada y corre todos los casos de una vez.", "flow"),
    ("crypto", "Cifrado", "JWE, JWS, AES, RSA y HMAC sin escribir scripts.", "flow"),
    ("lookup", "Equivalencias", "Traduce códigos con tablas: 002 → BCP, JP → Japón.", "flow"),
    ("response", "Historia", "El mismo flujo como pasos numerados, para explicarlo a cualquiera.", "flow"),
];

fn done_set() -> Vec<String> {
    local_get("martino.tutorial").map(|s| s.split(',').filter(|x| !x.is_empty()).map(String::from).collect()).unwrap_or_default()
}

#[component]
pub fn Home() -> impl IntoView {
    let a = app();
    let done = RwSignal::new(done_set());
    let mark = move |k: &str| {
        done.update(|d| if !d.iter().any(|x| x == k) { d.push(k.to_string()) });
        local_set("martino.tutorial", &done.get_untracked().join(","));
    };
    let go = move |to: &str, flow: Option<&str>| {
        if let Some(f) = flow {
            a.flow_open.set(Some(f.to_string()));
        }
        a.mode.set(if to == "req" { Mode::Requests } else { Mode::Flows });
        a.home.set(false);
    };
    let demo = [("input", "ENT-01", "Entrada"), ("request", "SRV-01", "Servicio"), ("condition", "CND-01", "¿Existe?"), ("crypto", "CIF-01", "Cifrar"), ("response", "RES-01", "Responde 201")];
    view! {
        <div class="home">
            <section class="h-hero">
                <div class="h-cat"><crate::ui::Cat /></div>
                <h1>"Martino"</h1>
                <p class="h-lede">"Prueba APIs, encadénalas en flujos que se entienden de un vistazo y verifica que respondan lo que esperas. Todo en tu disco, sin cuenta y sin nube."</p>
                <div class="h-cta">
                    <a class="btn primary" href="#tutorial">"Empezar el tutorial"</a>
                    <button class="btn" on:click=move |_| go("flow", None)>"Ir a Flujos"</button>
                    <button class="btn" on:click=move |_| go("req", None)>"Ir a Peticiones"</button>
                </div>
            </section>

            <section class="h-demo" aria-label="Así se ve un flujo al ejecutarse">
                {demo.into_iter().enumerate().map(|(i, (k, id, name))| view! {
                    {(i > 0).then(|| view! { <span class="h-wire" style=format!("--i:{i}")><i></i></span> })}
                    <div class=format!("h-step k-{k}") style=format!("--i:{i}")>
                        <span class="h-id">{id}</span><span class=format!("kshape k-{k}")></span>
                        <b>{name}</b>
                    </div>
                }).collect_view()}
            </section>
            <p class="h-cap">"Cada paso tiene su ID y su forma; al ejecutar, el camino se enciende paso a paso."</p>

            <section class="h-grid">
                {FEATURES.into_iter().enumerate().map(|(i, (k, t, d, to))| view! {
                    <button class="h-card" style=format!("--i:{i}") on:click=move |_| go(to, None)>
                        <span class=format!("kshape k-{k}")></span>
                        <b>{t}</b>
                        <span>{d}</span>
                    </button>
                }).collect_view()}
            </section>

            <section class="h-tut" id="tutorial">
                <div class="h-tut-h">
                    <h2>"Tutorial en 4 pasos"</h2>
                    <span class="muted">{move || format!("{} de 4 hechos", done.with(|d| STEPS.iter().filter(|s| d.iter().any(|x| x == s.0)).count()))}</span>
                </div>
                <div class="h-bar"><i style=move || format!("width:{}%", done.with(|d| STEPS.iter().filter(|s| d.iter().any(|x| x == s.0)).count()) * 25)></i></div>
                <ol>
                    {STEPS.into_iter().map(|(k, t, d, flow)| view! {
                        <li class:ok=move || done.with(|x| x.iter().any(|y| y == k))>
                            <span class="h-num"></span>
                            <div><b>{t}</b><p>{d}</p></div>
                            <button class="btn sm" on:click=move |_| { mark(k); go("flow", flow) }>{if flow.is_some() { "Abrir" } else { "Ir al lienzo" }}</button>
                        </li>
                    }).collect_view()}
                </ol>
            </section>

            <p class="h-keys muted">"Atajos: Ctrl+1 Peticiones · Ctrl+2 Flujos · / buscar un paso · Ctrl+Enter ejecutar · Ctrl+S guardar. El gato de arriba a la izquierda te trae de vuelta aquí."</p>
        </div>
    }
}
