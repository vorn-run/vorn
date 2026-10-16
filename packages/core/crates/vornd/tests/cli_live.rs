//! The `vorn` command against a real vornd and session holder, on a home and
//! data directory of the test's own: targets, `session wait`, `session read`,
//! the event stream and saved layouts. Skipped without a `vorn` binary built
//! beside vornd.

#![cfg(unix)]

mod common;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};

use common::{Reap, Socket, Vornd, CREDENTIAL, PATIENCE};

fn vorn_bin() -> Option<PathBuf> {
    let bin = Path::new(common::VORND).parent()?.join("vorn");
    if bin.exists() {
        Some(bin)
    } else {
        eprintln!("skipped: no vorn binary beside vornd (cargo build -p vorn-cli)");
        None
    }
}

/// vornd, its holder, a socket to call it on and the `vorn` command pointed at it.
struct Rig {
    v: Vornd,
    _reap: Reap,
    ws: Socket,
    next_id: u64,
    bin: PathBuf,
}

impl Rig {
    async fn start() -> Option<Rig> {
        let bin = vorn_bin()?;
        let v = Vornd::start();
        let reap = Reap(v.holder());
        let ws = common::connect(v.port, CREDENTIAL).await;
        Some(Rig {
            v,
            _reap: reap,
            ws,
            next_id: 0,
            bin,
        })
    }

    /// Calls `method` until it answers without an error: vornd reads its sessions once it is up.
    async fn call(&mut self, method: &str, params: Value) -> Value {
        let t = Instant::now();
        loop {
            self.next_id += 1;
            let id = self.next_id;
            common::send(
                &mut self.ws,
                json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
            )
            .await;
            let answer = loop {
                let frame = common::next(&mut self.ws).await;
                if frame["id"] == json!(id) {
                    break frame;
                }
            };
            if answer.get("error").is_none() {
                return answer["result"].clone();
            }
            assert!(
                t.elapsed() < PATIENCE,
                "{method}: {answer}\n{}",
                self.v.log()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// A shell in `cwd`, named `name`, typing `command` when it is up.
    async fn shell(&mut self, cwd: &Path, name: &str, command: Option<&str>) -> String {
        let mut ask = json!({ "cwd": cwd, "displayName": name });
        if let Some(c) = command {
            ask["command"] = json!(c);
        }
        let made = self.call("shell:create", ask).await;
        made["id"].as_str().expect("an id").to_owned()
    }

    fn command(&self, args: &[&str]) -> tokio::process::Command {
        let mut c = tokio::process::Command::new(&self.bin);
        c.args(args)
            .arg("--data-dir")
            .arg(self.v.home.path())
            .env("HOME", self.v.home.path())
            .env("VORN_VORND_PATH", self.v.home.path().join("no-vornd"))
            .env_remove("NO_COLOR")
            .stdin(Stdio::null());
        c
    }

    /// Runs `vorn args`: its exit code, stdout and stderr.
    async fn vorn(&self, args: &[&str]) -> (i32, String, String) {
        let out = tokio::time::timeout(Duration::from_secs(30), self.command(args).output())
            .await
            .expect("vorn finishes")
            .unwrap();
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// `vorn session read <id> <extra>` until its output holds `want`.
    async fn printed(&self, id: &str, extra: &[&str], want: &str) -> String {
        let t = Instant::now();
        loop {
            let mut args = vec!["session", "read", id];
            args.extend_from_slice(extra);
            let (code, out, err) = self.vorn(&args).await;
            if code == 0 && out.contains(want) {
                return out;
            }
            assert!(
                t.elapsed() < PATIENCE,
                "no {want:?} from {id}: {code} {out:?} {err:?}\n{}",
                self.v.log()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

macro_rules! rig {
    () => {
        match Rig::start().await {
            Some(rig) => rig,
            None => return,
        }
    };
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ambiguous_target_exits_3_and_lists_the_candidates() {
    let mut r = rig!();
    let cwd = tempfile::tempdir().unwrap();
    let first = r.shell(cwd.path(), "build", None).await;
    let second = r.shell(cwd.path(), "build", None).await;
    r.shell(cwd.path(), "other", None).await;

    let (code, out, err) = r.vorn(&["session", "read", "build"]).await;
    assert_eq!((code, out.as_str()), (3, ""), "{err}");
    assert!(err.contains("\"build\" matches 2 sessions"), "{err}");
    assert!(
        err.contains(&first[..8]) && err.contains(&second[..8]),
        "{err}"
    );

    let (code, out, _) = r.vorn(&["session", "read", "build", "--json"]).await;
    assert_eq!(code, 3);
    let data: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(data["error"], "ambiguous");
    let found: Vec<(u64, &str)> = data["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["position"].as_u64().unwrap(), c["id"].as_str().unwrap()))
        .collect();
    assert_eq!(found.len(), 2);
    assert!(found.iter().any(|(_, id)| *id == first));
    assert!(found.iter().any(|(_, id)| *id == second));

    // A position, an id's end and the whole id each name one.
    let tail = &first[first.len() - 12..];
    for target in ["3", "other", tail, first.as_str()] {
        let (code, _, err) = r.vorn(&["session", "read", target]).await;
        assert_eq!(code, 0, "{target}: {err}");
    }
    let (code, _, err) = r.vorn(&["session", "read", "nothing-like-it"]).await;
    assert_eq!(code, 1, "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wait_returns_the_childs_exit_code_and_124_on_timeout() {
    let mut r = rig!();
    let cwd = tempfile::tempdir().unwrap();
    let leaves = r.shell(cwd.path(), "leaves", Some("sleep 1; exit 7")).await;
    let (code, out, err) = r.vorn(&["session", "wait", &leaves, "--json"]).await;
    assert_eq!(code, 7, "{err}\n{}", r.v.log());
    let record: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(record["session"], json!(leaves));
    assert_eq!(record["exitCode"], 7);

    // A shell that ended before `wait` began is still listed, and its code is read from it.
    let gone = r.shell(cwd.path(), "gone", Some("exit 9")).await;
    let t = Instant::now();
    loop {
        let listed = r.call("terminal:listActive", Value::Null).await;
        let ended = listed
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == json!(gone) && s["ended"] == json!(true));
        if ended {
            break;
        }
        assert!(t.elapsed() < PATIENCE, "never ended: {listed}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let (code, _, err) = r.vorn(&["session", "wait", &gone, "--timeout", "5s"]).await;
    assert_eq!(code, 9, "{err}");

    let stays = r.shell(cwd.path(), "stays", None).await;
    let t = Instant::now();
    let (code, out, err) = r
        .vorn(&["session", "wait", &stays, "--timeout", "1s", "--json"])
        .await;
    assert_eq!(code, 124, "{err}");
    assert!(t.elapsed() < Duration::from_secs(10));
    let record: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(record["timedOut"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn read_trims_unwraps_and_draws_colours() {
    let mut r = rig!();
    let cwd = tempfile::tempdir().unwrap();
    let id = r
        .shell(
            cwd.path(),
            "colours",
            Some(r"printf '\033[31mred\033[0m <b>\n'; printf 'z%.0s' $(seq 1 200); echo; echo done-marker"),
        )
        .await;
    r.printed(&id, &["--scrollback"], "done-marker").await;

    // Joined: a 200-character line is one line though the terminal is 80 wide.
    let out = r.printed(&id, &["--unwrap", "--trim"], "done-marker").await;
    let long = out
        .lines()
        .find(|l| l.starts_with('z'))
        .expect("the long line");
    assert_eq!(long.len(), 200);
    assert!(out.lines().all(|l| l == l.trim_end()), "{out:?}");
    assert!(!out.ends_with("\n\n"), "trailing blank lines: {out:?}");

    let vt = r
        .printed(&id, &["--format", "vt", "--trim"], "done-marker")
        .await;
    assert!(vt.contains("\x1b[0;38;5;1mred\x1b[0m"), "{vt:?}");

    let html = r
        .printed(&id, &["--format", "html", "--trim"], "done-marker")
        .await;
    assert!(html.starts_with("<pre>"), "{html}");
    assert!(
        html.contains("<span style=\"color:#cd0000;\">red</span> &lt;b&gt;"),
        "{html}"
    );

    let (code, out, _) = r
        .vorn(&[
            "session", "read", &id, "--format", "vt", "--lines", "2", "--trim",
        ])
        .await;
    assert_eq!(code, 0);
    assert_eq!(out.lines().count(), 2);
    assert_eq!(out.lines().next(), Some("done-marker"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_event_stream_hears_a_shell_start_and_exit() {
    let mut r = rig!();
    // Ready once the server accepted the subscription.
    let mut stream = r
        .command(&[
            "events",
            "--json",
            "--filter",
            "session.created,session.exited",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(stream.stdout.take().unwrap()).lines();
    // Events are only heard once the queue exists; creating before it would race.
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let cwd = tempfile::tempdir().unwrap();
    let id = r
        .shell(cwd.path(), "streamed", Some("sleep 1; exit 5"))
        .await;
    let mut heard: Vec<Value> = Vec::new();
    let t = Instant::now();
    while heard.len() < 2 {
        let line = tokio::time::timeout(PATIENCE, lines.next_line())
            .await
            .unwrap_or_else(|_| panic!("heard {heard:?}\n{}", r.v.log()))
            .unwrap()
            .expect("the stream stays open");
        let event: Value = serde_json::from_str(&line).expect("one JSON object a line");
        if event["session"] == json!(id) {
            heard.push(event);
        }
        assert!(t.elapsed() < PATIENCE * 2);
    }
    assert_eq!(heard[0]["event"], "session.created");
    assert_eq!(heard[0]["v"], 1);
    assert!(heard[0]["at"].as_u64().unwrap() > 0);
    assert_eq!(heard[1]["event"], "session.exited");
    assert_eq!(heard[1]["data"]["exitCode"], 5);
    assert!(r
        .vorn(&["events", "--filter", "session.nope"])
        .await
        .2
        .contains("unknown event"));
}

fn write_layout(r: &Rig, name: &str, text: &str) {
    let dir = r.v.home.path().join("layouts");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{name}.toml")), text).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_layout_opens_saves_and_reopens_its_sessions() {
    let mut r = rig!();
    let project = tempfile::tempdir().unwrap();
    let project_path = project.path().canonicalize().unwrap();
    std::fs::create_dir(project_path.join("sub")).unwrap();
    let p = project_path.to_str().unwrap();
    write_layout(
        &r,
        "dev",
        &format!(
            "name = \"dev\"\nproject = \"{p}\"\nsplit = \"horizontal\"\n\n[[children]]\ncommand = [\"echo\", \"layout ran\"]\ncwd = \"sub\"\nlabel = \"logs\"\n\n[[children]]\nshell = true\nlabel = \"plain\"\n"
        ),
    );

    let (code, out, err) = r.vorn(&["layout", "open", "dev", "--json"]).await;
    assert_eq!(code, 0, "{err}\n{}", r.v.log());
    let done: Value = serde_json::from_str(&out).unwrap();
    let ids: Vec<String> = done["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids.len(), 2);
    assert_eq!(done["sessions"][0]["label"], "logs");

    // The labels and directories are the sessions' own, and the command ran in the first.
    let listed = r.call("terminal:listActive", Value::Null).await;
    let by_id = |id: &str| {
        listed
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == json!(id))
            .cloned()
            .expect("listed")
    };
    assert_eq!(by_id(&ids[0])["displayName"], "logs");
    assert_eq!(by_id(&ids[0])["shellCwd"], json!(project_path.join("sub")));
    assert_eq!(by_id(&ids[1])["displayName"], "plain");
    r.printed(&ids[0], &["--scrollback"], "layout ran").await;

    // The plain output of `open` is the ids, in order.
    let (code, out, _) = r.vorn(&["layout", "open", "dev"]).await;
    assert_eq!(code, 0);
    assert_eq!(out.lines().count(), 2);

    // Save what is open, then open the copy.
    let (code, out, err) = r.vorn(&["layout", "save", "snap", "--project", p]).await;
    assert_eq!(code, 0, "{err}");
    assert!(out.trim_end().ends_with("snap.toml"), "{out}");
    let (code, _, err) = r.vorn(&["layout", "save", "snap", "--project", p]).await;
    assert_eq!(code, 1, "a second save needs --force: {err}");
    let (code, out, err) = r.vorn(&["layout", "open", "snap", "--json"]).await;
    assert_eq!(code, 0, "{err}");
    let again: Value = serde_json::from_str(&out).unwrap();
    // `dev` was opened twice, so four sessions were saved and four reopen.
    assert_eq!(again["sessions"].as_array().unwrap().len(), 4);
    let labels: Vec<&str> = again["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, ["logs", "plain", "logs", "plain"]);

    // A name no layout has is a failure, not an ambiguity.
    let (code, _, err) = r.vorn(&["layout", "open", "nope"]).await;
    assert_eq!(code, 1, "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn check_names_the_line_and_an_extensions_layout_is_listed() {
    let mut r = rig!();
    let bad = r.v.home.path().join("bad.toml");
    std::fs::write(
        &bad,
        "name = \"bad\"\nsplit = \"horizontal\"\n\n[[children]]\nagent = \"nobody\"\n",
    )
    .unwrap();
    let bad = bad.to_str().unwrap();
    let (code, out, err) = r.vorn(&["layout", "check", bad]).await;
    assert_eq!((code, out.as_str()), (1, ""));
    assert!(
        err.starts_with(&format!("{bad}:5: unknown agent \"nobody\"")),
        "{err}"
    );

    // An extension offers its own, beside the person's.
    let ext = tempfile::tempdir().unwrap();
    std::fs::write(
        ext.path().join("vorn-extension.toml"),
        "id = \"acme.layouts\"\nname = \"Acme layouts\"\nversion = \"1.0.0\"\napi = \"vorn:extension@0.9\"\n\n[[contributes.layouts]]\nid = \"acme-dev\"\ntitle = \"Acme dev\"\nfile = \"layouts/dev.toml\"\n",
    )
    .unwrap();
    std::fs::create_dir(ext.path().join("layouts")).unwrap();
    std::fs::write(ext.path().join("layouts/dev.toml"), "shell = true\n").unwrap();
    r.call("extension:link", json!({ "path": ext.path() }))
        .await;
    write_layout(&r, "mine", "shell = true\n");

    let (code, out, err) = r.vorn(&["layout", "list", "--json"]).await;
    assert_eq!(code, 0, "{err}");
    let rows: Value = serde_json::from_str(&out).unwrap();
    let find = |name: &str| {
        rows.as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == name)
            .cloned()
    };
    assert_eq!(find("mine").unwrap()["source"], "user");
    let theirs = find("acme-dev").expect("the extension's layout is listed");
    assert_eq!(theirs["source"], "acme.layouts");
    assert_eq!(theirs["sessions"], 1);
}
