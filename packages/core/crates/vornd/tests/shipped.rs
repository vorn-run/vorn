//! The shipped set against the real vornd: a first launch installs it with
//! no network, each one an ordinary install that an uninstall removes whole
//! (files, tables, the agent it brought, the service it ran) and no later
//! start puts back; each moved built-in still works installed; an
//! older shipped install is updated; and with every adapter gone an agent
//! still runs in a plain terminal. Skipped by a build that carries no packs.

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;
use vorn_term_proto::bytes::BytesFrame;

use common::{connect, health, send, serve_in, Socket, CREDENTIAL, PATIENCE};

struct Ws {
    ws: Socket,
    next_id: u64,
    output: Vec<u8>,
}

impl Ws {
    async fn open(port: u16) -> Ws {
        Ws {
            ws: connect(port, CREDENTIAL).await,
            next_id: 0,
            output: Vec::new(),
        }
    }

    async fn call(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        send(
            &mut self.ws,
            json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        )
        .await;
        loop {
            let frame = self.read().await;
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

    /// The next text frame, keeping any terminal output on the way.
    async fn read(&mut self) -> Value {
        loop {
            let msg = tokio::time::timeout(PATIENCE, self.ws.next())
                .await
                .expect("a frame in time")
                .expect("open")
                .unwrap();
            match msg {
                Message::Binary(b) => {
                    if let Ok(f) = BytesFrame::decode(&b) {
                        self.output.extend_from_slice(f.data);
                    }
                }
                Message::Text(t) => return serde_json::from_str(t.as_str()).unwrap(),
                _ => {}
            }
        }
    }
}

fn carried() -> bool {
    if vornd::shipped::SET.is_empty() {
        eprintln!("skipped: this build carries no shipped set (see packages/core/shipped.toml)");
        return false;
    }
    true
}

fn defaults() -> Vec<&'static str> {
    vornd::shipped::SET
        .iter()
        .filter(|s| s.default)
        .map(|s| s.id)
        .collect()
}

fn agents() -> Vec<&'static str> {
    defaults()
        .into_iter()
        .filter(|id| id.starts_with("vorn.agents."))
        .collect()
}

fn ids(list: &Value, key: &str) -> Vec<String> {
    let mut out: Vec<String> = list
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p[key].as_str().map(str::to_owned))
        .collect();
    out.sort();
    out
}

fn tables(db: &Path, prefix: &str) -> Vec<String> {
    let conn = rusqlite::Connection::open(db).unwrap();
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE ?1")
        .unwrap();
    stmt.query_map([format!("{prefix}%")], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_first_launch_installs_the_shipped_set_offline_and_an_uninstall_leaves_nothing() {
    if !carried() {
        return;
    }
    let served = serve_in(tempfile::tempdir().unwrap());
    let data = served.data.path().to_path_buf();
    let mut c = Ws::open(served.port).await;

    // Installed before vornd answered, from the bytes it carries, like any install.
    let installed = c.result("extension:installed", json!({})).await;
    let mut want: Vec<String> = defaults().iter().map(|s| (*s).to_owned()).collect();
    want.sort();
    assert_eq!(ids(&installed, "id"), want, "{installed}");
    for pack in installed.as_array().unwrap() {
        let id = pack["id"].as_str().unwrap();
        let shipped = vornd::shipped::SET.iter().find(|s| s.id == id).unwrap();
        assert_eq!(
            pack["source"],
            json!({ "shipped": shipped.version, "sha256": shipped.sha256 }),
            "{pack}"
        );
        assert!(
            pack["icon"].is_object(),
            "{id} has no icon of its own: {pack}"
        );
        let dir = data.join("connectors").join(id).join(shipped.version);
        assert!(
            dir.join("vorn-extension.toml").is_file(),
            "{}",
            dir.display()
        );
    }
    let listed = c.result("agent:list", json!(null)).await;
    let mut names: Vec<String> = agents()
        .iter()
        .map(|id| id.trim_start_matches("vorn.agents.").to_owned())
        .collect();
    names.sort();
    assert_eq!(ids(&listed, "id"), names, "{listed}");

    // What worktrees keeps in the database goes with it.
    let db = data.join("vorn.db");
    let prefix = vorn_store::extension::table_prefix("vorn.worktrees");
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute_batch(&format!("CREATE TABLE \"{prefix}kv\" (k TEXT, v TEXT)"))
        .unwrap();
    assert_eq!(tables(&db, &prefix).len(), 1);

    for id in defaults() {
        let removed = c.result("extension:remove", json!({ "id": id })).await;
        assert_eq!(removed["removed"], id);
        assert!(
            !data.join("connectors").join(id).exists(),
            "{id} left files"
        );
    }
    assert!(tables(&db, &prefix).is_empty(), "worktrees left its tables");
    let shipped_running = || {
        let services = health(served.port)["extensions"]["services"].clone();
        let ids = services.as_array().cloned().unwrap_or_default();
        ids.iter()
            .any(|s| s.as_str().is_some_and(|s| defaults().contains(&s)))
    };
    let t = Instant::now();
    while shipped_running() {
        assert!(
            t.elapsed() < PATIENCE,
            "a service outlived its extension: {}",
            health(served.port)
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let left = c.result("extension:installed", json!({})).await;
    assert_eq!(left, json!([]), "{left}");
    assert_eq!(c.result("agent:list", json!(null)).await, json!([]));
    let points = c.result("extension:contributions", json!({})).await;
    assert!(
        !points.as_array().unwrap().iter().any(|p| p["extension"]
            .as_str()
            .is_some_and(|e| defaults().contains(&e))),
        "{points}"
    );

    // No later start puts back what was removed.
    drop(c);
    let data = {
        let mut served = served;
        std::mem::replace(&mut served.data, tempfile::tempdir().unwrap())
    };
    let again = serve_in(data);
    let mut c = Ws::open(again.port).await;
    assert_eq!(c.result("extension:installed", json!({})).await, json!([]));
}

/// The tree `id` draws at `point`, as a client asks for it.
async fn drawn(c: &mut Ws, id: &str, point: &str) -> String {
    let answer = c
        .result(
            "extension:call",
            json!({ "id": id, "action": vorn_tree::RENDER,
                    "args": { "point": point, "surface": {} } }),
        )
        .await;
    let tree = vorn_tree::read(&answer)
        .unwrap_or_else(|errs| panic!("{id} drew no tree: {errs:?} {answer}"));
    serde_json::to_string(&tree.tree).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_moved_built_ins_work_installed_and_their_switch_carries_over() {
    let moved = [
        "vorn.files",
        "vorn.blocks",
        "vorn.widget",
        "vorn.browser",
        "vorn.device",
        "vorn.ssh",
    ];
    if moved
        .iter()
        .any(|id| !vornd::shipped::SET.iter().any(|s| s.id == *id))
    {
        eprintln!("skipped: this build does not ship every moved built-in");
        return;
    }
    // The Widget is off by default; a switch left on installs it, and one left off removes SSH.
    let data = tempfile::tempdir().unwrap();
    std::fs::write(
        data.path().join("builtins.json"),
        r#"{"vorn.widget": true, "vorn.ssh": false}"#,
    )
    .unwrap();
    let served = serve_in(data);
    let mut c = Ws::open(served.port).await;
    let installed = ids(&c.result("extension:installed", json!({})).await, "id");
    assert!(
        installed.iter().any(|i| i == "vorn.widget"),
        "{installed:?}"
    );
    assert!(!installed.iter().any(|i| i == "vorn.ssh"), "{installed:?}");

    assert!(!drawn(&mut c, "vorn.widget", "compact").await.is_empty());
    assert!(!drawn(&mut c, "vorn.files", "files").await.is_empty());
    let history = c
        .result(
            "extension:call",
            json!({ "id": "vorn.blocks", "action": "history", "args": { "text": "echo" } }),
        )
        .await;
    assert!(history.is_array() || history.is_object(), "{history}");

    // Browser and Device start with vornd, through their manifest's start.
    let t = Instant::now();
    loop {
        let services = health(served.port)["extensions"]["services"].clone();
        let has = |id: &str| services.as_array().unwrap().iter().any(|s| s == id);
        if has("vorn.browser") && has("vorn.device") {
            break;
        }
        assert!(t.elapsed() < PATIENCE, "not started: {services}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    for (id, method, name) in [
        ("vorn.browser", "browser:listTabs", "Browser"),
        ("vorn.device", "device:claim", "Device"),
    ] {
        c.result("extension:remove", json!({ "id": id })).await;
        let refused = c
            .call(method, json!({ "sessionId": "s", "udid": "u" }))
            .await;
        assert_eq!(
            refused["error"]["message"],
            format!("{name} is not installed. Install it in Settings \u{2192} Extensions."),
            "{refused}"
        );
    }
    let ssh = c
        .call(
            "ssh:testConnection",
            json!({ "id": "h", "hostname": "nowhere.invalid", "user": "nobody", "port": 22 }),
        )
        .await;
    assert_eq!(
        ssh["error"]["message"],
        "SSH is not installed. Install it in Settings \u{2192} Extensions.",
        "{ssh}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_older_shipped_install_is_updated_and_the_persons_own_is_left_alone() {
    if !carried() {
        return;
    }
    let Some(claude) = vornd::shipped::SET
        .iter()
        .find(|s| s.id == "vorn.agents.claude")
    else {
        return;
    };
    let data = tempfile::tempdir().unwrap();
    let packages = data.path().join("connectors");
    let manifest = |id: &str, version: &str| {
        format!(
            "id = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\napi = \"vorn:extension@1.0\"\npublisher = \"Vorn\"\n\n[actions.hello]\ntitle = \"Hello\"\ncommand = [\"sh\", \"hello.sh\"]\n"
        )
    };
    // An older shipped claude, and an extension of the person's own.
    for (id, version, source) in [
        ("vorn.agents.claude", "0.9.0", json!({ "shipped": "0.9.0" })),
        (
            "mine",
            "0.1.0",
            json!({ "git": "file:///mine", "rev": "abc" }),
        ),
    ] {
        let dir = packages.join(id).join(version);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("vorn-extension.toml"), manifest(id, version)).unwrap();
        std::fs::write(
            packages.join(id).join("current.json"),
            json!({ "version": version, "source": source }).to_string(),
        )
        .unwrap();
    }
    std::fs::write(data.path().join("shipped.json"), r#"{"declined":[]}"#).unwrap();

    let served = serve_in(data);
    let mut c = Ws::open(served.port).await;
    let installed = c.result("extension:installed", json!({})).await;
    let find = |id: &str| {
        installed
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("{id} is not installed: {installed}"))
    };
    let updated = find("vorn.agents.claude");
    assert_eq!(updated["version"], claude.version, "{updated}");
    assert_eq!(updated["previousVersion"], "0.9.0", "{updated}");
    assert_eq!(find("mine")["version"], "0.1.0");
}

/// Inspects `folder`, holding a manifest for `id` (an adapter of `agent`
/// when given), to install from git and to link.
async fn inspect_both(c: &mut Ws, folder: &Path, id: &str, agent: Option<&str>) -> Vec<Value> {
    let adds = match agent {
        Some(agent) => format!(
            "[agent]\nid = \"{agent}\"\nname = \"Mine\"\n\n[agent.launch]\ncommand = \"{agent}\"\n"
        ),
        None => "[actions.hello]\ntitle = \"Hello\"\ncommand = [\"sh\", \"hello.sh\"]\n".to_owned(),
    };
    std::fs::write(
        folder.join("vorn-extension.toml"),
        format!("id = \"{id}\"\nname = \"Mine\"\nversion = \"1.0.0\"\napi = \"vorn:extension@1.0\"\npublisher = \"me\"\n\n{adds}"),
    )
    .unwrap();
    let url = folder.display().to_string();
    let mut out = Vec::new();
    for params in [json!({ "url": url }), json!({ "url": url, "link": true })] {
        out.push(c.call("extension:inspect", params).await);
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_install_takes_a_shipped_agent_or_a_bundled_id() {
    if !carried() || !agents().contains(&"vorn.agents.claude") {
        return;
    }
    let served = serve_in(tempfile::tempdir().unwrap());
    let mut c = Ws::open(served.port).await;
    let folder = tempfile::tempdir().unwrap();
    for answer in inspect_both(&mut c, folder.path(), "acme.claude", Some("claude")).await {
        let err = answer["error"]["message"].as_str().unwrap_or_default();
        assert!(
            err.contains("which vorn.agents.claude already runs"),
            "{answer}"
        );
    }
    for id in ["vorn.files", "vorn.tasks", "vorn.agents.claude"] {
        for answer in inspect_both(&mut c, folder.path(), id, None).await {
            assert!(
                answer.get("error").is_some(),
                "{id} was not refused: {answer}"
            );
        }
    }
    let listed = c.result("agent:list", json!(null)).await;
    let claude = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "claude")
        .cloned();
    assert_eq!(
        claude.unwrap()["extension"],
        "vorn.agents.claude",
        "{listed}"
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_runs_in_a_plain_terminal_with_every_adapter_removed() {
    if !carried() {
        return;
    }
    let v = common::Vornd::start();
    let _reap = common::Reap(v.holder());
    let mut c = Ws::open(v.port).await;
    for id in agents() {
        c.result("extension:remove", json!({ "id": id })).await;
    }
    assert_eq!(c.result("agent:list", json!(null)).await, json!([]));

    // The agent's own program, run by hand in a shell like any other.
    let project = tempfile::tempdir().unwrap();
    let agent = project.path().join("claude");
    std::fs::write(&agent, "#!/bin/sh\necho agent-$((6*7))\n").unwrap();
    let cwd = project.path().to_str().unwrap().to_owned();
    let t = Instant::now();
    let shell = loop {
        let answer = c.call("shell:create", json!(cwd)).await;
        if answer.get("error").is_none() {
            break answer["result"]["id"].as_str().unwrap().to_owned();
        }
        assert!(t.elapsed() < PATIENCE, "{answer}\n{}", v.log());
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    c.result("terminal:attach", json!({ "id": shell })).await;
    c.result(
        "terminal:write",
        json!({ "id": shell, "data": "sh ./claude\r" }),
    )
    .await;
    let t = Instant::now();
    while !String::from_utf8_lossy(&c.output).contains("agent-42") {
        assert!(
            t.elapsed() < PATIENCE,
            "the agent did not run: {:?}\n{}",
            String::from_utf8_lossy(&c.output),
            v.log()
        );
        let _ = tokio::time::timeout(Duration::from_millis(200), c.read()).await;
    }
    c.call("terminal:kill", json!(shell)).await;
}
