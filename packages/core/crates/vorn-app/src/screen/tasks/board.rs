//! The board: the Tasks built-in's template, drawn by the kit's board, its
//! rows the tasks the view shows. The rows that place a card (its id and
//! status) are kept between frames ([`Placed`]); the rest of a card's row
//! is filled for the cards in view only.

use std::borrow::Cow;

use serde_json::{json, Map, Value};
use vorn_components::board as kit;
use vorn_components::template::{self, Bind, Press};
use vorn_protocol::TaskConfig;
use vorn_view::{Kind, Row, Source, Template};

use super::Screen;
use crate::points::{kind, Point};
use crate::screen::points::Binder;
use crate::tasks;
use crate::ui::{El, Id};

/// The Tasks built-in, whose template the board is.
pub const TASKS: &str = "vorn.tasks";
/// The collection its board draws.
pub const ROWS: &str = "tasks";

pub fn column_id(status: &str) -> Id {
    kit::column_id(ROWS, status)
}

pub fn card_id(task: &str) -> Id {
    kit::card_id(ROWS, task)
}

pub fn column_scroll_id(status: &str) -> Id {
    kit::scroll_id(ROWS, status)
}

/// The board's point, while Tasks is on and its template checks.
pub(super) fn point<'a>(s: &Screen<'a>) -> Option<&'a Point> {
    s.st.points.find(TASKS, kind::VIEWS, "board")
}

fn template<'a>(p: &'a Point) -> Option<&'a Template> {
    match &p.template {
        Ok(Some(t)) if matches!(t.view.kind, Kind::Board(_)) => Some(t),
        _ => None,
    }
}

/// The cards the board shows, placed: each task's id and status, in board
/// order, and where it is in the store's tasks. Read again only when the
/// tasks or what filters them change, never per frame.
#[derive(Debug, Default)]
pub struct Placed {
    key: Option<Key>,
    rows: Vec<Row>,
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
        // A column draws its cards in the order the rows come: the board's own.
        at.sort_by(|&a, &b| all[a].order.total_cmp(&all[b].order));
        self.rows = at
            .iter()
            .map(|&i| {
                let t = &all[i];
                let mut fields = Map::with_capacity(1);
                fields.insert("status".into(), json!(t.status.as_str()));
                Row {
                    id: t.id.clone(),
                    fields,
                }
            })
            .collect();
        self.at = at;
        self.key = Some(key);
    }
}

/// A task as the board's template reads it.
fn full(s: &Screen<'_>, t: &TaskConfig) -> Row {
    let status = t.status.as_str();
    let archived = t.archived_at.is_some();
    let terminal = tasks::is_terminal(status);
    let open = super::can_open(s, t);
    let live = super::live(s, t);
    let agent = t.assigned_agent.as_ref().map(ToString::to_string);
    let source = t
        .source_external_id
        .as_deref()
        .filter(|_| t.source_connector_id.is_some())
        .unwrap_or_default();
    let fields = json!({
        "title": t.title,
        "status": status,
        "short": tasks::short_id(&t.project_name, &t.id),
        "created": tasks::short_date(&t.created_at),
        "agent": agent.clone().unwrap_or_default(),
        "live": agent.is_some() && live,
        "source": source,
        "archived": archived,
        "dim": status == "cancelled" || archived,
        "focus": status == "in_progress" && open && live,
        "resume": status == "in_progress" && open && !live,
        "review": status == "in_review",
        "reopen": status == "cancelled",
        "cancel": !terminal,
        "archive": !archived && terminal,
    });
    let Value::Object(fields) = fields else {
        unreachable!("a JSON object literal")
    };
    Row {
        id: t.id.clone(),
        fields,
    }
}

/// The board's rows: those placed, each filled whole when drawn.
struct Rows<'s, 'a> {
    s: &'s Screen<'a>,
}

impl Source for Rows<'_, '_> {
    fn rows(&self, collection: &str) -> &[Row] {
        if collection == ROWS {
            &self.s.st.tasks.placed.rows
        } else {
            &[]
        }
    }

    fn row_at(&self, collection: &str, index: usize) -> Option<Cow<'_, Row>> {
        if collection != ROWS {
            return None;
        }
        let at = *self.s.st.tasks.placed.at.get(index)?;
        let t = self.s.store.tasks.get(at)?;
        Some(Cow::Owned(full(self.s, t)))
    }
}

/// The point's controls, with the cards bound to open their task and the
/// agents' marks drawn as the app draws them.
struct Binding<'s, 'a> {
    s: &'s Screen<'a>,
    point: Binder<'a>,
}

impl Bind for Binding<'_, '_> {
    fn press(&self, slot: u64, press: Press) -> Id {
        self.point.press(slot, press)
    }

    fn field(&self, slot: u64, row: Option<&str>, call: &str, value: &str) -> Id {
        self.point.field(slot, row, call, value)
    }

    fn confirming(&self, id: Id) -> Option<(Id, Id)> {
        self.point.confirming(id)
    }

    fn menu(&self, slot: u64, row: Option<&str>) -> Id {
        self.point.menu(slot, row)
    }

    fn menu_open(&self, id: Id) -> bool {
        self.point.menu_open(id)
    }

    fn card(&self, _rows: &str, row: &str, open: Option<&str>) -> Id {
        let id = card_id(row);
        if open.is_some() {
            self.s.acts.bind(
                id,
                crate::act::Act::Task(tasks::TaskAct::Open(row.to_owned())),
            );
        }
        id
    }

    fn dragging(&self, _rows: &str) -> Option<(String, Option<String>)> {
        self.s.st.tasks.dragging.clone()
    }

    fn icon(&self, name: &str, size: f32) -> Option<El> {
        let agent = name.strip_prefix("agent:")?;
        Some(super::super::agent_icon(self.s, agent, size - 1.0))
    }
}

/// The board, while Tasks is on; `None` while it is off.
pub(super) fn view(s: &Screen<'_>) -> Option<El> {
    let p = point(s)?;
    let Some(t) = template(p) else {
        let errors = match &p.template {
            Err(e) => e.clone(),
            _ => vec!["the board's template is not a board".to_owned()],
        };
        return Some(template::invalid(&p.place(), &errors));
    };
    let bind = Binding {
        s,
        point: Binder::whole(s.acts, &s.st.points, p),
    };
    Some(template::view(s.ui, t, None, &Rows { s }, &bind))
}

/// The dragged card, drawn at the pointer over everything.
pub(super) fn ghost(s: &Screen<'_>, task: &str, grab: (f32, f32, f32)) -> Option<El> {
    let p = point(s)?;
    let t = template(p)?;
    let Kind::Board(b) = &t.view.kind else {
        return None;
    };
    let bind = Binding {
        s,
        point: Binder::whole(s.acts, &s.st.points, p),
    };
    kit::ghost_of(s.ui, b, &Rows { s }, &bind, task, grab)
}

/// The statuses of the board's columns, in its order.
pub fn columns(s: &Screen<'_>) -> Vec<String> {
    point(s)
        .and_then(template)
        .and_then(|t| match &t.view.kind {
            Kind::Board(b) => Some(b.columns.iter().map(|c| c.value.clone()).collect()),
            _ => None,
        })
        .unwrap_or_default()
}
