use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};
use vorn_extensions::api::types::{Request, Response};
use vorn_extensions::api::ExtError;
use vorn_extensions::events::{AGENT_PERMISSION, AGENT_PERMISSION_CLOSED, WORKTREE_IDLE};
use vorn_extensions::gate::{Answer, Caller, World};
use vorn_extensions::lifecycle::{Lifecycle, Policy};
use vorn_extensions::pack::PackStore;
use vorn_extensions::router::{Router, RouterSettings};

use super::*;

/// Answers the core methods the built-ins call, and keeps what they asked.
#[derive(Default)]
struct Core {
    calls: Mutex<Vec<(String, Value)>>,
}

impl World for Core {
    fn call(&self, caller: &Caller, method: &str, args: Value) -> Answer<Value> {
        guard(&self.calls).push((format!("{} {method}", caller.extension), args));
        let answer = match method {
            "worktrees.list" => json!([
                { "projectPath": "/p", "projectName": "p", "path": "/p", "name": "p",
                  "branch": "main", "main": true, "orphan": false, "dirty": false,
                  "sizeBytes": 0, "artifactBytes": 0, "sessionIds": [], "verdict": "keep", "reasons": "main" },
                { "projectPath": "/p", "projectName": "p", "path": "/p/.vorn-worktrees/fix", "name": "fix",
                  "branch": "fix", "main": false, "orphan": false, "dirty": true,
                  "sizeBytes": 2048, "artifactBytes": 0, "sessionIds": ["t1"], "verdict": "review", "reasons": "dirty" }
            ]),
            "worktrees.remove" => json!({ "removed": true }),
            "sessions.list" => json!([
                { "id": "t1", "name": "api", "agent": "claude", "status": "waiting" }
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
}

impl Core {
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
    let router = Router::new(
        PackStore::new(dir.path().join("connectors")),
        Lifecycle::new(Policy::default()),
        Arc::clone(&core) as Arc<dyn World>,
        RouterSettings::new(Vec::new(), "test"),
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
async fn every_shipped_template_reads_without_an_error() {
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
            ("vorn.worktrees", "sidebar"),
            ("vorn.worktrees", "settings"),
            ("vorn.worktrees", "menus"),
            ("vorn.worktrees", "dialogs"),
            ("vorn.widget", "commands"),
            ("vorn.widget", "windows"),
            ("vorn.widget", "windows"),
        ]
    );
    for c in &listed {
        assert_eq!(c["errors"], json!([]), "{c}");
        assert_eq!(c["builtin"], true);
    }
}

#[tokio::test]
async fn offers_an_idle_worktree_and_forgets_it_when_kept() {
    let s = setup();
    s.builtins.start();
    // A worktree with a session still in it is not offered.
    s.router.bus().emit(WORKTREE_IDLE, || {
        json!({ "sessionId": "t1", "projectPath": "/p", "worktreePath": "/p/.vorn-worktrees/fix" })
    });
    until(|| !s.core.asked("worktrees.list").is_empty()).await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(s.router.rows("vorn.worktrees", "idle").is_empty());

    let asked = s
        .builtins
        .call(
            "vorn.worktrees",
            "ask",
            json!({ "worktree": "/p/.vorn-worktrees/fix", "project": "/p" }),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(asked, json!({ "open": "confirm" }));
    let row = &s.router.rows("vorn.worktrees", "asked")[0];
    assert_eq!(
        (row["sessions"].clone(), row["dirty"].clone()),
        (json!(1), json!(true))
    );

    let removed = s
        .builtins
        .call("vorn.worktrees", "confirm", Value::Null)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(removed["toast"], "Worktree removed");
    assert_eq!(
        s.core.asked("sessions.close"),
        [json!({ "sessionId": "t1" })]
    );
    assert_eq!(
        s.core.asked("worktrees.remove"),
        [json!({ "worktree": "/p/.vorn-worktrees/fix", "project": "/p", "force": true })]
    );
    assert!(s.router.rows("vorn.worktrees", "asked").is_empty());

    let main = s
        .builtins
        .call(
            "vorn.worktrees",
            "ask",
            json!({ "worktree": "/p", "project": "/p" }),
        )
        .await
        .unwrap();
    assert!(main
        .unwrap_err()
        .message
        .contains("not a worktree Vorn removes"));
}

#[tokio::test]
async fn lists_every_worktree_for_settings() {
    let s = setup();
    s.builtins
        .call("vorn.worktrees", "scan", Value::Null)
        .await
        .unwrap()
        .unwrap();
    let rows = s.router.rows("vorn.worktrees", "worktrees");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1]["size"], "2 KB");
    assert_eq!(
        (
            rows[0]["remove_clean"].clone(),
            rows[1]["remove_dirty"].clone()
        ),
        (json!(false), json!(true))
    );
}

#[tokio::test]
async fn the_widget_is_off_until_switched_on_and_kept_so() {
    let s = setup();
    s.builtins.start();
    let off = s
        .builtins
        .call("vorn.widget", "allow", json!({ "id": "r1" }))
        .await
        .unwrap();
    assert_eq!(off.unwrap_err().code, "off");
    assert!(s
        .builtins
        .contributions()
        .iter()
        .all(|c| c["extension"] != "vorn.widget"));

    s.builtins.set("vorn.widget", true).unwrap();
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
    s.builtins
        .call("vorn.widget", "allow", json!({ "id": "r1" }))
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
