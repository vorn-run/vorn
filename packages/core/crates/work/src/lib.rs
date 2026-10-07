//! Vorn's work model without a host: when workflows fire, how a fire is
//! received once however often it is delivered, and the reads of workflows,
//! their runs, the schedule log and artifacts, from the server's `vorn.db`.
//!
//! The server's TypeScript is the reference. Every reply here is what its
//! handler returns for the same database, or [`reads::Reply::Server`] where
//! only the server can tell; nothing here writes a workflow, a run or an
//! artifact.

pub mod cron;
pub mod reads;
pub mod receipts;
pub mod schedule;
pub mod trigger;

use serde_json::Value;

/// `value` with every whole number JavaScript would print without a
/// fraction made an integer, as the server's JSON carries it. The store
/// reads numeric columns as `f64`.
pub fn js_numbers(value: Value) -> Value {
    const SAFE: f64 = 9_007_199_254_740_992.0;
    match value {
        Value::Number(n) => match n.as_f64() {
            Some(f) if !n.is_i64() && !n.is_u64() && f.fract() == 0.0 && f.abs() < SAFE => {
                // `-0` prints as `0`.
                Value::from(f as i64)
            }
            _ => Value::Number(n),
        },
        Value::Array(items) => Value::Array(items.into_iter().map(js_numbers).collect()),
        Value::Object(map) => {
            Value::Object(map.into_iter().map(|(k, v)| (k, js_numbers(v))).collect())
        }
        other => other,
    }
}

/// JavaScript truthiness of a field.
pub(crate) fn is_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn whole_floats_become_integers_and_the_rest_stay() {
        let got = js_numbers(json!({ "a": [3.0, 2.5, -0.0, 1e300], "b": { "c": 7 }, "d": "3.0" }));
        assert_eq!(
            got,
            json!({ "a": [3, 2.5, 0, 1e300], "b": { "c": 7 }, "d": "3.0" })
        );
        assert!(got["a"][0].is_i64());
    }
}
