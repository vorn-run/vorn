//! The phone's native screens by route: each a vornui tree the native backend reads, and the acts its controls run.

mod sessions;
mod tasks;
mod workflows;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};
use vorn_components::status::Status;
use vornui::accesskit::Role;
use vornui::native::Sem;
use vornui::text::TextStyle;
use vornui::theme::color::INK;
use vornui::widgets::{area, text_input};
use vornui::{div, text, Action, El, Id, Ui};

use crate::model::{Fetched, For, Link};
use crate::phone::Phone;
use crate::screens;

/// Rows a long list shows before "Show more".
pub const PAGE: usize = 100;

/// Which of the Workflows tab's lists shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WfTab {
    #[default]
    Workflows,
    Runs,
    Review,
}

/// The workflows the Workflows list keeps, by how they start.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Trigger {
    #[default]
    All,
    Manual,
    Scheduled,
}

/// A scope narrower than everything: a project, or a branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Project(String),
    Branch(String),
}

impl Scope {
    pub fn name(&self) -> &str {
        match self {
            Scope::Project(n) | Scope::Branch(n) => n,
        }
    }
}

/// What a control does when the host acts on it.
#[derive(Debug, Clone, PartialEq)]
pub enum Do {
    /// Opens a route, `replace:<route>` or `back`.
    Go(String),
    SessionsScope(Option<Scope>),
    TasksScope(Option<String>),
    Column(&'static str),
    WfTab(WfTab),
    Trigger(Trigger),
    LaunchProject(String),
    LaunchAgent(String),
    LaunchWorktree(bool),
    Launch,
    Kill(String),
    Rename(String),
    Permission {
        request: String,
        allow: bool,
    },
    TaskStatus(String, &'static str),
    NewTaskProject(String),
    NewTask,
    SaveTask(String),
    /// Runs a workflow with no inputs from its page.
    RunWorkflow(String),
    /// Runs a workflow with the form's inputs.
    StartRun(String),
    Flip(String, String),
    Gate {
        run: String,
        node: String,
        decision: &'static str,
        back: bool,
    },
    SaveEdit(String, String),
    StopRun(String),
    More(String),
}

/// What the native screens remember between trees.
#[derive(Debug, Default)]
pub struct View {
    acts: RefCell<HashMap<Id, Do>>,
    seeded: HashSet<Id>,
    pub sessions_scope: Option<Scope>,
    pub tasks_scope: Option<String>,
    pub column: &'static str,
    pub wf_tab: WfTab,
    pub trigger: Trigger,
    pub launch_project: Option<String>,
    pub launch_agent: Option<String>,
    pub launch_worktree: bool,
    pub new_task_project: Option<String>,
    /// Boolean run inputs, by workflow and key.
    pub flags: HashMap<(String, String), bool>,
    /// A gate's rewritten text, by run and node, sent with its answer.
    pub edited: HashMap<(String, String), String>,
    pub limits: HashMap<String, usize>,
}

impl View {
    pub fn new() -> View {
        View {
            column: "todo",
            ..View::default()
        }
    }

    /// What `id` does, as the last tree read bound it.
    pub fn act(&self, id: Id) -> Option<Do> {
        self.acts.borrow().get(&id).cloned()
    }

    fn bind(&self, id: Id, d: Do) {
        self.acts.borrow_mut().insert(id, d);
    }

    fn limit(&self, key: &str) -> usize {
        self.limits.get(key).copied().unwrap_or(PAGE)
    }
}

/// What a screen is built from.
pub struct Cx<'a> {
    pub p: &'a Phone,
    pub ui: &'a Ui,
    /// Now, epoch milliseconds.
    pub now: f64,
}

impl Cx<'_> {
    fn view(&self) -> &View {
        &self.p.view
    }

    fn button(&self, id: Id, label: &str, style: &'static str, d: Do) -> El {
        self.view().bind(id, d);
        div()
            .id(id)
            .on(Action::Click)
            .role(Role::Button, label)
            .sem(Sem::Button {
                style,
                confirm: None,
                icon: None,
            })
    }

    /// A destructive button asking `confirm` first.
    fn asking(&self, id: Id, label: &str, confirm: &str, d: Do) -> El {
        self.view().bind(id, d);
        div()
            .id(id)
            .on(Action::Click)
            .role(Role::Button, label)
            .sem(Sem::Button {
                style: "danger",
                confirm: Some(confirm.to_owned()),
                icon: None,
            })
    }

    fn option(&self, id: Id, label: &str, on: bool, d: Do) -> El {
        self.view().bind(id, d);
        div()
            .id(id)
            .on(Action::Click)
            .role(Role::RadioButton, label)
            .toggled(on)
    }

    fn toggle(&self, id: Id, label: &str, on: bool, d: Do) -> El {
        self.view().bind(id, d);
        div()
            .id(id)
            .on(Action::Click)
            .role(Role::Switch, label)
            .toggled(on)
    }

    /// A one-line text field, or a multiline one.
    fn field(&self, id: Id, placeholder: &str, multiline: bool) -> El {
        if multiline {
            area(id, placeholder, placeholder, TextStyle::ui(15.0), INK)
        } else {
            text_input(self.ui, id, placeholder, placeholder)
        }
    }

    /// "Show more" under a list cut at its limit.
    fn more(&self, key: &str, shown: usize, total: usize) -> Option<El> {
        (shown < total).then(|| {
            self.button(
                Id::new("more").child(key),
                &format!("Show more ({} left)", total - shown),
                "plain",
                Do::More(key.to_owned()),
            )
        })
    }

    fn error(&self, key: &str) -> Option<El> {
        self.p
            .model
            .errors
            .get(key)
            .map(|e| say(e.as_str(), "error"))
    }

    fn fetch(&self, method: &str, params: Value) -> Fetched {
        self.p.model.fetch(method, params)
    }

    fn online(&self) -> bool {
        self.p.model.link == Link::Online
    }
}

/// The screen's title, and the buttons and links for the navigation bar.
fn bar(title: impl Into<String>, items: Vec<El>) -> El {
    div().row().sem(Sem::Bar(title.into())).children(items)
}

/// Nothing to list: why, and what to do about it.
fn empty(title: &str, body: &str, actions: Vec<El>) -> El {
    div()
        .col()
        .sem(Sem::Empty {
            title: title.to_owned(),
            body: body.to_owned(),
        })
        .children(actions)
}

fn say(s: impl Into<String>, role: &'static str) -> El {
    text(s, 15.0, INK).sem(Sem::Text(role))
}

fn section(title: &str, rows: Vec<El>) -> El {
    div()
        .col()
        .sem(Sem::Section(title.to_owned()))
        .children(rows)
}

fn listing(rows: Vec<El>) -> El {
    div().col().sem(Sem::List).children(rows)
}

fn row(kids: Vec<El>) -> El {
    div().row().sem(Sem::Row).children(kids)
}

fn col(kids: Vec<El>) -> El {
    div().col().sem(Sem::Column).children(kids)
}

/// A row that opens `route`, showing `kids`.
fn link(id: Id, label: &str, route: String, kids: Vec<El>) -> El {
    div()
        .id(id)
        .on(Action::Click)
        .role(Role::Link, label)
        .sem(Sem::Link(route))
        .children(kids)
}

/// A row that opens `route` over the screen.
fn present(id: Id, label: &str, route: String, icon: Option<&str>) -> El {
    present_with(id, label, route, icon.map(self::icon))
}

fn present_with(id: Id, label: &str, route: String, icon: Option<El>) -> El {
    div()
        .id(id)
        .on(Action::Click)
        .role(Role::Link, label)
        .sem(Sem::Present(route))
        .child_if(icon)
        .child(say(label, "body"))
}

/// The filter button that opens `route`, lit while a filter is on, as the desktop's sidebar draws it.
fn filter(id: Id, label: &str, route: String, lit: bool) -> El {
    let icon = div().sem(Sem::Icon {
        name: "list-filter".into(),
        tone: if lit { "lit" } else { "" },
        svg: None,
    });
    present_with(id, label, route, Some(icon))
}

fn icon(name: &str) -> El {
    div().sem(Sem::icon(name))
}

/// `agent`'s mark, as the desktop draws it.
fn agent(agent: &str) -> El {
    div().sem(Sem::icon(vorn_components::agent::icon_name(agent)))
}

fn status(value: &str, tone: &'static str) -> El {
    div().sem(Sem::Status {
        value: value.to_owned(),
        tone,
    })
}

fn badge(value: &str, tone: &'static str) -> El {
    div().sem(Sem::Badge {
        value: value.to_owned(),
        tone,
    })
}

/// A labelled picker of the options inside; unlabelled, the screen's tabs.
fn choice(label: &str, options: Vec<El>) -> El {
    div()
        .col()
        .role(Role::RadioGroup, label)
        .sem(Sem::Choice)
        .children(options)
}

fn copy(id: Id, label: &str, what: &str) -> El {
    div()
        .id(id)
        .on(Action::Click)
        .role(Role::Button, label)
        .sem(Sem::Copy(what.to_owned()))
}

/// The tone a native host colours a session's status with.
fn tone(s: Status) -> &'static str {
    screens::tone(s)
}

/// A run state's tone.
fn run_tone(status: &str) -> &'static str {
    match status {
        "waiting" => "warn",
        "error" => "bad",
        "running" => "busy",
        "success" => "good",
        _ => "muted",
    }
}

fn now_ms() -> f64 {
    vorn_app::tasks::ms(&vorn_app::tasks::now_iso()).unwrap_or(0.0)
}

fn seed(p: &mut Phone, ui: &mut Ui, id: Id, text: &str) {
    if p.view.seeded.insert(id) {
        ui.set_edit_text(id, text);
    }
}

/// Puts what a screen's fields start with into them, once per opening.
fn seeds(p: &mut Phone, ui: &mut Ui, route: &str, arg: &str) {
    match route {
        "rename" => {
            let name = p
                .model
                .store
                .session(arg)
                .map(vorn_app::client::store::Session::title)
                .unwrap_or_default();
            seed(p, ui, sessions::rename_id(arg), &name);
        }
        "task-edit" => {
            let Some(t) = p.model.store.tasks.iter().find(|t| t.id == arg) else {
                return;
            };
            let (title, body) = (t.title.clone(), t.description.clone());
            seed(p, ui, tasks::title_id(arg), &title);
            seed(p, ui, tasks::body_id(arg), &body);
        }
        "gate-edit" => {
            let Some((run, node)) = arg.split_once(':') else {
                return;
            };
            let text = workflows::gate_text(p, run, node);
            seed(p, ui, workflows::edit_id(run, node), &text);
        }
        "workflow-run" => {
            for (key, value) in workflows::input_defaults(p, arg) {
                seed(p, ui, workflows::input_id(arg, &key), &value);
            }
        }
        _ => {}
    }
}

/// The tree for `screen`: a route like `sessions` or `task:<id>`, listed in `apps/ios/README.md`.
pub fn screen(p: &mut Phone, ui: &mut Ui, screen: &str) -> El {
    p.view.acts.borrow_mut().clear();
    let (route, arg) = screen.split_once(':').unwrap_or((screen, ""));
    seeds(p, ui, route, arg);
    let cx = Cx {
        p,
        ui,
        now: now_ms(),
    };
    let body = match route {
        "sessions" => sessions::list(&cx),
        "sessions-scope" => sessions::scope(&cx),
        "launch" => sessions::launch(&cx),
        "session" => sessions::session(&cx, arg),
        "rename" => sessions::rename(&cx, arg),
        "tasks" => tasks::board(&cx),
        "tasks-scope" => tasks::scope(&cx),
        "task" => tasks::task(&cx, arg),
        "task-new" => tasks::new(&cx),
        "task-edit" => tasks::edit(&cx, arg),
        "workflows" => workflows::list(&cx),
        "workflows-trigger" => workflows::trigger(&cx),
        "workflow" => workflows::workflow(&cx, arg),
        "workflow-run" => workflows::form(&cx, arg),
        "run" => workflows::run(&cx, arg),
        "run-log" => workflows::log(&cx, arg),
        "review" => workflows::review(&cx, arg),
        "gate-changes" => workflows::changes(&cx, arg),
        "gate-edit" => workflows::edit(&cx, arg),
        _ => return screens::native(cx.p, cx.ui, screen),
    };
    div().col().child(body)
}

/// Runs what a control does; `ui` is its screen's, holding the fields it reads.
pub fn run(p: &mut Phone, ui: &mut Ui, d: Do) {
    let back = |p: &mut Phone| p.model.go = Some("back".into());
    match d {
        Do::Go(to) => p.model.go = Some(to),
        Do::SessionsScope(s) => {
            p.view.sessions_scope = s;
            back(p);
        }
        Do::TasksScope(s) => {
            p.view.tasks_scope = s;
            back(p);
        }
        Do::Column(c) => p.view.column = c,
        Do::WfTab(t) => p.view.wf_tab = t,
        Do::Trigger(t) => {
            p.view.trigger = t;
            back(p);
        }
        Do::LaunchProject(n) => p.view.launch_project = Some(n),
        Do::LaunchAgent(a) => p.view.launch_agent = Some(a),
        Do::LaunchWorktree(w) => p.view.launch_worktree = w,
        Do::Launch => sessions::launch_now(p, ui),
        Do::Kill(sid) => {
            p.model.call(For::Quiet, "terminal:kill", json!(sid));
            back(p);
        }
        Do::Rename(sid) => {
            let id = sessions::rename_id(&sid);
            let name = ui.edit_text(id).trim().to_owned();
            p.view.seeded.remove(&id);
            if !name.is_empty() {
                p.model.call(
                    For::Quiet,
                    "terminal:rename",
                    json!({ "id": sid, "displayName": name }),
                );
            }
            back(p);
        }
        Do::Permission { request, allow } => {
            p.model
                .permissions
                .retain(|q| q["requestId"] != request.as_str());
            p.model.call(
                For::Quiet,
                "permission:resolve",
                json!({ "requestId": request, "allow": allow }),
            );
        }
        Do::TaskStatus(id, status) => tasks::set_status(p, &id, status),
        Do::NewTaskProject(n) => p.view.new_task_project = Some(n),
        Do::NewTask => tasks::add(p, ui),
        Do::SaveTask(id) => tasks::save(p, ui, &id),
        Do::RunWorkflow(id) => workflows::start(p, &id, json!({}), false),
        Do::StartRun(id) => workflows::start_form(p, ui, &id),
        Do::Flip(wf, key) => {
            let on = p.view.flags.entry((wf, key)).or_insert(false);
            *on = !*on;
        }
        Do::Gate {
            run,
            node,
            decision,
            back,
        } => workflows::answer(p, ui, &run, &node, decision, back),
        Do::SaveEdit(run, node) => {
            let id = workflows::edit_id(&run, &node);
            let text = ui.edit_text(id).to_owned();
            p.view.seeded.remove(&id);
            p.view.edited.insert((run, node), text);
            back(p);
        }
        Do::StopRun(run) => {
            p.model
                .call(For::Quiet, "workflow:stopRun", json!({ "runId": run }));
        }
        Do::More(key) => {
            let n = p.view.limit(&key) + PAGE;
            p.view.limits.insert(key, n);
        }
    }
}
