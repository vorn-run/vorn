//! Workflows as the renderer's landing, sidebar and editor read them: run
//! buckets and status lines (`run-presentation`), the canvas's flow and its
//! edits (`workflow-helpers`), the node factories and the run inputs.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Map, Value};

use crate::graph;
use crate::runs::{self, s};

/// The landing's two tabs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Tab {
    #[default]
    All,
    Review,
}

/// `RunBucket`: the Status filter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Bucket {
    #[default]
    All,
    Running,
    Waiting,
    Error,
    Success,
}

impl Bucket {
    pub const ALL: [Bucket; 5] = [
        Bucket::All,
        Bucket::Running,
        Bucket::Waiting,
        Bucket::Error,
        Bucket::Success,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Bucket::All => "All",
            Bucket::Running => "Running",
            Bucket::Waiting => "Waiting",
            Bucket::Error => "Failed",
            Bucket::Success => "Succeeded",
        }
    }
}

/// The sidebar's workflow filter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum SideFilter {
    #[default]
    All,
    Manual,
    Scheduled,
}

impl SideFilter {
    pub const ALL: [SideFilter; 3] = [SideFilter::All, SideFilter::Manual, SideFilter::Scheduled];

    pub fn label(self) -> &'static str {
        match self {
            SideFilter::All => "All",
            SideFilter::Manual => "Manual",
            SideFilter::Scheduled => "Scheduled",
        }
    }

    pub fn keeps(self, wf: &Value) -> bool {
        let scheduled = is_scheduled(wf);
        match self {
            SideFilter::All => true,
            SideFilter::Manual => !scheduled,
            SideFilter::Scheduled => scheduled,
        }
    }
}

/// Where the step library adds what is picked.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Anchor {
    /// The trigger's slot.
    pub trigger: bool,
    pub after: Option<String>,
    /// A node to splice before, [`graph::LOOP_BODY`] or [`graph::FORK`];
    /// none to append.
    pub before: Option<String>,
    /// The node the pick replaces in place.
    pub replace: Option<String>,
    /// Loop and parallel insertion stay off anchors inside a fork branch.
    pub inside_branch: bool,
    /// A loop body takes only the steps it can repeat.
    pub body_only: bool,
    /// Flow coordinates where a dragged edge was let go.
    pub position: Option<(i64, i64)>,
}

/// The editor's right panel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Panel {
    #[default]
    None,
    Library(Anchor),
    Node,
    Properties,
    StartFrom,
    History,
}

/// What the run prompt asks for.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Prompt {
    pub workflow: String,
    pub name: String,
    pub contextual: bool,
    /// The caller's source, when it gave one.
    pub source: Option<Value>,
    pub needs_project: bool,
    pub needs_branch: bool,
    pub needs_worktree: bool,
    pub project: Option<String>,
    pub worktree: bool,
    pub inputs: Vec<Value>,
    /// Toggle and choice inputs, which have no text field.
    pub picked: HashMap<String, String>,
    pub target: Option<String>,
}

/// One missed schedule the dialog offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missed {
    pub workflow: String,
    pub name: String,
    pub at: String,
    pub on: bool,
}

/// The editor's working copy of one workflow.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub wf: Value,
    /// Saved before, so Save updates rather than adds.
    pub existing: bool,
}

impl Draft {
    pub fn id(&self) -> &str {
        s(&self.wf, "id").unwrap_or("")
    }

    pub fn nodes(&self) -> &[Value] {
        self.wf
            .get("nodes")
            .and_then(Value::as_array)
            .map_or(&[], Vec::as_slice)
    }

    pub fn edges(&self) -> &[Value] {
        self.wf
            .get("edges")
            .and_then(Value::as_array)
            .map_or(&[], Vec::as_slice)
    }

    pub fn node(&self, id: &str) -> Option<&Value> {
        self.nodes().iter().find(|n| s(n, "id") == Some(id))
    }

    pub fn node_mut(&mut self, id: &str) -> Option<&mut Value> {
        self.wf
            .get_mut("nodes")?
            .as_array_mut()?
            .iter_mut()
            .find(|n| s(n, "id") == Some(id))
    }

    pub fn set_graph(&mut self, nodes: Vec<Value>, edges: Vec<Value>) {
        self.wf["nodes"] = Value::Array(nodes);
        self.wf["edges"] = Value::Array(edges);
    }
}

/// What the workflow surfaces hold between frames.
#[derive(Debug, Default)]
pub struct Workflows {
    pub tab: Tab,
    pub bucket: Bucket,
    /// The landing's selected run, by key.
    pub selected: Option<String>,
    /// `workflowRun:listAll`.
    pub runs: Vec<Value>,
    pub side: SideFilter,
    /// The sidebar section folded under its header.
    pub folded: bool,
    /// A sidebar row being dragged, and the index it would land on.
    pub drag: Option<(String, Option<usize>)>,
    pub editor: Option<Draft>,
    /// The node whose config is open.
    pub node: Option<String>,
    pub panel: Panel,
    /// The editor's run history.
    pub history: Vec<Value>,
    /// The step library's search.
    pub search: String,
    /// `launchingSince`: a run asked for from the editor, not yet reported.
    pub launching: Option<f64>,
    pub prompt: Option<Prompt>,
    pub missed: Vec<Missed>,
    /// `connection:list`, for naming a sign-in and the library's groups.
    pub connections: Vec<Value>,
    /// The canvas's view and the gesture on it.
    pub canvas: CanvasState,
    /// What the step library and start-from draw from vornd.
    pub lib: LibraryData,
    /// The definition's undo and redo.
    pub undo: Undo,
    /// The `{{` dropdown, when open.
    pub ac: Option<Ac>,
    /// Advanced sections open, by node.
    pub advanced: HashSet<String>,
    /// Text kept aside because it is not yet valid (a half-typed schema),
    /// by node and path.
    pub raw: HashMap<(String, String), String>,
    /// Connector arguments shown as a template rather than their list.
    pub free_args: HashSet<(String, String)>,
    /// `webhook:info`'s base URL.
    pub webhook: Option<String>,
    /// The URL just copied, for the check mark.
    pub copied: Option<f64>,
    /// `agent:listModels`, by agent and project path.
    pub models: HashMap<(String, String), Vec<Value>>,
    /// `git:isGitRepo`, by path.
    pub git_repo: HashMap<String, bool>,
    /// What an import left unbound, by the workflow it landed in.
    pub imported: Option<(String, Vec<Value>)>,
    /// A workflow vornd just made, opened once the frame has a `Ui`.
    pub open_next: Option<String>,
    /// When the editor launched a run whose entry should open by itself.
    pub follow_armed: Option<f64>,
    /// The followed run's key: its running step opens as it starts.
    pub follow: Option<String>,
}

impl Workflows {
    /// `stillNeeded`: the canvas's unbound steps and an import's leftovers,
    /// when no selection, library, history or start-from holds the slot.
    pub fn still_needed(&self) -> Vec<crate::templates::Need> {
        use crate::templates as t;
        let Some(d) = &self.editor else {
            return Vec::new();
        };
        let free = match self.panel {
            Panel::Properties | Panel::None => true,
            Panel::Node => self.node.as_deref().and_then(|n| d.node(n)).is_none(),
            _ => false,
        };
        if !free {
            return Vec::new();
        }
        let imported = self
            .imported
            .as_ref()
            .filter(|(id, _)| id == d.id())
            .map_or(&[][..], |(_, r)| r.as_slice());
        let all = t::merge(t::of_definition(d.nodes()), imported);
        t::with_bindings(&all, &self.connections)
            .into_iter()
            .filter(|n| n.bound.is_none())
            .collect()
    }
}

/// The variable dropdown under a field (`VariableAutocomplete`).
#[derive(Debug, Clone, PartialEq)]
pub struct Ac {
    pub bind: Bind,
    /// Where the `{{` that opened it starts, in bytes.
    pub start: usize,
    pub filter: String,
    pub collapsed: HashSet<String>,
    pub highlight: usize,
}

/// A gesture on the canvas.
#[derive(Debug, Clone, PartialEq)]
pub enum Gesture {
    /// The pane dragged, from this view.
    Pan(crate::canvas::Viewport),
    /// Cards dragged: each top-level card's position when it began.
    Node {
        id: String,
        starts: Vec<(String, (f64, f64))>,
    },
    /// A connection drawn from a port to the pointer, in flow coordinates.
    Connect {
        source: String,
        body_port: bool,
        to: (f64, f64),
    },
    /// The minimap dragged, from this view.
    Mini(crate::canvas::Viewport),
}

/// The canvas between frames.
#[derive(Debug, Default)]
pub struct CanvasState {
    /// Where each workflow was last looked at (`canvas-views`).
    pub views: HashMap<String, crate::canvas::Viewport>,
    pub gesture: Option<Gesture>,
    /// Cards' positions while they are dragged.
    pub dragged: HashMap<String, (f64, f64)>,
    /// The edge the pointer is over, whose + shows.
    pub edge: Option<String>,
}

/// The step library's sources and memory (`step-library-memory`).
#[derive(Debug, Default)]
pub struct LibraryData {
    /// `connector:catalog`: items, templates, MCP servers.
    pub catalog: Vec<Value>,
    pub templates: Vec<Value>,
    pub packs: Vec<Value>,
    /// `connector:list`.
    pub connectors: Vec<Value>,
    /// `connection:listActions`, by connection.
    pub actions: HashMap<String, Vec<Value>>,
    pub recent: Vec<crate::library::Recent>,
    pub open: HashSet<String>,
    /// The highlighted line, among those the arrow keys reach.
    pub highlight: usize,
}

/// `useDefinitionHistory`: snapshots of nodes and edges, edits within half
/// a second undone as one.
#[derive(Debug, Default)]
pub struct Undo {
    pub past: Vec<(Value, Value)>,
    pub future: Vec<(Value, Value)>,
    pub last: f64,
}

impl Undo {
    /// `STACK_LIMIT`.
    const LIMIT: usize = 100;
    /// `MERGE_WINDOW_MS`.
    const MERGE: f64 = 500.0;

    /// Records the graph as it was before an edit at `now`.
    pub fn record(&mut self, before: (Value, Value), now: f64) {
        if now - self.last > Self::MERGE || self.past.is_empty() {
            self.past.push(before);
            if self.past.len() > Self::LIMIT {
                self.past.remove(0);
            }
        }
        self.last = now;
        self.future.clear();
    }

    pub fn clear(&mut self) {
        *self = Undo::default();
    }
}

/// `liveExecSignature`'s part for one run: its status, end and the nodes
/// parked on a gate.
pub fn live_signature(run: &Value) -> (String, String, Vec<String>) {
    let waiting = crate::runs::states(run)
        .filter(|n| s(n, "status") == Some("waiting"))
        .filter_map(|n| s(n, "nodeId").map(str::to_owned))
        .collect();
    (
        text_of(run.get("status")),
        text_of(run.get("completedAt")),
        waiting,
    )
}

/// The editor field that edits `path` on `node`, or on the workflow.
pub fn field_id(node: Option<&str>, path: &str) -> crate::ui::Id {
    crate::ui::Id::new("wf-field").child((node, path))
}

/// The run prompt's field for the input `key`.
pub fn prompt_id(key: &str) -> crate::ui::Id {
    crate::ui::Id::new("wf-prompt").child(key)
}

/// The run prompt's branch.
pub fn prompt_branch() -> crate::ui::Id {
    crate::ui::Id::new("wf-prompt-branch")
}

/// The step library's search.
pub fn search_id() -> crate::ui::Id {
    crate::ui::Id::new("wf-search")
}

impl Workflows {
    /// The landing's runs: the live copy of each, through the tab and filter,
    /// newest first.
    pub fn visible(&self, live: &HashMap<String, Value>) -> Vec<Value> {
        let mut out = self.merged(live);
        let want = match self.tab {
            Tab::Review => Bucket::Waiting,
            Tab::All => self.bucket,
        };
        out.retain(|r| want == Bucket::All || bucket(r) == want);
        out
    }

    /// Every run the landing knows, live copies first, newest first.
    pub fn merged(&self, live: &HashMap<String, Value>) -> Vec<Value> {
        let mut out: Vec<Value> = self
            .runs
            .iter()
            .map(|r| {
                s(r, "runId")
                    .and_then(|id| live.get(id))
                    .unwrap_or(r)
                    .clone()
            })
            .collect();
        for (id, r) in live {
            if !out.iter().any(|o| s(o, "runId") == Some(id.as_str())) {
                out.push(r.clone());
            }
        }
        out.sort_by(|a, b| s(b, "startedAt").cmp(&s(a, "startedAt")));
        out
    }

    /// The runs waiting on someone.
    pub fn waiting(&self, live: &HashMap<String, Value>) -> Vec<Value> {
        let mut out = self.merged(live);
        out.retain(|r| bucket(r) == Bucket::Waiting);
        out
    }

    /// Whether the editor's workflow is running, and since when.
    pub fn running_since(&self, live: &HashMap<String, Value>) -> Option<f64> {
        let d = self.editor.as_ref()?;
        self.active_run(d.id(), live)
            .and_then(|r| s(&r, "startedAt").and_then(crate::tasks::ms))
            .or(self.launching)
    }

    /// The run of workflow `id` still going, the newest first.
    pub fn active_run(&self, id: &str, live: &HashMap<String, Value>) -> Option<Value> {
        live.values()
            .chain(self.history.iter())
            .filter(|r| s(r, "workflowId") == Some(id) && runs::stoppable(r))
            .max_by(|a, b| s(a, "startedAt").cmp(&s(b, "startedAt")))
            .cloned()
    }

    /// The selected run, else the first one shown.
    pub fn current<'a>(&self, visible: &'a [Value]) -> Option<&'a Value> {
        self.selected
            .as_deref()
            .and_then(|k| visible.iter().find(|r| runs::key(r) == k))
            .or_else(|| visible.first())
    }
}

/// `bucketOf`: a paused run is waiting, not running.
pub fn bucket(run: &Value) -> Bucket {
    match s(run, "status") {
        Some("running" | "waiting") => {
            if runs::states(run).any(|n| s(n, "status") == Some("waiting")) {
                Bucket::Waiting
            } else {
                Bucket::Running
            }
        }
        Some("success") => Bucket::Success,
        _ => Bucket::Error,
    }
}

/// `failedStep`.
fn failed_step(run: &Value) -> Option<&Value> {
    runs::states(run).find(|n| s(n, "status") == Some("error"))
}

/// `runStatusLine`.
pub fn status_line(run: &Value, nodes: &Value) -> String {
    let named = |id: &str| runs::node_label(runs::node(nodes, id), id);
    if let Some(w) = runs::states(run).find(|n| s(n, "status") == Some("waiting")) {
        let id = s(w, "nodeId").unwrap_or("");
        let what = if is_sign_in(w) {
            "Waiting for sign-in"
        } else {
            "Waiting"
        };
        return format!("{what} at {}", named(id));
    }
    match s(run, "status") {
        Some("running") => runs::states(run)
            .find(|n| s(n, "status") == Some("running"))
            .map_or_else(
                || "Running".to_owned(),
                |n| format!("Running {}", named(s(n, "nodeId").unwrap_or(""))),
            ),
        Some("error") => match failed_step(run) {
            None => "Failed".to_owned(),
            Some(f) => {
                let id = s(f, "nodeId").unwrap_or("");
                if f.get("rejectedAt").is_none_or(Value::is_null) {
                    return format!("Failed at {}", named(id));
                }
                let last = f
                    .get("feedback")
                    .and_then(Value::as_array)
                    .and_then(|a| a.last());
                let note = last
                    .filter(|l| s(l, "decision") == Some("reject"))
                    .and_then(|l| s(l, "comment"))
                    .map(|c| format!(" \u{b7} {c}"))
                    .unwrap_or_default();
                format!("Rejected at {}{note}", named(id))
            }
        },
        Some("cancelled") => "Stopped".to_owned(),
        _ => "Completed".to_owned(),
    }
}

/// `isSignInWait`.
pub fn is_sign_in(state: &Value) -> bool {
    s(state, "status") == Some("waiting") && s(state, "waitingFor") == Some("signIn")
}

/// `stepProgress`: succeeded steps out of those reached, the trigger left out.
pub fn progress(run: &Value, nodes: &Value) -> (usize, usize) {
    let steps = runs::steps(run, nodes, false);
    let done = steps
        .iter()
        .filter(|n| s(n, "status") == Some("success"))
        .count();
    (done, steps.len())
}

/// `describeRun`'s title, subtitle, source label and glyphs.
pub struct Described {
    pub title: String,
    pub subtitle: Option<String>,
    /// `sourceLabel`.
    pub source: String,
    /// The connector a connector run draws when its workflow has no icon.
    pub connector: Option<String>,
    /// `SOURCE_ICONS[source]`, for a run whose workflow has no icon.
    pub fallback: &'static str,
}

/// `SOURCE_ICONS`.
fn source_icon(source: &str) -> &'static str {
    match source {
        "schedule" => "clock",
        "task" => "square-check-big",
        "connector" => "play",
        "restore" => "rotate-ccw",
        _ => "zap",
    }
}

/// `connectorTitle`: GitHub's issues and pull requests share one number
/// sequence, so the item's URL tells them apart.
fn connector_title(item: &Value, connector: &str) -> Option<String> {
    let id = s(item, "externalId").unwrap_or("");
    let url = s(item, "externalUrl").unwrap_or("");
    if url.contains("/pull/") {
        Some(format!("PR #{id}"))
    } else if url.contains("/issues/") {
        Some(format!("Issue #{id}"))
    } else {
        (!id.is_empty()).then(|| format!("{connector} {id}"))
    }
}

/// `describeRun`. `look_connector` is the connector the run's connection
/// resolves to, which a packaged connector's item only knows as `sdk`.
pub fn describe(run: &Value, wf: Option<&Value>, look_connector: Option<&str>) -> Described {
    let name = wf
        .and_then(|w| s(w, "name"))
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_owned);
    let trigger = wf
        .and_then(|w| trigger(w.get("nodes").and_then(Value::as_array)?))
        .and_then(|t| t.pointer("/config/triggerType"))
        .and_then(Value::as_str);
    if let Some(item) = run.get("connectorItem").filter(|v| v.is_object()) {
        let connector = look_connector
            .or_else(|| s(item, "connectorId"))
            .unwrap_or("")
            .to_owned();
        let item_title = s(item, "title").unwrap_or("");
        let title = connector_title(item, &connector).unwrap_or_else(|| item_title.to_owned());
        return Described {
            subtitle: if item_title != title {
                Some(item_title.to_owned())
            } else {
                name
            },
            title,
            source: connector.clone(),
            connector: Some(connector),
            fallback: source_icon("connector"),
        };
    }
    if let Some(ts) = run.get("triggerSession").filter(|v| v.is_object()) {
        let label = s(ts, "label").unwrap_or("");
        return Described {
            title: name.unwrap_or_else(|| label.to_owned()),
            subtitle: Some(format!(
                "restore \u{b7} {} \u{b7} {label}",
                s(ts, "restore").unwrap_or("")
            )),
            source: "restore".to_owned(),
            connector: None,
            fallback: source_icon("restore"),
        };
    }
    if let Some(task) = s(run, "triggerTaskId") {
        let short: String = task.chars().take(6).collect();
        return Described {
            title: name.unwrap_or_else(|| format!("Task {short}")),
            subtitle: Some(format!("Task {short}")),
            source: "task".to_owned(),
            connector: None,
            fallback: source_icon("task"),
        };
    }
    // `sourceOf`: a connector poll draws the play glyph but reads "manual".
    let (source, label) = match trigger {
        Some("once" | "recurring") => ("schedule", "scheduled"),
        Some("connectorPoll") => ("connector", "manual"),
        Some("taskCreated" | "taskStatusChanged") => ("task", "manual"),
        Some("sessionRestored") => ("restore", "restore"),
        _ => ("manual", "manual"),
    };
    Described {
        title: name.unwrap_or_else(|| s(run, "workflowId").unwrap_or("").chars().take(8).collect()),
        subtitle: None,
        source: label.to_owned(),
        connector: None,
        fallback: source_icon(source),
    }
}

/// The workflow's trigger node.
pub fn trigger(nodes: &[Value]) -> Option<&Value> {
    nodes.iter().find(|n| s(n, "type") == Some("trigger"))
}

fn trigger_type(wf: &Value) -> Option<&str> {
    trigger(wf.get("nodes")?.as_array()?)?
        .pointer("/config/triggerType")?
        .as_str()
}

/// `isScheduledWorkflow`: any trigger but a manual one.
pub fn is_scheduled(wf: &Value) -> bool {
    trigger_type(wf).is_some_and(|t| t != "manual")
}

/// `statusDotState`: the sidebar's dot for a workflow, the gate first.
pub fn side_status(wf: &Value, waiting: bool, failed: bool) -> Option<&'static str> {
    if waiting {
        Some("waiting")
    } else if is_scheduled(wf) {
        Some(
            if wf.get("enabled").and_then(Value::as_bool) == Some(false) {
                "disabled"
            } else {
                "scheduled"
            },
        )
    } else if failed {
        Some("error")
    } else {
        None
    }
}

/// `isContextual`: a manual trigger run against a session.
pub fn contextual(nodes: &[Value]) -> bool {
    trigger(nodes)
        .and_then(|t| t.get("config"))
        .is_some_and(|c| {
            s(c, "triggerType") == Some("manual")
                && c.get("contextual").and_then(Value::as_bool) == Some(true)
        })
}

/// The manual trigger's declared run inputs.
pub fn inputs(nodes: &[Value]) -> Vec<Value> {
    trigger(nodes)
        .and_then(|t| t.pointer("/config/inputs"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// `isValidInputKey`.
pub fn valid_key(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The inputs worth asking for.
pub fn asked(nodes: &[Value]) -> Vec<Value> {
    inputs(nodes)
        .into_iter()
        .filter(|i| valid_key(s(i, "key").unwrap_or("")))
        .collect()
}

/// `needsRunPrompt`.
pub fn needs_prompt(nodes: &[Value], has_source: bool) -> bool {
    (contextual(nodes) && !has_source) || !asked(nodes).is_empty()
}

fn context_ref(v: Option<&str>) -> bool {
    v.is_some_and(|v| {
        let t = v.trim();
        t.starts_with("{{") && t.ends_with("}}") && t.contains("context.")
    })
}

fn mentions(v: Option<&str>, field: &str) -> bool {
    v.is_some_and(|v| v.contains(&format!("context.{field}")))
}

/// `detectRequiredFields`: whether a contextual run asks for a branch and a
/// worktree.
pub fn required(nodes: &[Value]) -> (bool, bool) {
    let (mut branch, mut worktree) = (false, false);
    for n in nodes {
        let c = |k: &str| n.pointer(&format!("/config/{k}")).and_then(Value::as_str);
        match s(n, "type") {
            Some("launchAgent") => {
                branch |= context_ref(c("branch")) || mentions(c("prompt"), "branch");
                if c("useWorktree") == Some("fromContext") {
                    worktree = true;
                    branch = true;
                }
            }
            Some("script") => branch |= mentions(c("scriptContent"), "branch"),
            _ => {}
        }
    }
    (branch, worktree)
}

/// `areInputsValid`: each required input answered; a toggle always is.
pub fn inputs_valid(defs: &[Value], value: impl Fn(&str) -> String) -> bool {
    defs.iter().all(|d| {
        d.get("required").and_then(Value::as_bool) != Some(true)
            || s(d, "type") == Some("boolean")
            || !input_json(d, &value(s(d, "key").unwrap_or("")))
                .as_str()
                .is_some_and(|v| v.trim().is_empty())
    })
}

/// `defaultInputValue`, as the text a field starts with.
pub fn input_default(def: &Value) -> String {
    match s(def, "type") {
        Some("boolean") => (s(def, "defaultValue") == Some("true")).to_string(),
        _ => text_of(def.get("defaultValue")),
    }
}

/// An input's value as the run reads it: a toggle's bool, a number parsed
/// (blank when it does not parse).
pub fn input_json(def: &Value, raw: &str) -> Value {
    match s(def, "type").unwrap_or("text") {
        "boolean" => json!(raw == "true"),
        "number" => raw
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .map_or_else(|| json!(""), |n| json!(n)),
        _ => json!(raw),
    }
}

/// `slugify`: lower case, runs of anything else as one `_`.
pub fn slugify(label: &str) -> String {
    let mut out = String::new();
    for ch in label.to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let t = out.trim_matches('_');
    if t.is_empty() {
        "step".to_owned()
    } else {
        t.to_owned()
    }
}

/// `ensureUniqueSlug`.
pub fn unique_slug(base: &str, taken: &HashSet<String>) -> String {
    if !taken.contains(base) {
        return base.to_owned();
    }
    (2..)
        .map(|i| format!("{base}_{i}"))
        .find(|c| !taken.contains(c))
        .unwrap_or_default()
}

/// The library's triggers: kind, label and icon.
pub const TRIGGERS: [(&str, &str, &str); 7] = [
    ("manual", "Manual", "zap"),
    ("recurring", "Recurring schedule", "clock"),
    ("once", "Schedule once", "calendar"),
    ("taskCreated", "Task created", "list-plus"),
    ("taskStatusChanged", "Task moved", "arrow-right-left"),
    ("sessionRestored", "Session restored", "rotate-ccw"),
    ("webhook", "Webhook", "globe"),
];

/// The library's steps.
pub const STEPS: [(&str, &str, &str); 6] = [
    ("launchAgent", "Agent", "play"),
    ("script", "Script", "terminal"),
    ("httpRequest", "HTTP request", "globe"),
    ("condition", "Condition", "git-fork"),
    ("approval", "Approval gate", "hand"),
    ("loop", "Loop", "repeat"),
];

/// `TRIGGER_ICONS`.
pub fn trigger_icon(kind: &str) -> &'static str {
    match kind {
        "once" => "calendar",
        "recurring" => "clock",
        "taskCreated" => "list-plus",
        "taskStatusChanged" => "arrow-right-left",
        "sessionRestored" => "rotate-ccw",
        "connectorPoll" => "plug",
        "webhook" => "globe",
        _ => "zap",
    }
}

/// The trigger form's label and hint for each kind.
pub fn trigger_info(kind: &str) -> (&'static str, &'static str) {
    match kind {
        "manual" => ("Manual", "Run manually from the play button"),
        "once" => ("Once", "Runs once at the scheduled time"),
        "recurring" => ("Recurring", "Runs on a repeating schedule"),
        "taskCreated" => ("Task Created", "Fires when a new task is added"),
        "taskStatusChanged" => ("Status Change", "Fires when a task's status changes"),
        "sessionRestored" => (
            "Session Restored",
            "Fires when a session comes back after Vorn restarts",
        ),
        "connectorPoll" => ("Connector Poll", "Polls a connector for new items"),
        "webhook" => ("Webhook", "Fires when this machine receives a request"),
        _ => ("Trigger", ""),
    }
}

/// `switchTriggerType`.
pub fn trigger_config(kind: &str) -> Value {
    match kind {
        "once" => json!({ "triggerType": "once", "runAt": crate::tasks::now_iso() }),
        "recurring" => json!({ "triggerType": "recurring", "cron": "0 9 * * *" }),
        "connectorPoll" => json!({
            "triggerType": "connectorPoll", "connectionId": "", "event": "", "cron": "*/5 * * * *"
        }),
        "webhook" => {
            json!({ "triggerType": "webhook", "method": "POST", "token": crate::tasks::uuid() })
        }
        k => json!({ "triggerType": k }),
    }
}

/// A new workflow's defaults.
pub fn blank(workspace: Option<&str>) -> Value {
    let mut wf = json!({
        "id": crate::tasks::uuid(),
        "name": "New Workflow",
        "icon": "Workflow",
        "iconColor": "#3b82f6",
        "nodes": [],
        "edges": [],
        "enabled": true,
    });
    if let Some(w) = workspace {
        wf["workspaceId"] = json!(w);
    }
    wf
}

/// How a bound field's text becomes a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Conv {
    /// Text; empty removes the key.
    Text,
    /// Text; empty kept as an empty string.
    Keep,
    /// Whitespace-separated words into a list; empty removes it.
    Words,
    /// Space-separated words, empties dropped (a script's arguments).
    Spaces,
    /// Whole seconds into milliseconds; blank or not positive removes it.
    Seconds,
    /// Whole minutes into milliseconds; blank removes it.
    Minutes,
    /// A whole number, at least `0`; blank removes it.
    Count,
    /// A whole number clamped to 1..=10 (a loop's passes).
    Passes,
    /// A whole number clamped to 2..=10 (a gate's rounds).
    Rounds,
    /// A JSON object; text that is not one is kept aside, not stored.
    Schema,
    /// A run input's key, kept to the template grammar.
    Key,
    /// Comma-separated choices into `{value, label}` options.
    Options,
}

/// A text field's binding: which node (none for the workflow) and the
/// dotted path it writes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Bind {
    pub node: Option<String>,
    pub path: String,
    pub conv: Conv,
    /// Offers `{{` variables as it is typed in.
    pub vars: bool,
}

/// Writes `v` at the dotted `path` under `n` (`config.until.value`),
/// making the objects on the way; `None` removes the key.
pub fn set_path(n: &mut Value, path: &str, v: Option<Value>) {
    let parts: Vec<&str> = path.split('.').collect();
    let Some((last, head)) = parts.split_last() else {
        return;
    };
    let mut cur = n;
    for p in head {
        if !cur.get(*p).is_some_and(Value::is_object) {
            cur[*p] = json!({});
        }
        cur = &mut cur[*p];
    }
    match v {
        Some(v) => cur[*last] = v,
        None => {
            if let Some(m) = cur.as_object_mut() {
                m.remove(*last);
            }
        }
    }
}

/// The value at the dotted `path`.
pub fn get_path<'a>(n: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(n, |cur, p| cur.get(p))
}

/// What `text` stores under `conv`; `None` removes the key, `Err` keeps
/// the text aside.
pub fn convert(conv: Conv, text: &str) -> Result<Option<Value>, String> {
    let t = text.trim();
    let whole = |t: &str| {
        t.parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .map(f64::floor)
    };
    Ok(match conv {
        Conv::Text => (!text.is_empty()).then(|| json!(text)),
        Conv::Keep => Some(json!(text)),
        Conv::Words => (!t.is_empty()).then(|| json!(t.split_whitespace().collect::<Vec<_>>())),
        Conv::Spaces => {
            let w: Vec<&str> = text.split(' ').filter(|w| !w.is_empty()).collect();
            Some(json!(w))
        }
        Conv::Seconds => t
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite() && *n > 0.0)
            .map(|n| graph::num((n * 1000.0).round())),
        Conv::Minutes => {
            if t.is_empty() {
                None
            } else {
                let m = t
                    .parse::<f64>()
                    .ok()
                    .filter(|n| n.is_finite() && *n >= 0.0)
                    .ok_or("")?;
                Some(graph::num(m.round() * 60_000.0))
            }
        }
        Conv::Count => {
            if t.is_empty() {
                None
            } else {
                let n = t.parse::<i64>().ok().filter(|n| *n >= 0).ok_or("")?;
                Some(json!(n))
            }
        }
        Conv::Passes => Some(json!(whole(t).map_or(1.0, |n| n.clamp(1.0, 10.0)) as i64)),
        Conv::Rounds => Some(json!(whole(t).ok_or("")?.clamp(2.0, 10.0) as i64)),
        Conv::Schema => {
            if t.is_empty() {
                None
            } else {
                match serde_json::from_str::<Value>(t) {
                    Ok(v @ Value::Object(_)) => Some(v),
                    Ok(_) => return Err("Schema must be a JSON object".into()),
                    Err(_) => return Err("Not valid JSON".into()),
                }
            }
        }
        Conv::Key => {
            let mut k: String = text
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            if k.starts_with(|c: char| c.is_ascii_digit()) {
                k.insert(0, '_');
            }
            Some(json!(k))
        }
        Conv::Options => Some(json!(text
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|v| json!({ "value": v, "label": v }))
            .collect::<Vec<_>>())),
    })
}

/// `handleNodeLabelChange`: the label, a slug of it no other step has,
/// and every `steps.<old>.` reference renamed.
pub fn relabel(d: &mut Draft, id: &str, label: &str) {
    let taken: HashSet<String> = d
        .nodes()
        .iter()
        .filter(|n| s(n, "id") != Some(id))
        .filter_map(|n| s(n, "slug").map(str::to_owned))
        .collect();
    let new_slug = unique_slug(&slugify(label), &taken);
    let old = d.node(id).and_then(|n| s(n, "slug")).map(str::to_owned);
    let nodes: Vec<Value> = d
        .nodes()
        .iter()
        .cloned()
        .map(|mut n| {
            if s(&n, "id") == Some(id) {
                n["label"] = json!(label);
                n["slug"] = json!(new_slug);
            } else if let Some(o) = old.as_deref().filter(|o| *o != new_slug) {
                let from = format!("steps.{o}.");
                if let Some(c) = n.get("config") {
                    let text = c.to_string();
                    if text.contains(&from) {
                        let to = format!("steps.{new_slug}.");
                        if let Ok(v) = serde_json::from_str(&text.replace(&from, &to)) {
                            n["config"] = v;
                        }
                    }
                }
            }
            n
        })
        .collect();
    let edges = d.edges().to_vec();
    d.set_graph(nodes, edges);
}

/// What a step's template fields read: the steps upstream (and a loop's
/// own body), and the variables its form offers for `path`.
pub fn var_sources(
    wf: &Workflows,
    d: &Draft,
    node: &str,
    path: &str,
) -> (Vec<crate::vars::StepGroup>, Vec<crate::vars::Var>) {
    use crate::vars::{self as v, Category};
    let nodes = d.nodes();
    let Some(n) = d.node(node) else {
        return (Vec::new(), Vec::new());
    };
    let mut up = v::ancestors(nodes, d.edges(), node);
    if s(n, "type") == Some("loop") {
        for b in n
            .pointer("/config/bodyNodeIds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if let Some(m) = d.node(b) {
                up.push(m);
            }
        }
    }
    let latest = wf.history.iter().max_by(|a, b| {
        s(a, "startedAt")
            .unwrap_or("")
            .cmp(s(b, "startedAt").unwrap_or(""))
    });
    let last = latest.map(|r| v::last_run(r, nodes));
    let groups = v::step_groups(&up, &wf.lib.actions, last.as_ref());
    let trig = trigger(nodes);
    let ttype = trig
        .and_then(|t| t.pointer("/config/triggerType"))
        .and_then(Value::as_str);
    let contextual = ttype == Some("manual")
        && trig
            .and_then(|t| t.pointer("/config/contextual"))
            .and_then(Value::as_bool)
            == Some(true);
    let inputs: Vec<Value> = if ttype == Some("manual") {
        trig.and_then(|t| t.pointer("/config/inputs"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut node_vars = v::input_vars(&inputs);
    node_vars.extend(v::loop_vars(nodes, Some(node)));
    let custom = |connector_item: bool| -> Vec<v::Var> {
        v::template_variables()
            .into_iter()
            .filter(|var| match var.category {
                Category::Task => matches!(ttype, Some("taskCreated" | "taskStatusChanged")),
                Category::ConnectorItem => connector_item && ttype == Some("connectorPoll"),
                Category::Trigger => {
                    if ttype == Some("webhook") {
                        ["trigger.body", "trigger.headers", "trigger.query"]
                            .iter()
                            .any(|k| var.key.contains(k))
                    } else {
                        ttype == Some("taskStatusChanged") && var.key.contains("Status")
                    }
                }
                _ => false,
            })
            .collect()
    };
    let vars = match s(n, "type").unwrap_or("") {
        "condition" => [custom(false), node_vars].concat(),
        "callConnectorAction" => [custom(true), node_vars].concat(),
        "approval" => node_vars,
        "loop" if path == "config.items" => node_vars
            .into_iter()
            .filter(|x| x.category != Category::Loop)
            .collect(),
        "loop" => node_vars,
        _ => [v::context_vars(ttype, contextual), node_vars].concat(),
    };
    (groups, vars)
}

/// What the step library hands back (`LibraryPick`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Pick {
    /// A built-in step: `agent`, `script`, `httpRequest`, `condition`,
    /// `approval`, `loop` or `connectorAction`.
    Type(String),
    Parallel,
    TriggerType(String),
    ConnectorTrigger {
        connection: String,
        event: String,
    },
    ConnectorAction {
        connection: String,
        action: String,
        label: String,
    },
    CatalogAction {
        connector: String,
        action: String,
        label: String,
    },
    HttpProfile {
        connection: String,
        name: String,
    },
}

impl Pick {
    /// `stepForPick`: the node type and what it already knows.
    fn step(&self) -> Option<(&str, Value)> {
        Some(match self {
            Pick::Type(t) => (t.as_str(), json!({})),
            Pick::ConnectorAction {
                connection,
                action,
                label,
            } => (
                "connectorAction",
                json!({ "connectionId": connection, "action": action, "actionLabel": label }),
            ),
            Pick::CatalogAction {
                connector,
                action,
                label,
            } => (
                "connectorAction",
                json!({ "connectionId": "", "connectorId": connector, "action": action, "actionLabel": label }),
            ),
            Pick::HttpProfile { connection, .. } => {
                ("httpRequest", json!({ "profileConnectionId": connection }))
            }
            _ => return None,
        })
    }
}

/// The slugs in use, but `except`'s.
fn slugs(nodes: &[Value], except: Option<&str>) -> HashSet<String> {
    nodes
        .iter()
        .filter(|n| except != s(n, "id"))
        .filter_map(|n| s(n, "slug").map(str::to_owned))
        .collect()
}

/// `createNodeWithUniqueSlug`: an addable step, agents in the first project.
pub fn new_step(
    kind: &str,
    nodes: &[Value],
    project: Option<(&str, &str)>,
    agent: &str,
    except: Option<&str>,
) -> Option<Value> {
    let taken = slugs(nodes, except);
    Some(match kind {
        "agent" | "launchAgent" => graph::agent_node(project, agent, &taken),
        "script" => graph::script_node(&taken),
        "approval" => graph::approval_node(&taken),
        "loop" => graph::loop_node(&taken),
        "connectorAction" | "callConnectorAction" => graph::connector_action_node(&taken),
        "httpRequest" => graph::http_node(&taken),
        "condition" => {
            let mut c = graph::condition_node();
            c["slug"] = json!(unique_slug("condition", &taken));
            c
        }
        _ => return None,
    })
}

fn with_config(mut n: Value, preset: &Value) -> Value {
    if let (Some(c), Some(p)) = (
        n.get_mut("config").and_then(Value::as_object_mut),
        preset.as_object(),
    ) {
        for (k, v) in p {
            c.insert(k.clone(), v.clone());
        }
    }
    n
}

/// `handleLibraryPick`: what was picked, where the open anchor says; the
/// node to select after.
pub fn pick(
    d: &mut Draft,
    at: &Anchor,
    p: &Pick,
    project: Option<(&str, &str)>,
    agent: &str,
) -> Option<String> {
    let nodes = d.nodes().to_vec();
    let edges = d.edges().to_vec();
    if let Pick::TriggerType(_) | Pick::ConnectorTrigger { .. } = p {
        let config = match p {
            Pick::TriggerType(t) => trigger_config(t),
            Pick::ConnectorTrigger { connection, event } => json!({
                "triggerType": "connectorPoll", "connectionId": connection, "event": event,
                "cron": "*/5 * * * *"
            }),
            _ => return None,
        };
        if let Some(existing) = trigger(&nodes).cloned() {
            let cur = existing.get("config").cloned().unwrap_or_default();
            let same = match p {
                Pick::TriggerType(t) => s(&cur, "triggerType") == Some(t.as_str()),
                Pick::ConnectorTrigger { connection, event } => {
                    s(&cur, "triggerType") == Some("connectorPoll")
                        && s(&cur, "connectionId") == Some(connection.as_str())
                        && s(&cur, "event") == Some(event.as_str())
                }
                _ => false,
            };
            let eid = s(&existing, "id").unwrap_or("").to_owned();
            if same {
                return None;
            }
            let mut fresh = graph::trigger_node(config);
            fresh["id"] = existing["id"].clone();
            fresh["position"] = existing["position"].clone();
            if let Some(slug) = existing.get("slug") {
                fresh["slug"] = slug.clone();
            }
            let next = nodes
                .into_iter()
                .map(|n| {
                    if s(&n, "id") == Some(eid.as_str()) {
                        fresh.clone()
                    } else {
                        n
                    }
                })
                .collect();
            d.set_graph(next, edges);
            return Some(eid);
        }
        let t = graph::trigger_node(config);
        let tid = s(&t, "id").unwrap_or("").to_owned();
        let owners = graph::loop_body_owners(&nodes);
        let has_in: HashSet<&str> = edges.iter().filter_map(|e| s(e, "target")).collect();
        let mut next_edges = edges.clone();
        for n in &nodes {
            let nid = s(n, "id").unwrap_or("");
            if !has_in.contains(nid) && !owners.contains_key(nid) {
                next_edges.push(graph::edge(&tid, nid, None));
            }
        }
        let mut next = nodes.clone();
        next.push(t);
        d.set_graph(next, next_edges);
        return Some(tid);
    }
    if let Some(r) = &at.replace {
        let target = nodes
            .iter()
            .find(|n| s(n, "id") == Some(r.as_str()))?
            .clone();
        let fresh = match p {
            Pick::ConnectorAction {
                connection,
                action,
                label,
            } => {
                let n = new_step("connectorAction", &nodes, project, agent, Some(r))?;
                with_config(
                    n,
                    &json!({ "connectionId": connection, "action": action, "actionLabel": label }),
                )
            }
            Pick::Type(t) if t != "condition" && t != "loop" => {
                new_step(t, &nodes, project, agent, Some(r))?
            }
            _ => return None,
        };
        let mut swapped = fresh.clone();
        swapped["id"] = target["id"].clone();
        swapped["position"] = target["position"].clone();
        if let Some(slug) = target.get("slug") {
            swapped["slug"] = slug.clone();
        }
        let next = nodes
            .into_iter()
            .map(|n| {
                if s(&n, "id") == Some(r.as_str()) {
                    swapped.clone()
                } else {
                    n
                }
            })
            .collect();
        d.set_graph(next, edges);
        return Some(r.clone());
    }
    let after = at.after.clone()?;
    if !nodes.iter().any(|n| s(n, "id") == Some(after.as_str())) {
        return None;
    }
    if let Some(b) = at.before.as_deref() {
        if b != graph::FORK && b != graph::LOOP_BODY && !nodes.iter().any(|n| s(n, "id") == Some(b))
        {
            return None;
        }
    }
    if *p == Pick::Parallel {
        let n = new_step("agent", &nodes, project, agent, None)?;
        let nid = s(&n, "id").unwrap_or("").to_owned();
        let (nn, ne) = graph::add_parallel_branch(&nodes, &edges, &after, n);
        d.set_graph(nn, ne);
        return Some(nid);
    }
    let (kind, preset) = p.step()?;
    if let (Some(pos), None) = (at.position, &at.before) {
        return palette_insert(
            d,
            &after,
            kind,
            &preset,
            (pos.0 as f64, pos.1 as f64),
            project,
            agent,
        );
    }
    insert(
        d,
        &after,
        at.before.as_deref(),
        kind,
        &preset,
        project,
        agent,
    )
}

/// `handleInsertNode`.
fn insert(
    d: &mut Draft,
    after: &str,
    before: Option<&str>,
    kind: &str,
    preset: &Value,
    project: Option<(&str, &str)>,
    agent: &str,
) -> Option<String> {
    let nodes = d.nodes().to_vec();
    let edges = d.edges().to_vec();
    if kind == "condition" {
        let b = before.filter(|b| *b != graph::LOOP_BODY);
        let result = graph::insert_condition(&nodes, &edges, after, b, &slugs(&nodes, None));
        let (nn, ne) = graph::adopt_into_loop_body(&nodes, result, after, before);
        let cid = nn
            .iter()
            .find(|n| {
                s(n, "type") == Some("condition") && !nodes.iter().any(|o| o["id"] == n["id"])
            })
            .and_then(|n| s(n, "id"))
            .map(str::to_owned);
        d.set_graph(nn, ne);
        return cid;
    }
    let n = with_config(new_step(kind, &nodes, project, agent, None)?, preset);
    let nid = s(&n, "id").unwrap_or("").to_owned();
    let result = match before {
        Some(graph::LOOP_BODY) => {
            match graph::loop_owning(&nodes, after).and_then(|l| s(l, "id")) {
                Some(lid) => {
                    let lid = lid.to_owned();
                    let anchor = (after != lid).then_some(after);
                    graph::append_to_loop_body(&nodes, &edges, &lid, n, anchor)
                }
                None => graph::append_after(&nodes, &edges, after, n),
            }
        }
        Some(graph::FORK) => graph::insert_before_fork(&nodes, &edges, after, n),
        Some(b) => match edges
            .iter()
            .find(|e| s(e, "source") == Some(after) && s(e, "target") == Some(b))
            .and_then(|e| s(e, "id"))
        {
            Some(eid) => graph::insert_between(&nodes, &edges, eid, n),
            None => graph::append_after(&nodes, &edges, after, n),
        },
        None => graph::append_after(&nodes, &edges, after, n),
    };
    let (nn, ne) = graph::adopt_into_loop_body(&nodes, result, after, before);
    d.set_graph(nn, ne);
    Some(nid)
}

/// `handlePaletteInsert`: a step dropped where an edge was released, the
/// computed layout materialized first.
fn palette_insert(
    d: &mut Draft,
    after: &str,
    kind: &str,
    preset: &Value,
    pos: (f64, f64),
    project: Option<(&str, &str)>,
    agent: &str,
) -> Option<String> {
    let nodes = d.nodes().to_vec();
    let edges = d.edges().to_vec();
    let seed = graph::positions_are_seed(&nodes);
    let place = |placed: Vec<Value>, layout_edges: &[Value], new_id: Option<&str>| -> Vec<Value> {
        let base = if seed {
            let (positions, _) = crate::canvas::layout_positions(&placed, layout_edges);
            placed
                .into_iter()
                .map(|mut n| {
                    if let Some(p) = positions.get(s(&n, "id").unwrap_or("")) {
                        n["position"] = json!({ "x": graph::num(p.0), "y": graph::num(p.1) });
                    }
                    n
                })
                .collect()
        } else {
            placed
        };
        base.into_iter()
            .map(|mut n| {
                if new_id.is_some() && s(&n, "id") == new_id {
                    n["position"] = json!({ "x": graph::num(pos.0), "y": graph::num(pos.1) });
                }
                n
            })
            .collect()
    };
    if kind == "condition" {
        let (nn, ne) = graph::insert_condition(&nodes, &edges, after, None, &slugs(&nodes, None));
        let cid = nn
            .iter()
            .find(|n| {
                s(n, "type") == Some("condition") && !nodes.iter().any(|o| o["id"] == n["id"])
            })
            .and_then(|n| s(n, "id"))
            .map(str::to_owned);
        let placed = place(nn, &ne, cid.as_deref());
        d.set_graph(placed, ne);
        return cid;
    }
    let n = with_config(new_step(kind, &nodes, project, agent, None)?, preset);
    let nid = s(&n, "id").unwrap_or("").to_owned();
    let (nn, ne) = graph::append_after(&nodes, &edges, after, n);
    let placed = place(nn, &ne, Some(&nid));
    d.set_graph(placed, ne);
    Some(nid)
}

/// `handleDeleteNode`.
pub fn remove(d: &mut Draft, id: &str) {
    let (nodes, edges) = graph::remove(d.nodes(), d.edges(), id);
    d.set_graph(nodes, edges);
}

/// `handleConnectEdge`.
pub fn connect(d: &mut Draft, source: &str, target: &str) {
    let mut edges = d.edges().to_vec();
    edges.push(json!({ "id": crate::tasks::uuid(), "source": source, "target": target }));
    let nodes = d.nodes().to_vec();
    d.set_graph(nodes, edges);
}

/// `handlePositionsCommit`.
pub fn commit_positions(d: &mut Draft, positions: &HashMap<String, (f64, f64)>) {
    let nodes: Vec<Value> = d
        .nodes()
        .iter()
        .cloned()
        .map(|mut n| {
            if let Some(p) = positions.get(s(&n, "id").unwrap_or("")) {
                n["position"] = json!({ "x": graph::num(p.0), "y": graph::num(p.1) });
            }
            n
        })
        .collect();
    let edges = d.edges().to_vec();
    d.set_graph(nodes, edges);
}

/// `handleTidyUp`: the layout's positions written in.
pub fn tidy(d: &mut Draft) {
    let (positions, _) = crate::canvas::layout_positions(d.nodes(), d.edges());
    commit_positions(d, &positions);
}

/// The text of a config value, as a field shows it.
pub fn text_of(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// `OPERATOR_LABELS`.
fn operator_symbol(op: &str) -> &str {
    match op {
        "equals" => "=",
        "notEquals" => "!=",
        "contains" => "contains",
        "notContains" => "not contains",
        "isEmpty" => "is empty",
        "isNotEmpty" => "is not empty",
        o => o,
    }
}

fn cfg<'a>(node: &'a Value, key: &str) -> Option<&'a str> {
    node.get("config")
        .and_then(|c| c.get(key))
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
}

/// A canvas card's line under its label.
pub fn subtitle(node: &Value) -> String {
    let c = |k: &str| cfg(node, k).unwrap_or("");
    match s(node, "type").unwrap_or("") {
        "trigger" => match c("triggerType") {
            "manual" | "" => "Click to run".to_owned(),
            "once" => format!("Run at {}", crate::tasks::long_date(c("runAt"))),
            "recurring" => format!("Cron: {}", c("cron")),
            "taskCreated" => match cfg(node, "projectFilter") {
                Some(p) => format!("Project: {p}"),
                None => "Any project".to_owned(),
            },
            "taskStatusChanged" => {
                let parts: Vec<&str> = [cfg(node, "fromStatus"), cfg(node, "toStatus")]
                    .into_iter()
                    .flatten()
                    .collect();
                let t = match parts.as_slice() {
                    [a, b] => format!("{a} \u{2192} {b}"),
                    [a] => (*a).to_owned(),
                    _ => "Any change".to_owned(),
                };
                let p = cfg(node, "projectFilter")
                    .map(|p| format!(" \u{b7} {p}"))
                    .unwrap_or_default();
                t + &p
            }
            "sessionRestored" => format!(
                "{}{}",
                if c("restore") == "any" {
                    "cold or warm"
                } else {
                    "cold"
                },
                cfg(node, "projectFilter").map_or_else(
                    || " \u{b7} any project".to_owned(),
                    |p| format!(" \u{b7} {p}")
                )
            ),
            "connectorPoll" => format!("{} \u{b7} {}", c("event"), c("cron")),
            "webhook" => format!("{} \u{b7} this machine", c("method")),
            _ => String::new(),
        },
        "script" => match cfg(node, "projectName") {
            Some(p) => format!("{} \u{b7} {p}", c("scriptType")),
            None => c("scriptType").to_owned(),
        },
        "launchAgent" => {
            let mut t = cfg(node, "projectName").unwrap_or("No project").to_owned();
            if let Some(b) = cfg(node, "branch") {
                t += &format!(" \u{b7} {b}");
            }
            if let Some(m) = cfg(node, "model") {
                t += &format!(" \u{b7} {m}");
            }
            t
        }
        "approval" => match node.pointer("/config/timeoutMs").and_then(Value::as_f64) {
            Some(ms) if ms > 0.0 => format!(
                "Waits for approval \u{b7} {}s timeout",
                (ms / 1000.0).round()
            ),
            _ => "Waits for approval".to_owned(),
        },
        "condition" if cfg(node, "variable").is_some() => operator_symbol(c("operator")).to_owned(),
        "condition" => "Not configured".to_owned(),
        "httpRequest" => match cfg(node, "url") {
            Some(u) => format!("{} {u}", c("method")),
            None => "Set URL".to_owned(),
        },
        "callConnectorAction" => cfg(node, "actionLabel")
            .or_else(|| cfg(node, "action"))
            .unwrap_or("Select action")
            .to_owned(),
        "createTaskFromItem" => {
            let p = match c("project") {
                "fromConnection" => "Project from connection",
                other => other,
            };
            format!("{p} \u{b7} initial: {}", c("initialStatus"))
        }
        "loop" => {
            if c("mode") == "forEach" {
                "each".to_owned()
            } else {
                let n = node
                    .pointer("/config/maxIterations")
                    .and_then(Value::as_u64)
                    .unwrap_or(1);
                format!("max {n}")
            }
        }
        _ => String::new(),
    }
}

/// A canvas card's footer: the script's first line, the prompt, the
/// message, the condition or the body.
pub fn footer(node: &Value) -> Option<String> {
    // `truncate` in node-visuals: cut and three dots.
    let cut = |t: &str, n: usize| {
        if t.chars().count() > n {
            format!("{}...", t.chars().take(n).collect::<String>())
        } else {
            t.to_owned()
        }
    };
    match s(node, "type").unwrap_or("") {
        "script" => node
            .pointer("/config/scriptContent")
            .and_then(Value::as_str)
            .and_then(|c| {
                c.split('\n')
                    .find(|l| !l.trim().is_empty() && !l.starts_with('#'))
            })
            .map(|l| cut(l.trim(), 50)),
        "launchAgent" => match node.pointer("/config/prompt").and_then(Value::as_str) {
            Some(p) if !p.is_empty() => Some(cut(p, 60)),
            _ if node
                .pointer("/config/taskFromQueue")
                .and_then(Value::as_bool)
                == Some(true) =>
            {
                Some("Next task from queue".to_owned())
            }
            _ if cfg(node, "taskId").is_some() => Some("From task".to_owned()),
            _ => None,
        },
        "approval" => node
            .pointer("/config/message")
            .and_then(Value::as_str)
            .filter(|m| !m.is_empty())
            .map(|m| cut(m, 60)),
        "httpRequest" => cfg(node, "body").map(|b| cut(b.trim(), 50)),
        "condition" => {
            let v = cfg(node, "variable")?;
            let op = cfg(node, "operator")?;
            let val = match (op, cfg(node, "value")) {
                ("isEmpty" | "isNotEmpty", _) | (_, None) => String::new(),
                (_, Some(v)) => format!(" \"{v}\""),
            };
            Some(cut(&format!("{v} {}{val}", operator_symbol(op)), 60))
        }
        "callConnectorAction" => {
            crate::canvas::args_preview(node.pointer("/config/args")).map(|p| cut(&p, 50))
        }
        _ => None,
    }
}

/// Sets `path` (`label`, `config.key`, `input.N.key`, `onError`, `name`…)
/// on the draft's node `node` to `value`.
pub fn set(d: &mut Draft, node: Option<&str>, path: &str, value: Value) {
    let mut parts = path.split('.');
    let head = parts.next().unwrap_or("");
    if node.is_none() {
        match head {
            "name"
            | "enabled"
            | "staggerDelayMs"
            | "autoCleanupWorktrees"
            | "icon"
            | "iconColor" => {
                if value.is_null() {
                    if let Some(m) = d.wf.as_object_mut() {
                        m.remove(head);
                    }
                } else {
                    d.wf[head] = value;
                }
            }
            _ => {}
        }
        return;
    }
    let Some(n) = node.and_then(|id| d.node_mut(id)) else {
        return;
    };
    match head {
        "label" => {
            n["label"] = value;
        }
        "onError" => {
            n["onError"] = value;
        }
        "config" => {
            let Some(key) = parts.next() else { return };
            if !n.get("config").is_some_and(Value::is_object) {
                n["config"] = json!({});
            }
            if value.is_null() {
                if let Some(m) = n["config"].as_object_mut() {
                    m.remove(key);
                }
            } else {
                n["config"][key] = value;
            }
        }
        "input" => {
            let (Some(i), Some(prop)) = (
                parts.next().and_then(|i| i.parse::<usize>().ok()),
                parts.next(),
            ) else {
                return;
            };
            let Some(list) = n
                .pointer_mut("/config/inputs")
                .and_then(Value::as_array_mut)
            else {
                return;
            };
            let Some(item) = list.get_mut(i).and_then(Value::as_object_mut) else {
                return;
            };
            if value.is_null() || value.as_str() == Some("") && prop != "key" {
                item.remove(prop);
            } else {
                item.insert(prop.to_owned(), value);
            }
        }
        _ => {}
    }
}

/// `WorkflowInputsEditor`'s Add input: a fresh key, as text.
pub fn add_input(d: &mut Draft, node: &str) {
    let Some(n) = d.node_mut(node) else { return };
    if !n.get("config").is_some_and(Value::is_object) {
        n["config"] = json!({});
    }
    let list = n["config"]
        .as_object_mut()
        .map(|c| c.entry("inputs").or_insert_with(|| json!([])));
    let Some(Value::Array(list)) = list else {
        return;
    };
    let taken: HashSet<String> = list
        .iter()
        .filter_map(|i| s(i, "key").map(str::to_owned))
        .collect();
    let key = unique_slug("input", &taken);
    let mut item = Map::new();
    item.insert("key".into(), json!(key));
    item.insert("label".into(), json!(""));
    item.insert("type".into(), json!("text"));
    list.push(Value::Object(item));
}

pub fn remove_input(d: &mut Draft, node: &str, i: usize) {
    if let Some(list) = d
        .node_mut(node)
        .and_then(|n| n.pointer_mut("/config/inputs"))
        .and_then(Value::as_array_mut)
    {
        if i < list.len() {
            list.remove(i);
        }
    }
}

/// The keys more than one input uses.
pub fn duplicate_keys(defs: &[Value]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut dup = Vec::new();
    for d in defs {
        let k = s(d, "key").unwrap_or("").to_owned();
        if !k.is_empty() && !seen.insert(k.clone()) && !dup.contains(&k) {
            dup.push(k);
        }
    }
    dup
}

/// What the workflow controls do.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WfAct {
    Tab(Tab),
    Bucket(Bucket),
    Refresh,
    SelectRun(String),
    /// The run's workflow in the editor.
    OpenRun(String),
    SignIn(String),
    Side(SideFilter),
    /// The sidebar section's fold.
    Fold,
    AllRuns,
    Edit(String),
    New,
    Run(String),
    ToggleSchedule(String),
    Export(String),
    Import,
    Delete(String),
    /// The editor's controls.
    Close,
    Save,
    RunDraft,
    Stop,
    History,
    Settings,
    SelectNode(String),
    Library(Anchor),
    Pick(Pick),
    DeleteNode(String),
    RunTo(String),
    Toggle(String),
    AddInput,
    RemoveInput(usize),
    StartBlank,
    /// A start-from template, by its catalog index.
    Template(usize),
    /// A connection's default workflow, by suggestion key.
    Suggest(String),
    /// A requirement row's button.
    Fix(crate::templates::Fix),
    /// Stop asking about what an import left unbound.
    DismissNeeds,
    ClosePanel,
    /// The run prompt.
    PromptProject(String),
    PromptWorktree,
    PromptSet(String, String),
    PromptRun,
    /// The missed schedules.
    MissedToggle(usize),
    MissedRun,
    MissedSkip,
    /// The canvas controls.
    ZoomIn,
    ZoomOut,
    Fit,
    Tidy,
    /// Undo, or redo when true.
    Undo(bool),
    /// A library group folded open or shut.
    Group(String),
    /// The library line under the pointer, for Enter.
    Highlight(usize),
    /// The icon picker's glyph and colour.
    Icon(String),
    IconColor(String),
    /// Typing in a bound field.
    Text(Bind),
    /// A picker's choice: the path it writes and the value.
    Choose(Option<String>, String, String),
    /// A form's Advanced section folded open or shut.
    Advanced(String),
    /// A variable picked from the `{{` dropdown, by its pattern.
    Insert(String),
    /// The `{{` button beside a field.
    Vars(Bind),
    /// A variable group folded in the dropdown.
    VarGroup(String),
    /// A header row added or removed.
    AddHeader(String),
    RemoveHeader(String, usize),
    /// A connector argument switched between its list and a template.
    ArgFree(String, String, bool),
    /// Copies text (the webhook URL).
    Copy(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wf(nodes: Value, edges: Value) -> Draft {
        Draft {
            wf: json!({ "id": "w", "nodes": nodes, "edges": edges }),
            existing: true,
        }
    }

    fn n(id: &str, kind: &str) -> Value {
        json!({ "id": id, "type": kind, "label": id, "config": {} })
    }

    fn e(a: &str, b: &str) -> Value {
        json!({ "id": format!("{a}{b}"), "source": a, "target": b })
    }

    #[test]
    fn runs_fall_into_the_renderers_buckets() {
        let waiting = json!({ "status": "running", "nodeStates": [{ "status": "waiting" }] });
        assert_eq!(bucket(&waiting), Bucket::Waiting);
        assert_eq!(bucket(&json!({ "status": "running" })), Bucket::Running);
        assert_eq!(bucket(&json!({ "status": "cancelled" })), Bucket::Error);
        assert_eq!(bucket(&json!({ "status": "success" })), Bucket::Success);
    }

    #[test]
    fn status_lines_name_the_step() {
        let nodes = json!([{ "id": "g", "label": "Review" }]);
        let wait =
            json!({ "status": "running", "nodeStates": [{ "nodeId": "g", "status": "waiting" }] });
        assert_eq!(status_line(&wait, &nodes), "Waiting at Review");
        let sign = json!({ "status": "running", "nodeStates": [{ "nodeId": "g", "status": "waiting", "waitingFor": "signIn" }] });
        assert_eq!(status_line(&sign, &nodes), "Waiting for sign-in at Review");
        let failed =
            json!({ "status": "error", "nodeStates": [{ "nodeId": "g", "status": "error" }] });
        assert_eq!(status_line(&failed, &nodes), "Failed at Review");
        let rejected = json!({ "status": "error", "nodeStates": [{ "nodeId": "g", "status": "error", "rejectedAt": "x",
            "feedback": [{ "decision": "reject", "comment": "no" }] }] });
        assert_eq!(
            status_line(&rejected, &nodes),
            "Rejected at Review \u{b7} no"
        );
        assert_eq!(
            status_line(&json!({ "status": "cancelled" }), &nodes),
            "Stopped"
        );
        assert_eq!(
            status_line(&json!({ "status": "success" }), &nodes),
            "Completed"
        );
    }

    #[test]
    fn a_run_is_described_by_its_source() {
        let run = json!({ "workflowId": "abcdefghij", "triggerTaskId": "t1234567" });
        let d = describe(&run, None, None);
        assert_eq!(
            (d.title.as_str(), d.source.as_str()),
            ("Task t12345", "task")
        );
        assert_eq!(d.fallback, "square-check-big");
        let w = json!({ "name": "Nightly", "nodes": [{ "type": "trigger", "config": { "triggerType": "recurring" } }] });
        let d = describe(&json!({ "workflowId": "x" }), Some(&w), None);
        assert_eq!(
            (d.title.as_str(), d.source.as_str()),
            ("Nightly", "scheduled")
        );
        assert_eq!(d.fallback, "clock");
    }

    #[test]
    fn a_connector_poll_draws_play_but_reads_manual() {
        let w = json!({ "name": "Poll", "nodes": [{ "type": "trigger", "config": { "triggerType": "connectorPoll" } }] });
        let d = describe(&json!({ "workflowId": "x" }), Some(&w), None);
        assert_eq!((d.source.as_str(), d.fallback), ("manual", "play"));
        assert_eq!(d.connector, None);
    }

    #[test]
    fn a_connector_run_is_titled_by_its_item() {
        let w = json!({ "name": "Triage" });
        let item = |url: &str| {
            json!({ "connectorItem": {
                "connectorId": "sdk", "connectionId": "c1", "externalId": "42",
                "externalUrl": url, "title": "Fix the build",
            } })
        };
        let pr = describe(
            &item("https://github.com/o/r/pull/42"),
            Some(&w),
            Some("github"),
        );
        assert_eq!(pr.title, "PR #42");
        assert_eq!(pr.subtitle.as_deref(), Some("Fix the build"));
        assert_eq!(pr.source, "github");
        assert_eq!(pr.connector.as_deref(), Some("github"));
        assert_eq!(pr.fallback, "play");
        let issue = describe(&item("https://github.com/o/r/issues/42"), Some(&w), None);
        assert_eq!(
            (issue.title.as_str(), issue.source.as_str()),
            ("Issue #42", "sdk")
        );
        let other = describe(&item("https://x.test/t/42"), Some(&w), Some("linear"));
        assert_eq!(other.title, "linear 42");
        let bare = json!({ "connectorItem": { "connectorId": "linear", "title": "Fix" } });
        let d = describe(&bare, Some(&w), None);
        assert_eq!(
            (d.title.as_str(), d.subtitle.as_deref()),
            ("Fix", Some("Triage"))
        );
    }

    #[test]
    fn slugs_are_unique_and_tidy() {
        assert_eq!(slugify("Execute Script"), "execute_script");
        assert_eq!(slugify("--"), "step");
        let taken = HashSet::from(["input".to_owned(), "input_2".to_owned()]);
        assert_eq!(unique_slug("input", &taken), "input_3");
        assert!(valid_key("my_key1") && !valid_key("1x") && !valid_key(""));
    }

    fn after(a: &str, b: Option<&str>) -> Anchor {
        Anchor {
            after: Some(a.into()),
            before: b.map(Into::into),
            ..Anchor::default()
        }
    }

    fn ends(d: &Draft) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = d
            .edges()
            .iter()
            .map(|e| {
                (
                    s(e, "source").unwrap().into(),
                    s(e, "target").unwrap().into(),
                )
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn a_condition_forks_and_rejoins_and_goes_with_its_branches() {
        let mut d = wf(
            json!([n("t", "trigger"), n("b", "script")]),
            json!([e("t", "b")]),
        );
        let cid = pick(
            &mut d,
            &after("t", Some("b")),
            &Pick::Type("condition".into()),
            None,
            "acme",
        )
        .unwrap();
        assert_eq!(d.nodes().len(), 5);
        let rows = graph::compute_flow_layout(d.nodes(), d.edges());
        assert!(matches!(&rows[2], graph::FlowRow::Fork { branches, .. } if branches.len() == 2));
        remove(&mut d, &cid);
        assert_eq!(d.nodes().len(), 2);
        assert_eq!(ends(&d), [("t".into(), "b".into())]);
    }

    #[test]
    fn steps_insert_append_replace_and_remove() {
        let mut d = wf(
            json!([n("t", "trigger"), n("b", "script")]),
            json!([e("t", "b")]),
        );
        let id = pick(
            &mut d,
            &after("t", Some("b")),
            &Pick::Type("approval".into()),
            None,
            "acme",
        )
        .unwrap();
        assert_eq!(
            ends(&d),
            [(id.clone(), "b".into()), ("t".into(), id.clone())]
        );
        let end = pick(
            &mut d,
            &after("b", None),
            &Pick::Type("script".into()),
            None,
            "acme",
        )
        .unwrap();
        assert_eq!(s(d.node(&end).unwrap(), "slug"), Some("execute_script"));
        remove(&mut d, &id);
        assert!(ends(&d).contains(&("t".into(), "b".into())));
        let r = Anchor {
            replace: Some("b".into()),
            ..after("b", None)
        };
        pick(&mut d, &r, &Pick::Type("httpRequest".into()), None, "acme");
        assert_eq!(s(d.node("b").unwrap(), "type"), Some("httpRequest"));
        let p = pick(
            &mut d,
            &after("t", None),
            &Pick::Parallel,
            Some(("web", "/w")),
            "acme",
        )
        .unwrap();
        assert_eq!(d.node(&p).unwrap()["config"]["projectName"], "web");
    }

    #[test]
    fn a_trigger_goes_above_the_roots_and_is_swapped_in_place() {
        let mut d = wf(json!([n("b", "script")]), json!([]));
        let at = Anchor {
            trigger: true,
            ..Anchor::default()
        };
        let t = pick(
            &mut d,
            &at,
            &Pick::TriggerType("recurring".into()),
            None,
            "acme",
        )
        .unwrap();
        assert_eq!(
            s(d.node(&t).unwrap(), "label"),
            Some("Schedule (Recurring)")
        );
        assert!(d.edges().iter().any(|e| s(e, "source") == Some(t.as_str())));
        assert_eq!(subtitle(d.node(&t).unwrap()), "Cron: 0 9 * * *");
        assert_eq!(
            pick(
                &mut d,
                &at,
                &Pick::TriggerType("recurring".into()),
                None,
                "acme"
            ),
            None
        );
        let again = pick(
            &mut d,
            &at,
            &Pick::TriggerType("manual".into()),
            None,
            "acme",
        )
        .unwrap();
        assert_eq!(again, t, "the same node, its edges kept");
        assert_eq!(s(d.node(&t).unwrap(), "label"), Some("Manual Trigger"));
    }

    #[test]
    fn a_step_dropped_off_an_edge_lands_where_it_was_let_go() {
        let mut d = wf(json!([n("t", "trigger")]), json!([]));
        let at = Anchor {
            position: Some((400, 320)),
            ..after("t", None)
        };
        let id = pick(&mut d, &at, &Pick::Type("script".into()), None, "acme").unwrap();
        assert_eq!(graph::position(d.node(&id).unwrap()), (400.0, 320.0));
        assert_ne!(
            graph::position(d.node("t").unwrap()).0,
            0.0,
            "the layout materialized"
        );
    }

    #[test]
    fn inputs_decide_the_prompt() {
        let nodes = vec![
            json!({ "type": "trigger", "config": { "triggerType": "manual",
            "inputs": [{ "key": "ticket", "type": "number", "required": true }, { "key": "1bad" }] } }),
        ];
        assert!(needs_prompt(&nodes, true));
        assert_eq!(asked(&nodes).len(), 1);
        let defs = asked(&nodes);
        assert!(!inputs_valid(&defs, |_| String::new()));
        assert!(!inputs_valid(&defs, |_| "x".into()));
        assert_eq!(input_json(&defs[0], "x"), json!(""));
        assert!(inputs_valid(&defs, |_| "12".into()));
        assert_eq!(input_json(&defs[0], "12"), json!(12.0));
    }

    #[test]
    fn inputs_are_added_edited_and_removed() {
        let mut d = wf(
            json!([{ "id": "t", "type": "trigger", "config": { "triggerType": "manual" } }]),
            json!([]),
        );
        add_input(&mut d, "t");
        add_input(&mut d, "t");
        set(&mut d, Some("t"), "input.1.key", json!("input"));
        let defs = inputs(d.nodes());
        assert_eq!(duplicate_keys(&defs), ["input"]);
        remove_input(&mut d, "t", 0);
        assert_eq!(inputs(d.nodes()).len(), 1);
    }
}
