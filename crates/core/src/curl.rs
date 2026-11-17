//! cURL ⇄ petición. Sin DOM: lo usan el servidor y la UI.
use crate::model::{Pair, Request};

/// Divide como un shell: comillas simples, dobles, `$'…'` y `\` + salto de línea.
pub fn shell_split(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut has = false;
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\\' => match it.next() {
                Some('\n') | Some('\r') => {}
                Some(n) => {
                    cur.push(n);
                    has = true
                }
                None => {}
            },
            '\'' => {
                has = true;
                for d in it.by_ref() {
                    if d == '\'' {
                        break;
                    }
                    cur.push(d);
                }
            }
            '$' if it.peek() == Some(&'\'') => {
                it.next();
                has = true;
                while let Some(d) = it.next() {
                    match d {
                        '\'' => break,
                        '\\' => match it.next() {
                            Some('n') => cur.push('\n'),
                            Some('t') => cur.push('\t'),
                            Some(x) => cur.push(x),
                            None => {}
                        },
                        _ => cur.push(d),
                    }
                }
            }
            '"' => {
                has = true;
                while let Some(d) = it.next() {
                    match d {
                        '"' => break,
                        '\\' => match it.next() {
                            Some(x @ ('"' | '\\' | '$' | '`')) => cur.push(x),
                            Some('\n') => {}
                            Some(x) => {
                                cur.push('\\');
                                cur.push(x)
                            }
                            None => {}
                        },
                        _ => cur.push(d),
                    }
                }
            }
            c if c.is_whitespace() => {
                if has {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            _ => {
                cur.push(c);
                has = true
            }
        }
    }
    if has {
        out.push(cur);
    }
    out
}

pub fn is_curl(s: &str) -> bool {
    s.trim_start().starts_with("curl ")
}

pub fn parse_curl(src: &str) -> Result<Request, String> {
    let toks = shell_split(src.trim());
    if toks.first().map(String::as_str) != Some("curl") {
        return Err("No empieza con `curl`".into());
    }
    let mut r = Request::new("get", "");
    let mut method: Option<String> = None;
    let mut data: Vec<String> = vec![];
    let mut form: Vec<Pair> = vec![];
    let mut get = false;
    let mut i = 1;
    let next = |i: &mut usize| -> Result<String, String> {
        *i += 1;
        toks.get(*i).cloned().ok_or_else(|| format!("Falta el valor de {}", toks[*i - 1]))
    };
    while i < toks.len() {
        let t = toks[i].as_str();
        // --flag=valor
        let (flag, inline) = match t.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
            _ => (t, None),
        };
        let val = |i: &mut usize| inline.clone().map(Ok).unwrap_or_else(|| next(i));
        match flag {
            "-X" | "--request" => method = Some(val(&mut i)?.to_lowercase()),
            "-H" | "--header" => {
                let h = val(&mut i)?;
                if let Some((k, v)) = h.split_once(':') {
                    r.headers.push(Pair::new(k.trim(), v.trim()));
                }
            }
            "-d" | "--data" | "--data-raw" | "--data-binary" | "--data-ascii" | "--data-urlencode" | "--json" => {
                let v = val(&mut i)?;
                if flag == "--json" {
                    r.headers.push(Pair::new("Content-Type", "application/json"));
                }
                data.push(v)
            }
            "-F" | "--form" => {
                if let Some((k, v)) = val(&mut i)?.split_once('=') {
                    form.push(Pair::new(k, v));
                }
            }
            "-u" | "--user" => {
                let v = val(&mut i)?;
                let (u, p) = v.split_once(':').unwrap_or((&v, ""));
                r.auth_mode = "basic".into();
                r.auth.username = u.into();
                r.auth.password = p.into();
            }
            "-b" | "--cookie" => r.headers.push(Pair::new("Cookie", val(&mut i)?)),
            "-A" | "--user-agent" => r.headers.push(Pair::new("User-Agent", val(&mut i)?)),
            "-e" | "--referer" => r.headers.push(Pair::new("Referer", val(&mut i)?)),
            "--url" => r.url = val(&mut i)?,
            "-G" | "--get" => get = true,
            "-I" | "--head" => method = Some("head".into()),
            "-o" | "--output" | "-m" | "--max-time" | "--connect-timeout" | "-w" | "--write-out" | "--cert" | "--key"
            | "--cacert" | "-x" | "--proxy" | "--retry" => {
                val(&mut i)?;
            }
            f if f.starts_with('-') => {}
            _ => {
                if r.url.is_empty() {
                    r.url = t.to_string()
                }
            }
        }
        i += 1;
    }
    if r.url.is_empty() {
        return Err("El cURL no tiene URL".into());
    }
    // Bearer → auth:bearer
    if let Some(pos) = r.headers.iter().position(|h| h.name.eq_ignore_ascii_case("authorization") && h.value.starts_with("Bearer ")) {
        r.auth_mode = "bearer".into();
        r.auth.token = r.headers.remove(pos).value[7..].trim().into();
    }
    let body = data.join("&");
    if get && !body.is_empty() {
        r.url = format!("{}{}{}", r.url, if r.url.contains('?') { "&" } else { "?" }, body);
    } else if !body.is_empty() {
        let ct = header(&r, "content-type").unwrap_or_default().to_ascii_lowercase();
        let t = body.trim_start();
        if ct.contains("json") || ((t.starts_with('{') || t.starts_with('[')) && !ct.contains("form")) {
            r.body_mode = "json".into();
            r.body.json = if crate::interp::looks_minified_json(&body) { crate::interp::beautify_json(&body) } else { body };
        } else if ct.contains("xml") {
            r.body_mode = "xml".into();
            r.body.xml = body;
        } else if ct.contains("x-www-form-urlencoded") || (ct.is_empty() && body.contains('=')) {
            r.body_mode = "form-urlencoded".into();
            r.body.form = body
                .split('&')
                .map(|kv| {
                    let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                    Pair::new(pct_decode(k), pct_decode(v))
                })
                .collect();
        } else {
            r.body_mode = "text".into();
            r.body.text = body;
        }
    }
    if !form.is_empty() {
        r.body_mode = "multipart-form".into();
        r.body.multipart = form;
    }
    r.method = method.unwrap_or_else(|| if (r.body_mode != "none") && !get { "post".into() } else { "get".into() });
    split_query(&mut r);
    r.name = default_name(&r);
    Ok(r)
}

fn header(r: &Request, name: &str) -> Option<String> {
    r.headers.iter().find(|h| h.name.eq_ignore_ascii_case(name)).map(|h| h.value.clone())
}

/// La query de la URL pasa a `params` (como hace Bruno).
pub fn split_query(r: &mut Request) {
    if let Some((base, q)) = r.url.clone().split_once('?') {
        r.url = base.into();
        for kv in q.split('&').filter(|s| !s.is_empty()) {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            r.params.push(Pair::new(pct_decode(k), pct_decode(v)));
        }
    }
}

fn default_name(r: &Request) -> String {
    let (_, path) = crate::model::split_url(&r.url);
    let last = path.trim_end_matches('/').rsplit('/').next().unwrap_or("").split('?').next().unwrap_or("");
    format!("{} {}", r.method.to_uppercase(), if last.is_empty() { "/" } else { last })
}

pub fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()).ok_or(()) {
                    Ok(x) => {
                        out.push(x);
                        i += 2
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            x => out.push(x),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into()
}

fn q(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Genera el cURL equivalente (al revés de `parse_curl`).
pub fn to_curl(r: &Request) -> String {
    let mut url = r.url.clone();
    let qs: Vec<_> = r.params.iter().filter(|p| p.enabled).map(|p| format!("{}={}", p.name, p.value)).collect();
    if !qs.is_empty() {
        url = format!("{url}{}{}", if url.contains('?') { "&" } else { "?" }, qs.join("&"));
    }
    let mut parts = vec![format!("curl -X {} {}", r.method.to_uppercase(), q(&url))];
    for h in r.headers.iter().filter(|h| h.enabled) {
        parts.push(format!("-H {}", q(&format!("{}: {}", h.name, h.value))));
    }
    match r.auth_mode.as_str() {
        "bearer" => parts.push(format!("-H {}", q(&format!("Authorization: Bearer {}", r.auth.token)))),
        "basic" => parts.push(format!("-u {}", q(&format!("{}:{}", r.auth.username, r.auth.password)))),
        _ => {}
    }
    match r.body_mode.as_str() {
        "json" | "text" | "xml" => {
            let has_ct = r.headers.iter().any(|h| h.name.eq_ignore_ascii_case("content-type"));
            if r.body_mode == "json" && !has_ct {
                parts.push(format!("-H {}", q("Content-Type: application/json")));
            }
            parts.push(format!("--data-raw {}", q(&r.body_text().unwrap_or_default())));
        }
        "form-urlencoded" => {
            for p in r.body.form.iter().filter(|p| p.enabled) {
                parts.push(format!("--data-urlencode {}", q(&format!("{}={}", p.name, p.value))));
            }
        }
        "multipart-form" => {
            for p in r.body.multipart.iter().filter(|p| p.enabled) {
                parts.push(format!("-F {}", q(&format!("{}={}", p.name, p.value))));
            }
        }
        _ => {}
    }
    parts.join(" \\\n  ")
}

/// Cambia la URL base y las cabeceras escritas a mano por variables del entorno.
/// Devuelve qué se sustituyó, para mostrarlo en la UI.
pub fn templatize_request(r: &mut Request, env: &[Pair]) -> Vec<String> {
    let mut done = vec![];
    let mut urls: Vec<&Pair> = env.iter().filter(|p| p.value.starts_with("http") && p.value.len() > 8).collect();
    urls.sort_by_key(|p| std::cmp::Reverse(p.value.trim_end_matches('/').len()));
    if let Some(p) = urls.iter().find(|p| r.url.starts_with(p.value.trim_end_matches('/'))) {
        let base = p.value.trim_end_matches('/');
        r.url = format!("{{{{{}}}}}{}", p.name, &r.url[base.len()..]);
        done.push(format!("URL → {{{{{}}}}}", p.name));
    }
    let find = |v: &str| env.iter().find(|p| !p.value.is_empty() && p.value.len() > 2 && p.value == v).map(|p| p.name.clone());
    for h in r.headers.iter_mut() {
        if let Some(n) = find(&h.value) {
            done.push(format!("{} → {{{{{n}}}}}", h.name));
            h.value = format!("{{{{{n}}}}}");
        }
    }
    if r.auth_mode == "bearer" {
        if let Some(n) = find(&r.auth.token).or_else(|| find(&format!("Bearer {}", r.auth.token))) {
            done.push(format!("token → {{{{{n}}}}}"));
            r.auth.token = format!("{{{{{n}}}}}");
        }
    }
    for p in r.params.iter_mut() {
        if let Some(n) = find(&p.value) {
            done.push(format!("{} → {{{{{n}}}}}", p.name));
            p.value = format!("{{{{{n}}}}}");
        }
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typical_curl() {
        let c = r#"curl 'https://api.x.com/v1/users?page=2&q=a%20b' \
  -H 'Authorization: Bearer abc.def' \
  -H "Content-Type: application/json" \
  --data-raw '{"name":"Ana","n":1}'"#;
        let r = parse_curl(c).unwrap();
        assert_eq!(r.method, "post");
        assert_eq!(r.url, "https://api.x.com/v1/users");
        assert_eq!(r.params, vec![Pair::new("page", "2"), Pair::new("q", "a b")]);
        assert_eq!((r.auth_mode.as_str(), r.auth.token.as_str()), ("bearer", "abc.def"));
        assert_eq!(r.body_mode, "json");
        assert!(r.body.json.contains("\"name\": \"Ana\""));
        assert_eq!(r.name, "POST users");
    }

    #[test]
    fn other_forms() {
        let r = parse_curl("curl -X PUT --url=http://h/a -d 'a=1&b=x%2By' -u user:pw").unwrap();
        assert_eq!((r.method.as_str(), r.body_mode.as_str()), ("put", "form-urlencoded"));
        assert_eq!(r.body.form[1], Pair::new("b", "x+y"));
        assert_eq!(r.auth.username, "user");
        let g = parse_curl("curl -G http://h/s -d q=1").unwrap();
        assert_eq!((g.method.as_str(), g.params[0].value.as_str()), ("get", "1"));
        assert!(parse_curl("wget x").is_err());
        assert_eq!(shell_split("a $'x\\ny' \"q\\\"z\""), ["a", "x\ny", "q\"z"]);
    }

    #[test]
    fn round_trip_and_templatize() {
        let mut r = parse_curl("curl https://dev.api.com/v1/x -H 'x-client: CLI-123' -H 'Authorization: Bearer TOK'").unwrap();
        let back = parse_curl(&to_curl(&r)).unwrap();
        assert_eq!((back.url, back.auth.token, back.headers), (r.url.clone(), r.auth.token.clone(), r.headers.clone()));
        let env = vec![Pair::new("globalUrl", "https://dev.api.com/"), Pair::new("clientId", "CLI-123"), Pair::new("token", "TOK")];
        let done = templatize_request(&mut r, &env);
        assert_eq!(r.url, "{{globalUrl}}/v1/x");
        assert_eq!(r.headers[0].value, "{{clientId}}");
        assert_eq!(r.auth.token, "{{token}}");
        assert_eq!(done.len(), 3);
    }
}
