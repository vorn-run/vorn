//! The toolbar's own items at work: + starting a session at once, the
//! splits, an extension's item and its popover, and the arrangements the
//! customize sheet drags and switches, written to `desktop/arrange.json`.

use serde_json::{json, Value};

use super::{App, Call, Src};
use crate::act::Act;
use crate::arrange::List;
use crate::flex::Dir;
use crate::points::{kind, scoped, PointAct};
use crate::screen::MainView;
use crate::state::{LayoutMode, MenuKind};
use crate::ui::{Event as UiEvent, Ui};

/// The scope a toolbar popover draws for with no project chosen.
pub const EVERYWHERE: &str = "*";

/// Where new things start from the focused card: its project and the
/// worktree it runs in, when it runs in one.
struct Place {
    project: String,
    path: String,
    worktree: Option<String>,
}

impl App {
    /// The focused card's place, else the sidebar's project, else none.
    fn start_place(&self) -> Option<Place> {
        let card = self
            .current()
            .and_then(|id| self.store.session(&id))
            .filter(|s| !s.info.project_path.is_empty());
        if let Some(s) = card {
            let worktree = s
                .info
                .worktree_path
                .clone()
                .filter(|w| s.info.is_worktree == Some(true) && !w.is_empty());
            return Some(Place {
                project: s.info.project_name.clone(),
                path: s.info.project_path.clone(),
                worktree,
            });
        }
        self.active_project().map(|(project, path)| Place {
            project,
            path,
            worktree: None,
        })
    }

    /// What + starts: the agent General names, or a shell.
    pub(super) fn new_session_starts(&self) -> String {
        let pick = self
            .store
            .config
            .pointer(&format!("/defaults/{}", super::NEW_SESSION_STARTS))
            .and_then(Value::as_str);
        match pick {
            Some("shell") => "shell".to_owned(),
            _ => self.store.default_agent().to_owned(),
        }
    }

    /// A session of `agent` at once where the focused card is; the composer
    /// when there is no project to start in.
    pub(super) fn start_with(&mut self, ui: &mut Ui, agent: String) {
        let Some(p) = self.start_place() else {
            self.run(ui, Act::NewSession, Src::default());
            return;
        };
        let mut params = json!({
            "agentType": agent,
            "projectName": p.project,
            "projectPath": p.path,
        });
        if let Some(w) = p.worktree {
            params["existingWorktreePath"] = json!(w);
        }
        self.st.view = MainView::Sessions;
        self.create_session(Call::Created, params);
    }

    /// A shell in `session`'s folder (the selected card's when none), beside
    /// it or under it on the Flexible grid, which Grid turns into first.
    pub(super) fn split(&mut self, ui: &mut Ui, session: Option<String>, down: bool) {
        let Some(id) = session.or_else(|| self.current()) else {
            self.st.toast("Select a card to split", true);
            return;
        };
        let Some(s) = self.store.session(&id) else {
            return;
        };
        let info = &s.info;
        let cwd = info
            .shell_cwd
            .clone()
            .filter(|c| s.is_shell() && !c.is_empty())
            .or_else(|| {
                info.worktree_path
                    .clone()
                    .filter(|w| info.is_worktree == Some(true) && !w.is_empty())
            })
            .unwrap_or_else(|| info.project_path.clone());
        self.st.view = MainView::Sessions;
        self.st.focused = None;
        if self.st.layout != LayoutMode::Grid {
            self.run(ui, Act::Layout(LayoutMode::Grid), Src::default());
        }
        if self.st.columns != -1 {
            self.set_columns(ui, -1);
        }
        let dir = if down { Dir::Col } else { Dir::Row };
        self.st.flex_split = Some((id, dir));
        let cwd = if cwd.is_empty() {
            Value::Null
        } else {
            json!(cwd)
        };
        self.call(Call::Created, "shell:create", cwd);
    }

    /// An extension's toolbar item: its popover under it, or its call or window.
    pub(super) fn run_tool(&mut self, ui: &mut Ui, key: String, src: Src) {
        let Some(p) = self
            .st
            .points
            .at(kind::TOOLBAR)
            .find(|p| p.key() == key)
            .cloned()
        else {
            return;
        };
        if !p.tree {
            let args = "{}".to_owned();
            self.run_point(ui, PointAct::Choose { key, args });
            return;
        }
        let scope = self
            .active_project()
            .map_or_else(|| EVERYWHERE.to_owned(), |(_, path)| path);
        let tree = scoped(&key, &scope);
        // Drawn afresh each time it opens: what it lists may have changed since.
        self.st.points.stale_trees.insert(tree.clone());
        self.open_menu(ui, MenuKind::Popover(tree), src);
    }

    /// Changes one arrangement and writes them all down.
    pub(super) fn arrange(
        &mut self,
        list: List,
        change: impl FnOnce(&mut crate::arrange::Arrangement),
    ) {
        change(self.st.arrange.get_mut(list));
        if let Some(dir) = &self.data_dir {
            self.st.arrange.save(&crate::arrange::file(dir));
        }
    }

    /// A row of the customize sheet dragged: it follows the pointer, and
    /// lands where it is let go, a row height per place.
    pub(super) fn arrange_event(&mut self, ui: &mut Ui, e: &UiEvent) -> bool {
        let (id, dy, end) = match *e {
            UiEvent::Move { id, dy, .. } => (id, dy, false),
            UiEvent::Drop { id, dy, .. } => (id, dy, true),
            _ => return false,
        };
        let Some(Act::ArrangeMove { list, id: item, to }) = self.acts.clicked(id) else {
            return false;
        };
        let scroll = ui.scroll_offset(crate::screen::customize::LIST);
        let from = match &self.st.arranging {
            Some((_, was, _, from)) if *was == item => *from,
            _ => scroll,
        };
        if !end {
            self.st.arranging = Some((list, item, dy, from));
            return true;
        }
        self.st.arranging = None;
        // The list may have scrolled under the pointer while it dragged.
        let travel = dy + scroll - from;
        let steps = (travel / crate::screen::customize::ROW_H).round() as isize;
        let to = to.saturating_add_signed(steps);
        self.run(ui, Act::ArrangeMove { list, id: item, to }, Src::default());
        true
    }
}
