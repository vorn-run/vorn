use serde_json::{json, Value};

use super::call;
use crate::builtin::tests::Fake;
use crate::builtin::Tasks;

fn text(result: &Value) -> &str {
    result["content"][0]["text"].as_str().unwrap_or_default()
}

fn parsed(result: &Value) -> Value {
    serde_json::from_str(text(result)).unwrap()
}

async fn run(tasks: &std::sync::Arc<Tasks>, tool: &str, args: Value) -> Value {
    call(tasks, tool, args).await.expect("a Tasks tool")
}

#[tokio::test]
async fn creates_reads_and_lists_tasks() {
    let dir = tempfile::tempdir().unwrap();
    let host = Fake::new(dir.path());
    let tasks = Tasks::new(host.clone());
    let made = parsed(
        &run(
            &tasks,
            "create_task",
            json!({ "project_name": "app", "title": "First", "assigned_agent": "claude" }),
        )
        .await,
    );
    assert_eq!(made["status"], "todo");
    let id = made["id"].as_str().unwrap().to_owned();
    assert_eq!(
        parsed(&run(&tasks, "get_task", json!({ "id": id })).await),
        made
    );
    let lost = run(
        &tasks,
        "create_task",
        json!({ "project_name": "nowhere", "title": "x" }),
    )
    .await;
    assert_eq!(text(&lost), "Error: project \"nowhere\" not found");
    assert_eq!(lost["isError"], true);
    let off = run(
        &tasks,
        "create_task",
        json!({ "project_name": "app", "title": "T", "assigned_agent": "gamma" }),
    )
    .await;
    assert!(text(&off).contains("agent \"gamma\" is not on"), "{off}");

    run(
        &tasks,
        "create_task",
        json!({ "project_name": "other", "title": "Elsewhere" }),
    )
    .await;
    let all = parsed(&run(&tasks, "list_tasks", json!({})).await);
    assert_eq!(all.as_array().unwrap().len(), 2);
    let app = parsed(&run(&tasks, "list_tasks", json!({ "project_name": "app" })).await);
    assert_eq!(app.as_array().unwrap().len(), 1);
    let none = parsed(&run(&tasks, "list_tasks", json!({ "project_name": "" })).await);
    assert_eq!(none, json!([]), "an empty name matches no task");
    let mine = parsed(&run(&tasks, "list_tasks", json!({ "assigned_agent": "claude" })).await);
    assert_eq!(mine[0]["id"], id.as_str());
    let personal = parsed(&run(&tasks, "list_tasks", json!({ "workspace_id": "personal" })).await);
    assert_eq!(personal.as_array().unwrap().len(), 2);
    let ghost = run(&tasks, "get_task", json!({ "id": "ghost" })).await;
    assert_eq!(text(&ghost), "Error: task \"ghost\" not found");
}

#[tokio::test]
async fn an_updated_order_places_the_task_among_the_others() {
    let dir = tempfile::tempdir().unwrap();
    let host = Fake::new(dir.path());
    let tasks = Tasks::new(host.clone());
    let mut ids = Vec::new();
    for title in ["a", "b", "c"] {
        let made = parsed(
            &run(
                &tasks,
                "create_task",
                json!({ "project_name": "app", "title": title }),
            )
            .await,
        );
        ids.push(made["id"].as_str().unwrap().to_owned());
    }
    let placed = parsed(
        &run(
            &tasks,
            "update_task",
            json!({ "id": ids[2], "order": 0, "title": "C" }),
        )
        .await,
    );
    assert_eq!(
        (placed["title"].as_str(), placed["order"].as_i64()),
        (Some("C"), Some(1))
    );
    let gone = run(
        &tasks,
        "update_task",
        json!({ "id": "ghost", "title": "x" }),
    )
    .await;
    assert_eq!(text(&gone), "Error: task \"ghost\" not found");
}

#[tokio::test]
async fn archives_restores_and_deletes_a_finished_task() {
    let dir = tempfile::tempdir().unwrap();
    let host = Fake::new(dir.path());
    let tasks = Tasks::new(host.clone());
    let made = parsed(
        &run(
            &tasks,
            "create_task",
            json!({ "project_name": "app", "title": "T" }),
        )
        .await,
    );
    let id = made["id"].as_str().unwrap().to_owned();
    let refused = run(&tasks, "archive_task", json!({ "id": id })).await;
    assert_eq!(
        text(&refused),
        "Error: only done or cancelled tasks can be archived (status: todo)"
    );
    run(&tasks, "update_task", json!({ "id": id, "status": "done" })).await;
    let archived = parsed(&run(&tasks, "archive_task", json!({ "id": id })).await);
    assert!(archived["archivedAt"].is_string());
    let hidden = parsed(&run(&tasks, "list_tasks", json!({})).await);
    assert_eq!(hidden, json!([]));
    let shown = parsed(&run(&tasks, "list_tasks", json!({ "include_archived": true })).await);
    assert_eq!(shown.as_array().unwrap().len(), 1);
    let restored = parsed(&run(&tasks, "unarchive_task", json!({ "id": id })).await);
    assert!(restored.get("archivedAt").is_none());
    let deleted = run(&tasks, "delete_task", json!({ "id": id })).await;
    assert_eq!(text(&deleted), "Deleted task: T");
    let again = run(&tasks, "delete_task", json!({ "id": id })).await;
    assert_eq!(text(&again), format!("Error: task \"{id}\" not found"));
    assert!(call(&tasks, "get_my_context", json!({})).await.is_none());
}
