//! The extensions Vorn ships with, compiled in and written against the
//! public API, and the switch each has. A switch is kept in `builtins.json`
//! beside `vorn.db`; off, a built-in answers nothing, offers nothing and
//! runs nothing, and what it keeps stays where it is for when it is on again.
//!
//! Connectors is the one here ([`connectors`]).

pub mod connectors;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde_json::{json, Value};
use tracing::warn;

use super::{Answer, Native};

/// The calls about the built-ins themselves.
pub const METHODS: &[&str] = &["extension:builtins", "extension:setBuiltin"];

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Each built-in's switch, as the person left it.
#[derive(Debug)]
pub struct Switches {
    file: PathBuf,
    on: Mutex<BTreeMap<String, bool>>,
}

impl Switches {
    /// The switches kept in `file`; a missing or unreadable file leaves every built-in at its default.
    pub fn load(file: PathBuf) -> Switches {
        let on = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Switches {
            file,
            on: Mutex::new(on),
        }
    }

    /// Whether `id` is on, `default` when the person never switched it.
    pub fn is_on(&self, id: &str, default: bool) -> bool {
        lock(&self.on).get(id).copied().unwrap_or(default)
    }

    /// Keeps `id` switched `on`.
    pub fn set(&self, id: &str, on: bool) {
        let saved = {
            let mut map = lock(&self.on);
            map.insert(id.to_owned(), on);
            serde_json::to_vec_pretty(&*map).unwrap_or_default()
        };
        if let Err(err) = std::fs::write(&self.file, saved) {
            warn!("[extensions] could not keep the built-ins' switches: {err}");
        }
    }
}

/// The MCP tools the built-ins that are on add, under the names Vorn's own
/// tools have always had.
pub fn mcp_tools(native: &Native) -> Vec<Value> {
    match native.connectors() {
        Some(c) if c.is_on() => vorn_connectors::tools::listed().to_vec(),
        _ => Vec::new(),
    }
}

/// An agent called tool `name`: the built-in's answer, `None` when no
/// built-in that is on adds it.
pub async fn mcp_call(native: &Native, name: &str, arguments: Value) -> Option<Value> {
    let connectors = native.connectors().filter(|c| c.is_on())?;
    vorn_connectors::tools::call(connectors.inner(), name, arguments).await
}

/// Answers one of [`METHODS`].
pub async fn answer(native: &Arc<Native>, method: &str, params: &Value) -> Answer {
    let Some(connectors) = native.connectors().cloned() else {
        return Answer::Unanswered;
    };
    match method {
        "extension:builtins" => Answer::Result(json!([connectors.listed()])),
        "extension:setBuiltin" => {
            let id = params.get("id").and_then(Value::as_str).unwrap_or("");
            let Some(on) = params.get("on").and_then(Value::as_bool) else {
                return Answer::Error("extension:setBuiltin takes on: true or false".to_owned());
            };
            if id != vorn_connectors::builtin::ID {
                return Answer::Error(format!("Vorn ships no extension \"{id}\""));
            }
            connectors.set_on(on).await;
            native.broadcast("extension:builtinChanged", json!({ "id": id, "on": on }));
            Answer::Result(json!({ "id": id, "on": on }))
        }
        _ => Answer::Unanswered,
    }
}
