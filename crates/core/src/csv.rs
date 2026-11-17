//! CSV: lectura tolerante (`,` `;` o tabulador), casos QA desde CSV y escritura.
use crate::flow::{split_assert, Case, Rule};
use crate::interp::{get_path, leaf_paths, set_path, value_to_text};
use serde_json::{json, Value};

pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let first = text.lines().next().unwrap_or("");
    let sep = [',', ';', '\t'].into_iter().max_by_key(|s| first.matches(*s).count()).unwrap_or(',');
    let mut rows = vec![];
    let mut row = vec![];
    let mut cell = String::new();
    let mut quoted = false;
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '"' if quoted && it.peek() == Some(&'"') => {
                it.next();
                cell.push('"')
            }
            '"' => quoted = !quoted,
            c if c == sep && !quoted => row.push(std::mem::take(&mut cell)),
            '\r' if !quoted => {}
            '\n' if !quoted => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            _ => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows.retain(|r| r.iter().any(|c| !c.trim().is_empty()));
    rows
}

fn typed(cell: &str, sample: Option<&Value>) -> Value {
    let c = cell.trim();
    match sample {
        Some(Value::Number(_)) => c.parse::<i64>().map(Value::from).or_else(|_| c.parse::<f64>().map(Value::from)).unwrap_or(json!(c)),
        Some(Value::Bool(_)) => match c.to_lowercase().as_str() {
            "true" | "si" | "sí" | "1" => json!(true),
            "false" | "no" | "0" => json!(false),
            _ => json!(c),
        },
        Some(Value::Array(_)) | Some(Value::Object(_)) => serde_json::from_str(c).unwrap_or(json!(c)),
        _ if c == "null" => Value::Null,
        _ => json!(cell),
    }
}

/// CSV → casos. Columnas `_nombre`, `_status` y `_espera.<ruta>` (`contains x`, `gt 3`…); `a.b` es un campo anidado. Los tipos salen
/// de la entrada de ejemplo; una celda vacía conserva el valor de ejemplo.
pub fn csv_to_cases(text: &str, sample: &Value) -> Result<Vec<Case>, String> {
    let rows = parse_csv(text);
    let (head, body) = rows.split_first().ok_or("El CSV está vacío")?;
    if body.is_empty() {
        return Err("El CSV solo tiene cabecera".into());
    }
    let mut out = vec![];
    for (i, r) in body.iter().enumerate() {
        let mut input = if sample.is_object() { sample.clone() } else { json!({}) };
        let mut c = Case { id: format!("c{}", i + 1), name: format!("Caso {}", i + 1), ..Default::default() };
        for (col, cell) in head.iter().zip(r.iter()) {
            let col = col.trim();
            match col {
                "_nombre" | "_name" if !cell.trim().is_empty() => c.name = cell.trim().into(),
                "_status" if !cell.trim().is_empty() => c.expect_status = Some(cell.trim().parse().map_err(|_| format!("Fila {}: _status «{cell}» no es un número", i + 2))?),
                _ if col.starts_with("_espera.") && !cell.trim().is_empty() => {
                    let (op, right) = split_assert(cell.trim());
                    c.expect.push(Rule { left: col["_espera.".len()..].into(), op, right });
                }
                _ if col.starts_with('_') || cell.trim().is_empty() => {}
                _ => set_path(&mut input, col, typed(cell, get_path(sample, col))),
            }
        }
        c.input = input;
        out.push(c);
    }
    Ok(out)
}

fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n', ';']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.into()
    }
}

/// Plantilla CSV con las columnas de la entrada y una fila de ejemplo.
pub fn csv_template(sample: &Value) -> String {
    let leaves = leaf_paths(sample);
    let mut head = vec!["_nombre".to_string(), "_status".to_string()];
    let mut row = vec!["Caso 1".to_string(), "200".to_string()];
    for (p, v) in leaves.into_iter().filter(|(p, _)| !p.is_empty()) {
        head.push(p);
        row.push(csv_cell(&value_to_text(&v)));
    }
    format!("{}\n{}\n", head.join(","), row.join(","))
}

pub fn to_csv(rows: &[Vec<String>]) -> String {
    rows.iter().map(|r| r.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(",")).collect::<Vec<_>>().join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_cases() {
        let sample = json!({"doc": "1", "edad": 30, "vip": false, "dir": {"zona": "A"}});
        let csv = "_nombre;_status;doc;edad;vip;dir.zona\nMenor;400;\"9;9\";15;si;\nOk;;77;40;no;B\n";
        let cases = csv_to_cases(csv, &sample).unwrap();
        assert_eq!(cases.len(), 2);
        assert_eq!(cases[0].name, "Menor");
        assert_eq!(cases[0].expect_status, Some(400));
        assert_eq!(cases[0].input, json!({"doc": "9;9", "edad": 15, "vip": true, "dir": {"zona": "A"}}));
        assert_eq!(cases[1].input["dir"]["zona"], json!("B"));
        assert_eq!(cases[1].expect_status, None);
        let t = csv_template(&sample);
        assert!(t.starts_with("_nombre,_status,doc,edad,vip,dir.zona\n"));
        assert_eq!(csv_to_cases(&t, &sample).unwrap()[0].input, sample);
        assert_eq!(parse_csv("a\tb\n1\t2")[1], ["1", "2"]);
    }
}
