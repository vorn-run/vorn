//! The workflow triggers a configuration save fires (`taskTriggersForChange`):
//! a task that was not there before is created, and one whose status changed
//! moved. Each carries an effect id naming the change itself, so a trigger
//! delivered twice runs once ([`crate::receipts`]).

use std::cmp::Ordering;
use std::collections::HashMap;

use serde_json::{json, Value};

use crate::js;

/// The triggers `after` fires over `before`, both whole configurations, in
/// the order of `after`'s tasks. Each is `{effectId, kind, task, from?, to?}`.
pub fn for_change(before: &Value, after: &Value) -> Vec<Value> {
    let previous: HashMap<String, &Value> = tasks(before)
        .map(|t| (key(t.get("id")), t))
        .collect();
    let mut triggers = Vec::new();
    for task in tasks(after) {
        let id = text(task.get("id"));
        let Some(prior) = previous.get(&key(task.get("id"))) else {
            triggers.push(json!({
                "effectId": format!("task-created/{id}"),
                "kind": "taskCreated",
                "task": task,
            }));
            continue;
        };
        if task.get("status") == prior.get("status") {
            continue;
        }
        // A client that has not caught up with a status a step just set would otherwise read as a move back.
        if less(task.get("updatedAt"), prior.get("updatedAt")) {
            continue;
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
        triggers.push(trigger);
    }
    triggers
}

fn tasks(config: &Value) -> impl Iterator<Item = &Value> {
    config
        .get("tasks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

/// A task id as a `Map` key: its JSON, so `1` and `"1"` stay apart.
fn key(id: Option<&Value>) -> String {
    id.map_or_else(|| "undefined".to_owned(), Value::to_string)
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
        let before = json!({ "tasks": [task("a", "todo", "2030-01-01"), task("b", "todo", "2030-01-01")] });
        let after = json!({ "tasks": [
            task("a", "todo", "2030-01-02"),
            task("b", "in_progress", "2030-01-02"),
            task("c", "todo", "2030-01-02"),
        ] });
        let triggers = for_change(&before, &after);
        assert_eq!(
            triggers,
            vec![
                json!({
                    "effectId": "task-status/b/todo/in_progress/2030-01-02",
                    "kind": "taskStatusChanged",
                    "task": task("b", "in_progress", "2030-01-02"),
                    "from": "todo",
                    "to": "in_progress",
                }),
                json!({ "effectId": "task-created/c", "kind": "taskCreated", "task": task("c", "todo", "2030-01-02") }),
            ]
        );
    }

    #[test]
    fn a_stale_client_moving_a_task_back_fires_nothing() {
        let before = json!({ "tasks": [task("a", "done", "2030-01-02T00:00:00.000Z")] });
        let after = json!({ "tasks": [task("a", "todo", "2030-01-01T00:00:00.000Z")] });
        assert!(for_change(&before, &after).is_empty());
        // The same instant is not older: the move counts.
        let same = json!({ "tasks": [task("a", "todo", "2030-01-02T00:00:00.000Z")] });
        assert_eq!(for_change(&before, &same).len(), 1);
    }

    #[test]
    fn missing_or_odd_task_lists_fire_nothing_or_everything_new() {
        assert!(for_change(&json!({}), &json!({})).is_empty());
        assert!(for_change(&json!({ "tasks": null }), &json!({ "tasks": 3 })).is_empty());
        let created = for_change(&json!({}), &json!({ "tasks": [{ "id": 7 }] }));
        assert_eq!(created[0]["effectId"], "task-created/7");
        // A number id and a string id are different tasks, as in a Map.
        let both = for_change(
            &json!({ "tasks": [{ "id": 7, "status": "todo" }] }),
            &json!({ "tasks": [{ "id": "7", "status": "todo" }] }),
        );
        assert_eq!(both[0]["kind"], "taskCreated");
    }
}
