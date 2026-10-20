//! A shell composer's menu (IntentBar): what the Shell blocks extension
//! answered for what is typed (past commands, completions, the ghost of a
//! past command, what Tab fills in) and which row is picked. The answers come
//! from the extension; this holds only what the menu shows and where it is.

use std::collections::{HashMap, HashSet};
use std::time::Duration;
use web_time::Instant;

use serde_json::Value;

/// History rows a search offers.
pub const MAX_HISTORY: usize = 8;
/// History rows shown above completions.
pub const MAX_HISTORY_WITH_COMPLETIONS: usize = 3;
/// The pause in typing before completions are asked for.
pub const DEBOUNCE: Duration = Duration::from_millis(80);
/// Shell blocks, which answers the menu.
pub const BLOCKS: &str = "vorn.blocks";

/// Whether Shell blocks is on: it draws a shell's commands, and answers its composer.
pub fn blocks_on(points: &crate::points::Points) -> bool {
    points
        .find(BLOCKS, crate::points::kind::SHELL, "commands")
        .is_some()
}

/// A past command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Past {
    pub text: String,
    pub timestamp: u64,
}

/// A replacement for the token being typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub label: String,
    pub detail: Option<String>,
    pub icon: String,
    /// A folder: the menu stays open to go further in.
    pub continues: bool,
    /// What is typed once it is taken.
    pub result: String,
}

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_owned)
}

/// The rows and ghost of a `history` answer.
pub fn read_history(v: &Value) -> (Vec<Past>, Option<String>) {
    let rows = v["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            Some(Past {
                text: text(r, "text")?,
                timestamp: r.get("timestamp").and_then(Value::as_u64).unwrap_or(0),
            })
        })
        .take(MAX_HISTORY)
        .collect();
    (rows, text(v, "ghost"))
}

/// The completions and Tab's text of a `complete` answer.
pub fn read_completions(v: &Value) -> (Vec<Suggestion>, Option<String>) {
    let list = v["completions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            Some(Suggestion {
                label: text(c, "label")?,
                detail: text(c, "detail"),
                icon: text(c, "icon").unwrap_or_else(|| "terminal".to_owned()),
                continues: c.get("continues").and_then(Value::as_bool) == Some(true),
                result: text(c, "result")?,
            })
        })
        .collect();
    (list, text(v, "tab"))
}

/// A shell composer's menu: history and completions over what is typed.
#[derive(Debug, Default)]
pub struct Menu {
    pub open: bool,
    pub history: Vec<Past>,
    pub completions: Vec<Suggestion>,
    /// What Tab fills in with no row picked.
    pub tab: Option<String>,
    /// The past command the ghost shows, for `text`.
    pub ghost: Option<String>,
    /// The picked row, if any.
    pub highlight: Option<usize>,
    /// The text the menu was worked out for.
    pub text: String,
    /// When the text last changed, for the debounce.
    pub changed: Option<Instant>,
    /// Completions were asked for `text`.
    pub completed: bool,
    /// The next text change was put in, so it skips the debounce.
    pub filled: bool,
    /// The text past commands were asked for and not yet answered.
    pub asking: Option<String>,
    /// → or Tab came before the ghost did: it is taken once it comes.
    pub take_ghost: bool,
}

/// A menu row.
pub enum Row<'a> {
    History(&'a Past),
    Completion(&'a Suggestion),
}

impl Menu {
    pub fn rows(&self) -> Vec<Row<'_>> {
        let cap = if self.completions.is_empty() {
            MAX_HISTORY
        } else {
            MAX_HISTORY_WITH_COMPLETIONS
        };
        self.history
            .iter()
            .take(cap)
            .map(Row::History)
            .chain(self.completions.iter().map(Row::Completion))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.rows().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn shown(&self) -> bool {
        self.open && !self.is_empty()
    }

    pub fn close(&mut self) {
        self.open = false;
        self.highlight = None;
    }

    pub fn step(&mut self, by: isize) {
        let n = self.len();
        if n == 0 {
            return;
        }
        let at = self
            .highlight
            .map_or(if by > 0 { -1 } else { 0 }, |h| h as isize);
        self.highlight = Some((at + by).clamp(0, n as isize - 1) as usize);
    }
}

/// What a block's or the composer menu's controls do.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IntentAct {
    /// Copies a block's command and output.
    Copy {
        session: String,
        key: (u32, u64, u64),
    },
    /// Runs or fills in menu row `index`.
    Pick { session: String, index: usize },
}

/// Every shell's menu, and the composers standing aside.
#[derive(Debug, Default)]
pub struct Intent {
    pub menus: HashMap<String, Menu>,
    /// The block whose Copy was last pressed, and when.
    pub copied: Option<((u32, u64, u64), Instant)>,
    /// Shells whose composer stood aside for a running command while focused.
    pub stood_aside: HashSet<String>,
}

/// The ghost `ghost` offers over `typed`: what it adds, when it carries on from it.
pub fn ghost_over<'a>(ghost: Option<&'a str>, typed: &str) -> Option<&'a str> {
    ghost.filter(|g| {
        !typed.is_empty() && !typed.contains('\n') && g.len() > typed.len() && g.starts_with(typed)
    })
}

/// Relative time for a history row: now, m, h, d.
pub fn ago(then_ms: u64, now_ms: u64) -> String {
    let s = now_ms.saturating_sub(then_ms) / 1000;
    match s {
        0..60 => "now".to_owned(),
        60..3600 => format!("{}m", s / 60),
        3600..86_400 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_menu_moves_within_its_rows() {
        let past = Past {
            text: "a".into(),
            timestamp: 0,
        };
        let mut m = Menu {
            history: vec![past.clone(), past],
            ..Menu::default()
        };
        m.step(1);
        assert_eq!(m.highlight, Some(0));
        m.step(5);
        assert_eq!(m.highlight, Some(1));
        m.step(-1);
        assert_eq!(m.highlight, Some(0));
        assert_eq!(ago(0, 90_000), "1m");
    }

    #[test]
    fn answers_read_as_rows() {
        let (rows, ghost) = read_history(&json!({
            "rows": [{"text": "git status", "timestamp": 4}, {"nope": 1}],
            "ghost": "git status",
        }));
        assert_eq!(rows.len(), 1);
        assert_eq!(ghost.as_deref(), Some("git status"));
        let (c, tab) = read_completions(&json!({
            "completions": [{"label": "src/", "icon": "folder", "continues": true, "result": "cd src/"}],
            "tab": "cd src/",
        }));
        assert!(c[0].continues);
        assert_eq!(tab.as_deref(), Some("cd src/"));
        // With completions, history gives up room.
        let m = Menu {
            history: std::iter::repeat_n(rows[0].clone(), 5).collect(),
            completions: c,
            ..Menu::default()
        };
        assert_eq!(m.len(), MAX_HISTORY_WITH_COMPLETIONS + 1);
    }
}
