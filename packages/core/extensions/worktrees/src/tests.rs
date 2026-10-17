use vorn_extension_sdk::testing::FakeHost;
use vorn_extension_sdk::{dispatch, Ctx, Event, Extension, Host, Init};

use super::*;

/// Three worktrees of `/p`: the main checkout, `fix` with a session and
/// changes, and `old`, merged and idle with build output.
fn listed() -> Value {
    json!([
        { "projectPath": "/p", "projectName": "p", "path": "/p", "name": "p",
          "branch": "main", "main": true, "orphan": false, "dirty": false,
          "sizeBytes": 0, "artifactBytes": 0, "sessionIds": [], "verdict": "keep", "reasons": "main" },
        { "projectPath": "/p", "projectName": "p", "path": "/p/.vorn-worktrees/fix", "name": "fix",
          "branch": "fix", "main": false, "orphan": false, "dirty": true, "base": "main", "idleDays": 0,
          "sizeBytes": 2048, "artifactBytes": 0, "sessionIds": ["t1"], "verdict": "review", "reasons": "dirty" },
        { "projectPath": "/p", "projectName": "p", "path": "/p/.vorn-worktrees/old", "name": "old",
          "branch": "feat/old", "main": false, "orphan": false, "dirty": false, "base": "main", "idleDays": 20,
          "sizeBytes": 4096, "artifactBytes": 1024, "sessionIds": [], "verdict": "remove",
          "reasons": "merged; idle 20 days" }
    ])
}

fn host() -> FakeHost {
    FakeHost::default()
        .answer("worktrees.list", |_| Ok(listed()))
        .answer("worktrees.remove", |_| Ok(json!({ "removed": true })))
        .answer("worktrees.reclaim", |_| Ok(Value::Null))
        .answer("worktrees.prune", |_| Ok(Value::Null))
        .answer("sessions.close", |_| Ok(Value::Null))
}

fn started(host: &mut FakeHost) -> Cleanup {
    Cleanup::init(&Init::default(), host).unwrap()
}

fn call(c: &mut Cleanup, host: &mut FakeHost, action: &str, args: Value) -> Result<Value, Error> {
    dispatch(c, host, action, args, &Ctx::default())
}

fn idle(path: &str) -> Event {
    Event {
        name: WORKTREE_IDLE.into(),
        data: json!({ "sessionId": "t1", "projectPath": "/p", "worktreePath": path }),
    }
}

#[test]
fn offers_an_idle_worktree_only_once_no_session_holds_it_and_forgets_it_when_kept() {
    let mut h = host();
    let mut c = started(&mut h);
    c.on_events(&mut h, vec![idle("/p/.vorn-worktrees/fix"), idle("/p")]);
    assert!(h.rows(IDLE).unwrap().is_empty(), "held, or the main checkout");
    c.on_events(&mut h, vec![idle("/p/.vorn-worktrees/old")]);
    let rows = h.rows(IDLE).unwrap();
    assert_eq!(
        rows,
        [json!({ "id": "/p/.vorn-worktrees/old", "name": "old", "project": "p",
                 "project_path": "/p", "dirty": false })]
    );
    call(&mut c, &mut h, "keep", json!({ "id": "/p/.vorn-worktrees/old" })).unwrap();
    assert!(h.rows(IDLE).unwrap().is_empty());
}

#[test]
fn asks_before_removing_a_worktree_with_a_session_and_removes_it_once_confirmed() {
    let mut h = host();
    let mut c = started(&mut h);
    let ask = json!({ "worktree": "/p/.vorn-worktrees/fix", "project": "/p" });
    assert_eq!(
        call(&mut c, &mut h, "ask", ask).unwrap(),
        json!({ "open": "confirm" })
    );
    let asked = &h.rows(ASKED).unwrap()[0];
    assert_eq!((&asked["sessions"], &asked["dirty"]), (&json!(1), &json!(true)));
    assert!(h.asked("worktrees.remove").is_empty());

    // The dialog's worktree lives in the row, so a fresh instance confirms it too.
    let mut fresh = started(&mut h);
    let done = call(&mut fresh, &mut h, "confirm", Value::Null).unwrap();
    assert_eq!(done["toast"], "Worktree removed");
    assert_eq!(h.asked("sessions.close"), [json!({ "sessionId": "t1" })]);
    assert_eq!(
        h.asked("worktrees.remove"),
        [json!({ "worktree": "/p/.vorn-worktrees/fix", "project": "/p", "force": true })]
    );
    assert!(h.rows(ASKED).unwrap().is_empty());
    let err = call(&mut fresh, &mut h, "confirm", Value::Null).unwrap_err();
    assert_eq!((err.code.as_str(), err.message.as_str()), ("gone", "Nothing is asked"));
}

#[test]
fn removes_a_clean_unheld_worktree_without_asking_and_never_the_main_checkout() {
    let mut h = host();
    let mut c = started(&mut h);
    let ask = json!({ "worktree": "/p/.vorn-worktrees/old", "project": "/p" });
    let done = call(&mut c, &mut h, "ask", ask).unwrap();
    assert_eq!(done["toast"], "Worktree removed");
    assert!(h.asked("sessions.close").is_empty());
    let main = call(&mut c, &mut h, "ask", json!({ "worktree": "/p", "project": "/p" }));
    assert!(main.unwrap_err().message.contains("not a worktree Vorn removes"));
    call(&mut c, &mut h, "cancel", Value::Null).unwrap();
    let missing = call(&mut c, &mut h, "ask", json!({ "worktree": "/p/x" })).unwrap_err();
    assert_eq!(missing.message, "ask takes project");
}

#[test]
fn lists_every_worktree_but_the_main_checkout_largest_first_with_totals() {
    let mut h = host();
    let mut c = started(&mut h);
    assert_eq!(call(&mut c, &mut h, "scan", Value::Null).unwrap(), json!({ "worktrees": 3 }));
    let rows = h.rows(ALL).unwrap();
    let names: Vec<&str> = rows.iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["old", "fix"]);
    assert_eq!(rows[0]["detail"], "idle 20 days · merged");
    assert_eq!(rows[0]["verdict_label"], "Removable");
    assert_eq!(rows[1]["held"], true);
    assert_eq!(rows[1]["remove_dirty"], true);
    let summary = &h.rows(SUMMARY).unwrap()[0];
    assert_eq!(summary["total"], "6 KB");
    assert_eq!(summary["build"], "1 KB");
    assert_eq!(summary["removable"], "4 KB");
    assert_eq!(summary["reclaim_confirm"], "Delete the build output in 1 worktree? This frees 1 KB.");
    assert_eq!(h.rows(SCAN).unwrap()[0]["busy"], false);
    call(&mut c, &mut h, "refresh", Value::Null).unwrap();
    assert_eq!(h.asked("worktrees.list")[1]["refresh"], true);
}

#[test]
fn a_first_scan_that_keeps_failing_says_why_on_the_page() {
    let mut h = FakeHost::default().answer("worktrees.list", |_| {
        Err(Error::new("failed", "vornd has not read its sessions."))
    });
    let mut c = started(&mut h);
    let err = call(&mut c, &mut h, "scan", Value::Null).unwrap_err();
    assert_eq!(err.code, "failed");
    assert_eq!(h.asked("worktrees.list").len(), usize::try_from(FIRST_TRIES).unwrap());
    let state = &h.rows(SCAN).unwrap()[0];
    assert_eq!(state["busy"], false);
    assert_eq!(
        state["error"],
        "Could not measure the worktrees: vornd has not read its sessions. Refresh to try again."
    );
}

#[test]
fn reclaims_and_removes_what_the_page_checks() {
    let mut h = host();
    let mut c = started(&mut h);
    call(&mut c, &mut h, "scan", Value::Null).unwrap();
    let done = call(&mut c, &mut h, "reclaim_all", Value::Null).unwrap();
    // fix has no build output and a session, so only old's goes.
    assert_eq!(done["reclaimed"], 1);
    assert_eq!(h.asked("worktrees.reclaim"), [json!({ "worktree": "/p/.vorn-worktrees/old" })]);
    let done = call(
        &mut c,
        &mut h,
        "remove_many",
        json!({ "ids": ["/p/.vorn-worktrees/old", "/p/nowhere"] }),
    )
    .unwrap();
    assert_eq!(done["toast"], "Removed 1 worktree");
    let removed = h.asked("worktrees.remove");
    assert_eq!(removed[0]["worktree"], "/p/.vorn-worktrees/old");
    assert_eq!(removed[0]["force"], false);
    assert!(call(&mut c, &mut h, "remove_many", json!({ "ids": [] })).is_err());
    let gone = call(&mut c, &mut h, "reclaim", json!({ "id": "/p/elsewhere" })).unwrap_err();
    assert_eq!(gone.message, "/p/elsewhere is not listed");
    let unknown = call(&mut c, &mut h, "fly", Value::Null).unwrap_err();
    assert_eq!(unknown.message, "Worktree cleanup has no action fly");
}

#[test]
fn the_page_is_a_tree_whose_bulk_remove_takes_the_checked() {
    let mut h = host();
    let mut c = started(&mut h);
    call(&mut c, &mut h, "scan", Value::Null).unwrap();
    let render = json!({ "point": "worktrees", "surface": { "cells": false } });
    let answer = call(&mut c, &mut h, tree::RENDER, render).unwrap();
    let page = tree::read(&answer).expect("a tree the client draws");
    let text = vorn_extension_sdk::serde_json::to_string(&page.tree).unwrap();
    for said in ["Build output, rebuilt by a reinstall", "vs main", "feat/old", "Removable"] {
        assert!(text.contains(said), "{said} in {text}");
    }
    let click = json!({ "point": "worktrees", "event": {
        "id": "remove_many", "kind": "click", "selected": ["/p/.vorn-worktrees/old"] } });
    let done = call(&mut c, &mut h, tree::EVENT, click).unwrap();
    assert_eq!(done["toast"], "Removed 1 worktree");
    let stray = json!({ "point": "worktrees", "event": { "id": "confirm", "kind": "click" } });
    assert!(
        call(&mut c, &mut h, tree::EVENT, stray).is_err(),
        "a click runs only the page's own buttons"
    );
}

#[test]
fn sizes_read_in_the_largest_whole_unit() {
    assert_eq!(bytes(512), "512 B");
    assert_eq!(bytes(12 * 1024), "12 KB");
    assert_eq!(bytes(340 * 1024 * 1024), "340 MB");
    assert_eq!(bytes(9 * 1024 * 1024), "9.0 MB");
    assert_eq!(bytes(1288 * 1024 * 1024), "1.3 GB");
}
