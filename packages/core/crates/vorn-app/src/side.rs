//! The panes a session owns beside its terminal: which are open, how a
//! card's space is split between them, and which shells each session's
//! terminals panel holds. Kept per device. The Files pane and the cards files
//! pop out into are the Files extension's, which keeps them itself.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Neither side of a split may collapse to nothing (`MIN_SPLIT_RATIO`).
pub const MIN_SPLIT: f32 = 0.15;
pub const MAX_SPLIT: f32 = 0.85;
pub const DEFAULT_SPLIT: f32 = 0.5;

/// A ratio inside the visible range, an even split for
/// anything that is not a number.
pub fn clamp_split(r: f32) -> f32 {
    if r.is_finite() {
        r.clamp(MIN_SPLIT, MAX_SPLIT)
    } else {
        DEFAULT_SPLIT
    }
}

/// Each of `count` panes' share of the column, from a
/// stored list that may be the wrong length, renormalised to sum to 1.
pub fn pane_weights(stored: &[f32], count: usize) -> Vec<f32> {
    if count == 0 {
        return Vec::new();
    }
    let even = 1.0 / count as f32;
    let raw: Vec<f32> = (0..count)
        .map(|i| match stored.get(i) {
            Some(&n) if n.is_finite() && n > 0.0 => n,
            _ => even,
        })
        .collect();
    let total: f32 = raw.iter().sum();
    if total > 0.0 {
        raw.iter().map(|n| n / total).collect()
    } else {
        vec![even; count]
    }
}

/// The divider below pane `index` moved to `ratio` of
/// the whole column; only the two panes it touches change, and neither
/// collapses.
pub fn resize_weights(weights: &[f32], index: usize, ratio: f32) -> Vec<f32> {
    if index + 1 >= weights.len() {
        return weights.to_vec();
    }
    let before: f32 = weights[..index].iter().sum();
    let pair = weights[index] + weights[index + 1];
    if pair <= 0.0 {
        return weights.to_vec();
    }
    let target = if ratio.is_finite() {
        ratio - before
    } else {
        pair / 2.0
    };
    let first = target.clamp(MIN_SPLIT * pair, MAX_SPLIT * pair);
    let mut next = weights.to_vec();
    next[index] = first;
    next[index + 1] = pair - first;
    next
}

/// What the panes, their tabs and the cards do.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SideAct {
    /// Opens or closes the files pane, which the Files extension keeps.
    ToggleFiles(String),
    /// Opened with a first shell, closed with them all.
    ToggleTerminals(String),
    /// Adds a shell to the terminals panel.
    AddTerminal(String),
    /// Maximizes a pane over its owner's card, or restores it.
    Maximize(String),
    Close(SideKind, String),
    PanelTab(String, usize),
    /// A shell out of the panel, as a session of its own.
    Extract(String, String),
    CloseTerminal(String),
    /// A panel shell takes the keyboard.
    FocusTerminal(String),
    /// A card of its own, as a popped-out file's, is chosen.
    SelectCard(String),
}

/// What a session's column can hold, in the order it draws them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SideKind {
    Files,
    Terminals,
}

impl SideKind {
    pub fn key(self) -> &'static str {
        match self {
            SideKind::Files => "files",
            SideKind::Terminals => "terminals",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            SideKind::Files => "Files",
            SideKind::Terminals => "Terminals",
        }
    }
}

/// The id maximize is matched on.
pub fn pane_id(kind: SideKind, owner: &str) -> String {
    format!("{}:{owner}", kind.key())
}

/// The shells a session holds beside its agent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Panel {
    pub terminals: Vec<String>,
    pub active_tab: usize,
}

impl Panel {
    /// The tab in front, clamped to what is there.
    pub fn active(&self) -> usize {
        self.active_tab.min(self.terminals.len().saturating_sub(1))
    }
}

/// How a session card divides its interior.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Split {
    pub terminal: f32,
    pub panes: Vec<f32>,
}

/// What the column keeps across launches.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Saved {
    pub panels: BTreeMap<String, Panel>,
    pub splits: BTreeMap<String, Split>,
    pub maximized: Option<String>,
}

/// Every pane, panel and card, with what is being dragged.
#[derive(Debug, Default)]
pub struct Side {
    pub saved: Saved,
    /// A drag in flight: the session and its weights, or its card split.
    pub drag_panes: Option<(String, Vec<f32>)>,
    pub drag_card: Option<(String, f32)>,
    /// The sessions whose Files pane is open, as the Files extension lists them.
    pub files: HashSet<String>,
    /// A panel shell holding the keyboard.
    pub focus: Option<String>,
    /// Where the view is kept between launches; none keeps it in memory.
    pub file: Option<PathBuf>,
}

impl Side {
    /// The panes read from `file`, or none open when it is missing or
    /// unreadable.
    pub fn load(file: Option<PathBuf>) -> Side {
        let saved = file
            .as_deref()
            .and_then(crate::keep::read)
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Side {
            saved,
            file,
            ..Side::default()
        }
    }

    /// Writes the view down; a failure costs only its survival past this
    /// launch.
    pub fn save(&self) {
        let Some(f) = &self.file else { return };
        if let Ok(s) = serde_json::to_string(&self.saved) {
            let _ = crate::keep::write(f, &s);
        }
    }

    /// What `owner`'s column draws, in order.
    pub fn entries(&self, owner: &str) -> Vec<SideKind> {
        let mut out = Vec::new();
        if self.files.contains(owner) {
            out.push(SideKind::Files);
        }
        if self.saved.panels.contains_key(owner) {
            out.push(SideKind::Terminals);
        }
        out
    }

    /// `owner`'s maximized pane, when it is one of its own.
    pub fn maximized_in(&self, owner: &str) -> Option<SideKind> {
        let m = self.saved.maximized.as_deref()?;
        self.entries(owner)
            .into_iter()
            .find(|k| pane_id(*k, owner) == m)
    }

    /// The terminal's share of `owner`'s card.
    pub fn terminal_ratio(&self, owner: &str) -> f32 {
        if let Some((id, r)) = &self.drag_card {
            if id == owner {
                return *r;
            }
        }
        self.saved
            .splits
            .get(owner)
            .map_or(DEFAULT_SPLIT, |s| clamp_split(s.terminal))
    }

    /// Each pane's share of `owner`'s column.
    pub fn weights(&self, owner: &str) -> Vec<f32> {
        if let Some((id, w)) = &self.drag_panes {
            if id == owner {
                return w.clone();
            }
        }
        let stored = self
            .saved
            .splits
            .get(owner)
            .map(|s| s.panes.as_slice())
            .unwrap_or(&[]);
        pane_weights(stored, self.entries(owner).len())
    }

    pub fn set_terminal_ratio(&mut self, owner: &str, r: f32) {
        let s = self.saved.splits.entry(owner.to_owned()).or_insert(Split {
            terminal: DEFAULT_SPLIT,
            panes: Vec::new(),
        });
        s.terminal = clamp_split(r);
    }

    pub fn set_weights(&mut self, owner: &str, w: Vec<f32>) {
        let s = self.saved.splits.entry(owner.to_owned()).or_insert(Split {
            terminal: DEFAULT_SPLIT,
            panes: Vec::new(),
        });
        s.panes = w;
    }

    /// Every shell some panel holds: drawn there and
    /// nowhere else.
    pub fn claimed(&self) -> HashSet<&str> {
        self.saved
            .panels
            .values()
            .flat_map(|p| p.terminals.iter().map(String::as_str))
            .collect()
    }

    /// The session whose panel holds `terminal`.
    pub fn holder(&self, terminal: &str) -> Option<&str> {
        self.saved
            .panels
            .iter()
            .find(|(_, p)| p.terminals.iter().any(|t| t == terminal))
            .map(|(k, _)| k.as_str())
    }

    /// Shown in front, never listed twice.
    pub fn open_panel(&mut self, owner: &str, terminal: &str) {
        let p = self.saved.panels.entry(owner.to_owned()).or_default();
        if !p.terminals.iter().any(|t| t == terminal) {
            p.terminals.push(terminal.to_owned());
        }
        p.active_tab = p.terminals.iter().position(|t| t == terminal).unwrap_or(0);
    }

    /// The shells it held, to stop.
    pub fn close_panel(&mut self, owner: &str) -> Vec<String> {
        let held = self
            .saved
            .panels
            .remove(owner)
            .map(|p| p.terminals)
            .unwrap_or_default();
        self.clear_placement(&pane_id(SideKind::Terminals, owner));
        held
    }

    /// Brings shell `index` to the front.
    pub fn set_active_panel(&mut self, owner: &str, index: usize) {
        if let Some(p) = self.saved.panels.get_mut(owner) {
            if index < p.terminals.len() {
                p.active_tab = index;
            }
        }
    }

    /// The claim on `terminal` dropped; a panel left
    /// empty closes.
    pub fn release(&mut self, terminal: &str) -> bool {
        let mut emptied = Vec::new();
        let mut released = false;
        for (owner, p) in &mut self.saved.panels {
            if let Some(i) = p.terminals.iter().position(|t| t == terminal) {
                p.terminals.remove(i);
                released = true;
                if p.terminals.is_empty() {
                    emptied.push(owner.clone());
                }
            }
        }
        for owner in emptied {
            self.saved.panels.remove(&owner);
            self.clear_placement(&pane_id(SideKind::Terminals, &owner));
        }
        if self.focus.as_deref() == Some(terminal) {
            self.focus = None;
        }
        released
    }

    fn clear_placement(&mut self, pane: &str) {
        if self.saved.maximized.as_deref() == Some(pane) {
            self.saved.maximized = None;
        }
    }

    /// Drops what belongs to sessions that no longer exist, as the store
    /// prunes on load; true when anything went.
    pub fn prune(&mut self, live: &HashSet<&str>) -> bool {
        let before = (self.saved.panels.len(), self.saved.splits.len());
        self.saved.panels.retain(|k, _| live.contains(k.as_str()));
        for p in self.saved.panels.values_mut() {
            p.terminals.retain(|t| live.contains(t.as_str()));
        }
        self.saved.panels.retain(|_, p| !p.terminals.is_empty());
        self.saved.splits.retain(|k, _| live.contains(k.as_str()));
        let after = (self.saved.panels.len(), self.saved.splits.len());
        before != after
    }
}

/// Where the desktop app keeps its own view of `data_dir`'s vornd.
pub fn view_file(data_dir: &Path) -> PathBuf {
    data_dir.join("desktop").join("view.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn weights_pad_trim_and_renormalise() {
        assert!(close(&pane_weights(&[], 2), &[0.5, 0.5]));
        assert!(close(&pane_weights(&[0.25, 0.75, 9.0], 2), &[0.25, 0.75]));
        assert!(close(
            &pane_weights(&[f32::NAN, 1.0], 2),
            &[0.5 / 1.5, 1.0 / 1.5]
        ));
        assert!(pane_weights(&[1.0], 0).is_empty());
        assert_eq!(clamp_split(f32::NAN), DEFAULT_SPLIT);
        assert_eq!(clamp_split(0.99), MAX_SPLIT);
    }

    #[test]
    fn a_divider_moves_only_its_pair_and_neither_collapses() {
        let w = [1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0];
        let r = resize_weights(&w, 1, 0.5);
        assert!((r[0] - w[0]).abs() < 1e-6);
        assert!((r[1] - (0.5 - w[0])).abs() < 1e-5);
        assert!((r.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        let squeezed = resize_weights(&w, 0, 0.0);
        assert!((squeezed[0] - MIN_SPLIT * (2.0 / 3.0)).abs() < 1e-5);
        assert_eq!(resize_weights(&w, 2, 0.5), w.to_vec());
    }

    #[test]
    fn panels_claim_shells_and_close_when_their_last_one_leaves() {
        let mut p = Side::default();
        p.open_panel("agent", "sh1");
        p.open_panel("agent", "sh2");
        p.open_panel("agent", "sh1");
        assert_eq!(p.saved.panels["agent"].terminals, ["sh1", "sh2"]);
        assert_eq!(p.saved.panels["agent"].active(), 0);
        assert_eq!(p.holder("sh2"), Some("agent"));
        p.saved.maximized = Some(pane_id(SideKind::Terminals, "agent"));
        assert!(p.release("sh1"));
        assert!(p.release("sh2"));
        assert!(!p.saved.panels.contains_key("agent"));
        assert_eq!(p.saved.maximized, None);
        assert!(!p.release("sh2"));
    }

    #[test]
    fn pruning_drops_what_dead_sessions_held() {
        let mut p = Side::default();
        p.open_panel("live", "gone");
        p.set_terminal_ratio("dead", 0.3);
        let live: HashSet<&str> = ["live"].into();
        assert!(p.prune(&live));
        assert!(p.saved.panels.is_empty() && p.saved.splits.is_empty());
        assert!(!p.prune(&live));
    }

    #[test]
    fn the_view_round_trips_through_its_file() {
        let dir = std::env::temp_dir().join(format!("vorn-panes-{}", std::process::id()));
        let file = view_file(&dir);
        let mut p = Side::load(Some(file.clone()));
        p.open_panel("s", "sh");
        p.set_terminal_ratio("s", 0.6);
        p.save();
        let back = Side::load(Some(file));
        assert_eq!(back.saved, p.saved);
        let _ = std::fs::remove_dir_all(dir);
    }
}
