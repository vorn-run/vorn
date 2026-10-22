//! Extension panes: what each session's extensions offer, which pane a
//! card has open, and what to call it. The pure half; the RPC glue is
//! `app::extensions`.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::runs::s;

/// Custom ids at and above this are extension program terminals:
/// `TERM_ID_BASE + i` draws `Extensions::terminals()[i]`.
pub const TERM_ID_BASE: u64 = 800_000;

/// The terminal slot a custom box shows, if it is one.
pub fn term_index(id: u64) -> Option<usize> {
    if id >= crate::panes::WEB_ID_BASE {
        return None;
    }
    id.checked_sub(TERM_ID_BASE)
        .and_then(|i| usize::try_from(i).ok())
}

/// What a session's extension page key ends in.
pub const PAGE_SUFFIX: &str = "#extension";

/// The web view key of a session's extension page.
pub fn page_key(session: &str) -> String {
    format!("{session}{PAGE_SUFFIX}")
}

/// What the card menu and the pane do.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ExtAct {
    /// Open a pane on a card, or close it when it is the one showing.
    Toggle {
        session: String,
        extension: String,
        pane: String,
    },
    Close(String),
    /// Hand the keys to a session's program pane.
    Focus(String),
}

/// A pane an extension opened on a card: the host's grant and its names.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenPane {
    pub nonce: String,
    pub extension_id: String,
    pub pane_id: String,
    /// A page the pack ships, from the host's page origin.
    pub url: Option<String>,
    /// A program the host runs, drawn as its terminal.
    pub terminal_id: Option<String>,
    pub title: String,
    pub extension_name: String,
    /// The pane's own glyph, else the extension's.
    pub icon: Option<Value>,
}

impl OpenPane {
    /// The pane `grant` opened, named from the installed packs.
    pub fn from_grant(grant: &Value, packs: &[Value]) -> Option<OpenPane> {
        let ext = s(grant, "extensionId")?;
        let pane = s(grant, "paneId")?;
        let (title, extension_name, icon) = label(packs, ext, pane);
        Some(OpenPane {
            nonce: s(grant, "nonce")?.to_owned(),
            extension_id: ext.to_owned(),
            pane_id: pane.to_owned(),
            url: s(grant, "url").map(str::to_owned),
            terminal_id: s(grant, "terminalId").map(str::to_owned),
            title,
            extension_name,
            icon,
        })
    }
}

/// One card menu row: a pane an active extension offers on this card.
#[derive(Debug, Clone, PartialEq)]
pub struct PaneItem {
    pub extension_id: String,
    pub pane_id: String,
    pub title: String,
    pub extension_name: String,
    pub icon: Option<Value>,
    /// It is the pane showing now, so the row closes it.
    pub open: bool,
}

/// What every session's extensions say, and the panes open.
#[derive(Debug, Default)]
pub struct Extensions {
    /// `extension:list`: the installed extension packs.
    pub packs: Vec<Value>,
    pub loaded: bool,
    /// `extension:activation`, by session.
    pub activation: HashMap<String, Vec<Value>>,
    /// Sessions asked about once already.
    pub hydrated: HashSet<String>,
    /// The pane each card shows.
    pub panes: HashMap<String, OpenPane>,
    /// The session whose program pane takes the keys.
    pub focus: Option<String>,
    /// Program terminals in this frame's order, for their custom boxes.
    pub slots: Vec<String>,
}

impl Extensions {
    /// The card menu's pane rows for `session`, first in the menu.
    pub fn items(&self, session: &str) -> Vec<PaneItem> {
        let showing = self.panes.get(session);
        let mut out = Vec::new();
        for st in self.activation.get(session).into_iter().flatten() {
            let active = st.get("active").and_then(Value::as_bool) == Some(true);
            let offered: Vec<&str> = st
                .get("panes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let Some(ext) = s(st, "extensionId") else {
                continue;
            };
            if !active || offered.is_empty() {
                continue;
            }
            let pack = self.packs.iter().find(|p| s(p, "id") == Some(ext));
            for pane in panes_of(pack) {
                let Some(id) = s(pane, "id").filter(|id| offered.contains(id)) else {
                    continue;
                };
                out.push(PaneItem {
                    extension_id: ext.to_owned(),
                    pane_id: id.to_owned(),
                    title: s(pane, "title").unwrap_or(id).to_owned(),
                    extension_name: s(st, "extensionName").unwrap_or(ext).to_owned(),
                    icon: pane
                        .get("icon")
                        .or_else(|| pack.and_then(|p| p.get("icon")))
                        .filter(|i| !i.is_null())
                        .cloned(),
                    open: showing.is_some_and(|o| o.extension_id == ext && o.pane_id == id),
                });
            }
        }
        out
    }

    /// The card menu's rows for the built-ins' tree panes the host does not
    /// place itself, as listed now: a setting or switch turned off takes its row away.
    pub fn builtin_items(&self, session: &str, points: &crate::points::Points) -> Vec<PaneItem> {
        let showing = self.panes.get(session);
        (points.all.iter())
            .filter(|p| p.builtin && p.tree && p.kind == crate::points::kind::PANES)
            .filter(|p| !crate::panes::PLACED.contains(&(p.extension.as_str(), p.id.as_str())))
            .map(|p| PaneItem {
                extension_id: p.extension.clone(),
                pane_id: p.id.clone(),
                title: p.title.clone(),
                extension_name: p.extension_name.clone(),
                icon: None,
                open: showing.is_some_and(|o| o.extension_id == p.extension && o.pane_id == p.id),
            })
            .collect()
    }

    /// Whether `session` may keep its pane under activation `states`: the
    /// host drops the grants of an extension that no longer shows there.
    pub fn still_shown(&self, session: &str, states: &[Value]) -> bool {
        let Some(open) = self.panes.get(session) else {
            return true;
        };
        states.iter().any(|st| {
            st.get("active").and_then(Value::as_bool) == Some(true)
                && s(st, "extensionId") == Some(open.extension_id.as_str())
        })
    }

    /// The session whose pane runs terminal `id`.
    pub fn owner_of_terminal(&self, id: &str) -> Option<&str> {
        self.panes
            .iter()
            .find(|(_, p)| p.terminal_id.as_deref() == Some(id))
            .map(|(k, _)| k.as_str())
    }

    /// Every program terminal open, in a stable order.
    pub fn terminals(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .panes
            .values()
            .filter_map(|p| p.terminal_id.clone())
            .collect();
        out.sort();
        out
    }
}

/// What a template pane's nonce starts with: the client draws it, so vornd holds no grant.
pub const VIEW_NONCE: &str = "view:";

/// The key of a template pane's tree, drawn for its card's session, when `p` is one.
pub fn view_key(p: &OpenPane) -> Option<String> {
    p.nonce.strip_prefix(VIEW_NONCE).map(str::to_owned)
}

fn panes_of(pack: Option<&Value>) -> Vec<&Value> {
    pack.and_then(|p| p.pointer("/contributes/panes"))
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

/// The pane's title, its extension's name and a glyph, falling
/// back to the ids so a pane always opens with some name.
pub fn label(packs: &[Value], ext: &str, pane: &str) -> (String, String, Option<Value>) {
    let pack = packs.iter().find(|p| s(p, "id") == Some(ext));
    let contribution = panes_of(pack)
        .into_iter()
        .find(|p| s(p, "id") == Some(pane));
    let icon = contribution
        .and_then(|p| p.get("icon"))
        .or_else(|| pack.and_then(|p| p.get("icon")))
        .filter(|i| !i.is_null())
        .cloned();
    (
        contribution
            .and_then(|p| s(p, "title"))
            .unwrap_or(pane)
            .to_owned(),
        pack.and_then(|p| s(p, "name")).unwrap_or(ext).to_owned(),
        icon,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn packs() -> Vec<Value> {
        vec![
            json!({ "id": "rep", "name": "Reporter", "icon": { "paths": ["M0 0"] },
            "contributes": { "panes": [
                { "id": "report", "title": "Report" },
                { "id": "logs", "title": "Logs", "icon": { "paths": ["M1 1"] } }
            ] } }),
        ]
    }

    #[test]
    fn menu_rows_follow_activation_and_mark_the_open_pane() {
        let mut x = Extensions {
            packs: packs(),
            ..Extensions::default()
        };
        x.activation.insert(
            "s".into(),
            vec![
                json!({ "extensionId": "rep", "extensionName": "Reporter", "active": true,
                        "panes": ["logs", "report"] }),
                json!({ "extensionId": "off", "active": false, "panes": ["x"] }),
            ],
        );
        let rows = x.items("s");
        // The pack's order, not the activation's.
        assert_eq!(
            rows.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(),
            ["Report", "Logs"]
        );
        assert_eq!(rows[0].icon, Some(json!({ "paths": ["M0 0"] })));
        assert_eq!(rows[1].icon, Some(json!({ "paths": ["M1 1"] })));
        let grant = json!({ "nonce": "n", "extensionId": "rep", "paneId": "logs",
                            "url": "http://127.0.0.1:9/x/" });
        let open = OpenPane::from_grant(&grant, &x.packs).expect("a grant");
        assert_eq!(
            (open.title.as_str(), open.extension_name.as_str()),
            ("Logs", "Reporter")
        );
        x.panes.insert("s".into(), open);
        assert!(x.items("s")[1].open);
        assert!(!x.still_shown("s", &[json!({ "extensionId": "rep", "active": false })]));
    }

    #[test]
    fn unknown_panes_are_named_by_their_ids() {
        assert_eq!(
            label(&packs(), "nope", "p"),
            ("p".into(), "nope".into(), None)
        );
        assert_eq!(term_index(TERM_ID_BASE + 2), Some(2));
        assert_eq!(term_index(crate::panes::WEB_ID_BASE), None);
    }
}
