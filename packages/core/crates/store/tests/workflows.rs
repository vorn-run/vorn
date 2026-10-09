//! `dbUpdateWorkflow`, `dbListWorkflows`, and moving a task between projects with `dbUpdateTask`.

mod common;

use common::{call, memory, save_config, update_task};
use serde_json::{json, Value};
use vorn_store::Store;

/// Saves `workflows` (each merged over a default row) through `saveConfig`.
fn seed(store: &mut Store, workflows: Vec<Value>) {
    let workflows: Vec<Value> = workflows
        .into_iter()
        .map(|w| {
            let mut row = json!({
                "icon": "Zap", "iconColor": "#3b82f6", "nodes": [], "edges": [],
                "enabled": false, "id": "wf", "name": "A workflow"
            });
            for (key, value) in w.as_object().expect("an object") {
                row[key] = value.clone();
            }
            row
        })
        .collect();
    save_config(
        store,
        json!({
            "version": 1,
            "defaults": { "shell": "/bin/zsh", "fontSize": 13, "theme": "dark" },
            "projects": [], "tasks": [], "workflows": workflows
        }),
        &[],
    );
}

fn update(store: &mut Store, id: &str, updates: Value) -> Value {
    call(store, "dbUpdateWorkflow", json!([id, updates]))
}

fn get(store: &mut Store, id: &str) -> Value {
    call(store, "dbGetWorkflow", json!([id]))
}

mod enabling {
    use super::*;

    #[test]
    fn turning_one_on_is_written_to_its_row() {
        let mut store = memory();
        seed(
            &mut store,
            vec![json!({ "id": "wf-a", "name": "clean branches", "enabled": false })],
        );
        update(&mut store, "wf-a", json!({ "enabled": true }));
        assert_eq!(get(&mut store, "wf-a")["enabled"], true);
    }

    #[test]
    fn turning_one_off_again_is_written_too() {
        let mut store = memory();
        seed(
            &mut store,
            vec![json!({ "id": "wf-a", "name": "clean branches", "enabled": true })],
        );
        update(&mut store, "wf-a", json!({ "enabled": false }));
        assert_eq!(get(&mut store, "wf-a")["enabled"], false);
    }

    #[test]
    fn the_rest_of_the_workflow_is_left_as_it_was() {
        let mut store = memory();
        seed(
            &mut store,
            vec![json!({
                "id": "wf-a", "name": "Simple hello", "icon": "Cloud", "iconColor": "#3b82f6",
                "enabled": false,
                "nodes": [{ "id": "t", "type": "trigger", "label": "Manual Trigger", "config": {} }]
            })],
        );
        update(&mut store, "wf-a", json!({ "enabled": true }));
        let after = get(&mut store, "wf-a");
        assert_eq!(after["name"], "Simple hello");
        assert_eq!(after["icon"], "Cloud");
        assert_eq!(after["iconColor"], "#3b82f6");
        assert_eq!(after["nodes"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn no_other_workflow_is_touched() {
        let mut store = memory();
        seed(
            &mut store,
            vec![
                json!({ "id": "wf-a", "name": "Alpha", "enabled": false }),
                json!({ "id": "wf-b", "name": "Beta", "enabled": false }),
            ],
        );
        update(&mut store, "wf-a", json!({ "enabled": true }));
        assert_eq!(get(&mut store, "wf-b")["enabled"], false);
    }

    #[test]
    fn the_count_of_rows_changed_answers_an_unknown_id() {
        let mut store = memory();
        seed(
            &mut store,
            vec![json!({ "id": "wf-a", "name": "Alpha", "enabled": false })],
        );
        assert_eq!(update(&mut store, "wf-a", json!({ "enabled": true })), 1);
        assert_eq!(update(&mut store, "wf-nope", json!({ "enabled": true })), 0);
        assert_eq!(
            call(&mut store, "dbListWorkflows", json!([]))
                .as_array()
                .map(Vec::len),
            Some(1)
        );
    }

    #[test]
    fn the_count_is_of_rows_matched_not_values_moved() {
        let mut store = memory();
        seed(
            &mut store,
            vec![json!({ "id": "wf-a", "name": "Alpha", "enabled": true })],
        );
        assert_eq!(update(&mut store, "wf-a", json!({ "enabled": true })), 1);
        assert_eq!(get(&mut store, "wf-a")["enabled"], true);
    }

    #[test]
    fn no_columns_change_nothing_and_say_so() {
        let mut store = memory();
        seed(
            &mut store,
            vec![json!({ "id": "wf-a", "name": "Alpha", "enabled": false })],
        );
        assert_eq!(update(&mut store, "wf-a", json!({})), 0);
        assert_eq!(get(&mut store, "wf-a")["name"], "Alpha");
    }
}

mod listing {
    use super::*;

    #[test]
    fn every_workflow_is_listed_including_one_that_never_ran() {
        let mut store = memory();
        seed(
            &mut store,
            vec![
                json!({ "id": "wf-a", "name": "clean branches", "enabled": false }),
                json!({ "id": "wf-b", "name": "Simple hello", "enabled": true }),
            ],
        );
        let names = common::ids(&call(&mut store, "dbListWorkflows", json!([])), "name");
        assert!(names.iter().any(|n| n == "clean branches"));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn a_listed_workflow_carries_what_its_row_draws() {
        let mut store = memory();
        seed(
            &mut store,
            vec![json!({
                "id": "wf-a", "name": "Simple hello", "icon": "Cloud", "iconColor": "#3b82f6",
                "enabled": true,
                "nodes": [{
                    "id": "t", "type": "trigger", "label": "Manual Trigger",
                    "config": { "triggerType": "manual", "inputs": [{ "key": "pr_number", "type": "text" }] }
                }]
            })],
        );
        let listed = call(&mut store, "dbListWorkflows", json!([]));
        let workflow = &listed[0];
        assert_eq!(workflow["icon"], "Cloud");
        assert_eq!(workflow["iconColor"], "#3b82f6");
        assert_eq!(workflow["enabled"], true);
        let trigger = workflow["nodes"]
            .as_array()
            .and_then(|n| n.iter().find(|n| n["type"] == "trigger"))
            .expect("a trigger");
        assert_eq!(trigger["config"]["triggerType"], "manual");
    }
}

mod moving_a_task_between_projects {
    use super::*;

    fn task(id: &str, project: &str, order: i64) -> Value {
        json!({
            "id": id, "projectName": project, "title": "A task", "description": "",
            "status": "todo", "order": order,
            "createdAt": "2026-08-01T00:00:00.000Z", "updatedAt": "2026-08-01T00:00:00.000Z"
        })
    }

    fn get_task(store: &mut Store, id: &str) -> Value {
        call(store, "dbGetTask", json!([id]))
    }

    fn max_order(store: &mut Store, project: &str) -> Value {
        call(store, "dbGetMaxTaskOrder", json!([project]))
    }

    #[test]
    fn project_name_is_written() {
        let mut store = memory();
        call(&mut store, "dbInsertTask", json!([task("t", "alpha", 0)]));
        update_task(
            &mut store,
            "t",
            json!({ "projectName": "beta" }),
            &["projectName"],
        );
        assert_eq!(get_task(&mut store, "t")["projectName"], "beta");
    }

    #[test]
    fn the_project_is_left_alone_when_not_in_the_update() {
        let mut store = memory();
        call(&mut store, "dbInsertTask", json!([task("t", "alpha", 0)]));
        update_task(&mut store, "t", json!({ "title": "Renamed" }), &["title"]);
        let got = get_task(&mut store, "t");
        assert_eq!(got["projectName"], "alpha");
        assert_eq!(got["title"], "Renamed");
    }

    #[test]
    fn the_orders_of_the_board_a_task_arrives_on_are_counted() {
        let mut store = memory();
        call(&mut store, "dbInsertTask", json!([task("b", "beta", 4)]));
        assert_eq!(max_order(&mut store, "beta"), 4);
        assert_eq!(max_order(&mut store, "gamma"), -1);

        call(&mut store, "dbInsertTask", json!([task("m", "alpha", 0)]));
        let next = max_order(&mut store, "beta").as_i64().expect("a number") + 1;
        update_task(
            &mut store,
            "m",
            json!({ "projectName": "beta", "order": next }),
            &["projectName", "order"],
        );
        assert_eq!(get_task(&mut store, "m")["order"], 5);
    }
}
