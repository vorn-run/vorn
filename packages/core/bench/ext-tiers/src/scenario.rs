//! Each extension's scripted session: the same events, calls, trees and
//! clicks for either tier, written down as a transcript of every answer,
//! the rows it left and what it asked of Vorn. Two tiers behave the same
//! when their transcripts are equal; equal trees draw equal frames, since
//! one renderer draws both.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::rig::Rig;
use crate::world::{project, FEED_HOST};

/// Steps written down as they ran.
#[derive(Default)]
pub struct Transcript(Vec<Value>);

impl Transcript {
    /// Calls to Vorn are kept in order, and publishes in order for each
    /// collection: a program's messages are each answered on a task of
    /// their own, so a publish may land after a call the program made later.
    async fn step(&mut self, rig: &Rig, what: &str, answer: Value) {
        rig.quiet().await;
        let mut asked = Vec::new();
        let mut published: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for (method, args) in rig.world.take_log() {
            match method.strip_prefix("publish ") {
                Some(c) => published.entry(c.to_owned()).or_default().push(args),
                None => asked.push(json!([method, args])),
            }
        }
        self.0.push(
            json!({ "step": what, "answer": answer, "asked": asked, "published": published }),
        );
    }

    async fn call(&mut self, rig: &Rig, action: &str, args: Value) {
        let answer = match rig.call(action, args).await {
            Ok(v) => json!({ "ok": v }),
            Err(e) => json!({ "error": e.code, "message": e.message }),
        };
        self.step(rig, action, answer).await;
    }

    async fn emit(&mut self, rig: &Rig, name: &str, data: Value) {
        rig.emit(name, data);
        rig.handled().await;
        self.step(rig, name, Value::Null).await;
    }

    async fn render(&mut self, rig: &Rig, point: &str, cells: bool) {
        let surface = json!({ "cells": cells, "width": 280.0, "height": 400.0 });
        self.call(
            rig,
            "ui.render",
            json!({ "point": point, "surface": surface }),
        )
        .await;
    }

    async fn click(&mut self, rig: &Rig, point: &str, id: &str) {
        self.call(
            rig,
            "ui.event",
            json!({ "point": point, "event": { "id": id, "kind": "click" } }),
        )
        .await;
    }

    async fn rows(&mut self, rig: &Rig, collections: &[&str]) {
        rig.quiet().await;
        let rows: Value = collections
            .iter()
            .map(|c| ((*c).to_owned(), json!(rig.rows(c))))
            .collect();
        self.step(rig, "rows", rows).await;
    }

    pub fn into_value(self) -> Value {
        Value::Array(self.0)
    }
}

/// A request for a Bash command with a rule to keep, and one that asks a question.
pub fn asks() -> [Value; 2] {
    [
        json!({
            "requestId": "r1", "terminalId": "s1", "toolName": "Bash",
            "toolInput": { "command": "cargo test" },
            "permissionSuggestions": [
                { "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": "cargo test:*" }] },
                { "type": "setMode", "mode": "acceptEdits" },
            ],
        }),
        json!({
            "requestId": "r2", "terminalId": "s2", "toolName": "AskUserQuestion", "projectName": "api",
            "questions": [
                { "header": "Scope", "question": "Which crates?", "options": [{ "label": "core" }, { "label": "all" }] },
            ],
        }),
    ]
}

/// The Widget: sessions change, two agents ask, both windows draw on pixels and cells, and each kind of button is clicked.
pub async fn widget(rig: &Rig) -> Value {
    let mut t = Transcript::default();
    const ROWS: [&str; 4] = ["agents", "permissions", "suggestions", "options"];
    t.emit(rig, "session.created", json!({ "id": "s1" })).await;
    for ask in asks() {
        t.emit(rig, "agent.permission", ask).await;
    }
    t.rows(rig, &ROWS).await;
    for point in ["widget", "compact"] {
        for cells in [false, true] {
            t.render(rig, point, cells).await;
        }
    }
    for id in [
        "collapse",
        "expand",
        "vorn",
        "show:s1",
        "pick:r2:0:1",
        "submit:r2",
        "suggest:r1:0",
        "nope",
    ] {
        t.click(rig, "widget", id).await;
    }
    t.emit(rig, "agent.permission", asks()[0].clone()).await;
    t.call(rig, "deny", json!({ "id": "r1" })).await;
    t.emit(rig, "agent.permissionClosed", json!({ "requestId": "r1" }))
        .await;
    t.emit(rig, "session.exited", json!({ "id": "s1" })).await;
    t.rows(rig, &ROWS).await;
    t.render(rig, "widget", false).await;
    t.call(rig, "unknown", Value::Null).await;
    t.into_value()
}

fn worktree_path(i: usize) -> String {
    format!("{}/.worktrees/w{i}", project(i))
}

/// Worktree cleanup: an idle worktree is offered and kept, the settings
/// page scans and draws, one with changes is asked about and removed,
/// build output is reclaimed and two are removed at once.
pub async fn worktrees(rig: &Rig) -> Value {
    let mut t = Transcript::default();
    const ROWS: [&str; 5] = ["idle", "worktrees", "summary", "asked", "scan"];
    for i in [1, 5, 11] {
        let data = json!({ "worktreePath": worktree_path(i), "projectPath": project(i) });
        t.emit(rig, "worktree.idle", data).await;
    }
    t.rows(rig, &ROWS).await;
    t.render(rig, "idle", false).await;
    t.click(rig, "idle", &format!("keep:{}", worktree_path(1)))
        .await;
    t.call(rig, "scan", Value::Null).await;
    t.render(rig, "worktrees", false).await;
    t.call(
        rig,
        "ask",
        json!({ "worktree": worktree_path(3), "project": project(3) }),
    )
    .await;
    t.render(rig, "confirm", false).await;
    t.click(rig, "confirm", "confirm").await;
    t.call(rig, "reclaim_all", Value::Null).await;
    t.call(
        rig,
        "remove_many",
        json!({ "ids": [worktree_path(7), worktree_path(9)] }),
    )
    .await;
    t.call(rig, "remove", json!({ "id": "/nowhere" })).await;
    t.rows(rig, &ROWS).await;
    t.render(rig, "worktrees", true).await;
    t.into_value()
}

pub fn feed(name: &str) -> String {
    format!("https://{FEED_HOST}/{name}")
}

/// The feeds a poll reads: every format, one that is not there and one that does not parse.
pub fn feeds() -> Vec<String> {
    [
        "rss2.xml",
        "rss1.rdf",
        "atom.xml",
        "feed.json",
        "doctype.xml",
        "missing.xml",
        "malformed.xml",
    ]
    .map(feed)
    .to_vec()
}

/// The RSS actions' arguments, as either tier takes them.
pub fn rss_calls() -> Vec<(&'static str, Value)> {
    vec![
        ("readFeed", json!({ "url": feed("rss2.xml"), "limit": 5 })),
        ("readFeed", json!({ "url": feed("atom.xml") })),
        ("readFeed", json!({ "url": feed("feed.json"), "limit": 3 })),
        ("readFeed", json!({ "url": feed("rss1.rdf") })),
        ("readFeed", json!({ "url": feed("missing.xml") })),
        (
            "readFeeds",
            json!({ "urls": feeds().join("\n"), "perFeed": 2, "minFeedsOk": 1 }),
        ),
        (
            "readFeeds",
            json!({ "urls": feed("rss2.xml"), "keywords": "station" }),
        ),
        ("findFeeds", json!({ "url": feed("page.html") })),
        ("findFeeds", json!({ "url": feed("atom.xml") })),
    ]
}

/// The RSS feed reader on either Rust tier: every action, then two polls, the second answered 304.
pub async fn rss(rig: &Rig) -> Value {
    let mut t = Transcript::default();
    for (action, args) in rss_calls() {
        t.call(rig, action, args).await;
    }
    let poll = |cursor: Value| poll_args(&cursor);
    let first = rig
        .call("poll", poll(Value::Null))
        .await
        .map_err(|e| e.message);
    let cursor = first
        .as_ref()
        .ok()
        .map_or(Value::Null, |p| p["nextCursor"].clone());
    t.step(rig, "poll", json!(first.map(poll_shape))).await;
    let second = rig.call("poll", poll(cursor)).await.map_err(|e| e.message);
    t.step(rig, "poll again", json!(second.map(poll_shape)))
        .await;
    t.into_value()
}

/// A poll's arguments: the connection's feeds as its settings hold them, one a line.
pub fn poll_args(cursor: &Value) -> Value {
    json!({ "feeds": feeds().join("\n"), "lookbackHours": 8760, "cursor": cursor })
}

/// A poll's answer, its cursor parsed so key order does not count.
pub fn poll_shape(mut answer: Value) -> Value {
    if let Some(cursor) = answer["nextCursor"].as_str() {
        answer["nextCursor"] = serde_json::from_str(cursor).unwrap_or(Value::Null);
    }
    answer
}

/// The crash probe's harmless call.
pub async fn crash(rig: &Rig) -> Value {
    let mut t = Transcript::default();
    t.call(rig, "echo", json!({ "said": "hello" })).await;
    t.call(rig, "nope", Value::Null).await;
    t.into_value()
}

/// `ext`'s scripted session.
pub async fn run(rig: &Rig) -> Value {
    match rig.ext.id {
        "bench.widget" => widget(rig).await,
        "bench.worktrees" => worktrees(rig).await,
        "bench.rss" => rss(rig).await,
        _ => crash(rig).await,
    }
}
