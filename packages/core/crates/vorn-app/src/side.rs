//! The panes a session owns beside its terminal (`PaneColumn`), and the
//! files popped out of them into cards of their own: which are open, how
//! a card's space is split between them, and which shells each session's
//! terminals panel holds. Kept per device, as today's app keeps them.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Neither side of a split may collapse to nothing (`MIN_SPLIT_RATIO`).
pub const MIN_SPLIT: f32 = 0.15;
pub const MAX_SPLIT: f32 = 0.85;
pub const DEFAULT_SPLIT: f32 = 0.5;
/// The file tree's share of the Files pane (`DEFAULT_TREE_RATIO`).
pub const DEFAULT_TREE: f32 = 0.38;
/// Below this width the tree and the file take turns (`SIDE_BY_SIDE_MIN_PX`).
pub const SIDE_BY_SIDE_MIN: f32 = 480.0;

/// `clampSplitRatio`: a ratio inside the visible range, an even split for
/// anything that is not a number.
pub fn clamp_split(r: f32) -> f32 {
    if r.is_finite() {
        r.clamp(MIN_SPLIT, MAX_SPLIT)
    } else {
        DEFAULT_SPLIT
    }
}

/// `splitPaneWeights`: each of `count` panes' share of the column, from a
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

/// `resizePaneWeights`: the divider below pane `index` moved to `ratio` of
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

/// What the panes, their tabs, the file tree and the cards do.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SideAct {
    /// `toggleFilesPane`.
    ToggleFiles(String),
    /// `toggleTerminalsPanel`: opened with a first shell, closed with them all.
    ToggleTerminals(String),
    /// `addTerminalToPanel`.
    AddTerminal(String),
    /// Maximizes a pane over its owner's card, or restores it.
    Maximize(String),
    Close(SideKind, String),
    Tab(String, String),
    PinTab(String, String),
    CloseTab(String, String),
    CloseSavedTabs(String),
    /// The overflow list of a Files pane's tabs.
    TabsMenu(String),
    ToggleTree(String),
    /// A directory row: open or shut it.
    Dir(String, String),
    /// A file row: preview it; a double click pins it.
    File(String, String),
    PinFile(String, String),
    /// A file into a card of its own.
    PopOut(String, String),
    /// The front file of a Files pane into a card of its own.
    PopOutActive(String),
    PanelTab(String, usize),
    /// A shell out of the panel, as a session of its own.
    Extract(String, String),
    CloseTerminal(String),
    /// A panel shell takes the keyboard.
    FocusTerminal(String),
    ReturnCard(String),
    CloseCard(String),
    SelectCard(String),
    ClearFilter(String),
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

/// `paneIdFor`: the id maximize is matched on.
pub fn pane_id(kind: SideKind, owner: &str) -> String {
    format!("{}:{owner}", kind.key())
}

/// `fileTabKey`: an open file tab's buffer and draft key.
pub fn tab_key(owner: &str, path: &str) -> String {
    format!("files:{owner}:{path}")
}

/// A session's Files pane (`FilesPaneState`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesPane {
    pub tabs: Vec<String>,
    pub active: Option<String>,
    /// The tab a single click opened, replaced by the next one until it is
    /// pinned or typed in.
    pub preview: Option<String>,
    pub tree_visible: bool,
}

impl Default for FilesPane {
    fn default() -> FilesPane {
        FilesPane {
            tabs: Vec::new(),
            active: None,
            preview: None,
            tree_visible: true,
        }
    }
}

/// The shells a session holds beside its agent (`TerminalsPaneState`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Panel {
    pub terminals: Vec<String>,
    pub active_tab: usize,
}

impl Panel {
    /// `activePanelIndex`: the tab in front, clamped to what is there.
    pub fn active(&self) -> usize {
        self.active_tab.min(self.terminals.len().saturating_sub(1))
    }
}

/// A file popped out of a session into a card of its own (`EditorPaneState`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub session_id: String,
    pub file_path: String,
}

/// How a session card divides its interior (`CardSplit`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Split {
    pub terminal: f32,
    pub panes: Vec<f32>,
}

/// What the column, the Files panes and the cards keep across launches.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Saved {
    pub files: BTreeMap<String, FilesPane>,
    pub panels: BTreeMap<String, Panel>,
    pub cards: BTreeMap<String, Card>,
    pub splits: BTreeMap<String, Split>,
    pub maximized: Option<String>,
    pub tree_ratio: Option<f32>,
    pub next_card: u32,
}

/// One entry of a directory (`FileEntry`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub dir: bool,
}

impl Entry {
    pub fn from_value(v: &serde_json::Value) -> Option<Entry> {
        Some(Entry {
            name: v.get("name")?.as_str()?.to_owned(),
            path: v.get("path")?.as_str()?.to_owned(),
            dir: v
                .get("isDirectory")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        })
    }
}

/// A Files pane's tree (`FileTreePane`): what has been listed, what is
/// open, and the filter.
#[derive(Debug, Clone, Default)]
pub struct Tree {
    /// The directory listed at the top.
    pub cwd: String,
    pub root: Option<Vec<Entry>>,
    pub loading: bool,
    pub dirs: HashMap<String, Vec<Entry>>,
    pub expanded: HashSet<String>,
    /// Directories being listed.
    pub pending: HashSet<String>,
    pub filter: String,
}

/// One drawn row of a tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeRow {
    Dir {
        entry: Entry,
        depth: usize,
        open: bool,
        loading: bool,
    },
    File {
        entry: Entry,
        depth: usize,
    },
    /// An open directory with nothing in it.
    Empty {
        depth: usize,
    },
}

impl Tree {
    /// `computeFilterSets`: entries that match or hold a match, and the
    /// directories opened to show them; only what has been listed counts.
    pub fn filter_sets(&self) -> (HashSet<String>, HashSet<String>) {
        let (mut matched, mut expand) = (HashSet::new(), HashSet::new());
        if self.filter.is_empty() {
            return (matched, expand);
        }
        let lc = self.filter.to_lowercase();
        fn visit(
            t: &Tree,
            e: &Entry,
            lc: &str,
            matched: &mut HashSet<String>,
            expand: &mut HashSet<String>,
        ) -> bool {
            let own = e.name.to_lowercase().contains(lc);
            if !e.dir {
                if own {
                    matched.insert(e.path.clone());
                }
                return own;
            }
            let mut below = false;
            for c in t.dirs.get(&e.path).into_iter().flatten() {
                below |= visit(t, c, lc, matched, expand);
            }
            if below {
                expand.insert(e.path.clone());
            }
            if own || below {
                matched.insert(e.path.clone());
            }
            own || below
        }
        for e in self.root.iter().flatten() {
            visit(self, e, &lc, &mut matched, &mut expand);
        }
        (matched, expand)
    }

    /// The rows as `TreeNode` draws them, depth first.
    pub fn rows(&self) -> Vec<TreeRow> {
        let (matched, expand) = self.filter_sets();
        let filtering = !self.filter.is_empty();
        let mut out = Vec::new();
        fn walk(
            t: &Tree,
            entries: &[Entry],
            depth: usize,
            filter: Option<(&HashSet<String>, &HashSet<String>)>,
            out: &mut Vec<TreeRow>,
        ) {
            for e in entries {
                if filter.is_some_and(|(m, _)| !m.contains(&e.path)) {
                    continue;
                }
                if !e.dir {
                    out.push(TreeRow::File {
                        entry: e.clone(),
                        depth,
                    });
                    continue;
                }
                let open = t.expanded.contains(&e.path)
                    || filter.is_some_and(|(_, x)| x.contains(&e.path));
                out.push(TreeRow::Dir {
                    entry: e.clone(),
                    depth,
                    open,
                    loading: t.pending.contains(&e.path),
                });
                if !open {
                    continue;
                }
                if let Some(children) = t.dirs.get(&e.path) {
                    if children.is_empty() {
                        out.push(TreeRow::Empty { depth: depth + 1 });
                    }
                    walk(t, children, depth + 1, filter, out);
                }
            }
        }
        let filter = filtering.then_some((&matched, &expand));
        walk(
            self,
            self.root.as_deref().unwrap_or(&[]),
            0,
            filter,
            &mut out,
        );
        out
    }

    /// Whether the filter matched nothing listed.
    pub fn no_match(&self) -> bool {
        !self.filter.is_empty() && self.filter_sets().0.is_empty()
    }
}

/// Every pane, panel and card, with what is being dragged.
#[derive(Debug, Default)]
pub struct Side {
    pub saved: Saved,
    /// A drag in flight: the session and its weights, or its card split.
    pub drag_panes: Option<(String, Vec<f32>)>,
    pub drag_card: Option<(String, f32)>,
    pub drag_tree: Option<f32>,
    /// The trees, by owning session.
    pub trees: HashMap<String, Tree>,
    /// A panel shell holding the keyboard (`focusTarget`).
    pub focus: Option<String>,
    /// Where the view is kept between launches; none keeps it in memory.
    pub file: Option<PathBuf>,
    /// Files panes too narrow to show tree and file side by side.
    pub narrow: HashSet<String>,
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

    pub fn tree_ratio(&self) -> f32 {
        self.drag_tree
            .or(self.saved.tree_ratio)
            .map_or(DEFAULT_TREE, clamp_split)
    }

    /// `usePaneColumnEntries`: what `owner`'s column draws, in order.
    pub fn entries(&self, owner: &str) -> Vec<SideKind> {
        let mut out = Vec::new();
        if self.saved.files.contains_key(owner) {
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

    /// Every shell some panel holds (`claimedTerminalIds`): drawn there and
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

    /// `openFilesPane`.
    pub fn open_files(&mut self, owner: &str) {
        self.saved.files.entry(owner.to_owned()).or_default();
    }

    /// `closeFilesPane`: the tab keys whose buffers go with it.
    pub fn close_files(&mut self, owner: &str) -> Vec<String> {
        let tabs = self
            .saved
            .files
            .remove(owner)
            .map(|p| p.tabs)
            .unwrap_or_default();
        self.trees.remove(owner);
        self.clear_placement(&pane_id(SideKind::Files, owner));
        tabs.iter().map(|t| tab_key(owner, t)).collect()
    }

    /// `openFileTab`: a single click previews, replacing the last preview
    /// unless that one has edits; `pin` keeps the tab.
    pub fn open_tab(&mut self, owner: &str, path: &str, pin: bool, preview_dirty: bool) {
        let pane = self.saved.files.entry(owner.to_owned()).or_default();
        if pane.tabs.iter().any(|t| t == path) {
            if pin && pane.preview.as_deref() == Some(path) {
                pane.preview = None;
            }
        } else if pin {
            pane.tabs.push(path.to_owned());
        } else {
            match pane.preview.clone().filter(|_| !preview_dirty) {
                Some(old) => {
                    for t in &mut pane.tabs {
                        if *t == old {
                            *t = path.to_owned();
                        }
                    }
                }
                None => pane.tabs.push(path.to_owned()),
            }
            pane.preview = Some(path.to_owned());
        }
        pane.active = Some(path.to_owned());
    }

    /// `pinFileTab`.
    pub fn pin_tab(&mut self, owner: &str, path: &str) {
        if let Some(p) = self.saved.files.get_mut(owner) {
            if p.preview.as_deref() == Some(path) {
                p.preview = None;
            }
        }
    }

    /// `setActiveFileTab`.
    pub fn set_active_tab(&mut self, owner: &str, path: &str) {
        if let Some(p) = self.saved.files.get_mut(owner) {
            if p.tabs.iter().any(|t| t == path) {
                p.active = Some(path.to_owned());
            }
        }
    }

    /// `closeFileTab`: the next tab along takes the front.
    pub fn close_tab(&mut self, owner: &str, path: &str) {
        let Some(p) = self.saved.files.get_mut(owner) else {
            return;
        };
        let Some(i) = p.tabs.iter().position(|t| t == path) else {
            return;
        };
        p.tabs.remove(i);
        if p.active.as_deref() == Some(path) {
            p.active = p
                .tabs
                .get(i)
                .or_else(|| i.checked_sub(1).and_then(|j| p.tabs.get(j)))
                .cloned();
        }
        if p.preview.as_deref() == Some(path) {
            p.preview = None;
        }
    }

    /// `closeSavedFileTabs`: only tabs with edits stay.
    pub fn close_saved_tabs(&mut self, owner: &str, dirty: impl Fn(&str) -> bool) {
        let Some(p) = self.saved.files.get_mut(owner) else {
            return;
        };
        p.tabs.retain(|t| dirty(t));
        if p.active.as_ref().is_some_and(|a| !p.tabs.contains(a)) {
            p.active = p.tabs.first().cloned();
        }
        if p.preview.as_ref().is_some_and(|a| !p.tabs.contains(a)) {
            p.preview = None;
        }
    }

    /// `toggleFileTree`.
    pub fn toggle_tree(&mut self, owner: &str) {
        if let Some(p) = self.saved.files.get_mut(owner) {
            p.tree_visible = !p.tree_visible;
        }
    }

    pub fn set_tree_visible(&mut self, owner: &str, visible: bool) {
        if let Some(p) = self.saved.files.get_mut(owner) {
            p.tree_visible = visible;
        }
    }

    /// `openTerminalsPane`: shown in front, never listed twice.
    pub fn open_panel(&mut self, owner: &str, terminal: &str) {
        let p = self.saved.panels.entry(owner.to_owned()).or_default();
        if !p.terminals.iter().any(|t| t == terminal) {
            p.terminals.push(terminal.to_owned());
        }
        p.active_tab = p.terminals.iter().position(|t| t == terminal).unwrap_or(0);
    }

    /// `closeTerminalsPane`: the shells it held, to stop.
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

    /// `setActivePanelTerminal`.
    pub fn set_active_panel(&mut self, owner: &str, index: usize) {
        if let Some(p) = self.saved.panels.get_mut(owner) {
            if index < p.terminals.len() {
                p.active_tab = index;
            }
        }
    }

    /// `releaseFromPanels`: the claim on `terminal` dropped; a panel left
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

    /// `promoteFile`: one card per file per session; an existing one is
    /// returned rather than a second made.
    pub fn promote(&mut self, owner: &str, path: &str) -> String {
        if let Some((id, _)) = self
            .saved
            .cards
            .iter()
            .find(|(_, c)| c.session_id == owner && c.file_path == path)
        {
            return id.clone();
        }
        self.saved.next_card += 1;
        let id = format!("card:{owner}:{}", self.saved.next_card);
        self.saved.cards.insert(
            id.clone(),
            Card {
                session_id: owner.to_owned(),
                file_path: path.to_owned(),
            },
        );
        id
    }

    /// The cards `owner` popped out, in the order they were made.
    pub fn cards_of(&self, owner: &str) -> Vec<&str> {
        let mut ids: Vec<(&str, u32)> = self
            .saved
            .cards
            .iter()
            .filter(|(_, c)| c.session_id == owner)
            .map(|(id, _)| {
                (
                    id.as_str(),
                    id.rsplit(':')
                        .next()
                        .and_then(|n| n.parse().ok())
                        .unwrap_or(0),
                )
            })
            .collect();
        ids.sort_by_key(|(_, n)| *n);
        ids.into_iter().map(|(id, _)| id).collect()
    }

    fn clear_placement(&mut self, pane: &str) {
        if self.saved.maximized.as_deref() == Some(pane) {
            self.saved.maximized = None;
        }
    }

    /// Drops what belongs to sessions that no longer exist, as the store
    /// prunes on load; true when anything went.
    pub fn prune(&mut self, live: &HashSet<&str>) -> bool {
        let before = (
            self.saved.files.len(),
            self.saved.panels.len(),
            self.saved.cards.len(),
            self.saved.splits.len(),
        );
        self.saved.files.retain(|k, _| live.contains(k.as_str()));
        self.saved.panels.retain(|k, _| live.contains(k.as_str()));
        for p in self.saved.panels.values_mut() {
            p.terminals.retain(|t| live.contains(t.as_str()));
        }
        self.saved.panels.retain(|_, p| !p.terminals.is_empty());
        self.saved
            .cards
            .retain(|_, c| live.contains(c.session_id.as_str()));
        self.saved.splits.retain(|k, _| live.contains(k.as_str()));
        self.trees.retain(|k, _| live.contains(k.as_str()));
        let after = (
            self.saved.files.len(),
            self.saved.panels.len(),
            self.saved.cards.len(),
            self.saved.splits.len(),
        );
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
    fn a_preview_tab_is_replaced_until_pinned_or_edited() {
        let mut p = Side::default();
        p.open_tab("s", "/a", false, false);
        p.open_tab("s", "/b", false, false);
        assert_eq!(p.saved.files["s"].tabs, ["/b"]);
        p.open_tab("s", "/c", false, true);
        assert_eq!(p.saved.files["s"].tabs, ["/b", "/c"]);
        p.pin_tab("s", "/c");
        p.open_tab("s", "/d", true, false);
        assert_eq!(p.saved.files["s"].tabs, ["/b", "/c", "/d"]);
        assert_eq!(p.saved.files["s"].active.as_deref(), Some("/d"));
        p.close_tab("s", "/d");
        assert_eq!(p.saved.files["s"].active.as_deref(), Some("/c"));
        p.close_saved_tabs("s", |t| t == "/b");
        assert_eq!(p.saved.files["s"].tabs, ["/b"]);
        assert_eq!(p.saved.files["s"].active.as_deref(), Some("/b"));
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
    fn a_file_has_one_card_per_session() {
        let mut p = Side::default();
        let a = p.promote("s", "/x");
        assert_eq!(p.promote("s", "/x"), a);
        let b = p.promote("s", "/y");
        assert_ne!(a, b);
        assert_eq!(p.cards_of("s"), [a.as_str(), b.as_str()]);
        assert_ne!(p.promote("t", "/x"), a);
    }

    #[test]
    fn pruning_drops_what_dead_sessions_held() {
        let mut p = Side::default();
        p.open_files("live");
        p.open_files("dead");
        p.open_panel("live", "gone");
        p.promote("dead", "/x");
        let live: HashSet<&str> = ["live"].into();
        assert!(p.prune(&live));
        assert_eq!(p.saved.files.keys().collect::<Vec<_>>(), ["live"]);
        assert!(p.saved.panels.is_empty() && p.saved.cards.is_empty());
        assert!(!p.prune(&live));
    }

    fn e(name: &str, path: &str, dir: bool) -> Entry {
        Entry {
            name: name.into(),
            path: path.into(),
            dir,
        }
    }

    #[test]
    fn the_filter_opens_directories_down_to_what_matches() {
        let mut t = Tree {
            root: Some(vec![
                e("src", "/src", true),
                e("README.md", "/README.md", false),
            ]),
            ..Tree::default()
        };
        t.dirs.insert(
            "/src".into(),
            vec![
                e("app.ts", "/src/app.ts", false),
                e("lib", "/src/lib", true),
            ],
        );
        t.dirs.insert("/src/lib".into(), Vec::new());
        assert_eq!(t.rows().len(), 2);
        t.expanded.insert("/src".into());
        t.expanded.insert("/src/lib".into());
        let rows = t.rows();
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[3], TreeRow::Empty { depth: 2 });
        t.filter = "APP".into();
        t.expanded.clear();
        let rows = t.rows();
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(matches!(&rows[0], TreeRow::Dir { open: true, .. }));
        t.filter = "zzz".into();
        assert!(t.rows().is_empty() && t.no_match());
    }

    #[test]
    fn the_view_round_trips_through_its_file() {
        let dir = std::env::temp_dir().join(format!("vorn-panes-{}", std::process::id()));
        let file = view_file(&dir);
        let mut p = Side::load(Some(file.clone()));
        p.open_tab("s", "/a", true, false);
        p.open_panel("s", "sh");
        p.set_terminal_ratio("s", 0.6);
        p.save();
        let back = Side::load(Some(file));
        assert_eq!(back.saved, p.saved);
        let _ = std::fs::remove_dir_all(dir);
    }
}
