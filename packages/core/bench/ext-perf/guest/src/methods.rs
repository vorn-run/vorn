//! The `task:*` calls an extension answers, routed to vorn-work's Tasks code as vornd routes them.

use serde_json::Value;
use vorn_store::{Result, Store};

use crate::tasks;

/// The calls Tasks answers as an extension.
pub const METHODS: &[&str] = &[
    "task:list",
    "task:get",
    "task:setStatus",
    "task:create",
    "task:update",
    "task:delete",
    "task:reorder",
    "task:archive",
];

/// One call: its answer (`None` when the method is not Tasks') and whether the board changed.
pub fn call(
    store: &mut Store,
    method: &str,
    params: &Value,
    now: &str,
    id: String,
) -> Result<(Option<Value>, bool)> {
    let answered = |(v, changed): (Value, bool)| (Some(v), changed);
    Ok(match method {
        "task:list" => (Some(tasks::list(store, params)?), false),
        "task:get" => (Some(tasks::get(store, params)?), false),
        "task:setStatus" => answered(tasks::set_status(store, params, now)?),
        "task:create" => answered(tasks::create(store, params, id, now)?),
        "task:update" => answered(tasks::update(store, params, now)?),
        "task:delete" => answered(tasks::delete(store, params)?),
        "task:reorder" => answered(tasks::reorder(store, params, now)?),
        "task:archive" => answered(tasks::archive(store, params, now)?),
        _ => (None, false),
    })
}
