//! Tasks and the workflow step loop as built-in extensions run them, natively or in a component, on vorn-work's own files.

#[path = "../../../../crates/work/src/graph.rs"]
pub mod graph;
#[path = "../../../../crates/work/src/items.rs"]
pub mod items;
#[path = "../../../../crates/work/src/js.rs"]
pub mod js;
#[path = "../../../../crates/work/src/model.rs"]
pub mod model;
#[path = "../../../../crates/work/src/tasks.rs"]
pub mod tasks;
#[path = "../../../../crates/work/src/template.rs"]
pub mod template;

pub mod methods;
pub mod workflows;

use serde_json::Value;

/// `vorn_work::js_numbers`, which the included files call by this path.
pub fn js_numbers(value: Value) -> Value {
    const SAFE: f64 = 9_007_199_254_740_992.0;
    match value {
        Value::Number(n) => match n.as_f64() {
            Some(f) if !n.is_i64() && !n.is_u64() && f.fract() == 0.0 && f.abs() < SAFE => {
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

/// `vorn_work::is_truthy`, which the included files call by this path.
pub fn is_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}
