//! The status widget's list of agents (`widget:status-update`): each
//! terminal's id, agent, name, project and status, told half a second after
//! the registry first changes or a client asks (`widget:requestUpdate`), so a
//! burst of changes is told once.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value};
use tokio::sync::broadcast::error::RecvError;

use super::{Answer, Native};
use crate::registry::{Registry, SessionRegistry};

/// Every call this module answers.
pub const METHODS: &[&str] = &["widget:requestUpdate"];

/// How long a change waits for the ones after it.
const SETTLE: Duration = Duration::from_millis(500);

const FIELDS: [&str; 5] = ["id", "agentType", "displayName", "projectName", "status"];

/// `widget:requestUpdate`: the list is told once it settles.
pub fn answer(native: &Native) -> Answer {
    native.widget.notify_one();
    Answer::Void
}

/// The widget's entries, in the order `terminal:listActive` lists them.
pub fn agents(registry: &Registry) -> Value {
    let entry = |record: Value| {
        let picked: Map<String, Value> = FIELDS
            .iter()
            .filter_map(|k| Some(((*k).to_owned(), record.get(*k)?.clone())))
            .collect();
        Value::Object(picked)
    };
    registry
        .terminals()
        .into_iter()
        .map(|t| entry(serde_json::to_value(t).unwrap_or(Value::Null)))
        .collect()
}

/// Tells the list whenever the registry changes or a client asks, until the
/// registry goes.
pub async fn follow(native: Arc<Native>, registry: Arc<SessionRegistry>) {
    let mut notes = registry.subscribe();
    loop {
        tokio::select! {
            note = notes.recv() => if matches!(note, Err(RecvError::Closed)) { return },
            () = native.widget.notified() => {}
        }
        tokio::time::sleep(SETTLE).await;
        // What changed while it settled is in the list told now.
        notes = notes.resubscribe();
        if let Some(list) = registry.read(agents) {
            native.broadcast("widget:status-update", list);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Gen, TerminalSession};
    use serde_json::json;

    #[test]
    fn lists_each_terminal_by_the_fields_the_widget_draws() {
        let mut r = Registry::new(Gen::draw());
        r.decide_statuses();
        let record: TerminalSession = serde_json::from_value(json!({
            "id": "a", "agentType": "claude", "projectName": "p", "projectPath": "/p",
            "status": "running", "displayName": "mine", "pid": 1, "createdAt": 0
        }))
        .unwrap();
        r.create(record).unwrap();
        assert_eq!(
            agents(&r),
            json!([{ "id": "a", "agentType": "claude", "displayName": "mine", "projectName": "p", "status": "running" }])
        );
    }
}
