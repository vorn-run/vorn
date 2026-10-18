//! Tasks, the component Vorn ships, against the real vornd: the board's
//! writes, the core's reads of them, the card badge's rows, a task's image
//! where an agent can read it, the MCP tools, and the switch, which off
//! refuses writes while the core still lists every task. Skipped without the
//! vorn command beside vornd, which compiles the component.

mod common;

use std::path::Path;

use common::{connect, next, send, serve, Socket, CREDENTIAL};
use serde_json::{json, Value};

struct Ws {
    ws: Socket,
    next_id: u64,
}

impl Ws {
    async fn call(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        send(
            &mut self.ws,
            json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        )
        .await;
        loop {
            let frame = next(&mut self.ws).await;
            if frame["id"] == json!(id) {
                return frame;
            }
        }
    }

    async fn result(&mut self, method: &str, params: Value) -> Value {
        let answer = self.call(method, params).await;
        assert!(answer.get("error").is_none(), "{method}: {answer}");
        answer["result"].clone()
    }
}

fn compiler_beside_vornd() -> bool {
    let vornd = Path::new(env!("CARGO_BIN_EXE_vornd"));
    let vorn = vornd.with_file_name(format!("vorn{}", std::env::consts::EXE_SUFFIX));
    if !vorn.exists() {
        eprintln!("skipped: no vorn command beside {}", vornd.display());
    }
    vorn.exists()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tasks_write_through_its_component_and_keep_the_board_when_off() {
    if !compiler_beside_vornd() {
        return;
    }
    let served = serve();
    let mut c = Ws {
        ws: connect(served.port, CREDENTIAL).await,
        next_id: 0,
    };
    let cfg = c.result("config:load", Value::Null).await;
    let mut cfg = cfg.clone();
    cfg["projects"] = json!([{ "name": "app", "path": served.data.path(), "preferredAgents": [] }]);
    c.result("config:save", cfg).await;

    let made = c
        .result(
            "task:create",
            json!({ "projectName": "app", "title": "Ship it", "status": "in_progress" }),
        )
        .await;
    assert_eq!(made["ok"], true, "{made}");
    let id = made["task"]["id"].as_str().unwrap().to_owned();
    let read = c.result("task:get", json!({ "id": id })).await;
    assert_eq!(read["title"], "Ship it");

    // A session working on it shows it on its card, through the rows its badge tree draws.
    let set = json!({ "id": id, "set": { "assignedSessionId": "s1" } });
    c.result("task:patch", set).await;
    let mut rows = Value::Null;
    for _ in 0..200 {
        rows = c
            .result(
                "extension:rows",
                json!({ "id": "vorn.tasks", "collection": "working" }),
            )
            .await;
        if rows.as_array().is_some_and(|r| !r.is_empty()) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert_eq!(
        rows,
        json!([{ "id": "s1", "task": id, "title": "Ship it" }])
    );

    // An image is kept where the board always kept them, and its path reads.
    let name = c
        .result(
            "task:imageUpload",
            json!({ "taskId": id, "base64": "cG5n", "filename": "shot.png" }),
        )
        .await;
    let path = c
        .result(
            "task:imageGetPath",
            json!({ "taskId": id, "filename": name }),
        )
        .await;
    let path = path.as_str().unwrap();
    assert!(
        Path::new(path).starts_with(served.data.path().join("task-images").join(&id)),
        "{path}"
    );
    assert_eq!(std::fs::read(path).unwrap(), b"png");
    let refused = c
        .call(
            "task:imageGetPath",
            json!({ "taskId": "../x", "filename": "a.png" }),
        )
        .await;
    assert_eq!(refused["error"]["message"], "Invalid taskId: ../x");

    // Switched off, writes are refused and the core still lists the board.
    c.result(
        "extension:setBuiltin",
        json!({ "id": "vorn.tasks", "on": false }),
    )
    .await;
    let off = c
        .call("task:setStatus", json!({ "id": id, "status": "done" }))
        .await;
    assert_eq!(
        off["error"]["message"],
        "Tasks is off. Turn it on in Settings → Extensions."
    );
    let listed = c.result("task:list", json!({})).await;
    assert_eq!(listed[0]["id"], id);
    c.result(
        "extension:setBuiltin",
        json!({ "id": "vorn.tasks", "on": true }),
    )
    .await;
    let moved = c
        .result("task:setStatus", json!({ "id": id, "status": "done" }))
        .await;
    assert_eq!(moved, json!({ "ok": true }));
    assert_eq!(
        c.result("task:get", json!({ "id": id })).await["status"],
        "done"
    );
}
