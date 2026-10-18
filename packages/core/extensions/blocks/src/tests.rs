use vorn_extension_sdk::testing::FakeHost;
use vorn_extension_sdk::tree::{self, Kind};
use vorn_extension_sdk::{dispatch, json, Ctx, Event, Extension, Init, Value};

use super::*;

fn mark(session: &str, phase: &str, extra: Value) -> Event {
    let mut data = json!({ "sessionId": session, "phase": phase, "at": 1000 });
    if let (Some(d), Value::Object(more)) = (data.as_object_mut(), extra) {
        d.extend(more);
    }
    Event {
        name: SESSION_MARK.into(),
        data,
    }
}

fn log_of(b: &mut Blocks, host: &mut FakeHost, session: &str) -> Log {
    let args = json!({ "point": COMMANDS, "surface": { "scope": session } });
    let drawn = dispatch(b, host, tree::RENDER, args, &Ctx::default()).unwrap();
    match tree::read(&drawn).unwrap().tree.kind {
        Kind::Log(l) => *l,
        other => panic!("a log, not {other:?}"),
    }
}

#[test]
fn a_shells_marks_become_its_commands_and_outlive_the_instance() {
    let mut host = FakeHost::default();
    let mut b = Blocks::init(&Init::default(), &mut host).unwrap();
    b.on_events(
        &mut host,
        vec![
            mark("s1", "prompt", json!({ "cwd": "/a/b" })),
            mark("s1", "start", json!({ "command": "ls" })),
            mark("s2", "prompt", json!({})),
        ],
    );
    let running = log_of(&mut b, &mut host, "s1");
    assert_eq!(running.running().and_then(|c| c.command.as_deref()), Some("ls"));
    // One row per shell touched, so a client asks again for those alone.
    let rows = host.rows(SHELLS).unwrap();
    assert_eq!(rows.iter().map(|r| r["id"].clone()).collect::<Vec<_>>(), [json!("s1"), json!("s2")]);
    b.on_events(
        &mut host,
        vec![mark("s1", "end", json!({ "exitCode": 2, "durationMs": 40 }))],
    );
    let mut again = Blocks::init(&Init::default(), &mut host).unwrap();
    let done = log_of(&mut again, &mut host, "s1");
    assert_eq!(done.commands.len(), 1);
    let c = &done.commands[0];
    assert_eq!((c.exit, c.duration_ms, c.cwd.as_deref()), (Some(2), Some(40), Some("/a/b")));
    assert!(done.running().is_none());
    // An ended shell's commands go.
    again.on_events(
        &mut host,
        vec![Event {
            name: SESSION_EXITED.into(),
            data: json!({ "sessionId": "s1" }),
        }],
    );
    assert!(log_of(&mut again, &mut host, "s1").commands.is_empty());
    assert_eq!(host.rows(SHELLS).unwrap(), [json!({ "id": "s2", "n": 1 })]);
}

#[test]
fn the_composer_asks_for_history_completions_and_keeps_what_ran() {
    let mut host = FakeHost::default()
        .answer("git.branches", |a| {
            assert_eq!(a["worktree"], "/p");
            Ok(json!({ "local": ["main", "feature"] }))
        })
        .answer("shell.executables", |_| Ok(json!([])));
    let mut b = Blocks::init(&Init::default(), &mut host).unwrap();
    let ctx = Ctx::default();
    let call = |b: &mut Blocks, host: &mut FakeHost, action: &str, args: Value| {
        dispatch(b, host, action, args, &ctx).unwrap()
    };
    call(&mut b, &mut host, "record", json!({ "text": "git status", "project": "/p" }));
    call(&mut b, &mut host, "record", json!({ "text": "echo hi" }));
    let h = call(&mut b, &mut host, "history", json!({ "text": "", "project": "/p" }));
    assert_eq!(h["rows"][0]["text"], "git status");
    let h = call(&mut b, &mut host, "history", json!({ "text": "git s", "project": "/p" }));
    assert_eq!(h["ghost"], "git status");
    let c = call(
        &mut b,
        &mut host,
        "complete",
        json!({ "text": "git checkout ma", "cwd": "/p", "repo": "/p" }),
    );
    assert_eq!(c["completions"][0]["label"], "main");
    assert_eq!(c["completions"][0]["icon"], "git-branch");
    assert_eq!(c["completions"][0]["result"], "git checkout main ");
    assert_eq!(c["tab"], "git checkout main ");
    let c = call(&mut b, &mut host, "complete", json!({ "text": "git st" }));
    assert_eq!(c["tab"], "git sta");
    // The desktop's old history comes in once, behind what is kept.
    let n = call(
        &mut b,
        &mut host,
        "import",
        json!({ "entries": [{"text": "make", "kind": "shell", "timestamp": 1}] }),
    );
    assert_eq!(n["imported"], 1);
    let mut again = Blocks::init(&Init::default(), &mut host).unwrap();
    let h = dispatch(&mut again, &mut host, "history", json!({ "text": "ma" }), &ctx).unwrap();
    assert_eq!(h["rows"][0]["text"], "make");
}
