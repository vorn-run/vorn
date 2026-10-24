//! The agent step (`launchAgent`): resolves its task, project, worktree
//! and prompt, then starts the agent headless and waits for it, or opens it
//! in a terminal. A task the step works on is taken for its session, and
//! handed back when a headless agent fails, through the Tasks built-in.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};
use tokio::sync::broadcast::error::RecvError;
use tracing::{info, warn};
use vorn_work::graph::{
    append_bounded_log, worktree_mode, worktree_origin, WorktreeMode, DEFAULT_STEP_TIMEOUT_MINUTES,
};
use vorn_work::js;
use vorn_work::model::{Context, Node, RunExt, StateExt, Status};
use vorn_work::prompt::{task_prompt, workflow_prompt};
use vorn_work::structured;
use vorn_work::template::{context_field, resolve, Field};
use vorn_workflow::{BoxFuture, Host, Note, Step, StepCx};

use crate::{nonempty, put, text_of};

/// How often a headless step's output is written while it runs.
const PERSIST_INTERVAL: Duration = Duration::from_secs(3);

/// What the engine did on a step's behalf, timed from the step's start.
struct Diagnostics {
    lines: Vec<String>,
    started: Instant,
}

impl Diagnostics {
    fn new() -> Diagnostics {
        Diagnostics {
            lines: Vec::new(),
            started: Instant::now(),
        }
    }

    fn note(&mut self, message: impl AsRef<str>) {
        let seconds = self.started.elapsed().as_secs_f64();
        self.lines
            .push(format!("[+{seconds:.1}s] {}", message.as_ref()));
    }

    fn text(&self) -> String {
        self.lines.join("\n")
    }
}

/// How a headless step ended.
enum Outcome {
    Exit(i64),
    Timeout(u64),
    Stopped,
}

/// A project's first host that is not this machine: where its agent steps run.
pub fn remote_host_of(project: &Value) -> Option<String> {
    let ids = project
        .get("hostIds")
        .and_then(Value::as_array)
        .filter(|ids| !ids.is_empty())?;
    ids.iter()
        .filter_map(Value::as_str)
        .find(|id| *id != "local")
        .map(str::to_owned)
}

/// The agent step.
#[derive(Debug, Default)]
pub struct Agent;

impl<H: Host> Step<H> for Agent {
    fn run<'a>(&'a self, cx: StepCx<'a, H>) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move { self.agent(&cx).await })
    }

    fn made_worktree(&self, node: &Node) -> bool {
        worktree_mode(&node.config) == WorktreeMode::New
    }
}

impl Agent {
    /// An agent step: resolves its task, project, worktree and prompt, then
    /// starts it headless and waits for it, or opens it in a terminal.
    async fn agent<H: Host>(&self, cx: &StepCx<'_, H>) -> Result<(), String> {
        let (node, workflow, context, outputs) = (cx.node, cx.workflow, cx.context, cx.outputs);
        let config = &node.config;
        let headless = vorn_work::is_truthy(config.get("headless"));
        info!(step = %node.label, headless, "an agent step starts");
        let mut prompt = text_of(config.get("prompt"));
        let mut task_id: Option<String> = None;
        let mut branch = text_of(config.get("branch"))
            .filter(|b| !b.is_empty())
            .and_then(|b| nonempty(resolve(&b, context, None)));
        let from_context = config.get("useWorktree").and_then(Value::as_str) == Some("fromContext");
        let mut use_worktree: Option<Value> = if from_context {
            context
                .and_then(|c| context_field("useWorktree", c))
                .map(|f| match f {
                    Field::Flag(b) => Value::Bool(b),
                    Field::Text(t) => Value::String(t),
                })
        } else {
            config.get("useWorktree").filter(|v| !v.is_null()).cloned()
        };
        let mut existing: Option<String> = None;
        let current = cx.config().await;
        let empty = Value::Null;
        let cfg = current.as_ref().unwrap_or(&empty);

        match worktree_mode(config) {
            WorktreeMode::FromStep => {
                let slug = node
                    .text("worktreeFromStepSlug")
                    .filter(|s| !s.is_empty())
                    .ok_or("Worktree mode \"fromStep\" requires a source step slug".to_owned())?;
                let mut by_key: HashMap<&str, &Node> = HashMap::new();
                for n in &workflow.nodes {
                    by_key.insert(n.slug.as_deref().unwrap_or(&n.id), n);
                }
                let source = by_key
                    .get(slug)
                    .ok_or_else(|| format!("Worktree source step \"{slug}\" not found"))?;
                let path = cx
                    .read(|r| {
                        r.node_state(&source.id)
                            .and_then(|s| s.worktree_path.clone())
                    })
                    .filter(|p| !p.is_empty())
                    .ok_or_else(|| format!("Source step \"{slug}\" has no worktreePath"))?;
                existing = Some(path);
                use_worktree = None;
            }
            WorktreeMode::Existing => {
                let path = node
                    .text("existingWorktreePath")
                    .filter(|p| !p.is_empty())
                    .ok_or(
                        "Worktree mode \"existing\" requires an existingWorktreePath".to_owned(),
                    )?;
                existing = Some(path.to_owned());
                use_worktree = None;
            }
            WorktreeMode::FromContext => {
                if let Some(Field::Text(path)) =
                    context.and_then(|c| context_field("worktreePath", c))
                {
                    if !path.is_empty() {
                        existing = Some(path);
                        use_worktree = None;
                    }
                }
            }
            _ => {}
        }

        let projects: &[Value] = cfg
            .get("projects")
            .and_then(Value::as_array)
            .map_or(&[], Vec::as_slice);
        let project_named = |name: &str| {
            projects
                .iter()
                .find(|p| p.get("name").and_then(Value::as_str) == Some(name))
        };
        let static_task = config
            .get("taskId")
            .filter(|v| !v.is_null())
            .map(js::to_string);
        let effective_task =
            static_task.or_else(|| context.and_then(Context::task_id).map(str::to_owned));
        let resolved_task: Option<Value> =
            if let Some(id) = effective_task.filter(|id| !id.is_empty()) {
                cx.host().task(&id).await.filter(|t| {
                    !matches!(
                        t.get("status").and_then(Value::as_str),
                        Some("done" | "cancelled")
                    )
                })
            } else if vorn_work::is_truthy(config.get("taskFromQueue")) {
                let project = node.text("projectName").unwrap_or("");
                let mut queued = cx.host().tasks(project, Some("todo")).await;
                queued.sort_by(|a, b| {
                    let o = |t: &Value| t.get("order").and_then(Value::as_f64).unwrap_or(0.0);
                    o(a).total_cmp(&o(b))
                });
                queued.into_iter().next()
            } else {
                None
            };
        if let Some(task) = &resolved_task {
            let project_name = task
                .get("projectName")
                .and_then(Value::as_str)
                .unwrap_or("");
            prompt = Some(match project_named(project_name) {
                Some(project) => {
                    let siblings = cx.host().tasks(project_name, None).await;
                    task_prompt(task, project, &siblings)
                }
                None => text_of(task.get("description")).unwrap_or_default(),
            });
            task_id = text_of(task.get("id"));
            if existing.is_none() {
                branch = text_of(task.get("branch"))
                    .filter(|b| !b.is_empty())
                    .or(branch.take());
                use_worktree = task
                    .get("useWorktree")
                    .filter(|v| vorn_work::is_truthy(Some(v)))
                    .cloned()
                    .or(use_worktree.take());
            }
        }

        // With nothing assigned and no default, a task's step runs the first agent that is on.
        let first_on = cx.host().first_agent();
        let agent = self.effective_agent(
            config,
            context,
            resolved_task.as_ref(),
            cfg,
            first_on.as_deref().unwrap_or("shell"),
        )?;

        let mut project_name = resolve(node.text("projectName").unwrap_or(""), context, None);
        let mut project_path = resolve(node.text("projectPath").unwrap_or(""), context, None);
        if project_name.is_empty() || project_path.is_empty() {
            let owner = context
                .and_then(|c| c.task.as_ref())
                .or(resolved_task.as_ref());
            if let Some(project) = owner
                .and_then(|t| t.get("projectName").and_then(Value::as_str))
                .and_then(project_named)
            {
                if project_name.is_empty() {
                    project_name = text_of(project.get("name")).unwrap_or_default();
                }
                if project_path.is_empty() {
                    project_path = text_of(project.get("path")).unwrap_or_default();
                }
            }
        }
        if !project_name.is_empty() && project_path.is_empty() {
            if let Some(project) = project_named(&project_name) {
                project_path = text_of(project.get("path")).unwrap_or_default();
            }
        }

        let schema = headless
            .then(|| {
                config
                    .get("outputSchema")
                    .filter(|s| vorn_work::is_truthy(Some(s)))
                    .cloned()
            })
            .flatten();
        let step_name = text_of(config.get("displayName"))
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| node.label.clone());
        let prompt = prompt.map(|p| {
            if p.is_empty() {
                return p;
            }
            let p = resolve(&p, context, Some(outputs));
            if p.is_empty() {
                return p;
            }
            workflow_prompt(
                &workflow.id,
                &workflow.name,
                &step_name,
                &p,
                schema.as_ref(),
            )
        });
        let inherited = from_context;

        let mut payload = Map::new();
        put(&mut payload, "agentType", agent.clone().map(Value::String));
        payload.insert("projectName".into(), Value::String(project_name.clone()));
        payload.insert("projectPath".into(), Value::String(project_path.clone()));
        payload.insert("displayName".into(), Value::String(step_name));
        put(&mut payload, "branch", branch.map(Value::String));
        put(&mut payload, "useWorktree", use_worktree);
        put(
            &mut payload,
            "existingWorktreePath",
            existing.clone().map(Value::String),
        );
        put(
            &mut payload,
            "initialPrompt",
            prompt.clone().map(Value::String),
        );
        put(
            &mut payload,
            "promptDelayMs",
            config
                .get("promptDelayMs")
                .filter(|v| !v.is_null())
                .cloned(),
        );

        if !headless {
            put(
                &mut payload,
                "args",
                config.get("args").filter(|v| !v.is_null()).cloned(),
            );
            put(
                &mut payload,
                "model",
                config.get("model").filter(|v| !v.is_null()).cloned(),
            );
            let remote = project_named(&project_name).and_then(remote_host_of);
            put(&mut payload, "remoteHostId", remote.map(Value::String));
            let session = cx
                .host()
                .call("terminal:create", Value::Object(payload))
                .await?;
            let session_id = session.get("id").map(js::to_string).unwrap_or_default();
            if let (Some(task), Some(agent)) = (&task_id, &agent) {
                take_task(cx, task, &session_id, agent).await;
            }
            let worktree = text_of(session.get("worktreePath"));
            cx.finish(|s| {
                s.set_status(Status::Success);
                s.session_id = Some(session_id.clone());
                s.logs = Some(format!("Terminal session created: {session_id}"));
                s.task_id = task_id.clone();
                s.worktree_origin =
                    worktree_origin(worktree.as_deref(), inherited).map(str::to_owned);
                s.worktree_path = worktree;
                s.worktree_name = text_of(session.get("worktreeName"));
                s.agent_type = agent.clone().map(Into::into);
                s.project_name = Some(project_name);
                s.project_path = Some(project_path);
            });
            return Ok(());
        }

        payload.insert("headless".into(), Value::Bool(true));
        payload.insert(
            "origin".into(),
            json!({ "extension": "vorn.workflows", "id": workflow.id, "label": workflow.name }),
        );
        put(
            &mut payload,
            "args",
            config.get("args").filter(|v| !v.is_null()).cloned(),
        );
        put(
            &mut payload,
            "model",
            config.get("model").filter(|v| !v.is_null()).cloned(),
        );

        let mut diag = Diagnostics::new();
        let at = existing
            .clone()
            .filter(|p| !p.is_empty())
            .or_else(|| nonempty(project_path.clone()))
            .unwrap_or_else(|| "(no path)".into());
        diag.note(format!(
            "Launching {} in {at}{}",
            agent.as_deref().unwrap_or("undefined"),
            match prompt.as_deref().filter(|p| !p.is_empty()) {
                Some(p) => format!(" with a {}-line prompt", p.split('\n').count()),
                None => " with no prompt".into(),
            }
        ));
        cx.update(|s| s.diagnostics = Some(diag.text()));
        cx.publish();

        let timeout_ms = self.step_timeout_ms(config, cfg);
        let result = self
            .wait_headless(
                cx,
                Value::Object(payload),
                &mut diag,
                timeout_ms,
                task_id.as_deref(),
                agent.as_deref(),
                inherited,
                &project_name,
                &project_path,
            )
            .await;
        cx.update(|s| s.diagnostics = Some(diag.text()));
        result
    }

    /// Starts a headless agent and waits for it to exit, time out or be
    /// stopped. An exit seen before its id is known is kept for it.
    #[allow(clippy::too_many_arguments)]
    async fn wait_headless<H: Host>(
        &self,
        cx: &StepCx<'_, H>,
        payload: Value,
        diag: &mut Diagnostics,
        timeout_ms: u64,
        task_id: Option<&str>,
        agent: Option<&str>,
        inherited: bool,
        project_name: &str,
        project_path: &str,
    ) -> Result<(), String> {
        let mut notes = cx.host().notes();
        let create = cx.host().call("headless:create", payload);
        tokio::pin!(create);
        let mut early_exits: HashMap<String, i64> = HashMap::new();
        let mut early_data: Vec<(String, String)> = Vec::new();
        let created = loop {
            tokio::select! {
                created = &mut create => break created,
                note = notes.recv() => match note {
                    Ok(Note::HeadlessExit { id, code }) => { early_exits.insert(id, code); }
                    Ok(Note::HeadlessData { id, data }) => early_data.push((id, data)),
                    Err(RecvError::Closed) => break (&mut create).await,
                    _ => {}
                },
            }
        };
        // What was said while the answer was on its way.
        while let Ok(note) = notes.try_recv() {
            match note {
                Note::HeadlessExit { id, code } => {
                    early_exits.insert(id, code);
                }
                Note::HeadlessData { id, data } => early_data.push((id, data)),
                Note::ScriptData { .. } => {}
            }
        }
        let session = match created {
            Ok(session) => session,
            Err(message) => {
                diag.note(format!("Could not start: {message}"));
                return Err(message);
            }
        };
        let session_id = session.get("id").map(js::to_string).unwrap_or_default();
        cx.track_session(&session_id);
        let pid = session
            .get("pid")
            .filter(|p| vorn_work::is_truthy(Some(p)))
            .map_or_else(|| "unknown".to_owned(), js::to_string);
        let command = session
            .get("launchCommand")
            .filter(|c| vorn_work::is_truthy(Some(c)))
            .map(|c| format!(": {}", js::to_string(c)))
            .unwrap_or_default();
        diag.note(format!("Session {session_id} started (pid {pid}){command}"));
        if timeout_ms > 0 {
            diag.note(format!(
                "Will give up after {} min without an exit",
                (timeout_ms as f64 / 60_000.0).round()
            ));
        }
        let mut outcome = None;
        if let Some(code) = early_exits.get(&session_id) {
            diag.note(format!(
                "Agent had already exited (code {code}) before its id reached us"
            ));
            outcome = Some(Outcome::Exit(*code));
        }
        let worktree = text_of(session.get("worktreePath"));
        cx.update(|s| {
            s.session_id = Some(session_id.clone());
            s.task_id = task_id.map(str::to_owned);
            s.worktree_origin = worktree_origin(worktree.as_deref(), inherited).map(str::to_owned);
            s.worktree_path = worktree.clone();
            s.worktree_name = text_of(session.get("worktreeName"));
            s.agent_type = agent.map(|a| a.to_owned().into());
            s.project_name = Some(project_name.to_owned());
            s.project_path = Some(project_path.to_owned());
            if let Some(conv) = session
                .get("agentSessionId")
                .filter(|c| vorn_work::is_truthy(Some(c)))
            {
                s.agent_session_id = Some(js::to_string(conv));
            }
        });
        cx.persist();
        if let (Some(task), Some(agent)) = (task_id, agent) {
            take_task(cx, task, &session_id, agent).await;
        }

        let mut logs = String::new();
        let mut bytes: usize = 0;
        let mut persisted_len = 0;
        let mut save_at: Option<tokio::time::Instant> = None;
        let deadline = (timeout_ms > 0)
            .then(|| tokio::time::Instant::now() + Duration::from_millis(timeout_ms));
        for (id, data) in early_data {
            if id == session_id {
                agent_spoke(cx, diag, &mut logs, &mut bytes, &data);
                save_at.get_or_insert_with(|| tokio::time::Instant::now() + PERSIST_INTERVAL);
            }
        }
        let outcome = match outcome {
            Some(o) => o,
            None => loop {
                let save_sleep = async {
                    match save_at {
                        Some(at) => tokio::time::sleep_until(at).await,
                        None => std::future::pending().await,
                    }
                };
                let timeout_sleep = async {
                    match deadline {
                        Some(at) => tokio::time::sleep_until(at).await,
                        None => std::future::pending().await,
                    }
                };
                tokio::select! {
                    note = notes.recv() => match note {
                        Ok(Note::HeadlessExit { id, code }) if id == session_id => break Outcome::Exit(code),
                        Ok(Note::HeadlessData { id, data }) if id == session_id => {
                            agent_spoke(cx, diag, &mut logs, &mut bytes, &data);
                            save_at.get_or_insert_with(|| tokio::time::Instant::now() + PERSIST_INTERVAL);
                        }
                        Err(RecvError::Closed) => break Outcome::Stopped,
                        _ => {}
                    },
                    () = save_sleep => {
                        save_at = None;
                        if logs.len() != persisted_len {
                            persisted_len = logs.len();
                            cx.save();
                        }
                    }
                    () = timeout_sleep => break Outcome::Timeout(timeout_ms),
                    () = cx.stopped() => break Outcome::Stopped,
                }
            },
        };

        match outcome {
            Outcome::Stopped => {
                diag.note("Stopped by user");
                Ok(())
            }
            Outcome::Timeout(after) => {
                let minutes = (after as f64 / 60_000.0).round() as u64;
                let plural = if minutes == 1 { "" } else { "s" };
                let reason = if bytes == 0 {
                    format!("Step timed out after {minutes} minute{plural}. The agent was started but never produced any output, which usually means it never really ran or is waiting on input it will never get.")
                } else {
                    format!("Step timed out after {minutes} minute{plural} without the agent exiting, after {bytes} bytes of output.")
                };
                warn!(step = %cx.node.label, %reason, session = %session_id, "an agent step timed out");
                diag.note(&reason);
                match cx
                    .host()
                    .call("headless:kill", Value::String(session_id.clone()))
                    .await
                {
                    Ok(_) => diag.note("Agent killed"),
                    Err(err) => diag.note(format!("Could not kill the agent: {err}")),
                }
                let text = diag.text();
                cx.finish(|s| {
                    s.set_status(Status::Error);
                    s.output = Some(logs.clone());
                    s.logs = Some(logs);
                    s.error = Some(reason);
                    s.diagnostics = Some(text);
                });
                if let Some(task) = task_id {
                    reopen_task(cx, task).await;
                }
                Ok(())
            }
            Outcome::Exit(code) => {
                diag.note(format!(
                    "Agent exited with code {code} after {bytes} bytes of output{}",
                    if bytes == 0 {
                        " — it produced nothing at all"
                    } else {
                        ""
                    }
                ));
                if code != 0 {
                    logs.push_str(&format!("\nProcess exited with code {code}"));
                }
                let schema = cx
                    .node
                    .config
                    .get("outputSchema")
                    .filter(|s| vorn_work::is_truthy(Some(s)));
                let (typed, schema_error) = match (code, schema) {
                    (0, Some(schema)) => match structured::extract(&logs, schema) {
                        Ok(obj) => (Some(Value::Object(obj)), None),
                        Err(e) => (None, Some(e)),
                    },
                    _ => (None, None),
                };
                if let Some(e) = &schema_error {
                    diag.note(format!("Output did not match the declared schema: {e}"));
                }
                let failed = code != 0 || schema_error.is_some();
                let text = diag.text();
                cx.finish(|s| {
                    s.diagnostics = Some(text);
                    s.set_status(if failed {
                        Status::Error
                    } else {
                        Status::Success
                    });
                    s.output = Some(logs.clone());
                    s.logs = Some(logs);
                    if typed.is_some() {
                        s.structured_output = typed;
                    }
                    if code != 0 {
                        s.error = Some(format!("Exit code {code}"));
                    } else if let Some(e) = schema_error {
                        s.error = Some(e);
                    }
                });
                if failed {
                    if let Some(task) = task_id {
                        reopen_task(cx, task).await;
                    }
                }
                Ok(())
            }
        }
    }
}

fn agent_spoke<H: Host>(
    cx: &StepCx<'_, H>,
    diag: &mut Diagnostics,
    logs: &mut String,
    bytes: &mut usize,
    data: &str,
) {
    if *bytes == 0 {
        diag.note(format!(
            "First output from the agent ({} bytes)",
            js::utf16_len(data)
        ));
        let text = diag.text();
        cx.update(|s| s.diagnostics = Some(text));
    }
    *bytes += js::utf16_len(data);
    append_bounded_log(logs, data);
    cx.update(|s| s.logs = Some(logs.clone()));
    cx.publish();
}

impl Agent {
    /// The agent a step runs: a concrete one as it is; `fromTask` reads the
    /// trigger's task, then the step's, then the default, then `first_on`.
    fn effective_agent(
        &self,
        config: &Value,
        context: Option<&Context>,
        task: Option<&Value>,
        cfg: &Value,
        first_on: &str,
    ) -> Result<Option<String>, String> {
        let agent = config.get("agentType");
        if agent.and_then(Value::as_str) != Some("fromTask") {
            return Ok(agent.filter(|a| !a.is_null()).map(js::to_string));
        }
        if config.get("model").is_some_and(|m| !m.is_null()) {
            return Err(
                "A model needs a concrete agent. Clear the model or choose an agent for this step."
                    .into(),
            );
        }
        let assigned = |t: Option<&Value>| {
            t.and_then(|t| t.get("assignedAgent"))
                .filter(|a| !a.is_null())
                .map(js::to_string)
        };
        Ok(Some(
            assigned(context.and_then(|c| c.task.as_ref()))
                .or_else(|| assigned(task))
                .or_else(|| {
                    cfg.get("defaults")
                        .and_then(|d| d.get("defaultAgent"))
                        .filter(|a| !a.is_null())
                        .map(js::to_string)
                })
                .unwrap_or_else(|| first_on.to_owned()),
        ))
    }

    /// The step's own ceiling, else Workflows' Step time limit; 0 disables it.
    fn step_timeout_ms(&self, config: &Value, cfg: &Value) -> u64 {
        if let Some(ms) = config.get("timeoutMs").and_then(Value::as_f64) {
            return ms.max(0.0) as u64;
        }
        let minutes = cfg
            .get("preferences")
            .and_then(|p| p.get(crate::STEP_TIME_LIMIT))
            .and_then(Value::as_str)
            .and_then(crate::minutes)
            .unwrap_or(DEFAULT_STEP_TIMEOUT_MINUTES);
        if minutes > 0.0 {
            (minutes * 60_000.0) as u64
        } else {
            0
        }
    }
}

/// Takes the task for the step's session, through the Tasks built-in.
async fn take_task<H: Host>(cx: &StepCx<'_, H>, task: &str, session: &str, agent: &str) {
    let args = json!({ "id": task, "sessionId": session, "agent": agent });
    if let Err(err) = cx.host().api("task.take", args).await {
        warn!(%task, %err, "a step could not take its task");
    }
}

/// Hands the task back to the queue, unassigned, through the Tasks built-in.
async fn reopen_task<H: Host>(cx: &StepCx<'_, H>, task: &str) {
    if let Err(err) = cx.host().api("task.handBack", json!({ "id": task })).await {
        warn!(%task, %err, "a step could not hand its task back");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_runs_for_its_own_limit_else_the_workflows_preference() {
        let cfg = |limit: &str| json!({ "preferences": { crate::STEP_TIME_LIMIT: limit } });
        let none = json!({});
        assert_eq!(
            Agent.step_timeout_ms(&none, &Value::Null),
            (DEFAULT_STEP_TIMEOUT_MINUTES * 60_000.0) as u64
        );
        assert_eq!(Agent.step_timeout_ms(&none, &cfg("m15")), 15 * 60_000);
        assert_eq!(Agent.step_timeout_ms(&none, &cfg("m0")), 0);
        let own = json!({ "timeoutMs": 5_000 });
        assert_eq!(Agent.step_timeout_ms(&own, &cfg("m15")), 5_000);
    }
}
