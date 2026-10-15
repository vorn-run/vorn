//! The calls that carry a change to the app's tasks and workflows to vornd.
//!
//! The app changes its own copy of a task or workflow at once and shows it;
//! vornd keeps neither in the configuration, so the change goes as the task
//! and workflow calls that make vornd's rows what the copy now holds: a new
//! row is created, a gone one deleted, and a changed one patched with only
//! the fields that moved.

use serde_json::{json, Map, Value};

/// The fields `task:create` takes; anything else a new task holds is patched after.
const CREATED: [&str; 8] = [
    "id",
    "projectName",
    "title",
    "description",
    "status",
    "branch",
    "useWorktree",
    "assignedAgent",
];

/// The fields `task:patch` sets.
const PATCHED: [&str; 12] = [
    "projectName",
    "title",
    "description",
    "status",
    "order",
    "branch",
    "useWorktree",
    "assignedAgent",
    "assignedSessionId",
    "agentSessionId",
    "completedAt",
    "archivedAt",
];

fn rows(list: Option<&Value>) -> Vec<&Value> {
    list.and_then(Value::as_array).into_iter().flatten().collect()
}

fn id_of(row: &Value) -> Option<&str> {
    row.get("id").and_then(Value::as_str)
}

fn find<'a>(list: &[&'a Value], id: &str) -> Option<&'a Value> {
    list.iter().copied().find(|r| id_of(r) == Some(id))
}

/// The fields of `keys` that differ between `before` and `after`, a field gone as `null`.
fn moved(before: &Value, after: &Value, keys: &[&str]) -> Map<String, Value> {
    keys.iter()
        .filter(|k| before.get(**k) != after.get(**k))
        .map(|k| ((*k).to_owned(), after.get(*k).cloned().unwrap_or(Value::Null)))
        .collect()
}

/// The calls that take vornd's task from `before` to `after`, either absent for none.
pub fn task(before: Option<&Value>, after: Option<&Value>) -> Vec<(&'static str, Value)> {
    match (before, after) {
        (None, None) => Vec::new(),
        (Some(b), None) => id_of(b)
            .map(|id| vec![("task:delete", json!({ "id": id }))])
            .unwrap_or_default(),
        (None, Some(a)) => {
            let Some(id) = id_of(a) else {
                return Vec::new();
            };
            let made: Map<String, Value> = CREATED
                .iter()
                .filter_map(|k| a.get(*k).map(|v| ((*k).to_owned(), v.clone())))
                .collect();
            let mut calls = vec![("task:create", Value::Object(made))];
            let rest: Map<String, Value> = PATCHED
                .iter()
                .filter(|k| !CREATED.contains(k) && **k != "order")
                .filter_map(|k| {
                    a.get(*k)
                        .filter(|v| !v.is_null())
                        .map(|v| ((*k).to_owned(), v.clone()))
                })
                .collect();
            if !rest.is_empty() {
                calls.push(("task:patch", json!({ "id": id, "set": rest })));
            }
            calls
        }
        (Some(b), Some(a)) => {
            let Some(id) = id_of(a) else {
                return Vec::new();
            };
            let set = moved(b, a, &PATCHED);
            if set.is_empty() {
                return Vec::new();
            }
            vec![("task:patch", json!({ "id": id, "set": set }))]
        }
    }
}

/// The calls that take vornd's tasks from the list `before` to `after`.
pub fn tasks(before: Option<&Value>, after: Option<&Value>) -> Vec<(&'static str, Value)> {
    let (before, after) = (rows(before), rows(after));
    let mut calls = Vec::new();
    for b in &before {
        if let Some(id) = id_of(b) {
            calls.extend(task(Some(b), find(&after, id)));
        }
    }
    for a in &after {
        if id_of(a).is_some_and(|id| find(&before, id).is_none()) {
            calls.extend(task(None, Some(a)));
        }
    }
    calls
}

/// The calls that take vornd's workflows from the list `before` to `after`;
/// their order is the app's alone.
pub fn workflows(before: Option<&Value>, after: Option<&Value>) -> Vec<(&'static str, Value)> {
    let (before, after) = (rows(before), rows(after));
    let mut calls = Vec::new();
    for b in &before {
        let Some(id) = id_of(b) else { continue };
        match find(&after, id) {
            None => calls.push(("workflow:delete", json!({ "id": id }))),
            Some(a) if a != *b => {
                let mut updates = a.as_object().cloned().unwrap_or_default();
                updates.shift_remove("id");
                calls.push(("workflow:update", json!({ "id": id, "updates": updates })));
            }
            Some(_) => {}
        }
    }
    for a in &after {
        if id_of(a).is_some_and(|id| find(&before, id).is_none()) {
            calls.push(("workflow:create", json!({ "workflow": a })));
        }
    }
    calls
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_change_goes_as_the_fields_that_moved() {
        let before = json!({ "id": "t", "title": "A", "status": "todo", "branch": "b", "updatedAt": "1" });
        let after = json!({ "id": "t", "title": "A", "status": "in_progress",
            "assignedSessionId": "s", "updatedAt": "2", "images": ["x.png"] });
        assert_eq!(
            task(Some(&before), Some(&after)),
            [(
                "task:patch",
                json!({ "id": "t", "set": { "status": "in_progress", "branch": null, "assignedSessionId": "s" } })
            )]
        );
        assert!(task(Some(&before), Some(&before)).is_empty());
        assert_eq!(
            task(Some(&before), None),
            [("task:delete", json!({ "id": "t" }))]
        );
    }

    #[test]
    fn a_new_task_is_created_under_its_own_id_then_given_the_rest() {
        let made = json!({ "id": "n", "projectName": "p", "title": "N", "status": "in_progress",
            "order": 3, "assignedSessionId": "s", "createdAt": "1" });
        assert_eq!(
            task(None, Some(&made)),
            [
                (
                    "task:create",
                    json!({ "id": "n", "projectName": "p", "title": "N", "status": "in_progress" })
                ),
                ("task:patch", json!({ "id": "n", "set": { "assignedSessionId": "s" } })),
            ]
        );
    }

    #[test]
    fn workflows_are_created_updated_and_deleted_by_id_whatever_their_order() {
        let a = json!({ "id": "a", "name": "A" });
        let b = json!({ "id": "b", "name": "B" });
        let renamed = json!({ "id": "b", "name": "B2", "workspaceId": "w" });
        let c = json!({ "id": "c", "name": "C" });
        assert!(workflows(Some(&json!([a, b])), Some(&json!([b, a]))).is_empty());
        assert_eq!(
            workflows(Some(&json!([a, b])), Some(&json!([renamed, c]))),
            [
                ("workflow:delete", json!({ "id": "a" })),
                ("workflow:update", json!({ "id": "b", "updates": { "name": "B2", "workspaceId": "w" } })),
                ("workflow:create", json!({ "workflow": c })),
            ]
        );
        assert_eq!(
            tasks(Some(&json!([{ "id": "x" }])), None),
            [("task:delete", json!({ "id": "x" }))]
        );
    }
}
