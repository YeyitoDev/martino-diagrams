//! Tablas de equivalencia: CSV en `tablas/` de la colección y la búsqueda de una fila.
use crate::csv::parse_csv;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LookupSpec {
    /// Nombre de la tabla (archivo sin `.csv`).
    pub table: String,
    /// Plantilla con el valor a buscar (`{{cuenta.response.banco}}`).
    pub value: String,
    /// Columna donde se busca; vacío = la primera.
    pub column: String,
    /// Si no está: `error` (por defecto) | `keep` (deja el valor igual) | `default`.
    pub on_miss: String,
    pub default: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    pub cols: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

pub fn parse_table(csv: &str) -> Table {
    let mut rows = parse_csv(csv);
    if rows.is_empty() {
        return Table::default();
    }
    let cols: Vec<String> = rows.remove(0).into_iter().map(|c| c.trim().to_string()).collect();
    Table { cols, rows }
}

impl Table {
    pub fn to_csv(&self) -> String {
        crate::csv::to_csv(&std::iter::once(self.cols.clone()).chain(self.rows.iter().cloned()).collect::<Vec<_>>())
    }

    fn col(&self, name: &str) -> usize {
        self.cols.iter().position(|c| c == name).unwrap_or(0)
    }

    fn row_obj(&self, row: &[String]) -> Value {
        Value::Object(self.cols.iter().enumerate().map(|(i, c)| (c.clone(), Value::String(row.get(i).cloned().unwrap_or_default()))).collect::<Map<_, _>>())
    }

    /// Fila (como objeto `{columna: valor}`) cuyo valor en `column` coincide sin mirar mayúsculas ni espacios.
    pub fn find(&self, column: &str, value: &str) -> Option<Value> {
        let (i, v) = (self.col(column), norm(value));
        self.rows.iter().find(|r| r.get(i).is_some_and(|c| norm(c) == v)).map(|r| self.row_obj(r))
    }

    /// Busca y aplica la regla de «si no está».
    pub fn lookup(&self, spec: &LookupSpec, value: &str, name: &str) -> Result<Value, String> {
        if let Some(r) = self.find(&spec.column, value) {
            return Ok(r);
        }
        let fill = match spec.on_miss.as_str() {
            "keep" => value,
            "default" => spec.default.as_str(),
            _ => {
                let col = self.cols.get(self.col(&spec.column)).map(String::as_str).unwrap_or("?");
                return Err(format!("«{value}» no está en la tabla {name} (columna {col})"));
            }
        };
        let i = self.col(&spec.column);
        Ok(self.row_obj(&self.cols.iter().enumerate().map(|(j, _)| if j == i { value.to_string() } else { fill.to_string() }).collect::<Vec<_>>()))
    }
}

fn norm(s: &str) -> String {
    s.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lookup_rules() {
        let t = parse_table("codigo,nombre,swift\n002,BCP,BCPLPEPL\n003,Interbank,BINPPEPL\n");
        let s = |on_miss: &str, column: &str| LookupSpec { on_miss: on_miss.into(), column: column.into(), default: "?".into(), ..Default::default() };
        assert_eq!(t.lookup(&s("", ""), " 003 ", "bancos").unwrap()["nombre"], json!("Interbank"));
        assert_eq!(t.lookup(&s("", "nombre"), "bcp", "bancos").unwrap()["codigo"], json!("002"), "al revés");
        assert_eq!(t.lookup(&s("", ""), "999", "bancos").unwrap_err(), "«999» no está en la tabla bancos (columna codigo)");
        assert_eq!(t.lookup(&s("keep", ""), "999", "bancos").unwrap(), json!({"codigo": "999", "nombre": "999", "swift": "999"}));
        assert_eq!(t.lookup(&s("default", ""), "999", "bancos").unwrap()["nombre"], json!("?"));
        assert_eq!(parse_table(&t.to_csv()), t);
    }
}
