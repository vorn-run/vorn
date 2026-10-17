//! The Flexible grid's tree kept in step with the sessions, its dividers
//! dragged and nudged, and each workspace's grid saved as `gridLayout`.

use serde_json::{json, Value};

use super::{App, Call, Shortcut};
use crate::flex::{self, Dir, Grid, Node, Rect};
use crate::state::LayoutMode;
use crate::ui::{Id, Ui};

/// The smallest a flexible pane may be dragged to, in logical pixels.
pub const MIN_PANE: flex::Min = (240.0, 160.0);

/// How far a keyboard nudge moves a divider, of its split.
const NUDGE: f32 = 0.05;

impl App {
    /// The grid's box as last laid out, in logical pixels; the window
    /// before the first frame.
    fn flex_area(&self, ui: &Ui) -> Rect {
        match ui.authored_rect(crate::screen::grid_id()) {
            Some(r) if r.w > 0.0 && r.h > 0.0 => Rect::new(r.x, r.y, r.w, r.h),
            _ => Rect::new(0.0, 0.0, 1280.0, 800.0),
        }
    }

    /// The active workspace's grid, made when it has none.
    fn grid_mut(&mut self) -> &mut Grid {
        let ws = self.store.workspace().to_owned();
        self.st.grids.entry(ws).or_default()
    }

    fn flex_tree(&self) -> Option<&Node> {
        self.st.grids.get(self.store.workspace())?.tree.as_ref()
    }

    /// The columns the active workspace was left in.
    pub(super) fn load_columns(&mut self) {
        self.st.columns = self
            .st
            .grids
            .get(self.store.workspace())
            .map_or(0, Grid::mode);
    }

    /// The workspace's columns set, the tree built from the cards as they
    /// stand when it turns flexible with none.
    pub(super) fn set_columns(&mut self, ui: &Ui, n: i32) {
        self.st.columns = n;
        let area = self.flex_area(ui);
        let ids: Vec<String> = self.flex_shown().into_iter().map(|(id, _)| id).collect();
        let grid = self.grid_mut();
        grid.flexible = n == -1;
        if n >= 0 {
            grid.columns = n;
        }
        if grid.flexible && grid.tree.is_none() {
            let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
            let cols = match grid.columns {
                c if c > 0 => c as usize,
                _ => crate::layout::pick(refs.len(), area.w, area.h).cols,
            };
            grid.tree = Node::from_grid(&refs, cols);
        }
        self.save_grids();
    }

    /// The cards the tree holds: every session of the workspace not in a
    /// panel, each followed by the cards it popped out (with its id).
    fn flex_members(&self) -> Vec<(String, Option<String>)> {
        let mut out = Vec::new();
        for s in self.store.scoped_sessions() {
            let id = &s.info.id;
            if self.st.side.holder(id).is_some() {
                continue;
            }
            out.push((id.clone(), None));
            for card in self.st.side.cards_of(id) {
                out.push((card.to_owned(), Some(id.clone())));
            }
        }
        out
    }

    /// The cards the grid shows now, in its order: filtered and minimized
    /// ones are hidden, not taken out of the tree.
    fn flex_shown(&self) -> Vec<(String, Option<String>)> {
        let mut out = Vec::new();
        for i in self.st.gridded(&self.store) {
            let id = &self.store.sessions[i].info.id;
            out.push((id.clone(), None));
            for card in self.st.side.cards_of(id) {
                out.push((card.to_owned(), Some(id.clone())));
            }
        }
        out
    }

    /// Keeps the tree in step: a closed session's pane gives its room to
    /// its sibling, a new one splits the pane selected before it (a popped
    /// out card its session's).
    pub(super) fn flex_tick(&mut self, ui: &Ui) {
        if self.st.columns != -1 || !self.store.sessions_known {
            return;
        }
        let members = self.flex_members();
        if let Some(sel) = &self.st.selected {
            if self.flex_tree().is_some_and(|t| t.contains(sel)) {
                self.st.flex_anchor = Some(sel.clone());
            }
        }
        let tree = self.flex_tree();
        let in_step = tree.map_or(members.is_empty(), |t| {
            let leaves = t.leaves();
            leaves.len() == members.len() && members.iter().all(|(m, _)| leaves.contains(&&**m))
        });
        let beside = std::mem::take(&mut self.st.flex_beside);
        if in_step {
            return;
        }
        let area = self.flex_area(ui);
        let anchor = self.st.flex_anchor.clone();
        let grid = self.grid_mut();
        let mut tree = grid
            .tree
            .take()
            .and_then(|t| t.retain(&|id| members.iter().any(|(m, _)| m == id)));
        if let Some(t) = tree.take_if(|_| beside) {
            let held = t.leaves().len();
            let rest: Vec<Node> = members
                .iter()
                .filter(|(m, _)| !t.contains(m))
                .map(|(m, _)| Node::leaf(m))
                .collect();
            let share = held as f32 / (held + rest.len()) as f32;
            tree = Some(match flex::chain(Dir::Row, rest) {
                Some(rest) => Node::split(Dir::Row, share, t, rest),
                None => t,
            });
        }
        for (id, owner) in &members {
            match &mut tree {
                Some(t) if t.contains(id) => {}
                Some(t) => t.insert(id, owner.as_deref().or(anchor.as_deref()), area),
                None => tree = Some(Node::leaf(id)),
            }
        }
        grid.tree = tree;
        self.save_grids();
    }

    /// Writes every workspace's grid to this viewer's settings.
    pub(super) fn save_grids(&mut self) {
        let mut config = self.store.config.clone();
        if !config.get("defaults").is_some_and(Value::is_object) {
            config["defaults"] = json!({});
        }
        config["defaults"]["gridLayout"] = flex::grids_to_json(&self.st.grids);
        self.save_config(config, Call::Quiet);
    }

    /// A flexible divider dragged: the tree follows it, saved once it is let go.
    pub(super) fn flex_resized(&mut self, ui: &Ui, id: Id, ratio: f32, end: bool) -> bool {
        let Some(path) = self.flex_tree().and_then(|t| {
            t.splits()
                .into_iter()
                .find(|p| crate::screen::flex_divider_id(*p) == id)
        }) else {
            return false;
        };
        if end {
            // A press with no drag reports the split's default; nothing moved.
            if self.st.flex_drag.take() == Some(path) {
                self.save_grids();
            }
            return true;
        }
        let area = self.flex_area(ui);
        if let Some(tree) = self.grid_mut().tree.as_mut() {
            tree.resize(path, ratio, area, MIN_PANE);
        }
        self.st.flex_drag = Some(path);
        true
    }

    /// Every split back to equal panes.
    pub(super) fn flex_equalize(&mut self) {
        if let Some(tree) = self.grid_mut().tree.as_mut() {
            tree.equalize();
            self.save_grids();
        }
    }

    /// The pane keys: move to the pane beside, or move the divider nearest it.
    pub(super) fn flex_key(&mut self, ui: &Ui, s: Shortcut) {
        if self.st.columns != -1 || self.st.focused.is_some() {
            return;
        }
        let area = self.flex_area(ui);
        match s {
            Shortcut::PaneFocus(side) => {
                let shown = self.flex_shown();
                let Some(tree) = self
                    .flex_tree()
                    .cloned()
                    .and_then(|t| t.retain(&|id| shown.iter().any(|(s, _)| s == id)))
                else {
                    return;
                };
                let rects = tree.rects(area);
                let next = match self.st.selected.as_deref() {
                    Some(from) => flex::neighbour(&rects, from, side),
                    None => rects.first().map(|(id, _)| *id),
                };
                if let Some(next) = next.map(str::to_owned) {
                    self.go_to(next);
                }
            }
            Shortcut::PaneResize(side) => {
                let Some(from) = self.st.selected.clone() else {
                    return;
                };
                let moved = self
                    .grid_mut()
                    .tree
                    .as_mut()
                    .is_some_and(|t| t.nudge(&from, side, NUDGE, area, MIN_PANE));
                if moved {
                    self.save_grids();
                }
            }
            Shortcut::PaneEqualize => self.flex_equalize(),
            _ => {}
        }
    }

    /// A layout opened by vornd (`layout:opened`): its view and tree for
    /// a workspace, taken as this screen's own.
    pub(super) fn layout_opened(&mut self, params: &Value) {
        let Some(ws) = params.get("workspace").and_then(Value::as_str) else {
            return;
        };
        let view = params.get("view").and_then(Value::as_str);
        let mut set_tree = false;
        let grid = self.st.grids.entry(ws.to_owned()).or_default();
        match view {
            Some("flexible") => {
                grid.flexible = true;
                if let Some(tree) = params.get("tree").and_then(Node::from_json) {
                    grid.tree = Some(tree);
                    set_tree = true;
                }
            }
            Some("grid") => {
                grid.flexible = false;
                if let Some(n) = params.get("columns").and_then(Value::as_i64) {
                    grid.columns = i32::try_from(n).unwrap_or(0).max(0);
                }
            }
            _ => {}
        }
        let mode = match view {
            Some("tabs") => Some(LayoutMode::Tabs),
            Some("grid" | "flexible") => Some(LayoutMode::Grid),
            _ => None,
        };
        if ws == self.store.workspace() {
            self.st.flex_beside = set_tree;
            self.load_columns();
            if let Some(mode) = mode {
                self.st.layout = mode;
            }
        }
        // Saved again so this screen's next save does not put the old view back.
        let mut config = self.store.config.clone();
        if !config.get("defaults").is_some_and(Value::is_object) {
            config["defaults"] = json!({});
        }
        config["defaults"]["gridLayout"] = flex::grids_to_json(&self.st.grids);
        if let Some(mode) = mode {
            config["defaults"]["layoutMode"] = json!(mode.key());
        }
        self.save_config(config, Call::Quiet);
    }
}
