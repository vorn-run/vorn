//! `saveConfig` and `loadConfig`: what a save keeps and prunes, and what the store seeds.

mod common;

use common::{
    call, default_agent_commands, ids, lacks, load_config, memory, save_config,
    DEFAULT_TASK_WORKFLOW_ID, DEV_SERVER_WORKFLOW_ID,
};
use serde_json::{json, Value};
use vorn_store::Store;

/// A config as a client sends it, with `extra` merged into `defaults`.
fn config_with(extra: Value) -> Value {
    let mut defaults = json!({ "shell": "/bin/zsh", "fontSize": 13, "theme": "dark" });
    for (key, value) in extra.as_object().expect("an object") {
        defaults[key] = value.clone();
    }
    json!({ "version": 1, "defaults": defaults, "projects": [] })
}

/// A config carrying `collections` beside the usual three fields.
fn with_collections(collections: Value) -> Value {
    let mut config = config_with(json!({}));
    for (key, value) in collections.as_object().expect("an object") {
        config[key] = value.clone();
    }
    config
}

fn task(id: &str, title: &str) -> Value {
    json!({
        "id": id, "projectName": "vorn", "title": title, "description": "",
        "status": "todo", "order": 0,
        "createdAt": "2026-08-17T00:00:00.000Z", "updatedAt": "2026-08-17T00:00:00.000Z"
    })
}

fn defaults_of(store: &mut Store) -> Value {
    load_config(store)["defaults"].clone()
}

fn round_trips(key: &str, value: Value) {
    let mut store = memory();
    save_config(&mut store, config_with(json!({ key: value.clone() })), &[]);
    assert_eq!(defaults_of(&mut store)[key], value, "{key} = {value}");
}

#[test]
fn defaults_survive_a_save_and_load() {
    for (key, value) in [
        ("domBlockRendering", json!(true)),
        ("domBlockRendering", json!(false)),
        ("minimalShellPrompt", json!(true)),
        ("minimalShellPrompt", json!(false)),
        ("reopenSessions", json!(true)),
        ("keepSessionsRunning", json!(true)),
        ("keepSessionsRunning", json!(false)),
        ("widgetEnabled", json!(false)),
    ] {
        round_trips(key, value);
    }
}

#[test]
fn terminal_settings_default_on_when_the_user_has_not_chosen() {
    let mut store = memory();
    save_config(&mut store, config_with(json!({})), &[]);
    let defaults = defaults_of(&mut store);
    assert_eq!(defaults["domBlockRendering"], true);
    assert_eq!(defaults["minimalShellPrompt"], true);
    assert_eq!(defaults["keepSessionsRunning"], true);
}

#[test]
fn a_false_the_user_chose_is_kept_rather_than_treated_as_unset() {
    let mut store = memory();
    save_config(
        &mut store,
        config_with(json!({
            "domBlockRendering": false,
            "minimalShellPrompt": false,
            "keepSessionsRunning": false
        })),
        &[],
    );
    let defaults = defaults_of(&mut store);
    assert_eq!(defaults["domBlockRendering"], false);
    assert_eq!(defaults["minimalShellPrompt"], false);
    assert_eq!(defaults["keepSessionsRunning"], false);
}

#[test]
fn keys_that_were_declared_but_never_listed_survive() {
    for (key, value) in [
        ("updateAutoDownload", json!(false)),
        ("headlessStepTimeoutMinutes", json!(45)),
        ("enableHoverPreview", json!(false)),
    ] {
        round_trips(key, value);
    }
}

#[test]
fn worktree_retention_survives_since_the_server_reads_it() {
    round_trips("worktreeRetention", json!({ "mode": "days", "days": 7 }));
}

#[test]
fn a_key_the_saving_client_did_not_send_survives() {
    let mut store = memory();
    save_config(&mut store, config_with(json!({ "serverPort": 61601 })), &[]);
    save_config(&mut store, config_with(json!({ "theme": "light" })), &[]);
    let defaults = defaults_of(&mut store);
    assert_eq!(defaults["serverPort"], 61601);
    assert_eq!(defaults["theme"], "light");
}

#[test]
fn keys_survive_a_client_a_whole_release_behind() {
    let mut store = memory();
    save_config(
        &mut store,
        config_with(json!({
            "serverPort": 61601,
            "showHeadlessAgents": true,
            "headlessRetentionMinutes": 30
        })),
        &[],
    );
    save_config(&mut store, config_with(json!({})), &[]);
    let defaults = defaults_of(&mut store);
    assert_eq!(defaults["serverPort"], 61601);
    assert_eq!(defaults["showHeadlessAgents"], true);
    assert_eq!(defaults["headlessRetentionMinutes"], 30);
}

#[test]
fn a_setting_can_still_be_cleared_on_purpose() {
    let mut store = memory();
    save_config(
        &mut store,
        config_with(json!({ "widgetEnabled": true })),
        &[],
    );
    // `{ widgetEnabled: undefined }`: JSON drops the key, the wrapper names it.
    save_config(&mut store, config_with(json!({})), &["widgetEnabled"]);
    assert!(lacks(&defaults_of(&mut store), "widgetEnabled"));
}

#[test]
fn updating_a_task_does_not_cascade_away_what_references_it() {
    let mut store = memory();
    save_config(
        &mut store,
        with_collections(json!({ "tasks": [task("t1", "first")] })),
        &[],
    );
    call(
        &mut store,
        "dbInsertSourceConnection",
        json!([{
            "id": "conn1", "connectorId": "github", "name": "repo", "filters": {},
            "syncIntervalMinutes": 15, "statusMapping": {},
            "createdAt": "2026-08-17T00:00:00.000Z"
        }]),
    );
    call(
        &mut store,
        "dbInsertTaskSourceLink",
        json!([{
            "taskId": "t1", "connectionId": "conn1", "connectorId": "github",
            "externalId": "42", "externalUrl": "https://example.test/42",
            "sourceStatusRaw": "open", "sourceUpdatedAt": "2026-08-17T00:00:00.000Z",
            "lastSyncedAt": "2026-08-17T00:00:00.000Z", "conflictState": "none"
        }]),
    );

    save_config(
        &mut store,
        with_collections(json!({ "tasks": [task("t1", "renamed")] })),
        &[],
    );

    assert_eq!(load_config(&mut store)["tasks"][0]["title"], "renamed");
    assert!(!call(&mut store, "dbGetTaskSourceLink", json!(["t1"])).is_null());
}

#[test]
fn a_task_the_client_dropped_is_still_removed() {
    let mut store = memory();
    save_config(
        &mut store,
        with_collections(json!({ "tasks": [task("t1", "first"), task("t2", "second")] })),
        &[],
    );
    save_config(
        &mut store,
        with_collections(json!({ "tasks": [task("t1", "first")] })),
        &[],
    );
    assert_eq!(ids(&load_config(&mut store)["tasks"], "id"), ["t1"]);
}

#[test]
fn a_workspace_survives_an_unrelated_save() {
    let mut store = memory();
    let workspaces = json!([
        { "id": "personal", "name": "Personal", "order": 0 },
        { "id": "work", "name": "Work", "order": 1 }
    ]);
    save_config(
        &mut store,
        with_collections(json!({ "workspaces": workspaces })),
        &[],
    );
    save_config(
        &mut store,
        with_collections(json!({ "workspaces": workspaces })),
        &[],
    );
    let mut listed = ids(&load_config(&mut store)["workspaces"], "id");
    listed.sort();
    assert_eq!(listed, ["personal", "work"]);
}

#[test]
fn a_project_is_updated_in_place_since_tasks_reference_it_by_name() {
    let mut store = memory();
    let project = json!({ "name": "vorn", "path": "/a", "preferredAgents": [] });
    save_config(
        &mut store,
        with_collections(json!({ "projects": [project] })),
        &[],
    );
    let mut moved = project.clone();
    moved["path"] = json!("/b");
    save_config(
        &mut store,
        with_collections(json!({ "projects": [moved] })),
        &[],
    );
    let projects = load_config(&mut store)["projects"].clone();
    assert_eq!(projects.as_array().map(Vec::len), Some(1));
    assert_eq!(projects[0]["path"], "/b");
}

#[test]
fn a_task_added_by_the_client_that_saved_first_is_kept() {
    let mut store = memory();
    save_config(
        &mut store,
        with_collections(json!({ "tasks": [task("shared", "shared")] })),
        &[],
    );
    let laptop = load_config(&mut store);
    let mut phone = load_config(&mut store);

    phone["tasks"]
        .as_array_mut()
        .expect("tasks")
        .push(task("from-phone", "from-phone"));
    save_config(&mut store, phone, &[]);
    let mut laptop_save = laptop.clone();
    laptop_save["defaults"]["fontSize"] = json!(15);
    save_config(&mut store, laptop_save, &[]);

    let after = load_config(&mut store);
    let mut tasks = ids(&after["tasks"], "id");
    tasks.sort();
    assert_eq!(tasks, ["from-phone", "shared"]);
    assert_eq!(after["defaults"]["fontSize"], 15);
}

#[test]
fn a_task_the_client_removed_is_still_deleted() {
    let mut store = memory();
    save_config(
        &mut store,
        with_collections(json!({ "tasks": [task("a", "a"), task("b", "b")] })),
        &[],
    );
    let mut client = load_config(&mut store);
    client["tasks"]
        .as_array_mut()
        .expect("tasks")
        .retain(|t| t["id"] != "b");
    save_config(&mut store, client, &[]);
    assert_eq!(ids(&load_config(&mut store)["tasks"], "id"), ["a"]);
}

#[test]
fn everything_absent_is_pruned_when_the_caller_tracks_no_revision() {
    let mut store = memory();
    save_config(
        &mut store,
        with_collections(json!({ "tasks": [task("a", "a"), task("b", "b")] })),
        &[],
    );
    save_config(
        &mut store,
        with_collections(json!({ "tasks": [task("a", "a")] })),
        &[],
    );
    assert_eq!(ids(&load_config(&mut store)["tasks"], "id"), ["a"]);
}

#[test]
fn the_revision_moves_forward() {
    let mut store = memory();
    save_config(&mut store, config_with(json!({})), &[]);
    let first = load_config(&mut store)["revision"].as_i64().unwrap_or(0);
    save_config(&mut store, config_with(json!({})), &[]);
    let second = load_config(&mut store)["revision"]
        .as_i64()
        .expect("a revision");
    assert!(second > first, "{second} > {first}");
}

#[test]
fn reopening_panes_is_on_unless_turned_off() {
    let mut store = memory();
    assert_eq!(defaults_of(&mut store)["reopenSessions"], true);
}

#[test]
fn reopening_panes_stays_off_once_it_has_been() {
    let mut store = memory();
    let mut config = load_config(&mut store);
    config["defaults"]["reopenSessions"] = json!(false);
    save_config(&mut store, config, &[]);
    assert_eq!(defaults_of(&mut store)["reopenSessions"], false);
}

mod session_groups {
    use super::*;

    fn group() -> Value {
        json!({ "id": "g1", "name": "Sidebar work", "order": 0, "workspaceId": "personal" })
    }

    fn config() -> Value {
        let mut config = config_with(json!({}));
        config["projects"] =
            json!([{ "name": "vorn", "path": "/tmp/vorn", "preferredAgents": [] }]);
        config["sessionGroups"] = json!([group()]);
        config
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

    #[test]
    fn a_group_round_trips() {
        let mut store = memory();
        save_config(&mut store, config(), &[]);
        assert_eq!(load_config(&mut store)["sessionGroups"], json!([group()]));
    }

    #[test]
    fn a_session_comes_back_still_filed_under_its_group() {
        let mut store = memory();
        save_config(&mut store, config(), &[]);
        call(
            &mut store,
            "saveSessions",
            json!([[session("s1", Some("g1")), session("s2", None)]]),
        );
        let restored = call(&mut store, "getPreviousSessions", json!([]));
        let by_id = |id: &str| {
            restored
                .as_array()
                .expect("sessions")
                .iter()
                .find(|s| s["id"] == id)
                .cloned()
                .expect("the session")
        };
        assert_eq!(by_id("s1")["groupId"], "g1");
        assert!(lacks(&by_id("s2"), "groupId"));
    }

    #[test]
    fn deleting_the_group_lets_the_sessions_go_and_kills_none() {
        let mut store = memory();
        save_config(&mut store, config(), &[]);
        call(
            &mut store,
            "saveSessions",
            json!([[session("s1", Some("g1")), session("s2", Some("g1"))]]),
        );
        call(&mut store, "dbDeleteSessionGroup", json!(["g1"]));
        assert_eq!(
            call(&mut store, "dbListSessionGroups", json!([])),
            json!([])
        );
        let restored = call(&mut store, "getPreviousSessions", json!([]));
        let mut left = ids(&restored, "id");
        left.sort();
        assert_eq!(left, ["s1", "s2"]);
        assert!(restored
            .as_array()
            .expect("sessions")
            .iter()
            .all(|s| lacks(s, "groupId")));
    }

    #[test]
    fn a_group_another_client_added_after_this_snapshot_is_spared() {
        let mut store = memory();
        save_config(&mut store, config(), &[]);
        let stale = load_config(&mut store);
        let mut newer = stale.clone();
        let mut release = group();
        release["id"] = json!("g2");
        release["name"] = json!("Release");
        newer["sessionGroups"]
            .as_array_mut()
            .expect("groups")
            .push(release);
        save_config(&mut store, newer, &[]);
        save_config(&mut store, stale, &[]);
        let mut groups = ids(&call(&mut store, "dbListSessionGroups", json!([])), "id");
        groups.sort();
        assert_eq!(groups, ["g1", "g2"]);
    }
}

mod headless_args {
    use super::*;

    /// A config with the app's agent commands, `change` applied to them.
    fn make_config(change: impl FnOnce(&mut serde_json::Map<String, Value>)) -> Value {
        let mut commands = default_agent_commands();
        change(&mut commands);
        json!({
            "version": 1,
            "defaults": { "theme": "dark", "shell": "/bin/zsh", "fontSize": 13 },
            "projects": [],
            "agentCommands": commands,
            "workflows": [],
            "tasks": []
        })
    }

    #[test]
    fn headless_args_are_saved_and_loaded() {
        let mut store = memory();
        save_config(
            &mut store,
            make_config(|c| {
                c["claude"]["headlessArgs"] =
                    json!(["--dangerously-skip-permissions", "--verbose"]);
            }),
            &[],
        );
        assert_eq!(
            load_config(&mut store)["agentCommands"]["claude"]["headlessArgs"],
            json!(["--dangerously-skip-permissions", "--verbose"])
        );
    }

    #[test]
    fn headless_args_load_as_absent_when_not_set() {
        let mut store = memory();
        save_config(
            &mut store,
            make_config(|c| {
                c["opencode"] = json!({ "command": "opencode", "args": [] });
            }),
            &[],
        );
        assert!(lacks(
            &load_config(&mut store)["agentCommands"]["opencode"],
            "headlessArgs"
        ));
    }

    #[test]
    fn headless_args_survive_round_trips() {
        let mut store = memory();
        save_config(&mut store, make_config(|_| {}), &[]);

        let mut loaded = load_config(&mut store);
        let mut gemini = default_agent_commands()["gemini"].clone();
        gemini["headlessArgs"] = json!(["-y", "--no-confirm"]);
        loaded["agentCommands"]["gemini"] = gemini;
        save_config(&mut store, loaded, &[]);

        let reloaded = load_config(&mut store);
        assert_eq!(
            reloaded["agentCommands"]["gemini"]["headlessArgs"],
            json!(["-y", "--no-confirm"])
        );
        assert_eq!(
            reloaded["agentCommands"]["claude"]["headlessArgs"],
            json!(["--dangerously-skip-permissions"])
        );
    }
}

mod seeded_workflows {
    use super::*;

    fn seed(store: &mut Store) {
        call(store, "seedSystemDefaults", json!([]));
    }

    fn workflow(config: &Value, id: &str) -> Option<Value> {
        config["workflows"]
            .as_array()
            .and_then(|w| w.iter().find(|w| w["id"] == id).cloned())
    }

    fn node_of_type<'a>(workflow: &'a Value, kind: &str) -> Option<&'a Value> {
        workflow["nodes"]
            .as_array()
            .and_then(|n| n.iter().find(|n| n["type"] == kind))
    }

    fn base_config(defaults: Value, workflows: Value) -> Value {
        json!({
            "version": 1,
            "defaults": defaults,
            "projects": [],
            "agentCommands": default_agent_commands(),
            "workflows": workflows,
            "tasks": []
        })
    }

    #[test]
    fn the_default_task_workflow_is_inserted_on_the_first_call() {
        let mut store = memory();
        seed(&mut store);
        let config = load_config(&mut store);
        let seeded = workflow(&config, DEFAULT_TASK_WORKFLOW_ID).expect("seeded");
        assert_eq!(seeded["name"], "Default Task Workflow");
        assert_eq!(seeded["enabled"], true);
        assert!(node_of_type(&seeded, "trigger").is_some());
        assert!(node_of_type(&seeded, "launchAgent").is_some());
    }

    #[test]
    fn seeding_sets_its_flag() {
        let mut store = memory();
        seed(&mut store);
        assert_eq!(
            load_config(&mut store)["defaults"]["hasSeededDefaultTaskWorkflow"],
            true
        );
    }

    #[test]
    fn seeding_twice_does_not_reinsert() {
        let mut store = memory();
        seed(&mut store);
        seed(&mut store);
        let config = load_config(&mut store);
        let matching = config["workflows"]
            .as_array()
            .expect("workflows")
            .iter()
            .filter(|w| w["id"] == DEFAULT_TASK_WORKFLOW_ID)
            .count();
        assert_eq!(matching, 1);
    }

    #[test]
    fn a_deleted_default_workflow_is_not_seeded_again() {
        let mut store = memory();
        seed(&mut store);
        call(
            &mut store,
            "dbDeleteWorkflow",
            json!([DEFAULT_TASK_WORKFLOW_ID]),
        );
        seed(&mut store);
        let config = load_config(&mut store);
        assert!(workflow(&config, DEFAULT_TASK_WORKFLOW_ID).is_none());
        assert_eq!(config["defaults"]["hasSeededDefaultTaskWorkflow"], true);
    }

    #[test]
    fn the_flag_survives_a_save_and_load() {
        let mut store = memory();
        seed(&mut store);
        let loaded = load_config(&mut store);
        save_config(&mut store, loaded, &[]);
        assert_eq!(
            load_config(&mut store)["defaults"]["hasSeededDefaultTaskWorkflow"],
            true
        );
    }

    #[test]
    fn a_workflow_already_holding_the_id_is_kept_and_the_flag_still_set() {
        let mut store = memory();
        save_config(
            &mut store,
            base_config(
                json!({ "theme": "dark", "shell": "/bin/zsh", "fontSize": 13 }),
                json!([{
                    "id": DEFAULT_TASK_WORKFLOW_ID, "name": "User Override", "icon": "Zap",
                    "iconColor": "#ff0000", "nodes": [], "edges": [], "enabled": false,
                    "workspaceId": "personal"
                }]),
            ),
            &[],
        );
        seed(&mut store);
        let config = load_config(&mut store);
        let matches: Vec<&Value> = config["workflows"]
            .as_array()
            .expect("workflows")
            .iter()
            .filter(|w| w["id"] == DEFAULT_TASK_WORKFLOW_ID)
            .collect();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["name"], "User Override");
        assert_eq!(config["defaults"]["hasSeededDefaultTaskWorkflow"], true);
    }

    #[test]
    fn a_flag_set_to_false_still_seeds() {
        let mut store = memory();
        save_config(
            &mut store,
            base_config(
                json!({
                    "theme": "dark", "shell": "/bin/zsh", "fontSize": 13,
                    "hasSeededDefaultTaskWorkflow": false
                }),
                json!([]),
            ),
            &[],
        );
        seed(&mut store);
        let config = load_config(&mut store);
        assert!(workflow(&config, DEFAULT_TASK_WORKFLOW_ID).is_some());
        assert_eq!(config["defaults"]["hasSeededDefaultTaskWorkflow"], true);
    }

    #[test]
    fn the_restore_example_is_seeded_switched_off() {
        let mut store = memory();
        seed(&mut store);
        let config = load_config(&mut store);
        let seeded = workflow(&config, DEV_SERVER_WORKFLOW_ID).expect("seeded");
        assert_eq!(seeded["name"], "Bring the dev server back");
        assert_eq!(seeded["enabled"], false);
        let trigger = node_of_type(&seeded, "trigger").expect("a trigger");
        assert_eq!(trigger["config"]["triggerType"], "sessionRestored");
        assert!(node_of_type(&seeded, "script").is_some());
    }

    #[test]
    fn the_restore_example_stays_deleted_and_its_flag_survives_a_save() {
        let mut store = memory();
        seed(&mut store);
        call(
            &mut store,
            "dbDeleteWorkflow",
            json!([DEV_SERVER_WORKFLOW_ID]),
        );
        let loaded = load_config(&mut store);
        save_config(&mut store, loaded, &[]);
        seed(&mut store);
        let config = load_config(&mut store);
        assert!(workflow(&config, DEV_SERVER_WORKFLOW_ID).is_none());
        assert_eq!(config["defaults"]["hasSeededDevServerWorkflow"], true);
        assert!(workflow(&config, DEFAULT_TASK_WORKFLOW_ID).is_some());
    }
}
