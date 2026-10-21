//! The pane beside a card: the tasks of the card's project, by status, each
//! row opening in place to its description, its status and what can be done
//! with it, as the board's card menu offers.

use std::collections::BTreeSet;

use vorn_extension_sdk::tree::sdk::{button, choice, col, entry, group, list, notice, text};
use vorn_extension_sdk::tree::{EventKind, Node, TextStyle, Tone};
use vorn_extension_sdk::{json, Value};

use crate::view::LANES;

/// The pane's id in the manifest.
pub const PANE: &str = "tasks";

/// The collection cleared after every write, so each open pane draws again.
pub const REDRAW: &str = "pane";

/// What a pane's event asks of the extension.
#[derive(Debug, Clone, PartialEq)]
pub enum Asked {
    /// Open or close task `.0`'s row.
    Open(String, bool),
    /// One of the board's actions on a task (`move` with its `to`, `complete`, …).
    Action(&'static str, Value),
    /// Show session `.0` in the main window.
    Focus(String),
}

/// What node `id` reporting `kind` with `value` asks for; none for what the pane does not draw.
pub fn asked(id: &str, kind: EventKind, value: &str) -> Option<Asked> {
    match kind {
        EventKind::Toggle => Some(Asked::Open(id.to_owned(), value == "true")),
        EventKind::Select => {
            let task = id.strip_prefix("status:")?;
            LANES.iter().find(|l| l.0 == value)?;
            Some(Asked::Action("move", json!({ "id": task, "to": value })))
        }
        EventKind::Click => {
            let (action, rest) = id.split_once(':')?;
            if action == "focus" {
                return Some(Asked::Focus(rest.to_owned()));
            }
            let action = ACTIONS.iter().find(|a| a.0 == action)?.0;
            Some(Asked::Action(action, json!({ "id": rest })))
        }
        _ => None,
    }
}

/// The board's card actions the pane offers: id, label, icon.
const ACTIONS: [(&str, &str, &str); 6] = [
    ("complete", "Mark as done", "circle-check"),
    ("reopen", "Reopen task", "rotate-ccw"),
    ("cancel", "Cancel task", "circle-x"),
    ("archive", "Archive", "archive"),
    ("unarchive", "Unarchive", "archive-restore"),
    ("delete", "Delete", "trash-2"),
];

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or_default()
}

/// Three letters of the project and four of the id, as the board shows a task.
pub fn short_id(project: &str, id: &str) -> String {
    let prefix: String = project
        .chars()
        .filter(char::is_ascii_alphabetic)
        .take(3)
        .collect::<String>()
        .to_uppercase();
    let prefix = if prefix.is_empty() { "TSK" } else { &prefix };
    let suffix = id.chars().take(4).collect::<String>().to_uppercase();
    format!("{prefix}-{suffix}")
}

/// "Mar 5" from an ISO 8601 time; nothing from what does not read as one.
fn short_date(iso: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month = iso.get(5..7).and_then(|m| m.parse::<usize>().ok());
    let day = iso.get(8..10).and_then(|d| d.parse::<u8>().ok());
    match (month.and_then(|m| MONTHS.get(m.checked_sub(1)?)), day) {
        (Some(m), Some(d)) => format!("Created {m} {d}"),
        _ => String::new(),
    }
}

/// The pane for a card of `project`: `tasks` grouped by status, the rows in `open` opened.
pub fn pane(project: &str, tasks: &[Value], open: &BTreeSet<String>) -> Node {
    let groups = LANES.iter().filter_map(|(status, title, glyph, tone)| {
        let rows: Vec<Node> = tasks
            .iter()
            .filter(|t| s(t, "status") == *status)
            .map(|t| task(project, t, *tone, open.contains(s(t, "id"))))
            .collect();
        (!rows.is_empty()).then(|| {
            let n = rows.len();
            group(*title, rows).icon(*glyph).aside(n.to_string())
        })
    });
    list("tasks", groups).empty(format!("No tasks in {project} yet."))
}

/// What the pane says for a card it cannot list tasks for.
pub fn unknown() -> Node {
    notice("This card's project has no tasks to show.", Tone::Neutral)
}

fn task(project: &str, t: &Value, tone: Tone, open: bool) -> Node {
    let id = s(t, "id");
    let status = s(t, "status");
    let archived = t.get("archivedAt").is_some_and(|a| !a.is_null());
    let terminal = matches!(status, "done" | "cancelled");
    let offered = |action: &str| match action {
        "complete" => status == "in_review",
        "reopen" => status == "cancelled",
        "cancel" => !terminal,
        "archive" => !archived && terminal,
        "unarchive" => archived,
        _ => true,
    };
    let mut actions = Vec::new();
    let session = s(t, "assignedSessionId");
    if status == "in_progress" && !session.is_empty() {
        actions.push(button(format!("focus:{session}"), "Focus session").icon("terminal"));
    }
    let mut more = Vec::new();
    for (action, label, glyph) in ACTIONS.iter().filter(|a| offered(a.0)) {
        let b = button(format!("{action}:{id}"), *label).icon(*glyph);
        match *action {
            "complete" | "reopen" => actions.push(b),
            "cancel" => more.push(b.danger()),
            "delete" => more.push(b.danger().confirm("Delete this task permanently?")),
            _ => more.push(b),
        }
    }
    let e = entry(id, s(t, "title"))
        .tone(tone)
        .code(short_id(project, id))
        .detail(short_date(s(t, "createdAt")))
        .actions(actions)
        .more(more)
        .expanded(open);
    if !open {
        return e;
    }
    let description = match s(t, "description").trim() {
        "" => text("No description").style(TextStyle::Secondary),
        d => text(d),
    };
    let statuses = LANES.iter().map(|(v, title, ..)| (*v, *title));
    e.child(
        col([
            description,
            choice(format!("status:{id}"), statuses, status),
        ])
        .gap(8.0),
    )
}
