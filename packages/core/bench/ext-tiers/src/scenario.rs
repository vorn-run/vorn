//! The scripted sessions every tier plays: the same calls and events, and
//! the rows each step leaves, which must be the same under every tier.

use serde_json::{json, Value};

use crate::tier::Tier;
use crate::world::{session_id, TERMINALS};

/// One step's answer and the rows after it.
async fn step(log: &mut Vec<Value>, name: &str, answer: Value, tier: &Tier) {
    tier.quiet().await;
    log.push(json!({ "step": name, "answer": answer, "rows": tier.snapshot() }));
}

fn answered(r: Result<Value, vorn_extensions::api::ExtError>) -> Value {
    match r {
        Ok(v) => json!({ "ok": v }),
        Err(e) => json!({ "err": e.code }),
    }
}

/// A permission request as an agent's hook sends it.
pub fn request(id: &str, terminal: usize, kind: usize) -> Value {
    match kind % 3 {
        0 => json!({
            "requestId": id, "terminalId": session_id(terminal), "toolName": "Bash",
            "toolInput": { "command": "npm test" },
            "permissionSuggestions": [
                { "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": "npm test" }] },
                { "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": "src/**" }] },
            ],
        }),
        1 => json!({
            "requestId": id, "terminalId": session_id(terminal), "toolName": "AskUserQuestion",
            "questions": [
                { "header": "Colour", "question": "Which theme?", "options": [{ "label": "Dark" }, { "label": "Light" }] },
                { "question": "Ship it?", "options": [{ "label": "Yes" }, { "label": "No" }] },
            ],
        }),
        _ => json!({
            "requestId": id, "terminalId": session_id(terminal), "toolName": "Edit",
            "description": "Edit src/main.rs", "projectName": "api",
            "permissionSuggestions": [{ "type": "setMode", "mode": "acceptEdits" }],
        }),
    }
}

pub async fn widget(tier: &Tier) -> Vec<Value> {
    let mut log = Vec::new();
    tier.start().await.expect("started");
    tier.emit("session.created", &json!({ "sessionId": session_id(0) }))
        .await;
    for i in 0..TERMINALS {
        let status = json!({ "sessionId": session_id(i), "status": "waiting" });
        tier.emit("session.status", &status).await;
    }
    step(&mut log, "sessions", Value::Null, tier).await;
    for (n, id) in ["p1", "p2", "p3", "p4"].iter().enumerate() {
        tier.emit("agent.permission", &request(id, n + 1, n)).await;
    }
    step(&mut log, "asking", Value::Null, tier).await;
    for (action, id) in [
        ("pick", "p2:0:1"),
        ("submit", "p2"),
        ("pick", "p2:1:0"),
        ("submit", "p2"),
        ("suggest", "p1:1"),
        ("allow", "p4"),
        ("deny", "nope"),
        ("fly", "p3"),
    ] {
        let a = answered(tier.call(action, json!({ "id": id })).await);
        step(&mut log, &format!("{action} {id}"), a, tier).await;
    }
    tier.emit("agent.permissionClosed", &json!({ "requestId": "p3" }))
        .await;
    tier.emit("session.exited", &json!({ "sessionId": session_id(3) }))
        .await;
    step(&mut log, "closed", Value::Null, tier).await;
    log
}

pub async fn worktrees(tier: &Tier) -> Vec<Value> {
    let mut log = Vec::new();
    tier.start().await.expect("started");
    let w = |i: usize| {
        let project = if i.is_multiple_of(2) {
            "/p/api"
        } else {
            "/p/web"
        };
        (format!("{project}/.worktrees/w{i}"), project.to_owned())
    };
    let a = answered(tier.call("scan", json!({})).await);
    step(&mut log, "scan", a, tier).await;
    for i in [0, 1, 2, 5, 7, 8] {
        let (path, project) = w(i);
        tier.emit(
            "worktree.idle",
            &json!({ "worktreePath": path, "projectPath": project }),
        )
        .await;
    }
    step(&mut log, "idle", Value::Null, tier).await;
    let (dirty, dirty_project) = w(3);
    let (clean, clean_project) = w(4);
    let calls = [
        (
            "ask",
            json!({ "worktree": dirty, "project": dirty_project }),
        ),
        ("cancel", json!({})),
        ("confirm", json!({})),
        (
            "ask",
            json!({ "worktree": dirty, "project": dirty_project }),
        ),
        ("confirm", json!({})),
        ("keep", json!({ "id": w(1).0 })),
        ("remove", json!({ "id": w(2).0 })),
        (
            "ask",
            json!({ "worktree": clean, "project": clean_project }),
        ),
        ("reclaim", json!({ "id": w(8).0 })),
        ("prune", json!({ "id": w(11).0 })),
        ("remove", json!({ "id": w(0).0 })),
        ("refresh", json!({})),
        ("fly", json!({})),
    ];
    for (action, args) in calls {
        let a = answered(tier.call(action, args).await);
        step(&mut log, action, a, tier).await;
    }
    log
}
