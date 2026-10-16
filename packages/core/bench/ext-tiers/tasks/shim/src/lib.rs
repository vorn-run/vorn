//! The parts of `vorn-store` and `vorn-work` the Tasks built-in's code
//! names, for a guest that has no database: [`Store::call`] goes over the
//! bridge (`task.store`), and [`js`] is vorn-work's own file. The guest
//! compiles E6's `calls.rs` and `triggers.rs` unchanged against it.

use serde_json::Value;

#[path = "../../../../../crates/work/src/js.rs"]
pub mod js;

/// The board's tables, reached through whatever answers `call`.
pub struct Store<'a> {
    call: &'a mut dyn FnMut(&str, Value) -> std::result::Result<Value, String>,
}

impl<'a> Store<'a> {
    pub fn new(call: &'a mut dyn FnMut(&str, Value) -> std::result::Result<Value, String>) -> Self {
        Store { call }
    }

    /// `vorn_store::Store::call`: the store call `name` with its arguments.
    pub fn call(&mut self, name: &str, args: Value) -> Result<Value> {
        (self.call)(name, args).map_err(Error)
    }
}

/// A store call that failed, as the bridge worded it.
#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// `vorn_work::js_numbers`, which `js.rs` uses.
pub fn js_numbers(value: Value) -> Value {
    const SAFE: f64 = 9_007_199_254_740_992.0;
    match value {
        Value::Number(n) => match n.as_f64() {
            #[allow(clippy::cast_possible_truncation)]
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
