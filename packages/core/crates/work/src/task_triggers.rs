//! The workflow triggers a task's change fires (`taskTriggersForChange`,
//! per task): a task that was not there before is created, and one whose
//! status changed moved. Each carries an effect id naming the change itself,
//! so a trigger delivered twice runs once ([`crate::receipts`]). vornd emits
//! each as a public event when a task is written, and the work model fires
//! it from there.

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
    // An absent status is `undefined`, which JSON leaves out.
    for (name, value) in [("from", prior.get("status")), ("to", task.get("status"))] {
        if let Some(value) = value {
            trigger[name] = value.clone();
        }
    }
    Some(trigger)
}

/// A value in a template literal.
fn text(value: Option<&Value>) -> String {
    value.map_or_else(|| "undefined".to_owned(), js::to_string)
}

/// JavaScript's `a < b` for what a task's `updatedAt` holds.
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
        assert_eq!(for_task(Some(&task("a", "todo", "2030-01-01")), &same), None);
    }

    #[test]
    fn a_stale_client_moving_a_task_back_fires_nothing() {
        let done = task("a", "done", "2030-01-02T00:00:00.000Z");
        assert!(for_task(Some(&done), &task("a", "todo", "2030-01-01T00:00:00.000Z")).is_none());
        // The same instant is not older: the move counts.
        assert!(for_task(Some(&done), &task("a", "todo", "2030-01-02T00:00:00.000Z")).is_some());
    }
}
