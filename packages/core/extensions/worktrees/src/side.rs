//! The sidebar's idle worktrees and the dialog asked before one with sessions or changes goes, as element trees.

use vorn_extension_sdk::tree::sdk::*;
use vorn_extension_sdk::tree::{Event, Node, TextStyle, Tone};
use vorn_extension_sdk::{json, Error, Value};

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or_default()
}

fn on(v: &Value, key: &str) -> bool {
    v[key].as_bool() == Some(true)
}

/// Each idle worktree with Keep and Remove; nothing, and so no section, when there is none.
pub(super) fn idle(rows: &[Value]) -> Node {
    col(rows.iter().map(|r| {
        let path = s(r, "id");
        let name = s(r, "name");
        let mut remove = button(format!("remove:{path}"), "Remove").danger();
        if on(r, "dirty") {
            remove = remove.confirm(format!("Remove {name}? Its uncommitted changes are lost."));
        }
        let changes = on(r, "dirty").then(|| badge("changes", Tone::Warn));
        row([icon("git-branch"), text(name).grow()]
            .into_iter()
            .chain(changes)
            .chain([button(format!("keep:{path}"), "Keep"), remove]))
        .gap(8.0)
    }))
    .gap(4.0)
}

/// The action an idle row's button runs, with its worktree.
pub(super) fn idle_event(e: &Event) -> Result<(&'static str, Value), Error> {
    let (does, path) = e.id.split_once(':').unwrap_or((e.id.as_str(), ""));
    let does = match does {
        "keep" => "keep",
        "remove" => "remove",
        _ => return Err(Error::bad_args(format!("no button is '{}'", e.id))),
    };
    Ok((does, json!({ "id": path })))
}

/// What the dialog asks about the worktree in the `asked` row.
pub(super) fn confirm(asked: Option<&Value>) -> Node {
    let Some(a) = asked else {
        return col([]);
    };
    let sessions = a["sessions"].as_u64().unwrap_or(0);
    let mut kids = vec![
        text(format!("Remove {}?", s(a, "name"))).style(TextStyle::Title),
        text(format!("Branch {}", s(a, "branch"))).style(TextStyle::Secondary),
    ];
    if sessions > 0 {
        kids.push(text(format!(
            "Its {sessions} running sessions are closed first."
        )));
    }
    if on(a, "dirty") {
        kids.push(text("It has uncommitted changes, which are lost."));
    }
    kids.push(
        row([
            col([]).grow(),
            button("cancel", "Cancel"),
            button("confirm", "Remove").danger(),
        ])
        .gap(8.0),
    );
    col(kids).gap(6.0)
}

/// The action a dialog button runs.
pub(super) fn confirm_event(e: &Event) -> Result<&'static str, Error> {
    match e.id.as_str() {
        "cancel" => Ok("cancel"),
        "confirm" => Ok("confirm"),
        other => Err(Error::bad_args(format!("no button is '{other}'"))),
    }
}
