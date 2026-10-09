//! The sessions saved for the next launch, and the sidebar groups they are filed under.

mod common;

use common::{call, ids, lacks, memory};
use serde_json::{json, Value};
use vorn_store::Store;

fn now_ms() -> i64 {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the clock is past 1970");
    i64::try_from(since.as_millis()).expect("milliseconds fit")
}

fn make_session(overrides: Value) -> Value {
    let mut session = json!({
        "id": "test-id-1", "agentType": "claude", "projectName": "test-project",
        "projectPath": "/test/project", "status": "running", "createdAt": now_ms(),
        "pid": 12345
    });
    for (key, value) in overrides.as_object().expect("an object") {
        session[key] = value.clone();
    }
    session
}

fn save(store: &mut Store, sessions: Vec<Value>) {
    call(store, "saveSessions", json!([sessions]));
}

fn previous(store: &mut Store) -> Vec<Value> {
    call(store, "getPreviousSessions", json!([]))
        .as_array()
        .cloned()
        .expect("a list")
}

mod persistence {
    use super::*;

    #[test]
    fn a_session_is_saved_and_loaded_with_every_field() {
        let mut store = memory();
        save(
            &mut store,
            vec![make_session(json!({
                "displayName": "my session", "branch": "main",
                "worktreePath": "/test/worktree", "worktreeName": "friendly-name",
                "isWorktree": true, "remoteHostId": "host-1",
                "remoteHostLabel": "my-server", "hookSessionId": "hook-uuid-123",
                "statusSource": "hooks"
            }))],
        );
        let loaded = previous(&mut store);
        assert_eq!(loaded.len(), 1);
        let s = &loaded[0];
        for (key, value) in [
            ("id", json!("test-id-1")),
            ("agentType", json!("claude")),
            ("projectName", json!("test-project")),
            ("projectPath", json!("/test/project")),
            ("displayName", json!("my session")),
            ("branch", json!("main")),
            ("worktreePath", json!("/test/worktree")),
            ("worktreeName", json!("friendly-name")),
            ("isWorktree", json!(true)),
            ("remoteHostId", json!("host-1")),
            ("remoteHostLabel", json!("my-server")),
            ("hookSessionId", json!("hook-uuid-123")),
            ("statusSource", json!("hooks")),
        ] {
            assert_eq!(s[key], value, "{key}");
        }
    }

    #[test]
    fn the_hook_session_id_round_trips() {
        let mut store = memory();
        save(
            &mut store,
            vec![make_session(json!({ "hookSessionId": "abc-def-123" }))],
        );
        assert_eq!(previous(&mut store)[0]["hookSessionId"], "abc-def-123");
    }

    #[test]
    fn the_worktree_name_round_trips() {
        let mut store = memory();
        save(
            &mut store,
            vec![make_session(json!({
                "worktreeName": "galactic-eclipse", "worktreePath": "/test/wt", "isWorktree": true
            }))],
        );
        assert_eq!(previous(&mut store)[0]["worktreeName"], "galactic-eclipse");
    }

    #[test]
    fn sessions_come_back_in_the_order_saved() {
        let mut store = memory();
        save(
            &mut store,
            vec![
                make_session(json!({ "id": "a", "displayName": "first" })),
                make_session(json!({ "id": "b", "displayName": "second" })),
                make_session(json!({ "id": "c", "displayName": "third" })),
            ],
        );
        assert_eq!(
            ids(&Value::Array(previous(&mut store)), "id"),
            ["a", "b", "c"]
        );
    }

    #[test]
    fn clearing_removes_every_row() {
        let mut store = memory();
        save(&mut store, vec![make_session(json!({}))]);
        call(&mut store, "clearSessions", json!([]));
        assert!(previous(&mut store).is_empty());
    }

    #[test]
    fn a_save_replaces_the_sessions_saved_before() {
        let mut store = memory();
        save(&mut store, vec![make_session(json!({ "id": "old" }))]);
        save(&mut store, vec![make_session(json!({ "id": "new" }))]);
        let loaded = previous(&mut store);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0]["id"], "new");
    }

    #[test]
    fn saving_none_empties_the_table() {
        let mut store = memory();
        save(&mut store, vec![make_session(json!({}))]);
        save(&mut store, vec![]);
        assert!(previous(&mut store).is_empty());
    }

    #[test]
    fn optional_fields_left_out_stay_out() {
        let mut store = memory();
        save(&mut store, vec![make_session(json!({}))]);
        let s = &previous(&mut store)[0];
        for key in [
            "displayName",
            "branch",
            "worktreePath",
            "worktreeName",
            "isWorktree",
            "hookSessionId",
        ] {
            assert!(lacks(s, key), "{key}");
        }
    }

    #[test]
    fn the_directory_the_shell_was_in_round_trips() {
        let mut store = memory();
        save(
            &mut store,
            vec![make_session(json!({
                "agentType": "shell", "shellCwd": "/Users/x/dev/vorn/packages"
            }))],
        );
        assert_eq!(
            previous(&mut store)[0]["shellCwd"],
            "/Users/x/dev/vorn/packages"
        );
    }

    #[test]
    fn the_shell_directory_stays_absent_when_never_reported() {
        let mut store = memory();
        save(&mut store, vec![make_session(json!({}))]);
        assert!(lacks(&previous(&mut store)[0], "shellCwd"));
    }

    #[test]
    fn a_stamp_the_record_already_carried_is_kept() {
        let mut store = memory();
        let ended = now_ms() - 3 * 24 * 60 * 60 * 1000;
        save(&mut store, vec![make_session(json!({ "savedAt": ended }))]);
        assert_eq!(previous(&mut store)[0]["savedAt"], ended);
    }

    #[test]
    fn a_record_never_written_down_is_stamped() {
        let mut store = memory();
        let before = now_ms();
        save(&mut store, vec![make_session(json!({}))]);
        let saved_at = previous(&mut store)[0]["savedAt"]
            .as_i64()
            .expect("a stamp");
        assert!(saved_at >= before, "{saved_at} >= {before}");
    }

    #[test]
    fn opening_a_second_store_after_the_first_does_not_fail() {
        drop(memory());
        drop(memory());
    }
}

mod groups {
    use super::*;

    fn group(overrides: Value) -> Value {
        let mut group =
            json!({ "id": "g1", "name": "Sidebar work", "order": 0, "workspaceId": "personal" });
        for (key, value) in overrides.as_object().expect("an object") {
            group[key] = value.clone();
        }
        group
    }

    fn session(id: &str, group: Option<&str>) -> Value {
        let mut session = json!({
            "id": id, "agentType": "claude", "projectName": "vorn",
            "projectPath": "/tmp/vorn", "status": "idle", "createdAt": 1, "pid": 1
        });
        if let Some(group) = group {
            session["groupId"] = json!(group);
        }
        session
    }

    fn insert(store: &mut Store, group: Value) {
        call(store, "dbInsertSessionGroup", json!([group]));
    }

    fn list(store: &mut Store) -> Value {
        call(store, "dbListSessionGroups", json!([]))
    }

    #[test]
    fn a_group_comes_back_with_everything_it_was_given() {
        let mut store = memory();
        insert(
            &mut store,
            group(json!({ "icon": "Terminal", "iconColor": "#eab308" })),
        );
        assert_eq!(
            list(&mut store),
            json!([{
                "id": "g1", "name": "Sidebar work", "order": 0, "workspaceId": "personal",
                "icon": "Terminal", "iconColor": "#eab308"
            }])
        );
    }

    #[test]
    fn fields_not_given_are_left_out_rather_than_stored_as_null() {
        let mut store = memory();
        insert(&mut store, group(json!({})));
        let loaded = &list(&mut store)[0];
        assert!(lacks(loaded, "icon"));
        assert!(lacks(loaded, "iconColor"));
    }

    #[test]
    fn groups_list_in_the_order_arranged_not_insertion_order() {
        let mut store = memory();
        insert(
            &mut store,
            group(json!({ "id": "b", "name": "Second", "order": 2 })),
        );
        insert(
            &mut store,
            group(json!({ "id": "a", "name": "First", "order": 1 })),
        );
        assert_eq!(ids(&list(&mut store), "name"), ["First", "Second"]);
    }

    #[test]
    fn an_update_of_one_field_leaves_the_rest() {
        let mut store = memory();
        insert(
            &mut store,
            group(json!({ "icon": "Terminal", "iconColor": "#eab308" })),
        );
        call(
            &mut store,
            "dbUpdateSessionGroup",
            json!(["g1", { "name": "Renamed" }]),
        );
        let got = &list(&mut store)[0];
        assert_eq!(got["name"], "Renamed");
        assert_eq!(got["icon"], "Terminal");
        assert_eq!(got["iconColor"], "#eab308");
        assert_eq!(got["order"], 0);
    }

    #[test]
    fn a_group_can_be_moved_recoloured_and_rehomed() {
        let mut store = memory();
        insert(&mut store, group(json!({})));
        call(
            &mut store,
            "dbUpdateSessionGroup",
            json!(["g1", {
                "order": 5, "icon": "Rocket", "iconColor": "#3b82f6", "workspaceId": "work"
            }]),
        );
        let got = &list(&mut store)[0];
        assert_eq!(got["order"], 5);
        assert_eq!(got["icon"], "Rocket");
        assert_eq!(got["iconColor"], "#3b82f6");
        assert_eq!(got["workspaceId"], "work");
    }

    #[test]
    fn an_update_of_nothing_writes_nothing() {
        let mut store = memory();
        insert(&mut store, group(json!({})));
        call(&mut store, "dbUpdateSessionGroup", json!(["g1", {}]));
        assert_eq!(list(&mut store)[0]["name"], "Sidebar work");
    }

    #[test]
    fn deleting_a_group_lets_its_sessions_go_without_ending_them() {
        let mut store = memory();
        insert(&mut store, group(json!({})));
        save(
            &mut store,
            vec![
                session("s1", Some("g1")),
                session("s2", Some("g1")),
                session("s3", None),
            ],
        );

        call(&mut store, "dbDeleteSessionGroup", json!(["g1"]));

        assert_eq!(list(&mut store), json!([]));
        let left = previous(&mut store);
        let mut left_ids = ids(&Value::Array(left.clone()), "id");
        left_ids.sort();
        assert_eq!(left_ids, ["s1", "s2", "s3"]);
        assert!(left.iter().all(|s| lacks(s, "groupId")));
    }

    #[test]
    fn deleting_a_group_leaves_another_groups_sessions_filed() {
        let mut store = memory();
        insert(&mut store, group(json!({})));
        insert(&mut store, group(json!({ "id": "g2", "name": "Release" })));
        save(
            &mut store,
            vec![session("s1", Some("g1")), session("s2", Some("g2"))],
        );

        call(&mut store, "dbDeleteSessionGroup", json!(["g1"]));

        let left = previous(&mut store);
        let s2 = left.iter().find(|s| s["id"] == "s2").expect("s2");
        assert_eq!(s2["groupId"], "g2");
    }
}
