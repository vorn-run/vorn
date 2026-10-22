use std::sync::Mutex;

use serde_json::{json, Value};
use vorn_extensions::api::types::{Request, Response};
use vorn_extensions::api::ExtError;
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

struct Setup {
    dir: tempfile::TempDir,
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
        core as Arc<dyn World>,
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
        router,
        builtins,
    }
}

#[tokio::test]
async fn every_shipped_contribution_draws_a_tree_or_draws_nothing() {
    let s = setup();
    let registry = Builtins::new(
        Arc::clone(&s.router),
        s.dir.path().join("registry.json"),
        vec![Arc::new(wasm::workflows())],
    );
    let listed = registry.contributions();
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
            ("vorn.workflows", "menus"),
            ("vorn.workflows", "menus"),
            ("vorn.workflows", "menus"),
            ("vorn.workflows", "views"),
            ("vorn.workflows", "attention"),
        ]
    );
    for c in &listed {
        assert_eq!(c["builtin"], true);
        assert!(c.get("view").is_none() && c.get("errors").is_none(), "{c}");
        let draws = !matches!(c["kind"].as_str(), Some("menus" | "commands" | "attention"));
        assert_eq!(c["tree"] == true, draws, "{c}");
    }
}

#[test]
fn lists_what_each_adds_and_uses_for_settings() {
    let s = setup();
    let list = s.builtins.list();
    let artifacts = list
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["id"] == "vorn.artifacts")
        .unwrap();
    assert_eq!(artifacts["on"], true);
    let uses = artifacts["declares"]["uses"].as_array().unwrap();
    assert!(uses.iter().any(|u| u == "sessions.read"), "{uses:?}");
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

#[test]
fn every_bundled_extension_s_id_is_kept_from_installs() {
    let ids = bundled_ids().expect("every bundled manifest reads");
    let s = setup();
    // The registry vornd builds, given what a test can start of the started built-ins.
    let registry = Builtins::new(
        Arc::clone(&s.router),
        s.dir.path().join("registry.json"),
        vec![Arc::new(wasm::workflows())],
    );
    let mut registered: std::collections::BTreeSet<String> = registry
        .list()
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_owned())
        .collect();
    for started in Started::ALL {
        registered.insert(file::parse_builtin(started.manifest()).unwrap().id);
    }
    assert_eq!(ids, registered);
    assert_eq!(wasm::workflows().manifest, Started::Workflows.manifest());
    for id in [
        "vorn.tasks",
        "vorn.workflows",
        "vorn.artifacts",
        "vorn.connectors",
    ] {
        assert!(ids.contains(id), "{id} is not in {ids:?}");
    }
}
