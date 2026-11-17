//! Plantillas `{{var}}`, rutas tipo `a.b[0].c` y formateo de JSON que tolera `{{var}}`.
use serde_json::Value;

pub const DYNAMIC_VARS: [&str; 10] = [
    "$guid",
    "$randomUUID",
    "$timestamp",
    "$isoTimestamp",
    "$randomInt",
    "$randomEmail",
    "$randomFirstName",
    "$randomLastName",
    "$randomBoolean",
    "$randomAlphaNumeric",
];

/// Un tramo de texto: literal o `{{nombre}}`.
#[derive(Clone, Debug, PartialEq)]
pub enum Seg<'a> {
    Text(&'a str),
    /// (nombre sin espacios, texto completo con llaves)
    Var(&'a str, &'a str),
}

pub fn segments(s: &str) -> Vec<Seg<'_>> {
    let mut out = vec![];
    let mut rest = s;
    while let Some(i) = rest.find("{{") {
        let Some(j) = rest[i + 2..].find("}}") else { break };
        let end = i + 2 + j + 2;
        if i > 0 {
            out.push(Seg::Text(&rest[..i]));
        }
        out.push(Seg::Var(rest[i + 2..i + 2 + j].trim(), &rest[i..end]));
        rest = &rest[end..];
    }
    if !rest.is_empty() {
        out.push(Seg::Text(rest));
    }
    out
}

pub fn var_names(s: &str) -> Vec<String> {
    segments(s)
        .into_iter()
        .filter_map(|x| match x {
            Seg::Var(n, _) => Some(n.to_string()),
            _ => None,
        })
        .collect()
}

/// Sustituye `{{x}}` con lo que diga `resolve`; lo que no resuelve queda igual.
pub fn interpolate(s: &str, resolve: &dyn Fn(&str) -> Option<String>) -> String {
    segments(s)
        .into_iter()
        .map(|x| match x {
            Seg::Text(t) => t.to_string(),
            Seg::Var(n, raw) => resolve(n).unwrap_or_else(|| raw.to_string()),
        })
        .collect()
}

/// Si el texto es exactamente `{{ruta}}`, devuelve la ruta (para conservar el tipo JSON).
pub fn whole_ref(s: &str) -> Option<&str> {
    match segments(s.trim()).as_slice() {
        [Seg::Var(n, _)] => Some(n),
        _ => None,
    }
}

pub fn value_to_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        other => other.to_string(),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PathSeg {
    Key(String),
    Index(usize),
}

/// `a.b[0]["x-y"].c` → [a, b, 0, x-y, c]
pub fn parse_path(p: &str) -> Vec<PathSeg> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut chars = p.chars().peekable();
    let flush = |cur: &mut String, out: &mut Vec<PathSeg>| {
        if !cur.is_empty() {
            out.push(PathSeg::Key(std::mem::take(cur)));
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '.' => flush(&mut cur, &mut out),
            '[' => {
                flush(&mut cur, &mut out);
                let mut inner = String::new();
                for d in chars.by_ref() {
                    if d == ']' {
                        break;
                    }
                    inner.push(d);
                }
                let inner = inner.trim().trim_matches(['"', '\'']);
                match inner.parse::<usize>() {
                    Ok(i) => out.push(PathSeg::Index(i)),
                    Err(_) => out.push(PathSeg::Key(inner.into())),
                }
            }
            _ => cur.push(c),
        }
    }
    flush(&mut cur, &mut out);
    out
}

pub fn get_path<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = v;
    for seg in parse_path(path) {
        cur = match (seg, cur) {
            (PathSeg::Key(k), Value::Object(m)) => m.get(&k)?,
            (PathSeg::Index(i), Value::Array(a)) => a.get(i)?,
            (PathSeg::Key(k), Value::Array(a)) => a.get(k.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

/// Escribe `value` en la ruta, creando objetos intermedios.
pub fn set_path(v: &mut Value, path: &str, value: Value) {
    let segs = parse_path(path);
    let mut cur = v;
    for (i, seg) in segs.iter().enumerate() {
        let last = i == segs.len() - 1;
        match seg {
            PathSeg::Index(n) => {
                if !cur.is_array() {
                    *cur = Value::Array(vec![]);
                }
                let a = cur.as_array_mut().unwrap();
                while a.len() <= *n {
                    a.push(Value::Null);
                }
                if last {
                    a[*n] = value;
                    return;
                }
                cur = &mut a[*n];
            }
            PathSeg::Key(k) => {
                if !cur.is_object() {
                    *cur = Value::Object(Default::default());
                }
                let m = cur.as_object_mut().unwrap();
                if last {
                    m.insert(k.clone(), value);
                    return;
                }
                cur = m.entry(k.clone()).or_insert(Value::Null);
            }
        }
    }
}

/// Todas las rutas hoja de un JSON (`a.b`, `lista[0].x`).
pub fn leaf_paths(v: &Value) -> Vec<(String, Value)> {
    fn walk(v: &Value, p: String, out: &mut Vec<(String, Value)>) {
        match v {
            Value::Object(m) if !m.is_empty() => {
                for (k, x) in m {
                    walk(x, if p.is_empty() { k.clone() } else { format!("{p}.{k}") }, out)
                }
            }
            Value::Array(a) if !a.is_empty() => {
                for (i, x) in a.iter().enumerate() {
                    walk(x, format!("{p}[{i}]"), out)
                }
            }
            _ => out.push((p, v.clone())),
        }
    }
    let mut out = vec![];
    walk(v, String::new(), &mut out);
    out
}

/// Indenta JSON sin parsearlo: funciona aunque haya `{{var}}` sin comillas.
pub fn beautify_json(src: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    let mut in_str = false;
    let mut esc = false;
    let chars: Vec<char> = src.chars().collect();
    let nl = |out: &mut String, d: usize| {
        out.push('\n');
        out.push_str(&"  ".repeat(d));
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            out.push(c);
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        // `{{var}}`: se copia entero
        if c == '{' && chars.get(i + 1) == Some(&'{') {
            if let Some(end) = (i + 2..chars.len().saturating_sub(1)).find(|&k| chars[k] == '}' && chars[k + 1] == '}') {
                out.extend(&chars[i..end + 2]);
                i = end + 2;
                continue;
            }
        }
        match c {
            '"' => {
                in_str = true;
                out.push(c)
            }
            '{' | '[' => {
                let close = if c == '{' { '}' } else { ']' };
                let next = chars[i + 1..].iter().find(|x| !x.is_whitespace());
                if next == Some(&close) {
                    out.push(c);
                    out.push(close);
                    i += 1 + chars[i + 1..].iter().position(|x| *x == close).unwrap() + 1;
                    continue;
                }
                out.push(c);
                depth += 1;
                nl(&mut out, depth);
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                nl(&mut out, depth);
                out.push(c);
            }
            ',' => {
                out.push(c);
                nl(&mut out, depth);
            }
            ':' => out.push_str(": "),
            c if c.is_whitespace() => {}
            _ => out.push(c),
        }
        i += 1;
    }
    out
}

/// ¿Es un JSON de una sola línea que conviene formatear?
pub fn looks_minified_json(s: &str) -> bool {
    let t = s.trim();
    (t.starts_with('{') || t.starts_with('[')) && !t.contains('\n') && t.len() > 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn interpolation_and_segments() {
        let r = |n: &str| (n == "a").then(|| "1".to_string());
        assert_eq!(interpolate("x{{ a }}y{{b}}", &r), "x1y{{b}}");
        assert_eq!(var_names("{{a}} {{ b.c[0] }}"), ["a", "b.c[0]"]);
        assert_eq!(whole_ref(" {{svc.response.id}} "), Some("svc.response.id"));
        assert_eq!(whole_ref("id-{{x}}"), None);
    }

    #[test]
    fn paths() {
        let v = json!({"Lista": [{"Campo": 5}], "x-y": {"z": true}});
        assert_eq!(get_path(&v, "Lista[0].Campo"), Some(&json!(5)));
        assert_eq!(get_path(&v, "[\"x-y\"].z"), Some(&json!(true)));
        assert_eq!(get_path(&v, "Lista.0.Campo"), Some(&json!(5)));
        let mut w = json!({});
        set_path(&mut w, "a.b[1].c", json!(3));
        assert_eq!(w, json!({"a": {"b": [null, {"c": 3}]}}));
        assert_eq!(leaf_paths(&json!({"a": {"b": 1}, "l": [2]})), vec![("a.b".into(), json!(1)), ("l[0]".into(), json!(2))]);
    }

    #[test]
    fn beautify_tolerates_vars() {
        let s = beautify_json(r#"{"a":{{edad}},"b":[1,2],"c":{},"d":"x,{y}"}"#);
        assert_eq!(s, "{\n  \"a\": {{edad}},\n  \"b\": [\n    1,\n    2\n  ],\n  \"c\": {},\n  \"d\": \"x,{y}\"\n}");
    }
}
