//! What the screen's controls do. Each frame registers the [`Act`] behind
//! every clickable element under an id derived from it, so a click that
//! comes back as an id runs the act that was on screen.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::screen::MainView;
use crate::state::{Filter, LayoutMode, MenuKind, SidebarMode, Sort};
use crate::ui::{Action, El, Id, Sense};

/// One thing a control does.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Act {
    Minimize,
    ToggleMaximize,
    CloseWindow,
    View(MainView),
    ToggleSidebar,
    /// Selects a card (the active tab in the tabs layout).
    Select(String),
    /// Expands a card onto the focused stage.
    Expand(String),
    /// Puts the focused stage back into the grid.
    Collapse,
    /// Opens a menu anchored under the element that was clicked.
    Open(MenuKind),
    CloseMenu,
    Filter(Filter),
    Sort(Sort),
    Layout(LayoutMode),
    Columns(i32),
    /// The launcher, over the cards.
    NewSession,
    /// A session of the default agent in the active project at once, or
    /// the launcher when there is none.
    QuickSession,
    /// A session of the default agent (or `agent`) in a project, or one of
    /// its worktrees.
    NewSessionIn {
        project: String,
        worktree: Option<String>,
        agent: Option<String>,
    },
    /// A shell in a directory, or the active project's when empty.
    NewShellIn(String),
    /// An approval gate's controls.
    Gate(crate::gates::GateOp),
    RunWorkflow {
        id: String,
        source: Option<String>,
    },
    Rename(String),
    /// Asks before closing a session.
    AskClose(String),
    CloseSession(String),
    /// Stows a card in the session dock, or brings it back
    /// (`toggleMinimized`).
    ToggleMinimized(String),
    /// Folds the session dock into its chip, or opens it out.
    ToggleDock,
    /// Opens or closes the dock's popover.
    DockPopover,
    /// Opens a run on the workflows landing (`showRun`), by key.
    ShowRun(String),
    Resume(String),
    Paste(String),
    FitToDevice(String),
    LockSize(String, bool),
    Shortcuts,
    CloseDialog,
    Welcome,
    Settings,
    /// Filters the grid to a project, or all of them.
    Project(Option<String>),
    /// Filters the grid to one of a project's worktrees (`""` is the main
    /// checkout), or back to the whole project when it already is.
    Worktree {
        project: String,
        path: String,
    },
    /// Folds a sidebar worktree's sessions (`""` is the main checkout's).
    ToggleWorktree {
        project: String,
        path: String,
    },
    /// Opens a project in the editor `ide` (`ide:open`).
    OpenIde {
        ide: String,
        path: String,
    },
    AddProject,
    /// How the sidebar arranges the sessions.
    SidebarMode(SidebarMode),
    /// Folds the sessions sidebar's section.
    ToggleSection,
    /// Filters the grid to a group's sessions, or clears it.
    Group(Option<String>),
    /// Folds a group's sessions.
    ToggleGroup(String),
    /// A group named "New group", its name being edited; moving the
    /// session given into it.
    NewGroup(Option<String>),
    RenameGroup(String),
    GroupIcon(String),
    GroupIconName(String, String),
    GroupIconColor(String, String),
    /// Lets every session of a group go.
    UngroupAll(String),
    AskDeleteGroup(String),
    DeleteGroup(String),
    /// Moves a session into a group, or out of its own.
    MoveToGroup {
        session: String,
        group: Option<String>,
    },
    /// Folds a sidebar project's sessions.
    ToggleProject(String),
    EditProject(String),
    AskDeleteProject(String),
    DeleteProject(String),
    SaveProject,
    BrowseFolder,
    Restore,
    DismissBanner,
    DismissToast(String),
    ResumeRecent(usize),
    KillHeadless(String),
    HideHeadless(String),
    ToggleHeadless(String),
    /// The launcher's pickers.
    PickProject(String),
    PickAgent(String),
    PickWorktree(Option<String>),
    NewWorktree(bool),
    PickBranch(Option<String>),
    Launch,
    Task(crate::tasks::TaskAct),
    Run(crate::runs::RunAct),
    Workflow(crate::workflows::WfAct),
    Pane(crate::panes::PaneAct),
    /// Opens a link in the system browser.
    OpenUrl(String),
    /// A control that only takes the press, so nothing under it does.
    Nothing,
}

/// The acts registered while a frame was built.
#[derive(Default)]
pub struct Acts {
    clicks: RefCell<HashMap<Id, Act>>,
    contexts: RefCell<HashMap<Id, Act>>,
    doubles: RefCell<HashMap<Id, Act>>,
    /// Text fields bound to a value: what typing in them does, and the
    /// text the model holds.
    texts: RefCell<HashMap<Id, (Act, String)>>,
}

impl Acts {
    /// The id of `act`'s control in `place`, the same every frame; the
    /// place tells apart two controls on screen that do the same.
    pub fn id(place: &str, act: &Act) -> Id {
        Id::new(place).child(act)
    }

    /// Registers `act` and returns its id, for an element that needs it
    /// before it is built (to read its hover).
    pub fn on(&self, place: &str, act: Act) -> Id {
        let id = Acts::id(place, &act);
        self.clicks.borrow_mut().insert(id, act);
        id
    }

    /// A click on `id` does `act`, for an element whose id is fixed.
    pub fn bind(&self, id: Id, act: Act) {
        self.clicks.borrow_mut().insert(id, act);
    }

    /// `el` made a button in `place` that does `act`.
    pub fn click(&self, el: El, place: &str, act: Act) -> El {
        let id = self.on(place, act);
        el.id(id).on(Action::Click)
    }

    /// A secondary press on `id` does `act`.
    pub fn context(&self, el: El, id: Id, act: Act) -> El {
        self.contexts.borrow_mut().insert(id, act);
        el.id(id).sense(Sense::CONTEXT)
    }

    /// A double press on `id` does `act`.
    pub fn double(&self, id: Id, act: Act) {
        self.doubles.borrow_mut().insert(id, act);
    }

    /// Typing in the field `id` does `act`; `value` is what the model
    /// holds, which the field shows while it is not being typed in.
    pub fn text(&self, id: Id, act: Act, value: impl Into<String>) {
        self.texts.borrow_mut().insert(id, (act, value.into()));
    }

    pub fn text_of(&self, id: Id) -> Option<Act> {
        self.texts.borrow().get(&id).map(|(a, _)| a.clone())
    }

    /// Every bound field and the text its model holds.
    pub fn texts(&self) -> Vec<(Id, String)> {
        self.texts
            .borrow()
            .iter()
            .map(|(id, (_, v))| (*id, v.clone()))
            .collect()
    }

    pub fn clicked(&self, id: Id) -> Option<Act> {
        self.clicks.borrow().get(&id).cloned()
    }

    pub fn context_of(&self, id: Id) -> Option<Act> {
        self.contexts.borrow().get(&id).cloned()
    }

    pub fn double_of(&self, id: Id) -> Option<Act> {
        self.doubles.borrow().get(&id).cloned()
    }

    pub fn clear(&self) {
        self.clicks.borrow_mut().clear();
        self.contexts.borrow_mut().clear();
        self.doubles.borrow_mut().clear();
        self.texts.borrow_mut().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::div;

    #[test]
    fn a_registered_act_comes_back_by_its_id() {
        let acts = Acts::default();
        let _ = acts.click(div(), "card", Act::Select("a".into()));
        let id = Acts::id("card", &Act::Select("a".into()));
        assert_eq!(acts.clicked(id), Some(Act::Select("a".into())));
        assert_ne!(id, Acts::id("card", &Act::Select("b".into())));
        assert_ne!(id, Acts::id("row", &Act::Select("a".into())));
        acts.clear();
        assert_eq!(acts.clicked(id), None);
    }
}
