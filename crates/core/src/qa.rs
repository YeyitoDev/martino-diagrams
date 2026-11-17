//! Lo que espera un caso QA, evaluado contra el resultado de una ejecución.
use crate::flow::{check, Case, Rule};
use crate::interp::get_path;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Check {
    pub left: String,
    pub op: String,
    pub right: String,
    pub actual: Value,
    pub ok: bool,
}

/// Evalúa lo que espera un caso contra el resultado (`end_key` = clave del nodo Respuesta que ganó).
pub fn case_checks(c: &Case, status: Option<u16>, body: &Value, ms: u64, end_key: Option<&str>) -> Vec<Check> {
    let status_rule = c.expect_status.map(|s| Rule { left: "status".into(), op: "eq".into(), right: s.to_string() });
    status_rule.iter().chain(&c.expect).map(|r| {
        let actual = match r.left.as_str() {
            "status" => status.map(|s| json!(s)),
            "ms" => Some(json!(ms)),
            "nodo" => end_key.map(|k| json!(k)),
            p => get_path(body, p.strip_prefix("response.").or(p.strip_prefix("body.")).unwrap_or(p)).cloned(),
        };
        Check { left: r.left.clone(), op: r.op.clone(), right: r.right.clone(), ok: check(actual.as_ref(), &r.op, &r.right), actual: actual.unwrap_or(Value::Null) }
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::csv_to_cases;

    #[test]
    fn case_expectations() {
        let c = &csv_to_cases("_status,_espera.error,_espera.ms,doc\n400,contains formato,lt 500,x\n", &json!({"doc": ""})).unwrap()[0];
        assert_eq!(c.expect, vec![Rule { left: "error".into(), op: "contains".into(), right: "formato".into() }, Rule { left: "ms".into(), op: "lt".into(), right: "500".into() }]);
        let ok = |st, body: Value, ms| case_checks(c, Some(st), &body, ms, None).iter().map(|k| k.ok).collect::<Vec<_>>();
        assert_eq!(ok(400, json!({"error": "mal formato"}), 120), [true, true, true]);
        assert_eq!(ok(200, json!({}), 900), [false, false, false]);
        let n = Case { expect: vec![Rule { left: "nodo".into(), op: "eq".into(), right: "no_existe".into() }, Rule { left: "response.a.b".into(), op: "eq".into(), right: "1".into() }], ..Default::default() };
        assert!(case_checks(&n, None, &json!({"a": {"b": 1}}), 0, Some("no_existe")).iter().all(|k| k.ok));
    }
}
