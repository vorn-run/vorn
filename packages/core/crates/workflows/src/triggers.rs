//! The trigger the built-in fires itself: `sessionRestored`, when a session
//! comes back after a restart. A cold restore fires the workflows that ask
//! for cold ones (all of them, unless one asks for warm ones too); runs of
//! one project start one at a time, in order, unless the workflow lets them
//! run together.

use serde_json::{Map, Value};
use vorn_work::model::Context;
use vorn_workflow::{Engine, Host};

/// The trigger type it fires.
pub const SESSION_RESTORED: &str = "sessionRestored";

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// `fireSessionRestoredTrigger`: `session` (its record) came back `restore`
/// (`cold` or `warm`), with what its environment says.
pub async fn session_restored<H: Host>(
    engine: &Engine<H>,
    session: &Value,
    restore: &str,
    environment: Option<Value>,
) {
    if engine.registry().trigger(SESSION_RESTORED).is_none() {
        return;
    }
    let project = text(session, "projectName").unwrap_or_default().to_owned();
    for (workflow, trigger) in engine.armed(SESSION_RESTORED).await {
        if text(&trigger, "projectFilter").is_some_and(|f| f != project) {
            continue;
        }
        if text(&trigger, "restore").unwrap_or("cold") == "cold" && restore == "warm" {
            continue;
        }
        let mut t = Map::new();
        t.insert("type".into(), SESSION_RESTORED.into());
        t.insert("restore".into(), restore.into());
        if let Some(env) = environment.clone().filter(|e| !e.is_null()) {
            t.insert("environment".into(), env);
        }
        let context = Context {
            source: Some(session.clone()),
            trigger: Some(Value::Object(t)),
            ..Context::default()
        };
        if text(&trigger, "concurrency") == Some("unbounded") {
            engine.start(workflow, Some(context), None);
        } else {
            engine.start_in_turn(&project, workflow, context);
        }
    }
}
