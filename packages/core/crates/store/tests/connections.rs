//! Source connections, task source links, and finding a task again by where it came from.

mod common;

use common::{call, lacks, memory, open, update_connection};
use serde_json::{json, Value};
use vorn_store::Store;

fn make_conn(overrides: Value) -> Value {
    let mut conn = json!({
        "id": "conn-a", "connectorId": "github", "name": "owner/repo",
        "filters": { "owner": "o", "repo": "r" }, "syncIntervalMinutes": 5,
        "statusMapping": { "open": "todo", "closed": "done" },
        "createdAt": "2026-04-24T00:00:00Z"
    });
    merge(&mut conn, overrides);
    conn
}

fn make_task(overrides: Value) -> Value {
    let mut task = json!({
        "id": "task-test", "projectName": "proj", "title": "Test task", "description": "",
        "status": "todo", "order": 0,
        "createdAt": "2026-04-24T00:00:00Z", "updatedAt": "2026-04-24T00:00:00Z"
    });
    merge(&mut task, overrides);
    task
}

fn link(task: &str, external: &str, url: &str) -> Value {
    json!({
        "taskId": task, "connectionId": "conn-1", "connectorId": "github",
        "externalId": external, "externalUrl": url, "sourceStatusRaw": "open",
        "sourceUpdatedAt": "2026-04-24T00:00:00Z", "lastSyncedAt": "2026-04-24T00:00:00Z",
        "conflictState": "none"
    })
}

fn merge(into: &mut Value, overrides: Value) {
    for (key, value) in overrides.as_object().expect("an object") {
        into[key] = value.clone();
    }
}

fn insert_conn(store: &mut Store, conn: Value) {
    call(store, "dbInsertSourceConnection", json!([conn]));
}

fn insert_task(store: &mut Store, task: Value) {
    call(store, "dbInsertTask", json!([task]));
}

fn get_conn(store: &mut Store, id: &str) -> Value {
    call(store, "dbGetSourceConnection", json!([id]))
}

mod source_connections {
    use super::*;

    #[test]
    fn the_list_is_empty_at_first() {
        let mut store = memory();
        assert_eq!(
            call(&mut store, "dbListSourceConnections", json!([null])),
            json!([])
        );
    }

    #[test]
    fn insert_and_get_round_trip_every_field_including_optional_ones() {
        let mut store = memory();
        let conn = make_conn(json!({
            "id": "conn-1", "executionProject": "my-proj",
            "lastSyncAt": "2026-04-24T01:00:00Z", "lastSyncError": "boom",
            "syncCursor": "2026-04-24T00:59:00Z"
        }));
        insert_conn(&mut store, conn.clone());
        assert_eq!(get_conn(&mut store, "conn-1"), conn);
    }

    #[test]
    fn filters_and_status_mapping_cross_as_json() {
        let mut store = memory();
        insert_conn(
            &mut store,
            make_conn(json!({
                "filters": { "owner": "oct", "labels": "bug,fix" },
                "statusMapping": { "foo": "todo" }
            })),
        );
        let fetched = get_conn(&mut store, "conn-a");
        assert_eq!(
            fetched["filters"],
            json!({ "owner": "oct", "labels": "bug,fix" })
        );
        assert_eq!(fetched["statusMapping"], json!({ "foo": "todo" }));
    }

    #[test]
    fn the_list_filters_by_connector_id() {
        let mut store = memory();
        insert_conn(
            &mut store,
            make_conn(json!({ "id": "gh-1", "connectorId": "github" })),
        );
        insert_conn(
            &mut store,
            make_conn(json!({ "id": "lin-1", "connectorId": "linear" })),
        );
        let list = |store: &mut Store, connector: Value| {
            common::ids(
                &call(store, "dbListSourceConnections", json!([connector])),
                "id",
            )
        };
        assert_eq!(list(&mut store, json!("github")), ["gh-1"]);
        assert_eq!(list(&mut store, json!("linear")), ["lin-1"]);
        assert_eq!(list(&mut store, Value::Null).len(), 2);
    }

    #[test]
    fn get_answers_null_for_a_missing_id() {
        let mut store = memory();
        assert!(get_conn(&mut store, "nope").is_null());
    }

    #[test]
    fn an_update_sets_only_the_fields_given() {
        let mut store = memory();
        insert_conn(&mut store, make_conn(json!({ "id": "conn-1" })));
        update_connection(
            &mut store,
            "conn-1",
            json!({ "name": "renamed" }),
            &["name"],
        );
        let got = get_conn(&mut store, "conn-1");
        assert_eq!(got["name"], "renamed");
        assert_eq!(got["connectorId"], "github");
    }

    #[test]
    fn an_update_clears_last_sync_error_set_to_undefined() {
        let mut store = memory();
        insert_conn(
            &mut store,
            make_conn(json!({ "id": "conn-1", "lastSyncError": "oops" })),
        );
        update_connection(&mut store, "conn-1", json!({}), &["lastSyncError"]);
        assert!(lacks(&get_conn(&mut store, "conn-1"), "lastSyncError"));
    }

    #[test]
    fn an_update_advances_the_sync_cursor() {
        let mut store = memory();
        insert_conn(&mut store, make_conn(json!({ "id": "conn-1" })));
        update_connection(
            &mut store,
            "conn-1",
            json!({ "syncCursor": "2026-04-24T10:00:00Z" }),
            &["syncCursor"],
        );
        assert_eq!(
            get_conn(&mut store, "conn-1")["syncCursor"],
            "2026-04-24T10:00:00Z"
        );
    }

    #[test]
    fn an_update_with_no_fields_changes_nothing() {
        let mut store = memory();
        insert_conn(&mut store, make_conn(json!({ "id": "conn-1" })));
        let before = get_conn(&mut store, "conn-1");
        update_connection(&mut store, "conn-1", json!({}), &[]);
        assert_eq!(get_conn(&mut store, "conn-1"), before);
    }

    #[test]
    fn an_update_writes_new_filters() {
        let mut store = memory();
        insert_conn(&mut store, make_conn(json!({ "id": "conn-1" })));
        update_connection(
            &mut store,
            "conn-1",
            json!({ "filters": { "owner": "new" } }),
            &["filters"],
        );
        assert_eq!(
            get_conn(&mut store, "conn-1")["filters"],
            json!({ "owner": "new" })
        );
    }

    #[test]
    fn an_update_writes_a_new_status_mapping() {
        let mut store = memory();
        insert_conn(&mut store, make_conn(json!({ "id": "conn-1" })));
        update_connection(
            &mut store,
            "conn-1",
            json!({ "statusMapping": { "a": "in_progress" } }),
            &["statusMapping"],
        );
        assert_eq!(
            get_conn(&mut store, "conn-1")["statusMapping"],
            json!({ "a": "in_progress" })
        );
    }

    #[test]
    fn an_update_writes_the_interval_and_execution_project() {
        let mut store = memory();
        insert_conn(&mut store, make_conn(json!({ "id": "conn-1" })));
        update_connection(
            &mut store,
            "conn-1",
            json!({ "syncIntervalMinutes": 15, "executionProject": "proj-b" }),
            &["syncIntervalMinutes", "executionProject"],
        );
        let got = get_conn(&mut store, "conn-1");
        assert_eq!(got["syncIntervalMinutes"], 15);
        assert_eq!(got["executionProject"], "proj-b");
    }

    #[test]
    fn delete_removes_the_connection() {
        let mut store = memory();
        insert_conn(&mut store, make_conn(json!({ "id": "conn-1" })));
        call(&mut store, "dbDeleteSourceConnection", json!(["conn-1"]));
        assert!(get_conn(&mut store, "conn-1").is_null());
    }

    #[test]
    fn delete_cascades_to_the_task_links() {
        let mut store = memory();
        insert_conn(&mut store, make_conn(json!({ "id": "conn-1" })));
        insert_task(&mut store, make_task(json!({ "id": "task-1" })));
        call(
            &mut store,
            "dbInsertTaskSourceLink",
            json!([link("task-1", "7", "u")]),
        );
        let links = |store: &mut Store| call(store, "dbListTaskSourceLinks", json!(["conn-1"]));
        assert_eq!(links(&mut store).as_array().map(Vec::len), Some(1));
        call(&mut store, "dbDeleteSourceConnection", json!(["conn-1"]));
        assert_eq!(links(&mut store), json!([]));
    }
}

mod task_source_links {
    use super::*;

    /// A connection and a task to link, as the TypeScript `beforeEach` made.
    fn store() -> Store {
        let mut store = memory();
        insert_conn(&mut store, make_conn(json!({ "id": "conn-1" })));
        insert_task(&mut store, make_task(json!({ "id": "task-1" })));
        store
    }

    fn get_link(store: &mut Store) -> Value {
        call(store, "dbGetTaskSourceLink", json!(["task-1"]))
    }

    #[test]
    fn an_inserted_link_round_trips() {
        let mut store = store();
        call(
            &mut store,
            "dbInsertTaskSourceLink",
            json!([link("task-1", "42", "https://u/42")]),
        );
        let got = get_link(&mut store);
        assert_eq!(got["taskId"], "task-1");
        assert_eq!(got["externalId"], "42");
        assert_eq!(got["externalUrl"], "https://u/42");
        assert_eq!(got["conflictState"], "none");
    }

    #[test]
    fn an_update_patches_only_the_fields_given() {
        let mut store = store();
        call(
            &mut store,
            "dbInsertTaskSourceLink",
            json!([link("task-1", "42", "u")]),
        );
        call(
            &mut store,
            "dbUpdateTaskSourceLink",
            json!(["task-1", { "sourceStatusRaw": "closed", "lastSyncedAt": "later" }]),
        );
        let got = get_link(&mut store);
        assert_eq!(got["sourceStatusRaw"], "closed");
        assert_eq!(got["lastSyncedAt"], "later");
        assert_eq!(got["externalId"], "42");
    }

    #[test]
    fn an_update_with_nothing_to_change_changes_nothing() {
        let mut store = store();
        call(
            &mut store,
            "dbInsertTaskSourceLink",
            json!([link("task-1", "42", "u")]),
        );
        let before = get_link(&mut store);
        call(&mut store, "dbUpdateTaskSourceLink", json!(["task-1", {}]));
        assert_eq!(get_link(&mut store), before);
    }

    #[test]
    fn delete_removes_one_link() {
        let mut store = store();
        call(
            &mut store,
            "dbInsertTaskSourceLink",
            json!([link("task-1", "42", "u")]),
        );
        call(&mut store, "dbDeleteTaskSourceLink", json!(["task-1"]));
        assert!(get_link(&mut store).is_null());
    }

    #[test]
    fn the_list_has_every_link_of_a_connection() {
        let mut store = store();
        insert_task(&mut store, make_task(json!({ "id": "task-2" })));
        for (task, external, url) in [("task-1", "1", "u1"), ("task-2", "2", "u2")] {
            let mut row = link(task, external, url);
            row["sourceStatusRaw"] = json!("");
            row["sourceUpdatedAt"] = json!("");
            row["lastSyncedAt"] = json!("");
            call(&mut store, "dbInsertTaskSourceLink", json!([row]));
        }
        assert_eq!(
            call(&mut store, "dbListTaskSourceLinks", json!(["conn-1"]))
                .as_array()
                .map(Vec::len),
            Some(2)
        );
    }

    #[test]
    fn a_missing_link_by_external_id_is_null() {
        let mut store = memory();
        assert!(call(
            &mut store,
            "dbGetTaskSourceLinkByExternalId",
            json!(["conn-test", "1"])
        )
        .is_null());
    }

    #[test]
    fn a_link_is_found_by_connection_and_external_id() {
        let mut store = memory();
        insert_conn(&mut store, make_conn(json!({ "id": "conn-test" })));
        insert_task(&mut store, make_task(json!({ "id": "task-linked" })));
        let mut row = link("task-linked", "7", "u");
        row["connectionId"] = json!("conn-test");
        call(&mut store, "dbInsertTaskSourceLink", json!([row]));
        let got = call(
            &mut store,
            "dbGetTaskSourceLinkByExternalId",
            json!(["conn-test", "7"]),
        );
        assert_eq!(got["taskId"], "task-linked");
    }
}

mod sign_in {
    use super::*;

    fn store() -> Store {
        let mut store = memory();
        insert_conn(
            &mut store,
            json!({
                "id": "substack", "connectorId": "mcp", "name": "Substack",
                "filters": { "sdkConnectorId": "substack" }, "syncIntervalMinutes": 5,
                "statusMapping": {}, "createdAt": "2026-09-10T20:00:00.000Z"
            }),
        );
        store
    }

    fn set(store: &mut Store, who: Value, at: Value) {
        call(store, "dbSetConnectionSignIn", json!(["substack", who, at]));
    }

    #[test]
    fn is_kept_once_the_window_signs_in_and_cleared_when_it_signs_out() {
        let mut store = store();
        assert!(lacks(&get_conn(&mut store, "substack"), "signedInAs"));

        set(
            &mut store,
            json!("Javier Canizalez (javiercanizalez)"),
            json!("2026-09-10T20:05:00.000Z"),
        );
        let signed_in = get_conn(&mut store, "substack");
        assert_eq!(
            signed_in["signedInAs"],
            "Javier Canizalez (javiercanizalez)"
        );
        assert_eq!(signed_in["signedInAt"], "2026-09-10T20:05:00.000Z");

        set(&mut store, Value::Null, Value::Null);
        let signed_out = get_conn(&mut store, "substack");
        assert!(lacks(&signed_out, "signedInAs"));
        assert!(lacks(&signed_out, "signedInAt"));
    }

    #[test]
    fn survives_an_edit_that_rewrites_the_filters_whole() {
        let mut store = store();
        set(
            &mut store,
            json!("Javier Canizalez"),
            json!("2026-09-10T20:05:00.000Z"),
        );
        update_connection(
            &mut store,
            "substack",
            json!({ "filters": { "sdkConnectorId": "substack", "publication": "novumai" } }),
            &["filters"],
        );
        assert_eq!(
            get_conn(&mut store, "substack")["signedInAs"],
            "Javier Canizalez"
        );
    }
}

mod workflows {
    use super::*;

    fn make_workflow(overrides: Value) -> Value {
        let mut workflow = json!({
            "id": "wf-test", "name": "Test", "icon": "Zap", "iconColor": "#fff",
            "nodes": [], "edges": [], "enabled": true, "workspaceId": "personal"
        });
        merge(&mut workflow, overrides);
        workflow
    }

    #[test]
    fn a_missing_workflow_is_null() {
        let mut store = memory();
        assert!(call(&mut store, "dbGetWorkflow", json!(["does-not-exist"])).is_null());
    }

    #[test]
    fn an_inserted_workflow_round_trips() {
        let mut store = memory();
        call(
            &mut store,
            "dbInsertWorkflow",
            json!([make_workflow(json!({
                "id": "wf-round-trip", "name": "Round-tripped",
                "iconColor": "#abc", "staggerDelayMs": 250
            }))]),
        );
        let loaded = call(&mut store, "dbGetWorkflow", json!(["wf-round-trip"]));
        assert_eq!(loaded["name"], "Round-tripped");
        assert_eq!(loaded["iconColor"], "#abc");
        assert_eq!(loaded["staggerDelayMs"], 250);
    }

    #[test]
    fn the_enabled_flag_survives_a_fetch() {
        let mut store = memory();
        call(
            &mut store,
            "dbInsertWorkflow",
            json!([make_workflow(
                json!({ "id": "wf-disabled", "enabled": false })
            )]),
        );
        assert_eq!(
            call(&mut store, "dbGetWorkflow", json!(["wf-disabled"]))["enabled"],
            false
        );
    }
}

mod finding_a_task_by_source {
    use super::*;

    fn find(store: &mut Store, connector: &str, external: &str) -> Value {
        call(
            store,
            "dbFindTaskByConnectorExternalId",
            json!([connector, external]),
        )
    }

    fn sourced(id: &str, connector: &str, external: &str) -> Value {
        make_task(json!({
            "id": id, "sourceConnectorId": connector, "sourceExternalId": external
        }))
    }

    #[test]
    fn null_when_no_task_has_the_source() {
        let mut store = memory();
        assert!(find(&mut store, "github", "999").is_null());
    }

    #[test]
    fn found_by_connector_and_external_id_without_a_link_row() {
        let mut store = memory();
        insert_task(&mut store, sourced("task-orphan", "github", "42"));
        let found = find(&mut store, "github", "42");
        assert_eq!(found["id"], "task-orphan");
        assert_eq!(found["sourceExternalId"], "42");
    }

    #[test]
    fn a_different_connector_does_not_match() {
        let mut store = memory();
        insert_task(&mut store, sourced("task-linear", "linear", "42"));
        assert!(find(&mut store, "github", "42").is_null());
    }

    #[test]
    fn a_different_external_id_does_not_match() {
        let mut store = memory();
        insert_task(&mut store, sourced("task-different", "github", "1"));
        assert!(find(&mut store, "github", "2").is_null());
    }

    #[test]
    fn one_of_several_matches_is_returned() {
        let mut store = memory();
        insert_task(&mut store, sourced("first", "github", "5"));
        insert_task(&mut store, sourced("second", "github", "5"));
        let found = find(&mut store, "github", "5");
        assert!(found["id"] == "first" || found["id"] == "second", "{found}");
    }

    /// A packaged connector's task is written under the connector's own id (`packdemo`), not the storage type its connection is kept as (`mcp`); the lookup that re-adopts it must ask under the same id.
    fn write_packaged_task(store: &mut Store) {
        insert_task(
            store,
            json!({
                "id": "task-1", "title": "Tick 7", "description": "", "status": "todo",
                "order": 1, "projectName": "Novum",
                "createdAt": "2026-09-01T00:00:00Z", "updatedAt": "2026-09-01T00:00:00Z",
                "sourceConnectorId": "packdemo", "sourceExternalId": "7"
            }),
        );
    }

    #[test]
    fn a_packaged_connectors_task_is_found_under_the_id_it_was_written_with() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let mut store = open(dir.path());
        write_packaged_task(&mut store);
        assert_eq!(find(&mut store, "packdemo", "7")["id"], "task-1");
    }

    #[test]
    fn a_packaged_connectors_task_is_not_found_under_its_storage_type() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let mut store = open(dir.path());
        write_packaged_task(&mut store);
        assert!(find(&mut store, "mcp", "7").is_null());
    }
}
