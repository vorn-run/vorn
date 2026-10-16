//! The Workflow editor built-in: the Workflows view (the client draws it,
//! on the kit's graph, while this is on), Run workflow in the grid's, a
//! card's and a terminal's menus (`[[contributes.menus]]` with the rows of
//! the workflows each offers), and the approval gates waiting for a person
//! (`[[contributes.attention]]`), which the waiting pill counts and the
//! approve and deny keys answer beside agents asking.
//!
//! The rows it publishes follow the workflows and their runs: the work
//! model tells it when either changes ([`Editor::workflows_changed`],
//! [`Editor::run_changed`]). Off, it publishes nothing and its view, menu
//! items and gates leave every client; the workflows still run.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};

use futures_util::future::BoxFuture;
use futures_util::FutureExt;
use serde_json::{json, Value};
use vorn_extensions::api::types::{Event, RowOp};
use vorn_extensions::api::ExtError;
use vorn_extensions::gate::Caller;
use vorn_protocol::WorkflowExecution;

use super::{arg, unknown, Api, Builtin};
use crate::native::{Answer, Native};

/// The id the built-in goes by.
pub const ID: &str = "vorn.workflow-editor";

/// The workflows the grid's menu offers: every one that is on and asks for no session.
pub const MANUAL: &str = "manual";
/// The workflows a card's and a terminal's menus offer: those that run on a session.
pub const CONTEXTUAL: &str = "contextual";
/// The gates waiting for a person.
pub const GATES: &str = "gates";

/// The built-in, and the gate rows it last published, by run.
pub struct Editor {
    native: Weak<Native>,
    gates: Mutex<BTreeMap<String, Vec<String>>>,
}

impl std::fmt::Debug for Editor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Editor").finish_non_exhaustive()
    }
}

/// Whether a workflow's manual trigger asks for a session.
pub fn contextual(nodes: &Value) -> bool {
    nodes.as_array().into_iter().flatten().any(|n| {
        let cfg = n.get("config");
        n.get("type").and_then(Value::as_str) == Some("trigger")
            && cfg
                .and_then(|c| c.get("triggerType"))
                .and_then(Value::as_str)
                == Some("manual")
            && cfg
                .and_then(|c| c.get("contextual"))
                .and_then(Value::as_bool)
                == Some(true)
    })
}

/// The id of a gate's row: its run and its step.
fn gate_row_id(run: &str, node: &str) -> String {
    format!("{run}|{node}")
}

/// Each step's label in a workflow's `nodes`, by id.
fn labels(nodes: Option<&Value>) -> BTreeMap<String, String> {
    nodes
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|n| {
            Some((
                n.get("id")?.as_str()?.to_owned(),
                n.get("label")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
            ))
        })
        .collect()
}

/// Whether a gate of `run` asks now.
fn asking(run: &WorkflowExecution) -> bool {
    run.status == "running" && run.node_states.iter().any(is_gate)
}

fn is_gate(s: &vorn_protocol::NodeExecutionState) -> bool {
    s.status.0 == "waiting" && s.waiting_for.as_deref() != Some("signIn")
}

/// The gates of `run` asking now, as rows: its workflow is `name`, its steps labelled `labels`.
pub fn gate_rows(
    run: &WorkflowExecution,
    name: &str,
    labels: &BTreeMap<String, String>,
) -> Vec<Value> {
    if run.status != "running" {
        return Vec::new();
    }
    run.node_states
        .iter()
        .filter(|s| is_gate(s))
        .map(|s| {
            let label = labels
                .get(&s.node_id)
                .filter(|l| !l.is_empty())
                .cloned()
                .unwrap_or_else(|| "Approval".to_owned());
            json!({
                "id": gate_row_id(&run.run_id, &s.node_id),
                "title": label,
                "subtitle": name,
                "message": s.message,
                "workflow": run.workflow_id,
                "run": run.run_id,
                "node": s.node_id,
                "at": s.started_at,
            })
        })
        .collect()
}

impl Editor {
    pub fn new(native: &Arc<Native>) -> Arc<Editor> {
        Arc::new(Editor {
            native: Arc::downgrade(native),
            gates: Mutex::default(),
        })
    }

    fn on(&self) -> Option<Arc<Native>> {
        let native = self.native.upgrade()?;
        native
            .extensions()
            .is_some_and(|e| e.builtins().is_on(ID))
            .then_some(native)
    }

    fn publish(native: &Native, collection: &str, ops: Vec<RowOp>) {
        if let Some(ext) = native.extensions() {
            let caller = Caller {
                extension: ID.to_owned(),
                project: None,
            };
            ext.router().publish(&caller, collection, ops);
        }
    }

    /// The workflows changed: the menus' rows again, whole.
    pub async fn workflows_changed(&self) {
        let Some(native) = self.on() else { return };
        let listed = match native
            .answer(
                "workflow:list".to_owned(),
                Value::Null,
                &crate::native::config::Viewer::Local,
            )
            .await
        {
            Answer::Result(Value::Array(list)) => list,
            _ => return,
        };
        let mut manual = vec![RowOp::Clear];
        let mut on_session = vec![RowOp::Clear];
        for wf in listed
            .iter()
            .filter(|w| w.get("enabled") == Some(&Value::Bool(true)))
        {
            let row = json!({
                "id": wf.get("id"),
                "title": wf.get("name"),
                "workspace": wf.get("workspaceId").cloned().unwrap_or(json!("personal")),
            });
            if contextual(wf.get("nodes").unwrap_or(&Value::Null)) {
                on_session.push(RowOp::Upsert(row));
            } else {
                manual.push(RowOp::Upsert(row));
            }
        }
        Self::publish(&native, MANUAL, manual);
        Self::publish(&native, CONTEXTUAL, on_session);
    }

    /// A run moved: its gates' rows, as they ask now.
    pub fn run_changed(&self, run: &WorkflowExecution) {
        // Called on every step's update: a run with no gate, now or before, costs a scan of its states.
        if !asking(run)
            && !self
                .gates
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key(&run.run_id)
        {
            return;
        }
        let Some(native) = self.on() else { return };
        let name = run
            .definition
            .as_ref()
            .and_then(|d| d.get("name"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let labelled = labels(run.definition.as_ref().and_then(|d| d.get("nodes")));
        let rows = gate_rows(run, &name, &labelled);
        let ids: Vec<String> = rows
            .iter()
            .filter_map(|r| r.get("id").and_then(Value::as_str).map(str::to_owned))
            .collect();
        let before = {
            let mut gates = self.gates.lock().unwrap_or_else(|e| e.into_inner());
            if ids.is_empty() {
                gates.remove(&run.run_id).unwrap_or_default()
            } else {
                gates
                    .insert(run.run_id.clone(), ids.clone())
                    .unwrap_or_default()
            }
        };
        if before.is_empty() && ids.is_empty() {
            return;
        }
        let mut ops: Vec<RowOp> = before
            .iter()
            .filter(|b| !ids.contains(b))
            .map(|b| RowOp::Delete(b.clone()))
            .collect();
        ops.extend(rows.into_iter().map(RowOp::Upsert));
        Self::publish(&native, GATES, ops);
    }

    /// Every gate asking now, published whole: as the built-in starts.
    async fn all_gates(&self) {
        let Some(native) = self.on() else { return };
        let waiting = match native
            .answer(
                "workflowRun:listWaiting".to_owned(),
                Value::Null,
                &crate::native::config::Viewer::Local,
            )
            .await
        {
            Answer::Result(Value::Array(list)) => list,
            _ => Vec::new(),
        };
        let workflows: BTreeMap<String, (String, BTreeMap<String, String>)> = match native
            .answer(
                "workflow:list".to_owned(),
                Value::Null,
                &crate::native::config::Viewer::Local,
            )
            .await
        {
            Answer::Result(Value::Array(list)) => list
                .iter()
                .filter_map(|w| {
                    Some((
                        w.get("id")?.as_str()?.to_owned(),
                        (w.get("name")?.as_str()?.to_owned(), labels(w.get("nodes"))),
                    ))
                })
                .collect(),
            _ => BTreeMap::new(),
        };
        let mut ops = vec![RowOp::Clear];
        let mut by_run = BTreeMap::new();
        for r in waiting {
            let Ok(run) = serde_json::from_value::<WorkflowExecution>(r) else {
                continue;
            };
            let (name, labelled) = workflows.get(&run.workflow_id).cloned().unwrap_or_default();
            let rows = gate_rows(&run, &name, &labelled);
            by_run.insert(
                run.run_id.clone(),
                rows.iter()
                    .filter_map(|r| r.get("id").and_then(Value::as_str).map(str::to_owned))
                    .collect::<Vec<_>>(),
            );
            ops.extend(rows.into_iter().map(RowOp::Upsert));
        }
        *self.gates.lock().unwrap_or_else(|e| e.into_inner()) = by_run;
        Self::publish(&native, GATES, ops);
    }

    /// A gate row's run and step.
    fn gate_of(args: &Value, action: &str) -> Result<(String, String), ExtError> {
        let id = arg(args, "id", action)?;
        let (run, node) = id
            .split_once('|')
            .ok_or_else(|| ExtError::new("bad-request", format!("{action} takes a gate's id")))?;
        Ok((run.to_owned(), node.to_owned()))
    }

    async fn handle_action(&self, action: &str, args: Value) -> Result<Value, ExtError> {
        let native = self
            .native
            .upgrade()
            .ok_or_else(|| ExtError::new("stopping", "vornd is stopping"))?;
        let ask = |method: &str, params: Value| {
            let native = Arc::clone(&native);
            let method = method.to_owned();
            async move {
                match native
                    .answer(method, params, &crate::native::config::Viewer::Local)
                    .await
                {
                    Answer::Result(v) => Ok(v),
                    Answer::Void | Answer::Unanswered => Ok(Value::Null),
                    Answer::Error(e) => Err(ExtError::new("failed", e)),
                }
            }
        };
        match action {
            "run" => {
                let id = arg(&args, "id", action)?;
                let inputs = args.get("inputs").cloned().unwrap_or(Value::Null);
                ask(
                    "workflow:runManual",
                    json!({ "workflowId": id, "inputs": inputs }),
                )
                .await
            }
            "approve" | "reject" => {
                let (run, node) = Self::gate_of(&args, action)?;
                let decision = if action == "approve" {
                    "approve"
                } else {
                    "reject"
                };
                ask(
                    "workflow:resolveGate",
                    json!({ "runId": run, "nodeId": node, "decision": decision }),
                )
                .await
            }
            // Opening a card, dropping it and drawing an edge are the client's edits.
            "open" | "move" | "connect" => Ok(Value::Null),
            _ => Err(unknown("Workflow editor", action)),
        }
    }
}

/// The editor as an entry of the built-ins' registry.
pub fn entry(editor: &Arc<Editor>) -> Arc<dyn Builtin> {
    Arc::new(Entry(Arc::clone(editor)))
}

struct Entry(Arc<Editor>);

impl Builtin for Entry {
    fn manifest(&self) -> &'static str {
        include_str!("workflow-editor.toml")
    }

    fn views(&self) -> &[(&'static str, &'static str)] {
        &[]
    }

    fn events(&self) -> &'static [&'static str] {
        &[]
    }

    fn on_by_default(&self) -> bool {
        true
    }

    fn start<'a>(&'a self, _api: &'a Api) -> BoxFuture<'a, ()> {
        async move {
            self.0.workflows_changed().await;
            self.0.all_gates().await;
        }
        .boxed()
    }

    fn handle<'a>(
        &'a self,
        _api: &'a Api,
        action: &'a str,
        args: Value,
    ) -> BoxFuture<'a, Result<Value, ExtError>> {
        async move { self.0.handle_action(action, args).await }.boxed()
    }

    fn on_events<'a>(&'a self, _api: &'a Api, _batch: Vec<Arc<Event>>) -> BoxFuture<'a, ()> {
        async {}.boxed()
    }

    fn stop<'a>(&'a self, _api: &'a Api) -> BoxFuture<'a, ()> {
        async move {
            self.0
                .gates
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
        }
        .boxed()
    }
}
