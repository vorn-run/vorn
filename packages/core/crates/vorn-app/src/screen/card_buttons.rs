//! A card's buttons as one list a person arranges: the splits, its panes
//! (an extension's too), what extensions add to the card's menu, and
//! minimize, expand and close. The first few show on the card's header; the
//! rest go into its More menu in the same order, with their shortcuts.

use super::customize::{pane_id, Mark};
use super::{mod_label, Screen};
use crate::act::Act;
use crate::client::Session;
use crate::points::kind;
use crate::state::{LayoutMode, MenuKind};

/// A shell to the card's right.
pub const SPLIT_RIGHT: &str = "split-right";
/// A shell under the card.
pub const SPLIT_DOWN: &str = "split-down";
/// Into the dock (the grid only).
pub const MINIMIZE: &str = "minimize";
/// Onto the focused stage, or back to the grid (the grid only).
pub const EXPAND: &str = "expand";
pub const CLOSE: &str = "close";

/// How many buttons a card shows before the rest go into More.
pub const DEFAULT_SHOWN: usize = 5;

/// One button as Customize lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub icon: Mark,
    pub title: String,
    /// The extension it comes from; none for the app's own.
    pub from: Option<String>,
}

fn own(id: &str, icon: &str, title: &str) -> Entry {
    Entry {
        id: id.to_owned(),
        icon: Mark::kit(icon),
        title: title.to_owned(),
        from: None,
    }
}

/// Every button a card can have, in the default order: the splits, the
/// folder, expand and close (the five a card starts with), then minimize,
/// the other panes, and what extensions add to the card's menu.
pub fn catalog(st: &crate::state::State) -> Vec<Entry> {
    let panes = super::customize::panes(st);
    let files = pane_id(crate::app::FILES, "files");
    let pane = |id: &str| panes.iter().find(|p| p.id == id);
    let as_entry = |p: &super::customize::Pane| Entry {
        id: p.id.clone(),
        icon: p.icon.clone(),
        title: p.title.clone(),
        from: p.from.clone(),
    };
    let mut out = vec![
        own(SPLIT_RIGHT, "columns-2", "Split right"),
        own(SPLIT_DOWN, "rows-2", "Split down"),
    ];
    // The folder keeps the outline icon it always had, whatever its pane declares.
    out.push(pane(&files).map_or_else(
        || own(&files, "folder-open", "Folder"),
        |p| Entry {
            icon: Mark::kit("folder-open"),
            ..as_entry(p)
        },
    ));
    out.push(own(EXPAND, "maximize-2", "Expand"));
    out.push(own(CLOSE, "x", "Close"));
    out.push(own(MINIMIZE, "minus", "Minimize"));
    for p in panes.iter().filter(|p| p.id != files) {
        out.push(as_entry(p));
    }
    for p in st.points.at(kind::MENUS) {
        if p.menu.as_deref() != Some("card") {
            continue;
        }
        out.push(Entry {
            id: p.key(),
            icon: Mark::of(st, &p.extension, None, p.icon.as_deref()),
            title: p.title.clone(),
            from: Some(p.extension_name.clone()),
        });
    }
    out
}

/// One of a card's buttons now: what it does, how it looks and its key.
pub struct Button {
    pub id: String,
    pub act: Act,
    pub icon: Mark,
    /// What its tooltip and menu entry say.
    pub label: String,
    pub shortcut: Option<String>,
    pub danger: bool,
}

/// The buttons `session`'s card has, in the arranged order, those hidden
/// and those that do not apply to it (minimize and expand off the grid, a
/// pane it cannot show) left out.
pub fn buttons(s: &Screen<'_>, session: &Session, focused: bool) -> Vec<Button> {
    let st = s.st;
    let sid = session.info.id.clone();
    let grid = st.layout == LayoutMode::Grid;
    let catalog = catalog(st);
    let ids: Vec<&str> = catalog.iter().map(|e| e.id.as_str()).collect();
    let mut panes = super::side::pane_entries(s, session);
    let browser = super::panes::browser_entry(s, session);
    let device = super::panes::device_entry(s, session);
    let ext: Vec<crate::extensions::PaneItem> = st
        .ext
        .items(&sid)
        .into_iter()
        .chain(st.ext.builtin_items(&sid, &st.points))
        .collect();
    let args = serde_json::json!({ "sessionId": sid }).to_string();
    let mut out = Vec::new();
    for id in st.arrange.card.shown(&ids) {
        let Some(e) = catalog.iter().find(|e| e.id == id) else {
            continue;
        };
        let simple = |act: Act, label: &str, shortcut: Option<String>| Button {
            id: id.to_owned(),
            act,
            icon: e.icon.clone(),
            label: label.to_owned(),
            shortcut,
            danger: false,
        };
        let b = match id {
            SPLIT_RIGHT | SPLIT_DOWN => {
                let down = id == SPLIT_DOWN;
                let act = Act::Split {
                    session: Some(sid.clone()),
                    down,
                };
                let label = if down { "Split down" } else { "Split right" };
                simple(act, label, Some(super::chrome::split_key(down)))
            }
            MINIMIZE if grid && !focused => {
                simple(Act::ToggleMinimized(sid.clone()), "Minimize", None)
            }
            EXPAND if grid && focused => Button {
                icon: Mark::kit("minimize-2"),
                ..simple(
                    Act::Collapse,
                    "Collapse to grid",
                    Some(format!("{}W", mod_label())),
                )
            },
            EXPAND if grid => simple(
                Act::Expand(sid.clone()),
                "Expand",
                Some(format!("{}O", mod_label())),
            ),
            MINIMIZE | EXPAND => continue,
            CLOSE => Button {
                danger: true,
                ..simple(Act::AskClose(sid.clone()), "Close session", None)
            },
            _ => {
                if let Some(at) = panes.iter().position(|p| p.0 == id) {
                    let (_, act, icon, label, key) = panes.remove(at);
                    let key = key.map(|k| format!("{}\u{21e7}{k}", mod_label()));
                    Button {
                        icon: Mark::kit(icon),
                        ..simple(act, label, key)
                    }
                } else if id == pane_id(crate::panes::BROWSER, crate::panes::BROWSER_PANE) {
                    let Some((act, label)) = browser.clone() else {
                        continue;
                    };
                    simple(act, label, None)
                } else if id == pane_id(crate::panes::DEVICE, crate::panes::DEVICE_PANE) {
                    let Some((act, label)) = device.clone() else {
                        continue;
                    };
                    simple(act, label, None)
                } else if let Some(p) = ext
                    .iter()
                    .find(|p| pane_id(&p.extension_id, &p.pane_id) == id)
                {
                    let act = Act::Ext(crate::extensions::ExtAct::Toggle {
                        session: sid.clone(),
                        extension: p.extension_id.clone(),
                        pane: p.pane_id.clone(),
                    });
                    let label = if p.open {
                        format!("Close {}", p.title)
                    } else {
                        p.title.clone()
                    };
                    simple(act, &label, None)
                } else if let Some(p) = st.points.get(id).filter(|p| p.kind == kind::MENUS) {
                    let act = if p.rows.is_some() {
                        Act::Open(MenuKind::CardPoint(sid.clone(), p.key()))
                    } else {
                        let key = p.key();
                        let args = args.clone();
                        Act::Point(crate::points::PointAct::Choose { key, args })
                    };
                    simple(act, &p.title, None)
                } else {
                    continue;
                }
            }
        };
        out.push(b);
    }
    out
}

/// How many buttons the card's header shows.
pub fn shown(s: &Screen<'_>) -> usize {
    s.st.arrange.card_shown
}
