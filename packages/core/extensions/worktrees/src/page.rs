//! The Worktrees settings page as an element tree: totals, then each project's worktrees.

use vorn_extension_sdk::tree::sdk::*;
use vorn_extension_sdk::tree::{Event, Node, Tone};
use vorn_extension_sdk::{json, Error, Host, Value};

use super::{ALL, SCAN, SUMMARY};

/// What the page's buttons run, by the id before the first `:`; after it is the worktree.
const DOES: [&str; 6] = [
    "refresh",
    "reclaim_all",
    "remove_many",
    "reclaim",
    "prune",
    "remove",
];

pub(super) fn render(host: &mut dyn Host) -> Result<Node, Error> {
    let scan = host.rows(SCAN)?.into_iter().next().unwrap_or(Value::Null);
    let summary = host.rows(SUMMARY)?.into_iter().next();
    Ok(page(&host.rows(ALL)?, &scan, summary.as_ref()))
}

/// The action a click on the page runs, and its args.
pub(super) fn event(e: &Event) -> Result<(&'static str, Value), Error> {
    let (does, path) = e.id.split_once(':').unwrap_or((e.id.as_str(), ""));
    let does = DOES
        .into_iter()
        .find(|d| *d == does)
        .ok_or_else(|| Error::new("bad_args", format!("no button is '{}'", e.id)))?;
    let args = match (path, does) {
        ("", "remove_many") => json!({ "ids": e.selected }),
        ("", _) => Value::Null,
        (path, _) => json!({ "id": path }),
    };
    Ok((does, args))
}

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or_default()
}

#[allow(clippy::cast_precision_loss)]
fn n(v: &Value, key: &str) -> f64 {
    v[key].as_u64().unwrap_or(0) as f64
}

fn on(v: &Value, key: &str) -> bool {
    v[key].as_bool() == Some(true)
}

fn page(rows: &[Value], scan: &Value, summary: Option<&Value>) -> Node {
    let mut page = col([toolbar([button("refresh", "Refresh").icon("refresh-cw")])]).gap(16.0);
    if !s(scan, "error").is_empty() {
        page = page.child(notice(s(scan, "error"), Tone::Bad));
    }
    if let Some(sum) = summary.filter(|v| n(v, "total_bytes") > 0.0) {
        page = page.child(totals(sum));
    }
    let groups = group_by(rows, |r| s(r, "project_path"))
        .into_iter()
        .map(|(_, rs)| {
            let first = rs[0];
            group(s(first, "project"), rs.into_iter().map(worktree))
                .note(s(first, "against"))
                .aside(s(first, "project_size"))
        });
    let mut list = list("worktrees", groups)
        .select([button("remove_many", "Remove")
            .icon("trash-2")
            .danger()
            .confirm("Remove the checked worktrees? Uncommitted changes in them are lost.")])
        .empty("No worktrees yet. Each one shows here once you create it.");
    if on(scan, "busy") && rows.is_empty() {
        list = list.loading("Measuring worktrees…");
    }
    page.child(list)
}

/// On disk, the build output a reinstall rebuilds, and what can go.
fn totals(sum: &Value) -> Node {
    let bar = meter(
        n(sum, "total_bytes"),
        [
            segment(n(sum, "build_bytes"), "Build output", Tone::Accent),
            segment(
                n(sum, "removable_bytes"),
                "Removable worktrees",
                Tone::Accent,
            ),
        ],
    )
    .aside(format!("{} in merged worktrees", s(sum, "removable")));
    let mut card = card([
        row([
            stat(s(sum, "total"), "On disk").icon("hard-drive"),
            stat(s(sum, "build"), "Build output, rebuilt by a reinstall")
                .tone(Tone::Accent)
                .end(),
        ])
        .between(),
        bar.node(),
    ]);
    if on(sum, "reclaimable") {
        card = card.child(row([button("reclaim_all", s(sum, "reclaim_label"))
            .icon("trash-2")
            .confirm(s(sum, "reclaim_confirm"))]));
    }
    card
}

/// One worktree; freeing space shows on hover, and on a terminal joins the menu.
fn worktree(r: &Value) -> Node {
    let path = s(r, "id");
    let name = s(r, "name");
    let mut actions = Vec::new();
    let mut more = Vec::new();
    if on(r, "reclaimable") {
        actions.push(button(format!("reclaim:{path}"), "Free space").icon("hard-drive"));
    }
    if on(r, "orphan") {
        more.push(
            button(format!("prune:{path}"), "Delete folder")
                .icon("trash-2")
                .danger()
                .confirm(format!("Delete the folder {name}?")),
        );
    }
    if on(r, "remove_dirty") || on(r, "remove_clean") {
        let lost = if on(r, "remove_dirty") {
            " Its uncommitted changes are lost."
        } else {
            ""
        };
        more.push(
            button(format!("remove:{path}"), "Remove")
                .icon("trash-2")
                .danger()
                .confirm(format!("Remove {name}?{lost}")),
        );
    }
    let mut e = entry(path, name)
        .icon("folder-git-2")
        .code(s(r, "branch"))
        .detail(s(r, "detail"))
        .aside(s(r, "size"))
        .meter(meter(
            n(r, "size_bytes"),
            [segment(
                n(r, "artifact_bytes"),
                "Build output",
                Tone::Accent,
            )],
        ))
        .actions(actions)
        .more(more);
    if let Some(tone) = verdict(s(r, "verdict")) {
        e = e.badge(s(r, "verdict_label"), tone);
    }
    if on(r, "held") {
        e = e.fixed();
    }
    e
}

fn verdict(v: &str) -> Option<Tone> {
    match v {
        "review" => Some(Tone::Warn),
        "reclaim" | "remove" => Some(Tone::Accent),
        "orphan" => Some(Tone::Bad),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(path: &str, project: &str, verdict: &str) -> Value {
        json!({
            "id": path, "name": path.rsplit('/').next(), "project": project,
            "project_path": format!("/{project}"), "project_size": "90 MB", "against": "vs main",
            "branch": "fix", "size": "24 MB", "size_bytes": 24, "artifact_bytes": 20,
            "detail": "today", "verdict": verdict, "verdict_label": "Review",
            "reclaimable": true, "remove_dirty": true, "held": verdict == "keep",
        })
    }

    #[test]
    fn groups_by_project_and_checks_clean() {
        let rows = [
            row("/w/a", "acme", "review"),
            row("/w/b", "web", "remove"),
            row("/w/c", "acme", "keep"),
        ];
        let sum = json!({ "total_bytes": 90, "total": "90 MB", "reclaimable": true,
            "reclaim_label": "Reclaim 60 MB of build output", "reclaim_confirm": "Delete?" });
        let tree = page(&rows, &Value::Null, Some(&sum));
        vorn_extension_sdk::tree::check(&tree).expect("a tree the client draws");
        let json = vorn_extension_sdk::serde_json::to_string(&tree).unwrap();
        for said in [
            "acme",
            "vs main",
            "Reclaim 60 MB",
            "Remove a?",
            "remove:/w/b",
        ] {
            assert!(json.contains(said), "{said} in {json}");
        }
    }

    #[test]
    fn a_click_names_its_action_and_rows() {
        let click = |id: &str, selected: &[&str]| Event {
            id: id.into(),
            kind: vorn_extension_sdk::tree::EventKind::Click,
            value: String::new(),
            selected: selected.iter().map(|s| (*s).to_owned()).collect(),
        };
        assert_eq!(
            event(&click("remove:/w/a", &[])).unwrap(),
            ("remove", json!({ "id": "/w/a" }))
        );
        assert_eq!(
            event(&click("remove_many", &["/w/a", "/w/b"])).unwrap(),
            ("remove_many", json!({ "ids": ["/w/a", "/w/b"] }))
        );
        assert!(event(&click("ui.render", &[])).is_err());
    }
}
