//! Settings Vorn kept in `defaults` for Shell blocks, Workflows and Tasks,
//! moved once into those extensions' own preferences, and settings nothing
//! reads any more; both are then dropped from `defaults`.

use std::collections::BTreeMap;

use serde_json::{Map, Value};
use tracing::warn;
use vorn_extensions::router::Router;

use crate::native::Native;

/// Shell blocks, whose Minimal prompt preference vornd reads as a shell starts.
pub const BLOCKS: &str = "vorn.blocks";
/// Its preference hiding the shell's own prompt.
pub const MINIMAL_PROMPT: &str = "minimalShellPrompt";

/// Each moved setting: its `defaults` key, the extension and that extension's preference id.
const MOVED: &[(&str, &str, &str)] = &[
    (MINIMAL_PROMPT, BLOCKS, MINIMAL_PROMPT),
    (
        vorn_workflows::STEP_TIME_LIMIT,
        vorn_workflows::ID,
        vorn_workflows::STEP_TIME_LIMIT,
    ),
    (
        "showHeadlessAgents",
        vorn_workflows::ID,
        "showHeadlessAgents",
    ),
    (
        "headlessRetentionMinutes",
        vorn_workflows::ID,
        "headlessRetentionMinutes",
    ),
    ("taskViewMode", "vorn.tasks", "default_view"),
];

/// Settings nothing reads: Command Blocks is the Blocks extension's own
/// switch, and updates always download.
const GONE: &[&str] = &[
    "domBlockRendering",
    "keepSessionsRunning",
    "enableHoverPreview",
    "updateAutoDownload",
];

/// The moved settings among `stored`, by extension, as their preferences take them.
fn moved(stored: &Map<String, Value>) -> BTreeMap<&'static str, Map<String, Value>> {
    let mut out: BTreeMap<&str, Map<String, Value>> = BTreeMap::new();
    for (key, extension, preference) in MOVED {
        let Some(value) = stored.get(*key).and_then(|v| preference_value(key, v)) else {
            continue;
        };
        out.entry(extension)
            .or_default()
            .insert((*preference).to_owned(), value);
    }
    out
}

/// `value` of `defaults` key `key` as its preference takes it: minutes become
/// a duration choice's `m<minutes>` id, and the board view is `board`.
fn preference_value(key: &str, value: &Value) -> Option<Value> {
    match key {
        "headlessStepTimeoutMinutes" | "headlessRetentionMinutes" => {
            let minutes = value.as_f64().filter(|m| *m >= 0.0)?;
            Some(Value::String(format!("m{}", minutes.round() as u64)))
        }
        "taskViewMode" => match value.as_str()? {
            "kanban" => Some("board".into()),
            "list" => Some("list".into()),
            _ => None,
        },
        _ => value.is_boolean().then(|| value.clone()),
    }
}

/// Moves what `defaults` still holds of [`MOVED`] into the extensions'
/// preferences, then drops those rows and [`GONE`]'s. Blocking: sqlite.
pub fn migrate(native: &Native, router: &Router) {
    let Some(db) = native.database() else { return };
    let store = match vorn_store::Store::open_beside(db) {
        Ok(Some(store)) => store,
        Ok(None) => return,
        Err(err) => {
            warn!(%err, "could not open the database to move settings");
            return;
        }
    };
    let stored = match store.stored_defaults() {
        Ok(stored) => stored,
        Err(err) => {
            warn!(%err, "could not read the settings to move");
            return;
        }
    };
    let dropped: Vec<&str> = MOVED
        .iter()
        .map(|(key, ..)| *key)
        .chain(GONE.iter().copied())
        .filter(|key| stored.contains_key(*key))
        .collect();
    if dropped.is_empty() {
        return;
    }
    for (extension, values) in moved(&stored) {
        router.adopt_preferences(extension, values);
    }
    if let Err(err) = store.drop_defaults(&dropped) {
        warn!(%err, "could not drop the settings that moved");
    }
}

/// Whether a new shell hides its own prompt: Shell blocks' Minimal prompt, on
/// unless it is installed and turned off.
pub fn minimal_prompt(native: &Native) -> bool {
    native
        .extensions()
        .and_then(|e| e.router().preference(BLOCKS, MINIMAL_PROMPT))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn each_moved_setting_lands_on_its_extension_as_its_preference_takes_it() {
        let stored = json!({
            "minimalShellPrompt": false,
            "headlessStepTimeoutMinutes": 0,
            "showHeadlessAgents": false,
            "headlessRetentionMinutes": 30,
            "taskViewMode": "kanban",
            "domBlockRendering": true,
            "shell": "/bin/zsh"
        });
        let moved = moved(stored.as_object().unwrap());
        assert_eq!(
            serde_json::to_value(&moved).unwrap(),
            json!({
                "vorn.blocks": { "minimalShellPrompt": false },
                "vorn.tasks": { "default_view": "board" },
                "vorn.workflows": {
                    "headlessStepTimeoutMinutes": "m0",
                    "showHeadlessAgents": false,
                    "headlessRetentionMinutes": "m30"
                }
            })
        );
    }

    #[test]
    fn a_value_of_the_wrong_shape_is_not_moved() {
        let stored = json!({
            "minimalShellPrompt": "yes",
            "headlessRetentionMinutes": -5,
            "taskViewMode": "grid"
        });
        assert!(moved(stored.as_object().unwrap()).is_empty());
    }
}
