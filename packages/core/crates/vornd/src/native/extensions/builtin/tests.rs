use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};
use vorn_extensions::api::types::{Request, Response};
use vorn_extensions::api::ExtError;
use vorn_extensions::events::{
    AGENT_PERMISSION, AGENT_PERMISSION_CLOSED, SESSION_EXITED,
};
use vorn_extensions::gate::{Answer, Caller, StorageCall, World};
use vorn_extensions::lifecycle::{Lifecycle, Policy};
use vorn_extensions::pack::PackStore;
use vorn_extensions::router::{Router, RouterSettings};

use super::*;

/// Answers the core methods the built-ins call, and keeps what they asked
/// and what they store, by extension and key.
#[derive(Default)]
struct Core {
    calls: Mutex<Vec<(String, Value)>>,
    kept: Mutex<BTreeMap<(String, String), Value>>,
}

impl World for Core {
    fn call(&self, caller: &Caller, method: &str, args: Value) -> Answer<Value> {
        guard(&self.calls).push((format!("{} {method}", caller.extension), args));
        let answer = match method {
            "sessions.list" => json!([
                { "id": "t1", "name": "api", "agent": "acme", "status": "waiting" }
            ]),
            _ => Value::Null,
        };
        Box::pin(async move { Ok(answer) })
    }

    fn fetch(&self, _: &Caller, _: Request) -> Answer<Response> {
        Box::pin(async { Err(ExtError::new("network", "no network here")) })
    }

    fn secret(&self, _: &Caller, _: &str, _: &str) -> Answer<Option<String>> {
        Box::pin(async { Ok(None) })
    }

    fn storage(
        &self,
        caller: &Caller,
        _: Option<std::path::PathBuf>,
        call: StorageCall,
    ) -> Answer<Value> {
        let mut kept = guard(&self.kept);
        let answer = match call {
            StorageCall::Get { key } => kept
                .get(&(caller.extension.clone(), key))
                .cloned()
                .unwrap_or(Value::Null),
            StorageCall::Set { key, value } => {
                kept.insert((caller.extension.clone(), key), value);
                Value::Null
            }
            _ => Value::Null,
        };
        Box::pin(async move { Ok(answer) })
    }
}

impl Core {
    fn keep(&self, extension: &str, key: &str, value: Value) {
        guard(&self.kept).insert((extension.to_owned(), key.to_owned()), value);
    }

    /// The simulators the Device asked the client to shut down, in order.
    fn shut_down(&self) -> Vec<String> {
        self.asked("video.call")
            .iter()
            .filter(|a| a["op"] == "shutdown")
            .filter_map(|a| a["params"]["udid"].as_str().map(str::to_owned))
            .collect()
    }

    fn asked(&self, method: &str) -> Vec<Value> {
        guard(&self.calls)
            .iter()
            .filter(|(m, _)| m.ends_with(&format!(" {method}")))
            .map(|(_, a)| a.clone())
            .collect()
    }
}

struct Setup {
    dir: tempfile::TempDir,
    core: Arc<Core>,
    router: Arc<Router>,
    builtins: Builtins,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let core = Arc::new(Core::default());
    let mut settings = RouterSettings::new(Vec::new(), "test");
    // Worktree cleanup is a component; vornd's own build compiles it with the vorn command.
    settings.compiler = vorn_extensions::component::Compiler::InProcess;
    let router = Router::new(
        PackStore::new(dir.path().join("connectors")),
        Lifecycle::new(Policy::default()),
        Arc::clone(&core) as Arc<dyn World>,
        settings,
        vorn_extensions::events::Bus::new(),
    );
    let builtins = Builtins::new(
        Arc::clone(&router),
        dir.path().join("builtins.json"),
        Vec::new(),
    );
    Setup {
        dir,
        core,
        router,
        builtins,
    }
}

async fn until(mut done: impl FnMut() -> bool) {
    for _ in 0..200 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("never happened");
}

#[tokio::test]
async fn every_shipped_contribution_draws_a_tree_or_draws_nothing() {
    let s = setup();
    s.builtins.set("vorn.widget", true).unwrap();
    let listed = s.builtins.contributions();
    let kinds: Vec<(&str, &str)> = listed
        .iter()
        .map(|c| {
            (
                c["extension"].as_str().unwrap(),
                c["kind"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        kinds,
        [
            ("vorn.files", "panes"),
            ("vorn.files", "cards"),
            ("vorn.browser", "panes"),
            ("vorn.device", "panes"),
            ("vorn.ssh", "settings"),
            ("vorn.blocks", "shell"),
            ("vorn.widget", "commands"),
            ("vorn.widget", "windows"),
            ("vorn.widget", "windows"),
        ]
    );
    for c in &listed {
        assert_eq!(c["builtin"], true);
        assert!(c.get("view").is_none() && c.get("errors").is_none(), "{c}");
        let draws = !matches!(c["kind"].as_str(), Some("menus" | "commands"));
        assert_eq!(c["tree"] == true, draws, "{c}");
    }
}

/// Starts the Widget's component, as a window asking for its tree does.
async fn widget_drawn(s: &Setup, point: &str) -> String {
    let answer = s
        .builtins
        .call(
            "vorn.widget",
            vorn_tree::RENDER,
            json!({ "point": point, "surface": {} }),
        )
        .await
        .unwrap()
        .unwrap();
    let tree = vorn_tree::read(&answer).expect("a tree the client draws");
    serde_json::to_string(&tree.tree).unwrap()
}

#[test]
fn lists_what_each_adds_and_uses_for_settings() {
    let s = setup();
    let list = s.builtins.list();
    let widget = list
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["id"] == "vorn.widget")
        .unwrap();
    assert_eq!(widget["on"], false);
    assert!(widget["contributes"]["windows"]
        .as_array()
        .is_some_and(|w| !w.is_empty()));
    let uses = widget["declares"]["uses"].as_array().unwrap();
    assert!(uses.iter().any(|u| u == "sessions.read"), "{uses:?}");
}

#[tokio::test]
async fn the_widget_is_off_until_switched_on_and_kept_so() {
    let s = setup();
    s.builtins.start();
    let off = s
        .builtins
        .call("vorn.widget", "allow", json!({ "id": "r1" }))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(off.code, "off");
    assert!(off.message.ends_with("Settings › Extensions"), "{off}");
    assert!(s
        .builtins
        .contributions()
        .iter()
        .all(|c| c["extension"] != "vorn.widget"));

    s.builtins.set("vorn.widget", true).unwrap();
    assert!(widget_drawn(&s, "compact").await.contains("waiting"));
    until(|| s.router.rows("vorn.widget", "agents").len() == 1).await;
    s.router.bus().emit(AGENT_PERMISSION, || {
        json!({ "requestId": "r1", "terminalId": "t1", "toolName": "Bash",
                "toolInput": { "command": "cargo test" } })
    });
    until(|| s.router.rows("vorn.widget", "permissions").len() == 1).await;
    let row = &s.router.rows("vorn.widget", "permissions")[0];
    assert_eq!(
        (row["agent"].clone(), row["description"].clone()),
        (json!("api"), json!("cargo test"))
    );
    assert!(widget_drawn(&s, "widget").await.contains("allow:r1"));
    let click = json!({ "point": "widget", "event": { "id": "allow:r1", "kind": "click" } });
    s.builtins
        .call("vorn.widget", vorn_tree::EVENT, click)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        s.core.asked("permissions.resolve"),
        [json!({ "requestId": "r1", "allow": true })]
    );
    assert!(s.router.rows("vorn.widget", "permissions").is_empty());

    s.router.bus().emit(
        AGENT_PERMISSION,
        || json!({ "requestId": "r2", "toolName": "Edit" }),
    );
    until(|| s.router.rows("vorn.widget", "permissions").len() == 1).await;
    s.router
        .bus()
        .emit(AGENT_PERMISSION_CLOSED, || json!({ "requestId": "r2" }));
    until(|| s.router.rows("vorn.widget", "permissions").is_empty()).await;

    let again = Builtins::new(
        Arc::clone(&s.router),
        s.dir.path().join("builtins.json"),
        Vec::new(),
    );
    assert!(again.is_on("vorn.widget"));
    s.builtins.set("vorn.widget", false).unwrap();
    assert!(s.router.rows("vorn.widget", "agents").is_empty());
    s.router
        .bus()
        .emit(AGENT_PERMISSION, || json!({ "requestId": "r3" }));
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(s.router.rows("vorn.widget", "permissions").is_empty());
}

#[tokio::test]
async fn the_widget_keeps_an_offered_rule_and_answers_questions() {
    let s = setup();
    s.builtins.start();
    s.builtins.set("vorn.widget", true).unwrap();
    widget_drawn(&s, "widget").await;
    until(|| s.router.rows("vorn.widget", "agents").len() == 1).await;
    let rows = |c: &str| s.router.rows("vorn.widget", c);
    s.router.bus().emit(AGENT_PERMISSION, || {
        json!({ "requestId": "r1", "terminalId": "t1", "toolName": "Bash",
                "toolInput": { "command": "npm test" },
                "permissionSuggestions": [
                    { "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": "npm test" }] },
                    { "type": "setMode", "mode": "acceptEdits" }
                ] })
    });
    until(|| rows("suggestions").len() == 2).await;
    assert_eq!(rows("suggestions")[1]["label"], "Auto-accept edits");
    s.builtins
        .call("vorn.widget", "suggest", json!({ "id": "r1:0" }))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        s.core.asked("permissions.resolve"),
        [json!({ "requestId": "r1", "allow": true, "suggestion": 0 })]
    );
    assert!(rows("permissions").is_empty() && rows("suggestions").is_empty());

    s.router.bus().emit(AGENT_PERMISSION, || {
        json!({ "requestId": "r2", "terminalId": "t1", "toolName": "AskUserQuestion",
                "questions": [
                    { "header": "Theme", "question": "Which?", "options": [{ "label": "Dark" }, { "label": "Light" }] },
                    { "question": "Tabs?", "options": [{ "label": "Yes" }] }
                ] })
    });
    until(|| rows("options").len() == 3).await;
    assert_eq!(rows("permissions")[0]["question"], json!(true));
    assert_eq!(rows("options")[0]["ask"], "THEME: Which?");
    let call = |action: &'static str, id: &'static str| {
        s.builtins.call("vorn.widget", action, json!({ "id": id }))
    };
    call("pick", "r2:0:0").await.unwrap().unwrap();
    call("pick", "r2:0:1").await.unwrap().unwrap();
    assert!(call("submit", "r2").await.unwrap().is_err());
    assert_eq!(rows("permissions")[0]["ready"], json!(false));
    call("pick", "r2:1:0").await.unwrap().unwrap();
    assert_eq!(rows("permissions")[0]["ready"], json!(true));
    let picked: Vec<Value> = rows("options")
        .iter()
        .map(|o| o["picked"].clone())
        .collect();
    assert_eq!(picked, [json!(false), json!(true), json!(true)]);
    call("submit", "r2").await.unwrap().unwrap();
    assert_eq!(
        s.core.asked("permissions.resolve")[1],
        json!({ "requestId": "r2", "allow": true, "answers": { "0": "Light", "1": "Yes" } })
    );
    assert!(rows("permissions").is_empty() && rows("options").is_empty());
}

#[tokio::test]
async fn the_device_lets_go_of_ended_sessions_simulators_and_of_all_once_off() {
    let s = setup();
    let device = device::component();
    // Compiled before the clock starts, as vornd does ahead: `until` times the Device, not the compiler.
    let bundled = vorn_extensions::component::Bundled {
        manifest: Arc::new(file::parse_builtin(device.manifest).unwrap()),
        wasm: device.wasm,
    };
    s.router.prepare_bundled(&bundled).await.unwrap();
    // Only the Device, so no other built-in compiles beside it.
    let builtins = Builtins::with(
        Arc::clone(&s.router),
        s.dir.path().join("device.json"),
        vec![Arc::new(device)],
    );
    let claim = |udid: &str| json!({ "udid": udid, "name": "iPhone", "bootedHere": true });
    // Kept from before vornd started: `gone` ended meanwhile, `t1` still runs.
    s.core.keep(
        device::ID,
        "claims",
        json!({ "gone": claim("u1"), "t1": claim("u2") }),
    );
    builtins.start();
    until(|| s.core.shut_down() == ["u1"]).await;

    s.router.bus().emit(
        SESSION_EXITED,
        || json!({ "sessionId": "t1", "kind": "terminal" }),
    );
    until(|| s.core.shut_down() == ["u1", "u2"]).await;
    assert_eq!(
        guard(&s.core.kept)[&(device::ID.to_owned(), "claims".to_owned())],
        json!({})
    );

    s.core
        .keep(device::ID, "claims", json!({ "t1": claim("u3") }));
    builtins.set(device::ID, false).unwrap();
    until(|| s.core.shut_down() == ["u1", "u2", "u3"]).await;
    assert!(s.router.rows(device::ID, "open").is_empty());
}

#[test]
fn the_old_workflow_editor_switch_folds_into_workflows_once() {
    // (Workflows as kept, the old editor as kept, Workflows after.)
    let cases = [
        (Some(true), true, true),
        (Some(false), true, true),
        (Some(true), false, true),
        (Some(false), false, false),
        (None, false, true),
    ];
    for (workflows, editor, merged) in cases {
        let s = setup();
        let file = s.dir.path().join("builtins.json");
        let mut kept = serde_json::Map::new();
        if let Some(on) = workflows {
            kept.insert(vorn_workflows::ID.into(), json!(on));
        }
        kept.insert(OLD_WORKFLOW_EDITOR.into(), json!(editor));
        std::fs::write(&file, Value::Object(kept).to_string()).unwrap();

        let started: Vec<Arc<dyn Builtin>> = vec![Arc::new(wasm::workflows())];
        let b = Builtins::new(Arc::clone(&s.router), file.clone(), started);
        assert_eq!(
            b.is_on(vorn_workflows::ID),
            merged,
            "{workflows:?} {editor}"
        );
        let saved: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(saved.get(OLD_WORKFLOW_EDITOR), None);
        assert_eq!(saved[vorn_workflows::ID], json!(merged));
    }
}

#[test]
fn tasks_and_workflows_buttons_follow_their_top_bar_setting() {
    let workflows = vorn_extensions::manifest::file::parse_builtin(wasm::workflows().manifest())
        .expect("the Workflows manifest");
    let s = setup();
    for (m, view) in [(tasks::manifest(), "board"), (workflows, "workflows")] {
        let listed = |on: bool| {
            let settings = serde_json::Map::from_iter([("top_bar".to_owned(), json!(on))]);
            vorn_extensions::points::listed(&m, &settings)
        };
        let button = |list: &[Value]| {
            let v = list
                .iter()
                .find(|p| p["kind"] == "views" && p["id"] == view);
            v.map(|v| v["button"].clone())
        };
        assert_eq!(button(&listed(true)), Some(json!(true)), "{view}");
        assert_eq!(
            button(&listed(false)),
            Some(json!(false)),
            "{view} stays a view"
        );
        let defaults = s.router.preferences(&m);
        assert_eq!(
            defaults.get("top_bar"),
            Some(&json!(true)),
            "{view} on by default"
        );
    }
    // The Tasks pane is in the card menu whatever the top bar shows.
    let m = tasks::manifest();
    let off = serde_json::Map::from_iter([("top_bar".to_owned(), json!(false))]);
    assert!(vorn_extensions::points::listed(&m, &off)
        .iter()
        .any(|p| p["kind"] == "panes" && p["id"] == "tasks"));
}
