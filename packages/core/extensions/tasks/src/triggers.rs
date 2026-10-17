//! The workflow triggers a task's change fires, per task: a task that was not
//! there before is created, and one whose status changed moved. Each carries
//! an effect id naming the change itself, so a trigger delivered twice runs
//! once. The extension emits each as a public event when a task is written,
//! and fires the workflows watching for it with `workflow.fire`.

use std::cmp::Ordering;

use serde_json::{json, Value};

use crate::js;

/// The trigger `task` fires over what it was before (`prior`, `None` for a
/// new task): `{effectId, kind, task, from?, to?}`, or `None` when its status
/// did not move.
pub fn for_task(prior: Option<&Value>, task: &Value) -> Option<Value> {
    let id = text(task.get("id"));
    let Some(prior) = prior else {
        return Some(json!({
            "effectId": format!("task-created/{id}"),
            "kind": "taskCreated",
            "task": task,
        }));
    };
    if task.get("status") == prior.get("status") {
        return None;
    }
    // A client that has not caught up with a status a step just set would otherwise read as a move back.
    if less(task.get("updatedAt"), prior.get("updatedAt")) {
        return None;
    }
    let (from, to) = (text(prior.get("status")), text(task.get("status")));
    let at = text(task.get("updatedAt"));
    let mut trigger = json!({
        "effectId": format!("task-status/{id}/{from}/{to}/{at}"),
        "kind": "taskStatusChanged",
        "task": task,
    });
    // An absent status leaves `from` or `to` out.
    for (name, value) in [("from", prior.get("status")), ("to", task.get("status"))] {
        if let Some(value) = value {
            trigger[name] = value.clone();
        }
    }
    Some(trigger)
}

/// What a trigger from [`for_task`] is fired with (`workflow.fire`): its
/// trigger type, the effect id it is received once by, the fields its
/// workflows filter on (the project, and a move's from and to) and the run's
/// context. `None` for a move to the status it was already in.
pub fn fire_args(trigger: &Value) -> Option<Value> {
    let kind = trigger.get("kind").and_then(Value::as_str)?;
    let task = trigger.get("task").filter(|t| t.is_object())?;
    let project = task
        .get("projectName")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty());
    let effect = trigger.get("effectId").cloned().unwrap_or(Value::Null);
    match kind {
        "taskCreated" => Some(json!({
            "trigger": kind,
            "effectId": effect,
            "fields": { "projectFilter": project },
            "context": { "task": task, "trigger": { "type": kind } },
        })),
        "taskStatusChanged" => {
            let side = |k: &str| {
                trigger
                    .get(k)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            let (from, to) = (side("from"), side("to"));
            if from == to {
                return None;
            }
            Some(json!({
                "trigger": kind,
                "effectId": effect,
                "fields": { "projectFilter": project, "fromStatus": from, "toStatus": to },
                "context": {
                    "task": task,
                    "trigger": { "type": kind, "fromStatus": from, "toStatus": to },
                },
            }))
        }
        _ => None,
    }
}

/// `value` as text, `undefined` when it is absent.
fn text(value: Option<&Value>) -> String {
    value.map_or_else(|| "undefined".to_owned(), js::to_string)
}

/// Whether `a` sorts before `b`, for what a task's `updatedAt` holds:
/// strings by UTF-16 units, numbers by value.
fn less(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (Some(Value::String(a)), Some(Value::String(b))) => {
            a.encode_utf16().cmp(b.encode_utf16()) == Ordering::Less
        }
        (Some(Value::Number(a)), Some(Value::Number(b))) => {
            matches!((a.as_f64(), b.as_f64()), (Some(a), Some(b)) if a < b)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, status: &str, at: &str) -> Value {
        json!({ "id": id, "status": status, "updatedAt": at, "title": id })
    }

    #[test]
    fn a_new_task_is_created_and_a_moved_one_changed_status() {
        let moved = task("b", "in_progress", "2030-01-02");
        assert_eq!(
            for_task(Some(&task("b", "todo", "2030-01-01")), &moved),
            Some(json!({
                "effectId": "task-status/b/todo/in_progress/2030-01-02",
                "kind": "taskStatusChanged",
                "task": moved,
                "from": "todo",
                "to": "in_progress",
            }))
        );
        let made = task("c", "todo", "2030-01-02");
        assert_eq!(
            for_task(None, &made),
            Some(json!({ "effectId": "task-created/c", "kind": "taskCreated", "task": made }))
        );
        let same = task("a", "todo", "2030-01-02");
        assert_eq!(
            for_task(Some(&task("a", "todo", "2030-01-01")), &same),
            None
        );
    }

    #[test]
    fn a_stale_client_moving_a_task_back_fires_nothing() {
        let done = task("a", "done", "2030-01-02T00:00:00.000Z");
        assert!(for_task(Some(&done), &task("a", "todo", "2030-01-01T00:00:00.000Z")).is_none());
        // The same instant is not older: the move counts.
        assert!(for_task(Some(&done), &task("a", "todo", "2030-01-02T00:00:00.000Z")).is_some());
    }
}
