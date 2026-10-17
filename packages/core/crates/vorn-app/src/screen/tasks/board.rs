//! The board: the Tasks built-in's main view, an element tree whose board
//! reads its cards from the tasks the view shows, drawn by the kit's
//! windowed board. The cards that place a card (its id and status) are kept
//! between frames ([`Placed`]); the rest of a card is filled for the cards
//! in view only. Its menus, add buttons, drag and detail are the app's.

use vorn_components::board::{self as kit, BoardBind, Card, CardAction, Cards, Lane};
use vorn_components::tree::{self, Ev, TreeBind};
use vorn_protocol::TaskConfig;
use vorn_tree::{Kind, Node};

use super::Screen;
use crate::act::Act;
use crate::points::{kind, Point, PointAct};
use crate::screen::points::TreeBinder;
use crate::tasks;
use crate::ui::{El, Id};

/// The Tasks built-in, whose main view the board is.
pub const TASKS: &str = "vorn.tasks";
/// The board's name in the kit's ids.
pub const BOARD: &str = "tasks";

pub fn column_id(status: &str) -> Id {
    kit::column_id(BOARD, status)
}

pub fn card_id(task: &str) -> Id {
    kit::card_id(BOARD, task)
}

pub fn column_scroll_id(status: &str) -> Id {
    kit::scroll_id(BOARD, status)
}

/// The board's point, while Tasks is on.
pub(super) fn point<'a>(s: &Screen<'a>) -> Option<&'a Point> {
    s.st.points.find(TASKS, kind::VIEWS, "board")
}

/// The board in `tree`: the first board that reads the tasks.
fn board_of(tree: &Node) -> Option<&Node> {
    if matches!(
        tree.kind,
        Kind::Board {
            cards: Some(vorn_tree::Cards::Tasks),
            ..
        }
    ) {
        return Some(tree);
    }
    tree.children.iter().find_map(board_of)
}

/// The board's lanes, as its tree's columns name them.
fn lanes(board: &Node) -> Vec<Lane<'_>> {
    board
        .children
        .iter()
        .filter_map(|c| match &c.kind {
            Kind::Column { title, tone, icon } => Some(Lane {
                value: c.id.as_deref()?,
                title,
                icon: icon.as_deref(),
                tone: *tone,
            }),
            _ => None,
        })
        .collect()
}

/// The cards the board shows, placed: each task's id and status, in board
/// order, and where it is in the store's tasks. Read again only when the
/// tasks or what filters them change, never per frame.
#[derive(Debug, Default)]
pub struct Placed {
    key: Option<Key>,
    cards: Vec<(String, String)>,
    at: Vec<usize>,
}

/// What [`Placed`] was read from.
#[derive(Debug, Clone, PartialEq)]
struct Key {
    gen: u64,
    filter: Option<String>,
    source: tasks::Source,
    archived: bool,
    project: Option<String>,
    projects: Vec<String>,
}

impl Placed {
    /// Reads the cards again if anything they come from changed.
    pub fn refresh(
        &mut self,
        st: &tasks::Tasks,
        all: &[TaskConfig],
        project: Option<&str>,
        projects: &[&str],
        gen: u64,
    ) {
        let key = Key {
            gen,
            filter: st.filter.clone(),
            source: st.source.clone(),
            archived: st.archived,
            project: project.map(str::to_owned),
            projects: projects.iter().map(|p| (*p).to_owned()).collect(),
        };
        if self.key.as_ref() == Some(&key) {
            return;
        }
        let mut at = st.visible_at(all, project, projects);
        // A column draws its cards in the order they come: the board's own.
        at.sort_by(|&a, &b| all[a].order.total_cmp(&all[b].order));
        self.cards = at
            .iter()
            .map(|&i| (all[i].id.clone(), all[i].status.as_str().to_owned()))
            .collect();
        self.at = at;
        self.key = Some(key);
    }
}

const fn action(
    id: &'static str,
    label: &'static str,
    icon: &'static str,
    danger: bool,
) -> CardAction {
    CardAction {
        id,
        label,
        icon,
        danger,
        confirm: None,
    }
}

const FOCUS: CardAction = action("focus", "Focus session", "terminal", false);
const RESUME: CardAction = action("resume", "Resume session", "play", false);
const REVIEW: CardAction = action("review", "Review diff", "file-code", false);
const COMPLETE: CardAction = action("complete", "Mark as done", "circle-check", false);
const REOPEN: CardAction = action("reopen", "Reopen task", "rotate-ccw", false);
const EDIT: CardAction = action("edit", "Edit", "pencil", false);
const CANCEL: CardAction = action("cancel", "Cancel task", "circle-x", true);
const ARCHIVE: CardAction = action("archive", "Archive", "archive", false);
const UNARCHIVE: CardAction = action("unarchive", "Unarchive", "archive-restore", false);
const DELETE: CardAction = CardAction {
    confirm: Some("Delete this task permanently?"),
    ..action("delete", "Delete", "trash-2", true)
};

/// A task as the board draws it, with the menu its state offers.
fn card(s: &Screen<'_>, t: &TaskConfig) -> Card {
    let status = t.status.as_str();
    let archived = t.archived_at.is_some();
    let terminal = tasks::is_terminal(status);
    let open = super::can_open(s, t);
    let live = super::live(s, t);
    let agent = t.assigned_agent.as_ref().map(ToString::to_string);
    let link = t
        .source_external_id
        .clone()
        .filter(|l| !l.is_empty() && t.source_connector_id.is_some());
    let working = status == "in_progress" && open;
    let review = status == "in_review";
    let menu = [
        (FOCUS, working && live),
        (RESUME, working && !live),
        (REVIEW, review),
        (COMPLETE, review),
        (REOPEN, status == "cancelled"),
        (EDIT, true),
        (CANCEL, !terminal),
        (ARCHIVE, !archived && terminal),
        (UNARCHIVE, archived),
        (DELETE, true),
    ];
    Card {
        id: t.id.clone(),
        key: tasks::short_id(&t.project_name, &t.id),
        title: t.title.clone(),
        link,
        archived,
        dim: status == "cancelled" || archived,
        live: agent.is_some() && live,
        agent,
        note: format!("Created {}", tasks::short_date(&t.created_at)),
        actions: menu.into_iter().filter(|(_, on)| *on).map(|(a, _)| a).collect(),
    }
}

/// The board's cards: those placed, each filled whole when drawn.
struct TaskCards<'s, 'a> {
    s: &'s Screen<'a>,
}

impl Cards for TaskCards<'_, '_> {
    fn placed(&self) -> &[(String, String)] {
        &self.s.st.tasks.placed.cards
    }

    fn card(&self, at: usize) -> Option<Card> {
        let i = *self.s.st.tasks.placed.at.get(at)?;
        Some(card(self.s, self.s.store.tasks.get(i)?))
    }
}

/// The board's controls bound to the app's task acts, as the board always ran them.
struct Binding<'s, 'a> {
    s: &'s Screen<'a>,
    key: String,
}

impl BoardBind for Binding<'_, '_> {
    fn card(&self, card: &str) -> Id {
        let id = card_id(card);
        self.s
            .acts
            .bind(id, Act::Task(tasks::TaskAct::Open(card.to_owned())));
        id
    }

    fn dragging(&self) -> Option<(String, Option<String>)> {
        self.s.st.tasks.dragging.clone()
    }

    fn add(&self, lane: &str, foot: bool) -> Id {
        let id = Id::new(self.key.as_str()).child("add").child(lane).child(foot);
        self.s
            .acts
            .bind(id, Act::Task(tasks::TaskAct::New(lane.to_owned())));
        id
    }

    fn menu(&self, card: &str) -> Id {
        let id = Id::new(self.key.as_str()).child("more").child(card);
        let open = self.s.st.points.more == Some(id.0);
        self.s
            .acts
            .bind(id, Act::Point(PointAct::More((!open).then_some(id.0))));
        id
    }

    fn menu_open(&self, id: Id) -> bool {
        self.s.st.points.more == Some(id.0)
    }

    fn action(&self, card: &str, a: &CardAction) -> Id {
        let id = Id::new(self.key.as_str()).child(card).child(a.id);
        let Some(act) = tasks::board_act(a.id, card.to_owned()) else {
            return id;
        };
        let acts = self.s.acts;
        if a.confirm.is_none() {
            acts.bind(id, Act::Task(act));
        } else if self.s.st.points.asking == Some(id.0) {
            acts.bind(id, Act::Point(PointAct::Ask(None)));
            acts.bind(id.child("no"), Act::Point(PointAct::Ask(None)));
            acts.bind(id.child("ok"), Act::Task(act));
        } else {
            acts.bind(id, Act::Point(PointAct::Ask(Some(id.0))));
        }
        id
    }

    fn confirming(&self, id: Id) -> Option<(Id, Id)> {
        (self.s.st.points.asking == Some(id.0)).then(|| (id.child("ok"), id.child("no")))
    }

    fn icon(&self, name: &str, size: f32) -> Option<El> {
        let agent = name.strip_prefix("agent:")?;
        Some(super::super::agent_icon(self.s, agent, size - 1.0))
    }
}

/// The view's tree, its board drawn over the tasks the view shows.
struct ViewBind<'s, 'a> {
    tree: TreeBinder<'s>,
    board: Binding<'s, 'a>,
}

impl TreeBind for ViewBind<'_, '_> {
    fn on(&self, ev: Ev) -> Id {
        self.tree.on(ev)
    }

    fn confirming(&self, id: Id) -> Option<(Id, Id)> {
        self.tree.confirming(id)
    }

    fn menu_open(&self, id: Id) -> bool {
        self.tree.menu_open(id)
    }

    fn checked(&self, list: &str, row: &str) -> bool {
        self.tree.checked(list, row)
    }

    fn board(
        &self,
        cards: vorn_tree::Cards,
        lanes: &[Lane<'_>],
        empty: Option<&str>,
    ) -> Option<El> {
        let vorn_tree::Cards::Tasks = cards;
        let cards = TaskCards { s: self.board.s };
        let board = kit::Board {
            ui: self.board.s.ui,
            name: BOARD,
            lanes,
            empty,
            cards: &cards,
            bind: &self.board,
        };
        Some(board.view())
    }
}

/// The board, while Tasks is on and its tree has come; its error box when
/// the tree cannot be drawn.
pub(super) fn view(s: &Screen<'_>) -> Option<El> {
    let p = point(s)?;
    let key = p.key();
    let tree = match s.st.points.trees.get(&key)? {
        Ok(t) => t,
        Err(errors) => return Some(tree::invalid(&p.place(), errors)),
    };
    let bind = ViewBind {
        tree: TreeBinder {
            acts: s.acts,
            points: &s.st.points,
            key: &key,
        },
        board: Binding {
            s,
            key: key.clone(),
        },
    };
    Some(tree::view(s.ui, tree, &bind))
}

/// The dragged card, drawn at the pointer over everything.
pub(super) fn ghost(s: &Screen<'_>, task: &str, grab: (f32, f32, f32)) -> Option<El> {
    let p = point(s)?;
    let key = p.key();
    let tree = s.st.points.trees.get(&key)?.as_ref().ok()?;
    let node = board_of(tree)?;
    let lanes = lanes(node);
    let cards = TaskCards { s };
    let bind = Binding { s, key };
    let empty = match &node.kind {
        Kind::Board { empty, .. } => empty.as_deref(),
        _ => None,
    };
    kit::Board {
        ui: s.ui,
        name: BOARD,
        lanes: &lanes,
        empty,
        cards: &cards,
        bind: &bind,
    }
    .ghost(task, grab)
}

/// The statuses of the board's columns, in its order.
pub fn columns(s: &Screen<'_>) -> Vec<String> {
    point(s)
        .and_then(|p| s.st.points.trees.get(&p.key())?.as_ref().ok())
        .and_then(board_of)
        .map(|b| lanes(b).iter().map(|l| l.value.to_owned()).collect())
        .unwrap_or_default()
}
