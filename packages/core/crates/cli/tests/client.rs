//! The binary's client commands against a fake server: what reaches stdout
//! and stderr, and the exit code, for answers, errors, closes and silence.

mod common;

use common::{announce, ws_server, Answer};
use serde_json::{json, Value};

async fn vorn(port: u16, args: &[&str]) -> (i32, String, String) {
    let dir = tempfile::tempdir().unwrap();
    announce(dir.path(), port);
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_vorn"))
        .args(args)
        .arg("--data-dir")
        .arg(dir.path())
        .env("HOME", dir.path())
        .env("VORN_VORND_PATH", dir.path().join("no-vornd"))
        .env_remove("NO_COLOR")
        .output()
        .await
        .unwrap();
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn workflows() -> Value {
    json!([
        {"id": "system:default-task-workflow", "name": "Default Task Workflow", "enabled": true,
         "nodes": [{"type": "trigger", "config": {"triggerType": "manual"}}]},
        {"id": "0f1e2d3c-1111-2222-3333-444455556666", "name": "Nightly", "enabled": false,
         "lastRunAt": "2020-01-01T00:00:00.000Z", "lastRunStatus": "error",
         "nodes": [{"type": "trigger", "config": {"triggerType": "cron"}}]}
    ])
}

#[tokio::test]
async fn lists_workflows_as_a_table_and_as_json() {
    let port = ws_server(|method, _| match method {
        "workflow:list" => Answer::Result(workflows()),
        other => Answer::Error(format!("Method not found: {other}")),
    })
    .await;

    let (code, out, err) = vorn(port, &["workflow", "list"]).await;
    assert_eq!((code, err.as_str()), (0, ""));
    let mut lines = out.lines();
    assert_eq!(
        lines.next(),
        Some(
            "ID                            NAME                   TRIGGER  ENABLED  LAST RUN  WHEN"
        )
    );
    assert_eq!(
        lines.next(),
        Some("system:default-task-workflow  Default Task Workflow  manual   yes      -         -")
    );
    let nightly = lines.next().unwrap();
    assert!(nightly.starts_with(
        "0f1e2d3c                      Nightly                cron     no       error     "
    ));
    assert!(nightly.ends_with("d ago"));

    let (code, out, _) = vorn(port, &["--json", "workflow", "list"]).await;
    assert_eq!(code, 0);
    assert!(out.starts_with("[\n  {\n    \"id\": \"system:default-task-workflow\",\n"));
    assert_eq!(serde_json::from_str::<Value>(&out).unwrap(), workflows());
}

#[tokio::test]
async fn explains_a_method_the_server_does_not_have() {
    let port = ws_server(|method, _| Answer::Error(format!("Method not found: {method}"))).await;
    let (code, out, err) = vorn(port, &["workflow", "runs"]).await;
    assert_eq!((code, out.as_str()), (1, ""));
    assert_eq!(
        err,
        "vorn: could not list runs: This server does not have workflow:list, so it is older than the vorn command asking for it.\nRestart Vorn to pick up the newer server, or run this against the matching build.\n"
    );
}

#[tokio::test]
async fn names_a_refused_credential_and_any_other_close() {
    let port = ws_server(|_, _| Answer::Close(4002)).await;
    let (code, _, err) = vorn(port, &["session", "list"]).await;
    assert_eq!(code, 1);
    assert!(
        err.starts_with(
            "vorn: could not list sessions: A Vorn server on this port refused the credential in "
        ),
        "{err}"
    );

    let port = ws_server(|_, _| Answer::Close(4000)).await;
    let (_, _, err) = vorn(port, &["workflow", "list"]).await;
    assert_eq!(
        err,
        "vorn: could not list workflows: The server closed the connection before answering (code 4000).\n"
    );
}

#[tokio::test]
async fn gives_up_after_the_timeout() {
    let port = ws_server(|_, _| Answer::Silence).await;
    let (code, _, err) = vorn(port, &["workflow", "list", "--timeout", "200"]).await;
    assert_eq!(code, 1);
    assert_eq!(
        err,
        "vorn: could not list workflows: RPC call \"workflow:list\" timed out after 200ms\n"
    );
}

#[tokio::test]
async fn says_when_nothing_listens_on_the_announced_port() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let (code, _, err) = vorn(port, &["workflow", "list"]).await;
    assert_eq!(code, 1);
    assert_eq!(
        err,
        format!("vorn: could not list workflows: Cannot connect to Vorn server: connect ECONNREFUSED 127.0.0.1:{port}. Is the app running?\n")
    );
}

#[tokio::test]
async fn sends_input_with_enter_and_kills_by_prefix() {
    let port = ws_server(|method, params| match method {
        "terminal:listActive" => Answer::Result(json!([
            {"id": "c3f1a2e8-1111-2222-3333-444455556666", "agentType": "claude",
             "projectName": "vorn", "status": "running"}
        ])),
        "headless:list" => Answer::Result(json!([
            {"id": "9b4d0117-1111-2222-3333-444455556666", "agentType": "codex",
             "projectName": "web", "status": "running"}
        ])),
        "headless:kill" => {
            assert_eq!(params, &json!("9b4d0117-1111-2222-3333-444455556666"));
            Answer::Result(Value::Null)
        }
        other => Answer::Error(format!("Method not found: {other}")),
    })
    .await;

    let (code, out, err) = vorn(port, &["session", "list"]).await;
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(
        out,
        "ID        AGENT   PROJECT  BRANCH  STATUS\nc3f1a2e8  claude  vorn     -       running\n9b4d0117  codex   web      -       running\n"
    );

    let (code, out, err) = vorn(port, &["session", "kill", "9b4d"]).await;
    assert_eq!(
        (code, out.as_str(), err.as_str()),
        (0, "", "Killed 9b4d0117.\n")
    );

    let (code, _, err) = vorn(port, &["session", "logs", "9b4d0117"]).await;
    assert_eq!(code, 1);
    assert!(err.contains("is a headless session, which has no terminal to read"));

    let (code, _, err) = vorn(port, &["session", "send", "c3f1", "hello"]).await;
    assert_eq!((code, err.as_str()), (0, "Sent to c3f1a2e8.\n"));
}

/// A stand-in for vornd: prints its arguments, then announces the server on
/// `port` in the data directory it was given, as vornd would, and exits with
/// `exit_code`.
#[cfg(unix)]
fn fake_vornd(dir: &std::path::Path, port: u16, exit_code: i32) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join("vornd");
    std::fs::write(
        &script,
        format!(
            r#"#!/bin/sh
printf '%s\n' "$*"
dir=""
prev=""
for a in "$@"; do
  if [ "$prev" = "--data-dir" ]; then dir="$a"; fi
  prev="$a"
done
if [ -n "$dir" ]; then
  printf '%s' "$*" > "$dir/argv.txt"
  printf '{{"port":{port}}}' > "$dir/ws-port"
  printf '{credential}' > "$dir/local-token"
fi
exit {exit_code}
"#,
            credential = common::CREDENTIAL
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

#[cfg(unix)]
#[tokio::test]
async fn starts_a_server_when_none_is_running() {
    let port = ws_server(|_, _| Answer::Result(json!([]))).await;
    let scripts = tempfile::tempdir().unwrap();
    let vornd = fake_vornd(scripts.path(), port, 0);
    let (data, home) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());

    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_vorn"))
        .args(["workflow", "list", "--data-dir"])
        .arg(data.path())
        .env("VORN_VORND_PATH", &vornd)
        .env("HOME", home.path())
        .output()
        .await
        .unwrap();
    let err = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{err}");
    assert_eq!(err, "No server running, starting one.\nNo workflows.\n");
    let argv = std::fs::read_to_string(data.path().join("argv.txt")).unwrap();
    assert_eq!(argv, format!("--data-dir {}", data.path().display()));
    // What the server printed went to its log, not to this command's output.
    assert!(output.stdout.is_empty());
    let log = std::fs::read_to_string(data.path().join("server.log")).unwrap();
    assert!(log.contains("--data-dir"), "{log}");
}

#[tokio::test]
async fn says_why_it_cannot_start_a_server() {
    let (data, home) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_vorn"))
        .args(["session", "list", "--data-dir"])
        .arg(data.path())
        .env("VORN_VORND_PATH", data.path().join("missing"))
        .env("HOME", home.path())
        .output()
        .await
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(
        err.starts_with(
            "No server running, starting one.\nvorn: This vorn cannot start a Vorn server"
        ),
        "{err}"
    );
    assert!(err.contains("missing"), "{err}");
}

#[cfg(unix)]
#[tokio::test]
async fn serve_runs_the_server_and_passes_its_exit_code_through() {
    let scripts = tempfile::tempdir().unwrap();
    let vornd = fake_vornd(scripts.path(), 1, 3);
    let (data, home) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_vorn"))
        .args([
            "server",
            "serve",
            "--port",
            "4100",
            "--host",
            "127.0.0.1",
            "--data-dir",
        ])
        .arg(data.path())
        .env("VORN_VORND_PATH", &vornd)
        .env("HOME", home.path())
        .output()
        .await
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    let out = String::from_utf8_lossy(&output.stdout);
    let dir = data.path().display();
    assert!(
        out.contains(&format!("Starting the Vorn server for {dir}\n")),
        "{out}"
    );
    assert!(
        out.ends_with(&format!("--data-dir {dir} --port 4100 --host 127.0.0.1\n")),
        "{out}"
    );
    // Not a terminal, so no token is shown: one is to be minted on purpose.
    assert!(out.contains("No device tokens exist."), "{out}");
}

#[tokio::test]
async fn a_debug_build_will_not_serve_the_default_data_directory() {
    let home = tempfile::tempdir().unwrap();
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_vorn"))
        .args(["server", "serve"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("VORN_VORND_PATH", home.path().join("never-run"))
        .env_remove("VORN_ALLOW_DEFAULT_DATA_DIR")
        .output()
        .await
        .unwrap();
    let err = String::from_utf8_lossy(&output.stderr);
    if cfg!(debug_assertions) {
        assert_eq!(output.status.code(), Some(1), "{err}");
        assert!(err.contains("default data directory"), "{err}");
        assert!(!home.path().join(".vorn").exists());
    }
}
