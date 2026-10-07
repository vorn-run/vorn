//! The parts of running a workflow that decide rather than do
//! (`workflow-graph.ts`): the graph's shape, which steps a failure or a
//! branch skips, a retry's starting states, loops' bodies, conditions, the
//! outputs later steps read, and a run's trigger fingerprint.

use std::collections::{HashMap, HashSet, VecDeque};

use serde_json::{json, Map, Value};
use vorn_protocol::{NodeExecutionState, WorkflowExecution};

use crate::items::{is_record_list, parse_json, to_item_list};
use crate::js;
use crate::model::{state, Context, Edge, Node, NodeKind, StateExt, Status, Workflow};
use crate::template::{resolve, StepOutputs};

/// Default ceiling for a headless step that never reports an exit.
pub const DEFAULT_STEP_TIMEOUT_MINUTES: f64 = 60.0;

const LOG_BUFFER_MAX: usize = 100_000;
const LOG_BUFFER_KEEP: usize = 80_000;

/// What a step that never reported a session is closed with.
pub const ABANDONED: &str = "Run abandoned (no session id recorded)";

/// How much of each step's output a loop keeps per pass.
pub const LOOP_RESULT_OUTPUT_CHARS: usize = 8000;

/// Ceiling on `maxIterations`, whatever a workflow asks for.
pub const MAX_LOOP_ITERATIONS: u32 = 10;

/// Ceiling on how many times a gate may ask.
pub const MAX_GATE_ROUNDS: u32 = 10;

pub const DEFAULT_GATE_ROUNDS: u32 = 3;

/// `appendBoundedLog`: past [`LOG_BUFFER_MAX`] units only the last
/// [`LOG_BUFFER_KEEP`] are kept.
pub fn append_bounded_log(buffer: &mut String, chunk: &str) {
    buffer.push_str(chunk);
    // Counting is only needed once the bytes could be past the limit.
    if buffer.len() > LOG_BUFFER_MAX && js::utf16_len(buffer) > LOG_BUFFER_MAX {
        let kept = js::tail(buffer, LOG_BUFFER_KEEP).to_owned();
        *buffer = kept;
    }
}

/// Where a step's worktree came from, for cleanup: none, one the context
/// handed it (never deleted), or one it created.
pub fn worktree_origin(worktree_path: Option<&str>, inherited: bool) -> Option<&'static str> {
    match worktree_path {
        None | Some("") => None,
        Some(_) if inherited => Some("inherited"),
        Some(_) => Some("created"),
    }
}

/// A gate's key: per run, so parallel runs each have their own.
pub fn gate_key(run_id: &str, node_id: &str) -> String {
    format!("{run_id}:{node_id}")
}

/// `${value}` in a template literal.
fn interpolated(value: Option<&Value>) -> String {
    value.map_or_else(|| "undefined".to_owned(), js::to_string)
}

/// `dedupeFingerprint`: what a run was triggered with. Two runs of a
/// workflow are the same trigger only when this matches.
pub fn dedupe_fingerprint(context: Option<&Context>) -> String {
    let inputs = context
        .and_then(|c| c.inputs.as_ref())
        .filter(|i| !i.is_empty())
        .map(|inputs| {
            let mut keys: Vec<&String> = inputs.keys().collect();
            keys.sort();
            let pairs: Vec<Value> = keys.iter().map(|k| json!([k, inputs[*k]])).collect();
            js::stringify(&Value::Array(pairs))
        });
    let params = inputs.map_or_else(String::new, |i| format!(":inputs:{i}"));
    let Some(ctx) = context else {
        return format!("manual{params}");
    };
    if let Some(item) = &ctx.connector_item {
        return format!(
            "item:{}:{}{params}",
            interpolated(item.get("connectionId")),
            interpolated(item.get("externalId"))
        );
    }
    if let Some(task) = &ctx.task {
        return format!("task:{}{params}", interpolated(task.get("id")));
    }
    if let Some(source) = &ctx.source {
        return format!("session:{}{params}", interpolated(source.get("id")));
    }
    format!("manual{params}")
}

/// A script step's config with its templates filled in, its directory
/// included; arguments stay arguments, never spliced into the source.
pub fn resolve_script_config(
    config: &Value,
    context: Option<&Context>,
    outputs: Option<&StepOutputs>,
) -> Value {
    let mut out = config.as_object().cloned().unwrap_or_default();
    let mut set_text = |key: &str, keep_empty: bool| match config.get(key) {
        Some(Value::String(s)) => {
            let resolved = resolve(s, context, outputs);
            if resolved.is_empty() && !keep_empty {
                out.remove(key);
            } else {
                out.insert(key.to_owned(), Value::String(resolved));
            }
        }
        None | Some(Value::Null) => {
            out.remove(key);
        }
        Some(_) => {}
    };
    set_text("scriptContent", true);
    set_text("cwd", false);
    set_text("projectPath", false);
    if let Some(Value::Array(args)) = config.get("args") {
        let resolved = args
            .iter()
            .map(|arg| match arg {
                Value::String(s) => Value::String(resolve(s, context, outputs)),
                other => other.clone(),
            })
            .collect();
        out.insert("args".into(), Value::Array(resolved));
    }
    Value::Object(out)
}

/// `buildStepOutputsMap`: by slug, each settled step's outputs, and each
/// gate that has asked.
pub fn step_outputs(execution: &WorkflowExecution, workflow: &Workflow) -> StepOutputs {
    let by_id: HashMap<&str, &Node> = workflow.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut outputs = StepOutputs::new();
    for ns in &execution.node_states {
        let Some(node) = by_id.get(ns.node_id.as_str()) else {
            continue;
        };
        let Some(slug) = &node.slug else {
            continue;
        };
        let settled = ns.is(Status::Success) || ns.is(Status::Error);
        let gate = (node.kind == NodeKind::Approval)
            .then(|| gate_outputs(ns))
            .flatten();
        if !settled && gate.is_none() {
            continue;
        }
        let mut entry = Map::new();
        if settled {
            spread(&mut entry, ns.structured_output.as_ref());
            entry.insert(
                "output".into(),
                Value::String(output_or_logs(ns).to_owned()),
            );
            entry.insert("status".into(), Value::String(ns.status.0.clone()));
            entry.insert(
                "error".into(),
                Value::String(ns.error.clone().unwrap_or_default()),
            );
            entry.insert(
                "worktreePath".into(),
                Value::String(ns.worktree_path.clone().unwrap_or_default()),
            );
        }
        for (k, v) in gate.unwrap_or_default() {
            entry.insert(k, v);
        }
        outputs.insert(slug.clone(), Value::Object(entry));
    }
    outputs
}

/// `ns.output || ns.logs || ''`.
pub fn output_or_logs(ns: &NodeExecutionState) -> &str {
    [ns.output.as_deref(), ns.logs.as_deref()]
        .into_iter()
        .flatten()
        .find(|s| !s.is_empty())
        .unwrap_or("")
}

/// `{ ...value }` into `into`.
fn spread(into: &mut Map<String, Value>, value: Option<&Value>) {
    match value {
        Some(Value::Object(map)) => {
            for (k, v) in map {
                into.insert(k.clone(), v.clone());
            }
        }
        Some(Value::Array(items)) => {
            for (i, v) in items.iter().enumerate() {
                into.insert(i.to_string(), v.clone());
            }
        }
        _ => {}
    }
}

/// The feedback entries a state holds.
fn feedback_entries(ns: &NodeExecutionState) -> &[Value] {
    ns.feedback
        .as_ref()
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// A gate's text, its comments and which time it asked; `None` before it
/// first asks.
fn gate_outputs(ns: &NodeExecutionState) -> Option<Map<String, Value>> {
    let entries = feedback_entries(ns);
    if ns.round.is_none() && entries.is_empty() {
        return None;
    }
    let text = ns
        .edited_text
        .as_deref()
        .or(ns.editable_text.as_deref())
        .unwrap_or("");
    let mut out = Map::new();
    if let Ok(list) = to_item_list(&Value::String(text.to_owned())) {
        if is_record_list(&list.items) {
            out.insert("items".into(), Value::Array(list.items));
        }
    }
    let comment_of = |e: &Value| interpolated(e.get("comment"));
    out.insert("text".into(), Value::String(text.to_owned()));
    out.insert(
        "feedback".into(),
        Value::String(entries.last().map(comment_of).unwrap_or_default()),
    );
    out.insert(
        "feedbackAll".into(),
        Value::String(
            entries
                .iter()
                .map(|e| format!("Round {}: {}", interpolated(e.get("round")), comment_of(e)))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
    );
    let comments = entries
        .last()
        .and_then(|e| e.get("comments"))
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|c| {
                    let quote = c.get("quote").filter(|q| !q.is_null());
                    let mut entry = Map::new();
                    entry.insert(
                        "quote".into(),
                        quote.cloned().unwrap_or(Value::String(String::new())),
                    );
                    if let Some(comment) = c.get("comment") {
                        entry.insert("comment".into(), comment.clone());
                    }
                    entry.insert("anchored".into(), Value::Bool(crate::is_truthy(quote)));
                    Value::Object(entry)
                })
                .collect()
        })
        .unwrap_or_default();
    out.insert("comments".into(), Value::Array(comments));
    out.insert("round".into(), ns.round.map_or(json!(1), js::number_value));
    Some(out)
}

/// `gateEditRefusal`: why a gate would refuse this rewrite. A gate that
/// showed a list of records hands those on as data, so a rewrite that no
/// longer parses is refused with where it broke.
pub fn gate_edit_refusal(editable: Option<&str>, edited: Option<&str>) -> Option<String> {
    let edited = edited.filter(|e| !e.trim().is_empty())?;
    let editable = editable.filter(|e| !e.is_empty())?;
    let original = to_item_list(&Value::String(editable.to_owned())).ok()?;
    if !is_record_list(&original.items) {
        return None;
    }
    if let Err(at) = parse_json(edited) {
        return Some(format!(
            "The edited list is not valid JSON: line {}, column {}.",
            at.line, at.column
        ));
    }
    to_item_list(&Value::String(edited.to_owned()))
        .err()
        .map(|why| format!("The edited text is no longer a list. {why}"))
}

/// `Number(value)`, as far as a config's numbers go.
pub fn js_number(value: Option<&Value>) -> Option<f64> {
    match value {
        None => None,
        Some(Value::Null) => Some(0.0),
        Some(Value::Bool(b)) => Some(f64::from(u8::from(*b))),
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => js::number(s),
        Some(_) => None,
    }
}

/// How many times a gate may ask, the first included.
pub fn gate_max_rounds(feedback: Option<&Value>) -> u32 {
    match js_number(feedback.and_then(|f| f.get("maxRounds"))).map(f64::floor) {
        Some(r) if r.is_finite() => r.clamp(1.0, f64::from(MAX_GATE_ROUNDS)) as u32,
        _ => DEFAULT_GATE_ROUNDS,
    }
}

/// Whether the reviewer may still send the work back at this round.
pub fn can_request_changes(config: &Value, round: Option<f64>) -> bool {
    let feedback = config.get("feedback");
    if !crate::is_truthy(feedback.and_then(|f| f.get("from"))) {
        return false;
    }
    round.unwrap_or(1.0) < f64::from(gate_max_rounds(feedback))
}

/// The steps on some path from `from` to `to`, both included; empty when
/// `to` is not below `from`.
pub fn nodes_between(from: &str, to: &str, edges: &[Edge]) -> HashSet<String> {
    if from == to {
        return HashSet::new();
    }
    let graph = Graph::of(edges);
    let below = reach(from, &graph.successors);
    if !below.contains(to) {
        return HashSet::new();
    }
    let above = reach(to, &graph.predecessors);
    below.intersection(&above).cloned().collect()
}

fn reach(start: &str, next: &HashMap<String, Vec<String>>) -> HashSet<String> {
    let mut seen = HashSet::from([start.to_owned()]);
    let mut queue = VecDeque::from([start.to_owned()]);
    while let Some(id) = queue.pop_front() {
        for n in next.get(&id).into_iter().flatten() {
            if seen.insert(n.clone()) {
                queue.push_back(n.clone());
            }
        }
    }
    seen
}

/// `evaluateCondition`.
pub fn evaluate_condition(operator: &str, resolved: &str, value: &str) -> bool {
    match operator {
        "equals" => resolved == value,
        "notEquals" => resolved != value,
        "contains" => resolved.contains(value),
        "notContains" => !resolved.contains(value),
        "isEmpty" => resolved.trim().is_empty(),
        "isNotEmpty" => !resolved.trim().is_empty(),
        _ => false,
    }
}

/// `loopShouldStop`: a half-written condition means "not configured yet",
/// never "stop now".
pub fn loop_should_stop(until: Option<&Value>, variable: &str, value: &str) -> bool {
    let Some(until) = until.filter(|u| !u.is_null()) else {
        return false;
    };
    let text = |k: &str| until.get(k).and_then(Value::as_str).unwrap_or("");
    if text("variable").trim().is_empty() {
        return false;
    }
    let operator = text("operator");
    let compares = matches!(
        operator,
        "equals" | "notEquals" | "contains" | "notContains"
    );
    if compares && value.trim().is_empty() {
        return false;
    }
    evaluate_condition(operator, variable, value)
}

/// The state a body step starts a pass from: only its id survives.
pub fn blank_pass_state(node_id: &str, iteration: Option<u32>) -> NodeExecutionState {
    let mut blank = state(node_id, Status::Pending);
    blank.iteration = iteration.map(f64::from);
    blank
}

/// Waiting for its connection to sign in again, not for an approval.
pub fn is_sign_in_wait(ns: &NodeExecutionState) -> bool {
    ns.is(Status::Waiting) && ns.waiting_for.as_deref() == Some("signIn")
}

/// A step that never ran, so no policy of its own speaks for it.
pub fn never_ran(ns: &NodeExecutionState) -> bool {
    ns.error
        .as_deref()
        .is_some_and(|e| e.starts_with("Skipped:") || e == ABANDONED)
}

/// The step that failed on its own terms, which a retry starts from.
pub fn failed_step(execution: &WorkflowExecution) -> Option<&NodeExecutionState> {
    execution
        .node_states
        .iter()
        .find(|ns| ns.is(Status::Error) && !never_ran(ns))
}

/// Whether the run failed, rather than holding a step that failed and said
/// so survivably.
pub fn run_ended_in_error(
    execution: &WorkflowExecution,
    workflow: &Workflow,
    skipped: Option<&HashSet<String>>,
) -> bool {
    execution.node_states.iter().any(|ns| {
        if !ns.is(Status::Error) || skipped.is_some_and(|s| s.contains(&ns.node_id)) {
            return false;
        }
        if never_ran(ns) {
            return true;
        }
        workflow
            .node(&ns.node_id)
            .is_none_or(Node::stops_run_on_error)
    })
}

/// Edges by node, each in edge order.
#[derive(Clone, Debug, Default)]
pub struct Graph {
    pub successors: HashMap<String, Vec<String>>,
    pub predecessors: HashMap<String, Vec<String>>,
}

impl Graph {
    /// `buildGraph`.
    pub fn of(edges: &[Edge]) -> Graph {
        let mut graph = Graph::default();
        for e in edges {
            graph
                .successors
                .entry(e.source.clone())
                .or_default()
                .push(e.target.clone());
            graph
                .predecessors
                .entry(e.target.clone())
                .or_default()
                .push(e.source.clone());
        }
        graph
    }

    pub fn predecessors_of(&self, id: &str) -> &[String] {
        self.predecessors.get(id).map_or(&[], Vec::as_slice)
    }

    pub fn successors_of(&self, id: &str) -> &[String] {
        self.successors.get(id).map_or(&[], Vec::as_slice)
    }
}

/// `collectSkippedBranch`: the branch from `start`, stopping at a join
/// whose other predecessors are not settled. In the order reached.
pub fn collect_skipped_branch(
    start: &str,
    graph: &Graph,
    is_terminal: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut skipped: Vec<String> = Vec::new();
    let mut member: HashSet<String> = HashSet::new();
    let mut queue = VecDeque::from([start.to_owned()]);
    while let Some(id) = queue.pop_front() {
        if member.contains(&id) || is_terminal(&id) {
            continue;
        }
        member.insert(id.clone());
        skipped.push(id.clone());
        for s in graph.successors_of(&id) {
            let others = graph
                .predecessors_of(s)
                .iter()
                .any(|p| p != &id && !member.contains(p) && !is_terminal(p));
            if !others {
                queue.push_back(s.clone());
            }
        }
    }
    skipped
}

/// `skipEntryPoints`: the first hops of the branch a failed step feeds,
/// leaving out a join another live predecessor still feeds.
pub fn skip_entry_points(
    failed: &str,
    edges: &[Edge],
    graph: &Graph,
    is_settled: impl Fn(&str) -> bool,
) -> Vec<String> {
    edges
        .iter()
        .filter(|e| e.source == failed)
        .filter(|e| {
            !graph
                .predecessors_of(&e.target)
                .iter()
                .any(|p| p != failed && !is_settled(p))
        })
        .map(|e| e.target.clone())
        .collect()
}

/// `seedRetryStates`: successes and deliberate skips kept, the rest pending;
/// a loop and its body go together.
pub fn seed_retry_states(
    workflow: &Workflow,
    failed: &WorkflowExecution,
) -> Vec<NodeExecutionState> {
    let prior: HashMap<&str, &NodeExecutionState> = failed
        .node_states
        .iter()
        .map(|ns| (ns.node_id.as_str(), ns))
        .collect();
    let owners = loop_body_owners(&workflow.nodes);
    workflow
        .nodes
        .iter()
        .map(|n| {
            let before = prior.get(n.id.as_str()).copied();
            if n.kind == NodeKind::Trigger {
                return state(&n.id, Status::Success);
            }
            if let Some(owner) = owners.get(&n.id) {
                let loop_done = prior
                    .get(owner.as_str())
                    .is_some_and(|s| s.is(Status::Success));
                return match before {
                    Some(b) if loop_done => b.clone(),
                    _ => state(&n.id, Status::Pending),
                };
            }
            match before {
                Some(b) if b.is(Status::Success) => b.clone(),
                Some(b)
                    if b.is(Status::Skipped)
                        && b.skip_reason.as_deref().is_some_and(|r| !r.is_empty()) =>
                {
                    b.clone()
                }
                _ => state(&n.id, Status::Pending),
            }
        })
        .collect()
}

/// A loop's `bodyNodeIds`.
fn body_ids(node: &Node) -> Vec<&str> {
    node.config
        .get("bodyNodeIds")
        .and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// Which loop owns each body step: ids no node has are left out, and the
/// first loop to claim a step keeps it.
pub fn loop_body_owners(nodes: &[Node]) -> HashMap<String, String> {
    let exists: HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    let mut owners = HashMap::new();
    for n in nodes.iter().filter(|n| n.kind == NodeKind::Loop) {
        for id in body_ids(n) {
            if exists.contains(id) && id != n.id && !owners.contains_key(id) {
                owners.insert(id.to_owned(), n.id.clone());
            }
        }
    }
    owners
}

/// The run graph as the main scheduler sees it: each loop stands in for its
/// body, and an edge leaving a body is redrawn from the loop, unlabelled.
pub fn collapse_loop_bodies(nodes: &[Node], edges: &[Edge]) -> Vec<Edge> {
    let owners = loop_body_owners(nodes);
    let mut seen = HashSet::new();
    let mut collapsed = Vec::new();
    for edge in edges {
        if owners.contains_key(&edge.target) {
            continue;
        }
        let next = match owners.get(&edge.source) {
            Some(owner) => Edge {
                id: format!("{}:via-loop", edge.id),
                source: owner.clone(),
                target: edge.target.clone(),
                branch: None,
            },
            None => edge.clone(),
        };
        if next.source == next.target {
            continue;
        }
        let key = format!(
            "{}->{}:{}",
            next.source,
            next.target,
            next.branch.as_deref().unwrap_or("")
        );
        if seen.insert(key) {
            collapsed.push(next);
        }
    }
    collapsed
}

/// A loop's body as a graph of its own.
#[derive(Clone, Debug)]
pub struct Body<'a> {
    pub members: Vec<&'a Node>,
    pub edges: Vec<Edge>,
    /// The members nothing inside the body feeds: where each pass starts.
    pub entries: Vec<String>,
}

/// `loopBodyGraph`: a body without edges between its members is chained in
/// `bodyNodeIds` order.
pub fn loop_body_graph<'a>(nodes: &'a [Node], edges: &[Edge], lp: &Node) -> Body<'a> {
    let owners = loop_body_owners(nodes);
    let members: Vec<&Node> = body_ids(lp)
        .into_iter()
        .filter(|id| owners.get(*id) == Some(&lp.id))
        .filter_map(|id| nodes.iter().find(|n| n.id == id))
        .collect();
    let ids: HashSet<&str> = members.iter().map(|m| m.id.as_str()).collect();
    let mut inner: Vec<Edge> = edges
        .iter()
        .filter(|e| ids.contains(e.source.as_str()) && ids.contains(e.target.as_str()))
        .cloned()
        .collect();
    if inner.is_empty() && members.len() > 1 {
        inner = members
            .windows(2)
            .map(|pair| Edge {
                id: format!("{}->{}:chain", pair[0].id, pair[1].id),
                source: pair[0].id.clone(),
                target: pair[1].id.clone(),
                branch: None,
            })
            .collect();
    }
    let fed: HashSet<&str> = inner.iter().map(|e| e.target.as_str()).collect();
    let entries = members
        .iter()
        .filter(|m| !fed.contains(m.id.as_str()))
        .map(|m| m.id.clone())
        .collect();
    Body {
        members,
        edges: inner,
        entries,
    }
}

/// `loopStructureError`: why a loop's body cannot run.
pub fn loop_structure_error(nodes: &[Node], edges: &[Edge], lp: &Node) -> Option<String> {
    let body = loop_body_graph(nodes, edges, lp);
    if body.members.is_empty() {
        return Some("Loop has no body steps. Add at least one step for it to repeat.".into());
    }
    let ids: HashSet<&str> = body.members.iter().map(|m| m.id.as_str()).collect();
    for m in &body.members {
        match m.kind {
            NodeKind::Approval => {
                return Some(format!(
                    "Loop body contains an approval gate (\"{}\"), which is not supported.",
                    m.label
                ))
            }
            NodeKind::Loop => {
                return Some(format!(
                    "Loop body contains another loop (\"{}\"), which is not supported.",
                    m.label
                ))
            }
            NodeKind::Trigger => {
                return Some(format!("Loop body contains a trigger (\"{}\").", m.label))
            }
            _ => {}
        }
    }
    for e in edges {
        if ids.contains(e.target.as_str()) && !ids.contains(e.source.as_str()) && e.source != lp.id
        {
            let from = nodes
                .iter()
                .find(|n| n.id == e.source)
                .map_or(e.source.as_str(), |n| n.label.as_str());
            return Some(format!(
                "\"{from}\" feeds a step inside the loop from outside it. Only the loop starts its steps."
            ));
        }
        if ids.contains(e.source.as_str()) && !ids.contains(e.target.as_str()) && e.branch.is_some()
        {
            return Some(
                "A condition inside the loop branches to a step outside it. Keep both branches inside the loop."
                    .into(),
            );
        }
    }
    // Kahn's algorithm: whatever is left over sits on a cycle.
    let mut indegree: HashMap<&str, usize> =
        body.members.iter().map(|m| (m.id.as_str(), 0)).collect();
    for e in &body.edges {
        *indegree.entry(e.target.as_str()).or_default() += 1;
    }
    let mut queue: VecDeque<&str> = body
        .members
        .iter()
        .map(|m| m.id.as_str())
        .filter(|id| indegree.get(id) == Some(&0))
        .collect();
    let mut visited = 0;
    while let Some(id) = queue.pop_front() {
        visited += 1;
        for e in body.edges.iter().filter(|e| e.source == id) {
            if let Some(d) = indegree.get_mut(e.target.as_str()) {
                *d = d.saturating_sub(1);
                if *d == 0 {
                    queue.push_back(e.target.as_str());
                }
            }
        }
    }
    (visited < body.members.len()).then(|| "The steps inside the loop form a cycle.".to_owned())
}

/// How an agent step finds its worktree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorktreeMode {
    None,
    New,
    FromStep,
    Existing,
    FromContext,
}

/// `getWorktreeMode`.
pub fn worktree_mode(config: &Value) -> WorktreeMode {
    if config.get("useWorktree").and_then(Value::as_str) == Some("fromContext") {
        return WorktreeMode::FromContext;
    }
    match config.get("worktreeMode") {
        None | Some(Value::Null) => {
            if config.get("useWorktree") == Some(&Value::Bool(true)) {
                WorktreeMode::New
            } else {
                WorktreeMode::None
            }
        }
        Some(mode) => match mode.as_str() {
            Some("new") => WorktreeMode::New,
            Some("fromStep") => WorktreeMode::FromStep,
            Some("existing") => WorktreeMode::Existing,
            _ => WorktreeMode::None,
        },
    }
}

/// `webhookTriggerFromItem`: a webhook run's `{{trigger.*}}`, rebuilt from
/// the event's stored request.
pub fn webhook_trigger_from_item(item: Option<&Value>) -> Option<Value> {
    let item = item?;
    if item.get("connectorId").and_then(Value::as_str) != Some("webhook") {
        return None;
    }
    let raw = item.get("raw").cloned().unwrap_or(Value::Null);
    let mut trigger = Map::new();
    trigger.insert("type".into(), json!("webhook"));
    for key in ["body", "headers", "query", "method"] {
        if let Some(v) = raw.get(key) {
            trigger.insert(key.to_owned(), v.clone());
        }
    }
    Some(Value::Object(trigger))
}

/// `schedulerExecutionContext`: a delivered event's context, a webhook's
/// request lifted into `{{trigger.*}}`.
pub fn scheduler_context(
    item: Option<Value>,
    inputs: Option<Map<String, Value>>,
) -> Option<Context> {
    if item.is_none() && inputs.is_none() {
        return None;
    }
    let trigger = webhook_trigger_from_item(item.as_ref());
    Some(Context {
        connector_item: item,
        inputs,
        trigger,
        ..Context::default()
    })
}

/// `getAncestorNodes`: every step above `current`, nearest first, triggers
/// left out.
pub fn ancestors<'a>(nodes: &'a [Node], edges: &[Edge], current: &str) -> Vec<&'a Node> {
    let graph = Graph::of(edges);
    let mut visited = HashSet::from([current.to_owned()]);
    let mut queue = VecDeque::from([current.to_owned()]);
    let mut out = Vec::new();
    while let Some(id) = queue.pop_front() {
        for p in graph.predecessors_of(&id) {
            if !visited.insert(p.clone()) {
                continue;
            }
            queue.push_back(p.clone());
            if let Some(node) = nodes.iter().find(|n| &n.id == p) {
                if node.kind != NodeKind::Trigger {
                    out.push(node);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wf(nodes: Value, edges: Value) -> Workflow {
        Workflow::from_json(&json!({ "id": "w", "name": "W", "nodes": nodes, "edges": edges }))
            .unwrap()
    }

    fn edges(pairs: &[(&str, &str)]) -> Vec<Edge> {
        pairs
            .iter()
            .map(|(s, t)| Edge {
                id: format!("{s}-{t}"),
                source: (*s).into(),
                target: (*t).into(),
                branch: None,
            })
            .collect()
    }

    #[test]
    fn a_log_past_its_limit_keeps_its_end() {
        let mut log = "a".repeat(LOG_BUFFER_MAX);
        append_bounded_log(&mut log, "b");
        assert_eq!(log.len(), LOG_BUFFER_KEEP);
        assert!(log.ends_with('b'));
        let mut short = String::from("x");
        append_bounded_log(&mut short, "y");
        assert_eq!(short, "xy");
    }

    #[test]
    fn fingerprints_name_the_trigger_and_its_inputs() {
        assert_eq!(dedupe_fingerprint(None), "manual");
        let ctx = Context::from_json(&json!({
            "connectorItem": { "connectionId": "c", "externalId": 5 },
            "task": { "id": "t" },
            "inputs": { "b": 2, "a": "x" }
        }))
        .unwrap();
        assert_eq!(
            dedupe_fingerprint(Some(&ctx)),
            r#"item:c:5:inputs:[["a","x"],["b",2]]"#
        );
        let task = Context::from_json(&json!({ "task": { "id": "t" }, "inputs": {} })).unwrap();
        assert_eq!(dedupe_fingerprint(Some(&task)), "task:t");
        let source = Context::from_json(&json!({ "source": {} })).unwrap();
        assert_eq!(dedupe_fingerprint(Some(&source)), "session:undefined");
    }

    #[test]
    fn script_config_fills_templates_and_drops_empty_paths() {
        let ctx = Context::from_json(&json!({ "task": { "title": "T" } })).unwrap();
        let got = resolve_script_config(
            &json!({ "scriptType": "bash", "scriptContent": "echo {{task.title}}", "cwd": "{{context.cwd}}", "args": ["{{task.title}}", 3] }),
            Some(&ctx),
            None,
        );
        assert_eq!(
            got,
            json!({ "scriptType": "bash", "scriptContent": "echo T", "args": ["T", 3] })
        );
    }

    #[test]
    fn outputs_spread_structured_fields_under_the_defaults() {
        let w = wf(
            json!([
                { "id": "a", "type": "callConnectorAction", "slug": "a" },
                { "id": "g", "type": "approval", "slug": "g" },
                { "id": "p", "type": "script", "slug": "p" },
                { "id": "q", "type": "script" }
            ]),
            json!([]),
        );
        let mut run = run_of(&["a", "g", "p", "q"]);
        run.update_all("a", |s| {
            s.set_status(Status::Success);
            s.structured_output = Some(json!({ "html_url": "u", "output": "shadowed" }));
            s.logs = Some("logged".into());
        });
        run.update_all("g", |s| {
            s.set_status(Status::Waiting);
            s.round = Some(2.0);
            s.editable_text = Some(r#"[{"id":1}]"#.into());
            s.feedback = Some(json!([{ "round": 1, "comment": "redo", "comments": [{ "quote": "x", "comment": "c" }] }]));
        });
        let out = step_outputs(&run, &w);
        assert_eq!(
            out["a"],
            json!({ "html_url": "u", "output": "logged", "status": "success", "error": "", "worktreePath": "" })
        );
        assert_eq!(out["g"]["items"], json!([{ "id": 1 }]));
        assert_eq!(out["g"]["feedback"], "redo");
        assert_eq!(out["g"]["feedbackAll"], "Round 1: redo");
        assert_eq!(
            out["g"]["comments"],
            json!([{ "quote": "x", "comment": "c", "anchored": true }])
        );
        assert_eq!(out["g"]["round"], json!(2));
        assert!(!out.contains_key("p"));
    }

    trait UpdateAll {
        fn update_all(&mut self, id: &str, f: impl FnOnce(&mut NodeExecutionState));
    }
    impl UpdateAll for WorkflowExecution {
        fn update_all(&mut self, id: &str, f: impl FnOnce(&mut NodeExecutionState)) {
            if let Some(s) = self.node_states.iter_mut().find(|s| s.node_id == id) {
                f(s);
            }
        }
    }

    fn run_of(ids: &[&str]) -> WorkflowExecution {
        serde_json::from_value(json!({
            "runId": "r", "workflowId": "w", "startedAt": "", "status": "running",
            "nodeStates": ids.iter().map(|id| json!({ "nodeId": id, "status": "pending" })).collect::<Vec<_>>()
        }))
        .unwrap()
    }

    #[test]
    fn a_gate_refuses_a_rewrite_that_stopped_being_a_list() {
        let table = r#"[{"a":1}]"#;
        assert_eq!(gate_edit_refusal(Some(table), Some("  ")), None);
        assert_eq!(gate_edit_refusal(Some("plain text"), Some("{")), None);
        assert!(gate_edit_refusal(Some(table), Some("[{"))
            .unwrap()
            .starts_with("The edited list is not valid JSON: line 1"));
        assert_eq!(
            gate_edit_refusal(Some(table), Some("3")).unwrap(),
            "The edited text is no longer a list. Not a list: expected a JSON array, or an object holding one."
        );
        assert_eq!(gate_edit_refusal(Some(table), Some(r#"[{"a":2}]"#)), None);
    }

    #[test]
    fn rounds_are_bounded_and_read_as_number_reads() {
        assert_eq!(gate_max_rounds(None), DEFAULT_GATE_ROUNDS);
        assert_eq!(gate_max_rounds(Some(&json!({ "maxRounds": "5" }))), 5);
        assert_eq!(
            gate_max_rounds(Some(&json!({ "maxRounds": 50 }))),
            MAX_GATE_ROUNDS
        );
        assert_eq!(gate_max_rounds(Some(&json!({ "maxRounds": null }))), 1);
        assert_eq!(
            gate_max_rounds(Some(&json!({ "maxRounds": "x" }))),
            DEFAULT_GATE_ROUNDS
        );
        let cfg = json!({ "feedback": { "from": "a", "maxRounds": 2 } });
        assert!(can_request_changes(&cfg, None));
        assert!(!can_request_changes(&cfg, Some(2.0)));
        assert!(!can_request_changes(&json!({}), None));
    }

    #[test]
    fn nodes_between_is_every_path() {
        let e = edges(&[("a", "b"), ("b", "c"), ("a", "x"), ("x", "c"), ("c", "d")]);
        let mut got: Vec<_> = nodes_between("a", "c", &e).into_iter().collect();
        got.sort();
        assert_eq!(got, ["a", "b", "c", "x"]);
        assert!(nodes_between("c", "a", &e).is_empty());
        assert!(nodes_between("a", "a", &e).is_empty());
    }

    #[test]
    fn conditions_and_loop_stops() {
        assert!(evaluate_condition("contains", "abc", "b"));
        assert!(evaluate_condition("isEmpty", "  ", ""));
        assert!(!evaluate_condition("odd", "a", "a"));
        let until = json!({ "variable": "{{x}}", "operator": "contains", "value": "" });
        assert!(!loop_should_stop(Some(&until), "anything", ""));
        let until = json!({ "variable": "{{x}}", "operator": "equals", "value": "yes" });
        assert!(loop_should_stop(Some(&until), "yes", "yes"));
        assert!(!loop_should_stop(
            Some(&json!({ "variable": " ", "operator": "isEmpty" })),
            "",
            ""
        ));
        assert!(!loop_should_stop(None, "", ""));
    }

    #[test]
    fn a_failed_branch_stops_at_a_live_join() {
        let e = edges(&[("a", "b"), ("b", "d"), ("c", "d"), ("d", "e")]);
        let g = Graph::of(&e);
        assert_eq!(collect_skipped_branch("b", &g, |_| false), ["b"]);
        assert_eq!(
            collect_skipped_branch("b", &g, |id| id == "c"),
            ["b", "d", "e"]
        );
        assert!(skip_entry_points("c", &e, &g, |_| false).is_empty());
        assert_eq!(skip_entry_points("c", &e, &g, |id| id == "b"), ["d"]);
    }

    #[test]
    fn loops_collapse_and_check_their_bodies() {
        let w = wf(
            json!([
                { "id": "l", "type": "loop", "label": "L", "config": { "bodyNodeIds": ["b1", "b2", "gone", "l"] } },
                { "id": "b1", "type": "script", "label": "B1" },
                { "id": "b2", "type": "script", "label": "B2" },
                { "id": "after", "type": "script", "label": "After" },
                { "id": "l2", "type": "loop", "label": "L2", "config": { "bodyNodeIds": ["b1"] } }
            ]),
            json!([
                { "id": "e1", "source": "l", "target": "b1" },
                { "id": "e2", "source": "b2", "target": "after", "conditionBranch": "true" }
            ]),
        );
        let owners = loop_body_owners(&w.nodes);
        assert_eq!(owners.get("b1").map(String::as_str), Some("l"));
        assert_eq!(owners.len(), 2);
        let collapsed = collapse_loop_bodies(&w.nodes, &w.edges);
        assert_eq!(collapsed.len(), 1);
        assert_eq!(
            (collapsed[0].source.as_str(), collapsed[0].branch.as_ref()),
            ("l", None)
        );
        let body = loop_body_graph(&w.nodes, &w.edges, w.node("l").unwrap());
        assert_eq!(body.edges.len(), 1);
        assert_eq!(body.entries, ["b1"]);
        assert_eq!(
            loop_structure_error(&w.nodes, &w.edges, w.node("l").unwrap()).unwrap(),
            "A condition inside the loop branches to a step outside it. Keep both branches inside the loop."
        );
        assert_eq!(
            loop_structure_error(&w.nodes, &w.edges, w.node("l2").unwrap()).unwrap(),
            "Loop has no body steps. Add at least one step for it to repeat."
        );
    }

    #[test]
    fn a_cycle_in_a_body_is_refused() {
        let w = wf(
            json!([
                { "id": "l", "type": "loop", "config": { "bodyNodeIds": ["a", "b"] } },
                { "id": "a", "type": "script" }, { "id": "b", "type": "script" }
            ]),
            json!([{ "source": "a", "target": "b" }, { "source": "b", "target": "a" }]),
        );
        assert_eq!(
            loop_structure_error(&w.nodes, &w.edges, w.node("l").unwrap()).as_deref(),
            Some("The steps inside the loop form a cycle.")
        );
    }

    #[test]
    fn a_retry_keeps_what_succeeded() {
        let w = wf(
            json!([
                { "id": "t", "type": "trigger" }, { "id": "a", "type": "script" },
                { "id": "b", "type": "script" }, { "id": "c", "type": "script" },
                { "id": "l", "type": "loop", "config": { "bodyNodeIds": ["d"] } }, { "id": "d", "type": "script" }
            ]),
            json!([]),
        );
        let mut failed = run_of(&["t", "a", "b", "c", "l", "d"]);
        failed.update_all("a", |s| s.set_status(Status::Success));
        failed.update_all("b", |s| {
            s.set_status(Status::Skipped);
            s.skip_reason = Some("branch".into());
        });
        failed.update_all("c", |s| s.set_status(Status::Error));
        failed.update_all("d", |s| s.set_status(Status::Success));
        let seeded = seed_retry_states(&w, &failed);
        let statuses: Vec<_> = seeded.iter().map(|s| s.status.0.as_str()).collect();
        assert_eq!(
            statuses,
            ["success", "success", "skipped", "pending", "pending", "pending"]
        );
    }

    #[test]
    fn worktree_modes_and_webhook_contexts() {
        assert_eq!(
            worktree_mode(&json!({ "useWorktree": true })),
            WorktreeMode::New
        );
        assert_eq!(
            worktree_mode(&json!({ "useWorktree": "fromContext", "worktreeMode": "new" })),
            WorktreeMode::FromContext
        );
        assert_eq!(
            worktree_mode(&json!({ "worktreeMode": "existing" })),
            WorktreeMode::Existing
        );
        assert_eq!(worktree_mode(&json!({})), WorktreeMode::None);
        let item =
            json!({ "connectorId": "webhook", "raw": { "body": { "a": 1 }, "method": "POST" } });
        let ctx = scheduler_context(Some(item), None).unwrap();
        assert_eq!(
            ctx.trigger,
            Some(json!({ "type": "webhook", "body": { "a": 1 }, "method": "POST" }))
        );
        assert!(scheduler_context(None, None).is_none());
    }

    #[test]
    fn ancestors_are_nearest_first_without_triggers() {
        let w = wf(
            json!([{ "id": "t", "type": "trigger" }, { "id": "a", "type": "script" }, { "id": "b", "type": "script" }]),
            json!([{ "source": "t", "target": "a" }, { "source": "a", "target": "b" }]),
        );
        let got: Vec<_> = ancestors(&w.nodes, &w.edges, "b")
            .iter()
            .map(|n| n.id.as_str())
            .collect();
        assert_eq!(got, ["a"]);
    }

    #[test]
    fn origins_and_never_ran() {
        assert_eq!(worktree_origin(Some(""), false), None);
        assert_eq!(worktree_origin(Some("/w"), true), Some("inherited"));
        let mut s = state("a", Status::Error);
        s.error = Some(ABANDONED.into());
        assert!(never_ran(&s));
        s.error = Some("boom".into());
        assert!(!never_ran(&s));
    }
}
