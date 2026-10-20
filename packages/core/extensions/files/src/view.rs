//! The trees Files draws: a session's pane (its tabs, the file tree and the
//! open file) and a popped-out file's card.

use vorn_extension_sdk::tree::sdk::{button, code, col, outline, split, tabs};
use vorn_extension_sdk::tree::{self, Code, Filter, Outline, Tab, Tabs, Tool};
use vorn_extension_sdk::{Error, Host};

use crate::{tab_key, Buffer, Files, DEFAULT_TREE, SIDE_BY_SIDE_MIN};

/// The last part of a path.
pub fn base(path: &str) -> &str {
    let t = path.trim_end_matches(['/', '\\']);
    t.rsplit(['/', '\\']).next().unwrap_or(t)
}

/// The folder a path sits in, by its own name.
fn parent(path: &str) -> String {
    let t = path.trim_end_matches(['/', '\\']);
    let dir = t.rsplit_once(['/', '\\']).map_or("", |(d, _)| d);
    base(dir).to_owned()
}

fn tool(id: &str, label: &str, icon: &str) -> Tool {
    Tool {
        id: id.to_owned(),
        label: label.to_owned(),
        icon: icon.to_owned(),
        on: false,
    }
}

/// An open file's code, its strip ending with `buttons`.
fn code_of(b: &Buffer, root: Option<String>, buttons: Vec<tree::Node>) -> tree::Node {
    code(
        crate::code_id(&b.path),
        Code {
            path: b.path.clone(),
            root,
            text: b.content.clone().unwrap_or_default(),
            draft: b.draft.clone(),
            rev: b.rev,
            state: b.state,
            conflict: b.conflict,
            saving: false,
            error: b.error.clone(),
        },
        buttons,
    )
}

/// `owner`'s Files pane.
pub fn pane(files: &mut Files, host: &mut dyn Host, owner: &str) -> Result<tree::Node, Error> {
    let p = files.pane(host, owner)?.unwrap_or_default();
    let names: Vec<&str> = p.tabs.iter().map(|t| base(t)).collect();
    let mut strip = Vec::with_capacity(p.tabs.len());
    for t in &p.tabs {
        let label = base(t);
        let shared = names.iter().filter(|n| **n == label).count() > 1;
        strip.push(Tab {
            id: t.clone(),
            label: label.to_owned(),
            note: shared.then(|| parent(t)),
            file: true,
            active: p.active.as_deref() == Some(t.as_str()),
            preview: p.preview.as_deref() == Some(t.as_str()),
            dirty: files.dirty(host, &tab_key(owner, t)),
            icon: None,
        });
    }
    let tree_shown = p.tree_shown();
    let mut lead = tool(
        "toggle-tree",
        if tree_shown {
            "Hide file tree"
        } else {
            "Show file tree"
        },
        "list-tree",
    );
    lead.on = tree_shown;
    let mut tools = Vec::new();
    if p.active.is_some() {
        tools.push(tool(
            "pop",
            "Open as its own card",
            "square-arrow-out-up-right",
        ));
    }
    let bar = tabs(
        "tabs",
        Tabs {
            tabs: strip,
            empty: Some("Files".to_owned()),
            lead: vec![lead],
            tools,
            more: vec![tool("close-saved", "Close saved tabs", "")],
        },
    );
    let mut body = Vec::new();
    if tree_shown {
        let t = files.tree(host, owner);
        let (rows, none) = t.rows();
        let empty = t.root.is_empty();
        body.push(outline(
            "tree",
            Outline {
                rows,
                selected: p.active.clone(),
                filter: Some(Filter {
                    value: t.filter.clone(),
                    placeholder: "Filter files\u{2026}".to_owned(),
                }),
                loading: false,
                empty: empty.then(|| "Empty directory".to_owned()),
                note: none.then(|| "No matching files loaded".to_owned()),
                action: Some(tool(
                    "pop",
                    "Open {} as its own card",
                    "square-arrow-out-up-right",
                )),
                files: true,
            },
        ));
    }
    if let Some(path) = p.active.as_deref() {
        let root = files.tree(host, owner).cwd.clone();
        let b = files.buffer(host, &tab_key(owner, path), path)?;
        body.push(code_of(b, Some(root).filter(|r| !r.is_empty()), Vec::new()));
    }
    let ratio = host
        .get("ratio")?
        .and_then(|v| v.as_f64())
        .map_or(DEFAULT_TREE, |r| r as f32);
    let mut out = col([bar]);
    if !body.is_empty() {
        out = out.child(
            split("split", ratio, "Resize file tree", body)
                .narrow(SIDE_BY_SIDE_MIN)
                .grow(),
        );
    }
    Ok(out.grow())
}

/// Card `id`: its file's code, with Put back and Close.
pub fn card(files: &mut Files, host: &mut dyn Host, id: &str) -> Result<tree::Node, Error> {
    let Some(c) = files.cards(host)?.get(id).cloned() else {
        return Ok(col([]));
    };
    let root = crate::Files::cwd(host, &c.session);
    let b = files.buffer(host, id, &c.path)?;
    Ok(code_of(
        b,
        root,
        vec![
            button("return", "Put back in its session card").icon("corner-down-left"),
            button("close", "Close").icon("x"),
        ],
    ))
}
