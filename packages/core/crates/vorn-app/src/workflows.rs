//! Workflows as the renderer's landing, sidebar and editor read them: run
//! buckets and status lines (`run-presentation`), the canvas's flow and its
//! edits (`workflow-helpers`), the node factories and the run inputs.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Map, Value};

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
    pub before: Option<String>,
    /// The node the pick replaces in place.
    pub replace: Option<String>,
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

    fn set_graph(&mut self, nodes: Vec<Value>, edges: Vec<Value>) {
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
    /// The export dialog's workflow; none for import.
    pub file: Option<String>,
    /// `connection:list`, for naming a sign-in.
    pub connections: Vec<Value>,
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

/// The export or import dialog's path.
pub fn file_id() -> crate::ui::Id {
    crate::ui::Id::new("wf-file")
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
    let steps = runs::steps(run, nodes);
    let done = steps
        .iter()
        .filter(|n| s(n, "status") == Some("success"))
        .count();
    (done, steps.len())
}

/// `describeRun`'s title, subtitle and source label.
pub struct Described {
    pub title: String,
    pub subtitle: Option<String>,
    pub source: &'static str,
}

pub fn describe(run: &Value, wf: Option<&Value>) -> Described {
    let name = wf
        .and_then(|w| s(w, "name"))
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_owned);
    let trigger = wf
        .and_then(|w| trigger(w.get("nodes").and_then(Value::as_array)?))
        .and_then(|t| t.pointer("/config/triggerType"))
        .and_then(Value::as_str);
    if let Some(ts) = run.get("triggerSession").filter(|v| v.is_object()) {
        let label = s(ts, "label").unwrap_or("");
        return Described {
            title: name.unwrap_or_else(|| label.to_owned()),
            subtitle: Some(format!(
                "restore \u{b7} {} \u{b7} {label}",
                s(ts, "restore").unwrap_or("")
            )),
            source: "restore",
        };
    }
    if let Some(task) = s(run, "triggerTaskId") {
        let short: String = task.chars().take(6).collect();
        return Described {
            title: name.unwrap_or_else(|| format!("Task {short}")),
            subtitle: Some(format!("Task {short}")),
            source: "task",
        };
    }
    let source = match trigger {
        Some("once" | "recurring") => "scheduled",
        Some("taskCreated" | "taskStatusChanged") => "task",
        Some("sessionRestored") => "restore",
        _ => "manual",
    };
    Described {
        title: name.unwrap_or_else(|| s(run, "workflowId").unwrap_or("").chars().take(8).collect()),
        subtitle: None,
        source,
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

/// A once or recurring trigger.
pub fn is_scheduled(wf: &Value) -> bool {
    matches!(trigger_type(wf), Some("once" | "recurring"))
}

/// The sidebar's status dot for a workflow: waiting, scheduled, disabled
/// or error.
pub fn side_status(wf: &Value, waiting: bool, failed: bool) -> Option<&'static str> {
    if waiting {
        Some("waiting")
    } else if failed {
        Some("error")
    } else if is_scheduled(wf) && wf.get("enabled").and_then(Value::as_bool) == Some(false) {
        Some("disabled")
    } else if is_scheduled(wf) {
        Some("scheduled")
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

fn trigger_label(kind: &str) -> &'static str {
    match kind {
        "manual" => "Manual Trigger",
        "once" => "Schedule (Once)",
        "recurring" => "Schedule (Recurring)",
        "taskCreated" => "When Task Created",
        "taskStatusChanged" => "When Task Status Changes",
        "connectorPoll" => "Connector Poll",
        "webhook" => "Webhook",
        _ => "Trigger",
    }
}

/// `createTriggerNode`.
pub fn trigger_node(kind: &str) -> Value {
    json!({
        "id": crate::tasks::uuid(),
        "type": "trigger",
        "label": trigger_label(kind),
        "config": trigger_config(kind),
        "position": { "x": 0, "y": 0 },
    })
}

/// The `create*Node` factories, with a slug no other node has.
pub fn step_node(kind: &str, nodes: &[Value]) -> Option<Value> {
    let (label, config) = match kind {
        "launchAgent" => (
            "Launch Agent",
            json!({ "agentType": "claude", "projectName": "", "projectPath": "", "headless": true }),
        ),
        "script" => (
            "Execute Script",
            json!({
                "scriptType": "bash", "scriptContent": "# Write your script here\n",
                "projectName": "", "projectPath": ""
            }),
        ),
        "approval" => ("Approval Gate", json!({ "message": "" })),
        "condition" => (
            "Condition",
            json!({ "variable": "", "operator": "equals", "value": "" }),
        ),
        "httpRequest" => (
            "HTTP Request",
            json!({ "nodeType": "httpRequest", "method": "GET", "url": "", "headers": {}, "body": "" }),
        ),
        "loop" => (
            "Repeat Steps",
            json!({ "nodeType": "loop", "bodyNodeIds": [], "maxIterations": 2 }),
        ),
        _ => return None,
    };
    let taken: HashSet<String> = nodes
        .iter()
        .filter_map(|n| s(n, "slug").map(str::to_owned))
        .collect();
    Some(json!({
        "id": crate::tasks::uuid(),
        "type": kind,
        "label": label,
        "slug": unique_slug(&slugify(label), &taken),
        "config": config,
        "position": { "x": 0, "y": 0 },
    }))
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

fn edge(source: &str, target: &str, branch: Option<&str>) -> Value {
    let mut e = json!({ "id": crate::tasks::uuid(), "source": source, "target": target });
    if let Some(b) = branch {
        e["conditionBranch"] = json!(b);
    }
    e
}

fn successors(edges: &[Value]) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<(u8, String)>> = HashMap::new();
    for e in edges {
        let (Some(a), Some(b)) = (s(e, "source"), s(e, "target")) else {
            continue;
        };
        let rank = match s(e, "conditionBranch") {
            Some("true") => 0,
            Some("false") => 1,
            _ => 2,
        };
        map.entry(a.to_owned())
            .or_default()
            .push((rank, b.to_owned()));
    }
    map.into_iter()
        .map(|(k, mut v)| {
            v.sort_by_key(|(r, _)| *r);
            (k, v.into_iter().map(|(_, t)| t).collect())
        })
        .collect()
}

fn reach(start: &str, succ: &HashMap<String, Vec<String>>) -> HashSet<String> {
    let mut seen = HashSet::new();
    let mut queue = vec![start.to_owned()];
    while let Some(n) = queue.pop() {
        if seen.insert(n.clone()) {
            queue.extend(succ.get(&n).into_iter().flatten().cloned());
        }
    }
    seen
}

/// `findJoinPoint`: the first node every branch reaches.
fn join(children: &[String], succ: &HashMap<String, Vec<String>>) -> Option<String> {
    if children.len() < 2 {
        return None;
    }
    let sets: Vec<HashSet<String>> = children.iter().map(|c| reach(c, succ)).collect();
    let mut visited = HashSet::new();
    let mut queue: std::collections::VecDeque<String> = children.iter().cloned().collect();
    while let Some(cur) = queue.pop_front() {
        if !visited.insert(cur.clone()) {
            continue;
        }
        if !children.contains(&cur) && sets.iter().all(|s| s.contains(&cur)) {
            return Some(cur);
        }
        queue.extend(succ.get(&cur).into_iter().flatten().cloned());
    }
    None
}

/// One row of the canvas, top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// A node, and the edge from it down to the next row when there is one.
    Node { id: String, next: Option<String> },
    /// A fork's branches side by side, each with its tag, until they join.
    Fork {
        from: String,
        branches: Vec<(Option<String>, Vec<Row>)>,
    },
}

/// `computeFlowLayout`, without the loop frames: the trigger's chain, a
/// fork's branches side by side.
pub fn flow(nodes: &[Value], edges: &[Value]) -> Vec<Row> {
    let succ = successors(edges);
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    let has_in: HashSet<&str> = edges.iter().filter_map(|e| s(e, "target")).collect();
    let roots: Vec<&str> = match trigger(nodes).and_then(|t| s(t, "id")) {
        Some(t) => vec![t],
        None => nodes
            .iter()
            .filter_map(|n| s(n, "id"))
            .filter(|id| !has_in.contains(id))
            .collect(),
    };
    for r in roots {
        rows.extend(chain(r, None, &succ, edges, &mut seen));
    }
    for n in nodes.iter().filter_map(|n| s(n, "id")) {
        if !seen.contains(n) {
            rows.extend(chain(n, None, &succ, edges, &mut seen));
        }
    }
    rows
}

fn branch_tag(edges: &[Value], from: &str, to: &str) -> Option<String> {
    edges
        .iter()
        .find(|e| s(e, "source") == Some(from) && s(e, "target") == Some(to))
        .and_then(|e| s(e, "conditionBranch"))
        .map(str::to_owned)
}

fn chain(
    start: &str,
    stop: Option<&str>,
    succ: &HashMap<String, Vec<String>>,
    edges: &[Value],
    seen: &mut HashSet<String>,
) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut cur = start.to_owned();
    loop {
        if Some(cur.as_str()) == stop || !seen.insert(cur.clone()) {
            break;
        }
        let next = succ.get(&cur).cloned().unwrap_or_default();
        match next.len() {
            0 => {
                rows.push(Row::Node {
                    id: cur,
                    next: None,
                });
                break;
            }
            1 => {
                let n = next[0].clone();
                let down = (Some(n.as_str()) != stop && !seen.contains(&n)).then(|| n.clone());
                rows.push(Row::Node {
                    id: cur.clone(),
                    next: down,
                });
                cur = n;
            }
            _ => {
                rows.push(Row::Node {
                    id: cur.clone(),
                    next: None,
                });
                let j = join(&next, succ);
                let branches = next
                    .iter()
                    .map(|c| {
                        let tag = branch_tag(edges, &cur, c);
                        (tag, chain(c, j.as_deref().or(stop), succ, edges, seen))
                    })
                    .collect();
                rows.push(Row::Fork {
                    from: cur.clone(),
                    branches,
                });
                match j {
                    Some(j) => cur = j,
                    None => break,
                }
            }
        }
    }
    rows
}

/// `insertNodeBetween`: the edge split, its branch tag kept on the first half.
pub fn insert_between(d: &mut Draft, after: &str, before: &str, node: Value) {
    let nid = s(&node, "id").unwrap_or("").to_owned();
    let mut edges = d.edges().to_vec();
    let Some(i) = edges
        .iter()
        .position(|e| s(e, "source") == Some(after) && s(e, "target") == Some(before))
    else {
        return;
    };
    let old = edges.remove(i);
    edges.push(edge(after, &nid, s(&old, "conditionBranch")));
    edges.push(edge(&nid, before, None));
    let mut nodes = d.nodes().to_vec();
    nodes.push(node);
    d.set_graph(nodes, edges);
}

/// `appendNodeAfter`.
pub fn append_after(d: &mut Draft, after: &str, node: Value) {
    let nid = s(&node, "id").unwrap_or("").to_owned();
    let mut edges = d.edges().to_vec();
    edges.push(edge(after, &nid, None));
    let mut nodes = d.nodes().to_vec();
    nodes.push(node);
    d.set_graph(nodes, edges);
}

/// `insertConditionBetween`: a condition whose two placeholder branches
/// rejoin at `before`.
pub fn insert_condition(d: &mut Draft, after: &str, before: Option<&str>) {
    let Some(cond) = step_node("condition", d.nodes()) else {
        return;
    };
    let cid = s(&cond, "id").unwrap_or("").to_owned();
    let mut nodes = d.nodes().to_vec();
    nodes.push(cond);
    let mut edges = d.edges().to_vec();
    match before {
        Some(before) => {
            let branch = |label: &str, content: &str, nodes: &mut Vec<Value>| {
                let mut n = step_node("script", nodes).unwrap_or_default();
                n["label"] = json!(label);
                n["slug"] = json!(slugify(label));
                n["config"]["scriptContent"] = json!(content);
                let id = s(&n, "id").unwrap_or("").to_owned();
                nodes.push(n);
                id
            };
            let t = branch("True Branch", "# True branch\nexit 0\n", &mut nodes);
            let f = branch("False Branch", "# False branch\nexit 0\n", &mut nodes);
            edges.retain(|e| !(s(e, "source") == Some(after) && s(e, "target") == Some(before)));
            edges.push(edge(after, &cid, None));
            edges.push(edge(&cid, &t, Some("true")));
            edges.push(edge(&cid, &f, Some("false")));
            edges.push(edge(&t, before, None));
            edges.push(edge(&f, before, None));
        }
        None => edges.push(edge(after, &cid, None)),
    }
    d.set_graph(nodes, edges);
}

/// `removeNode`: incoming edges rewired to outgoing ones; a condition takes
/// its branches with it, up to where they join.
pub fn remove(d: &mut Draft, id: &str) {
    let mut edges = d.edges().to_vec();
    let mut gone: HashSet<String> = HashSet::from([id.to_owned()]);
    let preds: Vec<String> = edges
        .iter()
        .filter(|e| s(e, "target") == Some(id))
        .filter_map(|e| s(e, "source").map(str::to_owned))
        .collect();
    if d.node(id).and_then(|n| s(n, "type")) == Some("condition") {
        let succ = successors(&edges);
        let starts = succ.get(id).cloned().unwrap_or_default();
        let j = join(&starts, &succ);
        let mut queue = starts;
        while let Some(cur) = queue.pop() {
            if Some(&cur) == j.as_ref() || !gone.insert(cur.clone()) {
                continue;
            }
            queue.extend(succ.get(&cur).into_iter().flatten().cloned());
        }
        edges.retain(|e| {
            !gone.contains(s(e, "source").unwrap_or(""))
                && !gone.contains(s(e, "target").unwrap_or(""))
        });
        if let Some(j) = j {
            for p in &preds {
                edges.push(edge(p, &j, None));
            }
        }
    } else {
        let outs: Vec<String> = edges
            .iter()
            .filter(|e| s(e, "source") == Some(id))
            .filter_map(|e| s(e, "target").map(str::to_owned))
            .collect();
        edges.retain(|e| s(e, "source") != Some(id) && s(e, "target") != Some(id));
        for p in &preds {
            for o in &outs {
                edges.push(edge(p, o, None));
            }
        }
    }
    let nodes: Vec<Value> = d
        .nodes()
        .iter()
        .filter(|n| !gone.contains(s(n, "id").unwrap_or("")))
        .cloned()
        .map(|mut n| {
            if let Some(body) = n
                .pointer_mut("/config/bodyNodeIds")
                .and_then(Value::as_array_mut)
            {
                body.retain(|b| !gone.contains(b.as_str().unwrap_or("")));
            }
            n
        })
        .collect();
    d.set_graph(nodes, edges);
}

/// A step swapped for another kind in place: same id, edges and slug.
pub fn replace(d: &mut Draft, id: &str, kind: &str) {
    let fresh = if d.node(id).and_then(|n| s(n, "type")) == Some("trigger") {
        trigger_node(kind)
    } else {
        match step_node(kind, d.nodes()) {
            Some(n) => n,
            None => return,
        }
    };
    if let Some(n) = d.node_mut(id) {
        n["type"] = fresh["type"].clone();
        n["label"] = fresh["label"].clone();
        n["config"] = fresh["config"].clone();
        if let Some(slug) = fresh.get("slug") {
            n["slug"] = slug.clone();
        }
    }
}

/// Adds what the library picked at `at`.
pub fn add(d: &mut Draft, at: &Anchor, kind: &str) -> Option<String> {
    if let Some(r) = &at.replace {
        replace(d, r, kind);
        return Some(r.clone());
    }
    if at.trigger {
        if let Some(t) = trigger(d.nodes())
            .and_then(|t| s(t, "id"))
            .map(str::to_owned)
        {
            replace(d, &t, kind);
            return Some(t);
        }
        let node = trigger_node(kind);
        let tid = s(&node, "id").unwrap_or("").to_owned();
        let has_in: HashSet<String> = d
            .edges()
            .iter()
            .filter_map(|e| s(e, "target").map(str::to_owned))
            .collect();
        let roots: Vec<String> = d
            .nodes()
            .iter()
            .filter_map(|n| s(n, "id").map(str::to_owned))
            .filter(|id| !has_in.contains(id))
            .collect();
        let mut nodes = vec![node];
        nodes.extend(d.nodes().iter().cloned());
        let mut edges = d.edges().to_vec();
        for r in roots {
            edges.push(edge(&tid, &r, None));
        }
        d.set_graph(nodes, edges);
        return Some(tid);
    }
    let after = at.after.clone();
    if kind == "condition" {
        let after = after.or_else(|| last_leaf(d))?;
        insert_condition(d, &after, at.before.as_deref());
        return d
            .nodes()
            .last()
            .filter(|n| s(n, "type") == Some("condition"))
            .or_else(|| {
                d.nodes()
                    .iter()
                    .rev()
                    .find(|n| s(n, "type") == Some("condition"))
            })
            .and_then(|n| s(n, "id"))
            .map(str::to_owned);
    }
    let node = step_node(kind, d.nodes())?;
    let id = s(&node, "id").unwrap_or("").to_owned();
    match (after, &at.before) {
        (Some(a), Some(b)) => insert_between(d, &a, b, node),
        (Some(a), None) => append_after(d, &a, node),
        (None, _) => match last_leaf(d) {
            Some(a) => append_after(d, &a, node),
            None => {
                let mut nodes = d.nodes().to_vec();
                nodes.push(node);
                let edges = d.edges().to_vec();
                d.set_graph(nodes, edges);
            }
        },
    }
    Some(id)
}

/// `appendNode`'s anchor: the last node with nothing after it.
fn last_leaf(d: &Draft) -> Option<String> {
    let outs: HashSet<&str> = d.edges().iter().filter_map(|e| s(e, "source")).collect();
    d.nodes()
        .iter()
        .rev()
        .filter_map(|n| s(n, "id"))
        .find(|id| !outs.contains(id))
        .map(str::to_owned)
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

/// A canvas card's glyph.
pub fn card_icon(node: &Value) -> &'static str {
    match s(node, "type").unwrap_or("") {
        "trigger" => trigger_icon(cfg(node, "triggerType").unwrap_or("manual")),
        "script" => match cfg(node, "scriptType") {
            Some("python" | "node") => "code",
            _ => "terminal",
        },
        "launchAgent" => "bot",
        k => crate::screen::runs::node_icon(k),
    }
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
    let cut = |t: &str, n: usize| crate::screen::truncate(t, n);
    match s(node, "type").unwrap_or("") {
        "script" => cfg(node, "scriptContent").and_then(|c| {
            c.lines()
                .find(|l| !l.trim().is_empty() && !l.starts_with('#'))
                .map(|l| l.trim().to_owned())
        }),
        "launchAgent" => cfg(node, "prompt").map(|p| cut(p, 60)),
        "approval" => cfg(node, "message").map(|m| cut(m, 60)),
        "httpRequest" => cfg(node, "body").map(|b| cut(b.trim(), 50)),
        "condition" => {
            let v = cfg(node, "variable")?;
            let op = cfg(node, "operator").unwrap_or("equals");
            let val = match (op, cfg(node, "value")) {
                ("isEmpty" | "isNotEmpty", _) | (_, None) => String::new(),
                (_, Some(v)) => format!(" \"{v}\""),
            };
            Some(cut(&format!("{v} {}{val}", operator_symbol(op)), 60))
        }
        "loop" => Some(if cfg(node, "mode") == Some("forEach") {
            format!(
                "for each in {}",
                cfg(node, "items").unwrap_or("a list not chosen yet")
            )
        } else {
            "runs every pass".to_owned()
        }),
        _ => None,
    }
}

/// The choices a config select offers, as value and label.
pub fn choices(path: &str) -> Vec<(&'static str, &'static str)> {
    let key = path.rsplit('.').next().unwrap_or(path);
    match key {
        "scriptType" => vec![
            ("bash", "Bash"),
            ("powershell", "PowerShell"),
            ("python", "Python"),
            ("node", "Node"),
        ],
        "operator" => vec![
            ("equals", "equals"),
            ("notEquals", "not equals"),
            ("contains", "contains"),
            ("notContains", "not contains"),
            ("isEmpty", "is empty"),
            ("isNotEmpty", "is not empty"),
        ],
        "method" if path == "config.method" => vec![
            ("GET", "GET"),
            ("POST", "POST"),
            ("PUT", "PUT"),
            ("PATCH", "PATCH"),
            ("DELETE", "DELETE"),
        ],
        "fromStatus" | "toStatus" => vec![
            ("", "Any status"),
            ("todo", "To Do"),
            ("in_progress", "In Progress"),
            ("in_review", "In Review"),
            ("done", "Done"),
            ("cancelled", "Cancelled"),
        ],
        "type" => vec![
            ("text", "Text"),
            ("textarea", "Long text"),
            ("number", "Number"),
            ("select", "Select"),
            ("boolean", "Toggle"),
            ("project", "Project"),
            ("branch", "Branch"),
        ],
        "mode" => vec![
            ("count", "A number of times"),
            ("forEach", "For each in a list"),
        ],
        "cron" => vec![
            ("0 9 * * 1-5", "Weekdays 9am"),
            ("0 9 * * *", "Daily 9am"),
            ("0 * * * *", "Every hour"),
            ("*/30 * * * *", "Every 30min"),
            ("0 9 * * 1", "Weekly (Mon 9am)"),
        ],
        "agentType" => crate::screen::menus::AGENTS.to_vec(),
        "onError" => vec![("stop", "Stop the run"), ("continue", "Carry on anyway")],
        _ => Vec::new(),
    }
}

/// The label a select shows for `value`.
pub fn choice_label(path: &str, value: &str) -> String {
    choices(path)
        .into_iter()
        .find(|(v, _)| *v == value)
        .map_or_else(|| value.to_owned(), |(_, l)| l.to_owned())
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

/// One control of a node's config form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Field {
    Text {
        path: String,
        label: &'static str,
        placeholder: &'static str,
        mono: bool,
        lines: bool,
        help: &'static str,
    },
    Choice {
        path: String,
        label: &'static str,
        help: &'static str,
    },
    Switch {
        path: String,
        label: &'static str,
        help: &'static str,
    },
    /// The trigger's kind, its hint and the library link.
    Kind,
    /// The manual trigger's run inputs.
    Inputs,
}

fn text(path: &str, label: &'static str, placeholder: &'static str) -> Field {
    Field::Text {
        path: path.to_owned(),
        label,
        placeholder,
        mono: false,
        lines: false,
        help: "",
    }
}

fn choice(path: &str, label: &'static str) -> Field {
    Field::Choice {
        path: path.to_owned(),
        label,
        help: "",
    }
}

fn with_help(mut f: Field, h: &'static str) -> Field {
    match &mut f {
        Field::Text { help, .. } | Field::Choice { help, .. } | Field::Switch { help, .. } => {
            *help = h
        }
        _ => {}
    }
    f
}

fn lines(mut f: Field, as_mono: bool) -> Field {
    if let Field::Text { lines, mono, .. } = &mut f {
        *lines = true;
        *mono = as_mono;
    }
    f
}

fn mono(mut f: Field) -> Field {
    if let Field::Text { mono, .. } = &mut f {
        *mono = true;
    }
    f
}

/// `NodeConfigPanel`'s form for `node`, top to bottom.
pub fn form(node: &Value) -> Vec<Field> {
    let c = |k: &str| cfg(node, k).unwrap_or("");
    let mut out = match s(node, "type").unwrap_or("") {
        "trigger" => {
            let mut f = vec![Field::Kind];
            match c("triggerType") {
                "manual" => {
                    f.push(Field::Switch {
                        path: "config.contextual".into(),
                        label: "Contextual",
                        help: "Run this workflow directly from any card or terminal, against that session's folder and branch.",
                    });
                    f.push(Field::Inputs);
                }
                "once" => f.push(text("config.runAt", "Run At", "2026-01-01T09:00:00.000Z")),
                "recurring" => {
                    f.push(choice("config.cron", "Preset"));
                    f.push(with_help(
                        mono(text("config.cron", "Cron Expression", "* * * * *")),
                        "min hour day month weekday",
                    ));
                    f.push(text("config.timezone", "Timezone", "UTC"));
                }
                "taskCreated" => f.push(with_help(
                    choice("config.projectFilter", "Project Filter"),
                    "Only trigger for tasks in this project",
                )),
                "taskStatusChanged" => {
                    f.push(choice("config.projectFilter", "Project Filter"));
                    f.push(choice("config.fromStatus", "From Status"));
                    f.push(choice("config.toStatus", "To Status"));
                }
                "sessionRestored" => f.push(choice("config.projectFilter", "Project Filter")),
                "webhook" => f.push(choice("config.method", "Method")),
                _ => {}
            }
            return f;
        }
        "script" => vec![
            choice("config.scriptType", "Type"),
            choice("config.projectName", "Project"),
            lines(text("config.scriptContent", "Script", ""), true),
            text("config.args", "Arguments", "arg1 arg2 --flag"),
        ],
        "launchAgent" => vec![
            choice("config.agentType", "Agent"),
            text("config.model", "Model", "Default"),
            choice("config.projectName", "Project"),
            lines(text("config.prompt", "Prompt", "Enter prompt..."), false),
            text("config.branch", "Branch", "feature/my-branch"),
            text("config.displayName", "Tab Name", ""),
        ],
        "approval" => vec![
            with_help(
                lines(
                    text(
                        "config.message",
                        "Message",
                        "Shown with the answers, e.g. {{steps.draft.output}}",
                    ),
                    false,
                ),
                "Step outputs fill in when the gate opens.",
            ),
            with_help(
                text(
                    "timeout",
                    "Timeout (seconds, optional)",
                    "Leave blank to wait forever",
                ),
                "Auto-rejects the gate if nobody answers in time. Each round starts a new timeout.",
            ),
        ],
        "condition" => {
            let mut f = vec![
                mono(text(
                    "config.variable",
                    "Variable",
                    "e.g. {{steps.build.status}}",
                )),
                choice("config.operator", "Operator"),
            ];
            if !matches!(c("operator"), "isEmpty" | "isNotEmpty") {
                f.push(text("config.value", "Value", "Compare against..."));
            }
            f
        }
        "httpRequest" => vec![
            choice("config.method", "Method"),
            mono(text("config.url", "URL", "https://")),
            with_help(
                lines(text("config.body", "Body", ""), true),
                "Sent as-is. GET requests send no body.",
            ),
        ],
        "loop" => {
            let mut f = vec![choice("config.mode", "Repeat")];
            if c("mode") == "forEach" {
                f.push(mono(text(
                    "config.items",
                    "Items",
                    "{{steps.review.items}}",
                )));
            } else {
                f.push(text("config.maxIterations", "Maximum passes", "2"));
            }
            f
        }
        _ => Vec::new(),
    };
    let stop = s(node, "onError").unwrap_or("stop") == "stop";
    out.push(with_help(
        choice("onError", "If this step fails"),
        if stop {
            "Everything downstream is skipped and the run ends as failed."
        } else {
            "Later steps run as if this one had succeeded, and the run still counts as successful \u{2014} for a step whose failure does not invalidate the rest."
        },
    ));
    out
}

/// The text a field at `path` shows for `node`.
pub fn field_text(node: &Value, path: &str) -> String {
    if path == "timeout" {
        return node
            .pointer("/config/timeoutMs")
            .and_then(Value::as_f64)
            .filter(|ms| *ms > 0.0)
            .map(|ms| ((ms / 1000.0).round() as u64).to_string())
            .unwrap_or_default();
    }
    let mut parts = path.split('.');
    match parts.next() {
        Some("input") => {
            let i = parts
                .next()
                .and_then(|i| i.parse::<usize>().ok())
                .unwrap_or(0);
            let prop = parts.next().unwrap_or("");
            let item = node.pointer(&format!("/config/inputs/{i}"));
            if prop == "options" {
                return item
                    .and_then(|i| i.get("options"))
                    .and_then(Value::as_array)
                    .map(|o| {
                        o.iter()
                            .filter_map(|o| s(o, "value"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
            }
            text_of(item.and_then(|i| i.get(prop)))
        }
        Some(head) => {
            let ptr = std::iter::once(head)
                .chain(parts)
                .collect::<Vec<_>>()
                .join("/");
            text_of(node.pointer(&format!("/{ptr}")))
        }
        None => String::new(),
    }
}

/// Every text path the form of `node` edits, inputs included.
pub fn text_paths(node: &Value) -> Vec<String> {
    let mut out = vec!["label".to_owned()];
    for f in form(node) {
        match f {
            Field::Text { path, .. } => out.push(path),
            Field::Inputs => {
                let defs = node
                    .pointer("/config/inputs")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                for i in 0..defs {
                    for p in ["key", "label", "options", "defaultValue"] {
                        out.push(format!("input.{i}.{p}"));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// What typing `text` into the field at `path` stores.
pub fn field_value(path: &str, text: &str) -> (String, Value) {
    let num = |t: &str| t.trim().parse::<u64>().ok();
    match path {
        "timeout" => (
            "config.timeoutMs".into(),
            text.trim()
                .parse::<f64>()
                .ok()
                .filter(|v| *v > 0.0 && v.is_finite())
                .map_or(Value::Null, |v| json!((v * 1000.0).round() as u64)),
        ),
        "config.maxIterations" => (path.into(), num(text).map_or(Value::Null, |n| json!(n))),
        "staggerDelayMs" => (path.into(), num(text).map_or(Value::Null, |n| json!(n))),
        p if p.ends_with(".options") => (
            p.into(),
            Value::Array(
                text.split(',')
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(|v| json!({ "value": v, "label": v }))
                    .collect(),
            ),
        ),
        p => (p.into(), json!(text)),
    }
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
    Gate {
        run: String,
        node: String,
        approve: bool,
    },
    SignIn(String),
    Side(SideFilter),
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
    Pick(String),
    DeleteNode(String),
    RunTo(String),
    Set(String, String),
    Toggle(String),
    AddInput,
    RemoveInput(usize),
    StartBlank,
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
    /// The export or import dialog's go.
    File,
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
        let d = describe(&run, None);
        assert_eq!((d.title.as_str(), d.source), ("Task t12345", "task"));
        let w = json!({ "name": "Nightly", "nodes": [{ "type": "trigger", "config": { "triggerType": "recurring" } }] });
        let d = describe(&json!({ "workflowId": "x" }), Some(&w));
        assert_eq!((d.title.as_str(), d.source), ("Nightly", "scheduled"));
    }

    #[test]
    fn slugs_are_unique_and_tidy() {
        assert_eq!(slugify("Execute Script"), "execute_script");
        assert_eq!(slugify("--"), "step");
        let taken = HashSet::from(["input".to_owned(), "input_2".to_owned()]);
        assert_eq!(unique_slug("input", &taken), "input_3");
        assert!(valid_key("my_key1") && !valid_key("1x") && !valid_key(""));
    }

    #[test]
    fn a_condition_forks_and_rejoins() {
        let mut d = wf(
            json!([n("t", "trigger"), n("b", "script")]),
            json!([e("t", "b")]),
        );
        insert_condition(&mut d, "t", Some("b"));
        let rows = flow(d.nodes(), d.edges());
        assert_eq!(rows.len(), 4, "{rows:?}");
        let Row::Fork { branches, .. } = &rows[2] else {
            panic!("{rows:?}")
        };
        assert_eq!(branches.len(), 2);
        assert_eq!(branches[0].0.as_deref(), Some("true"));
        assert_eq!(
            rows[3],
            Row::Node {
                id: "b".into(),
                next: None
            }
        );
        let cid = s(&d.nodes()[2], "id").unwrap().to_owned();
        remove(&mut d, &cid);
        assert_eq!(d.nodes().len(), 2);
        assert_eq!(flow(d.nodes(), d.edges()).len(), 2);
    }

    #[test]
    fn steps_insert_append_and_remove() {
        let mut d = wf(
            json!([n("t", "trigger"), n("b", "script")]),
            json!([e("t", "b")]),
        );
        let at = Anchor {
            after: Some("t".into()),
            before: Some("b".into()),
            ..Anchor::default()
        };
        let id = add(&mut d, &at, "approval").unwrap();
        let order: Vec<String> = flow(d.nodes(), d.edges())
            .into_iter()
            .map(|r| match r {
                Row::Node { id, .. } => id,
                Row::Fork { .. } => String::new(),
            })
            .collect();
        assert_eq!(order, ["t".to_owned(), id.clone(), "b".to_owned()]);
        let end = add(&mut d, &Anchor::default(), "script").unwrap();
        assert_eq!(s(d.node(&end).unwrap(), "slug"), Some("execute_script"));
        remove(&mut d, &id);
        assert!(d
            .edges()
            .iter()
            .any(|e| s(e, "source") == Some("t") && s(e, "target") == Some("b")));
        replace(&mut d, "b", "httpRequest");
        assert_eq!(s(d.node("b").unwrap(), "type"), Some("httpRequest"));
    }

    #[test]
    fn a_trigger_goes_above_the_roots() {
        let mut d = wf(json!([n("b", "script")]), json!([]));
        let at = Anchor {
            trigger: true,
            ..Anchor::default()
        };
        let t = add(&mut d, &at, "recurring").unwrap();
        assert_eq!(s(&d.nodes()[0], "label"), Some("Schedule (Recurring)"));
        assert!(d.edges().iter().any(|e| s(e, "source") == Some(t.as_str())));
        assert_eq!(subtitle(&d.nodes()[0]), "Cron: 0 9 * * *");
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
