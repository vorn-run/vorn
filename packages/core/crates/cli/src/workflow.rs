//! `vorn workflow list|run|stop|runs`: workflows and their runs in the
//! running server.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Map, Value};

use crate::client::{CommandError, Context};
use crate::exit::ExitCode;
use crate::js::{self, field, present, truthy};
use crate::output::{paint_status, short_id, table, time_ago};

pub const WORKFLOW_USAGE: &str = "Usage
  vorn workflow list [--json]
  vorn workflow run <name or id> [--input key=value] [--json]
  vorn workflow stop <run>
  vorn workflow runs [--workflow <name or id>] [--limit <n>] [--json]

A run happens in the server, so it keeps going when nothing is watching. The
command answers with the run rather than waiting for it to finish.
";

fn items(value: Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items,
        _ => Vec::new(),
    }
}

fn text(value: Option<&Value>) -> String {
    js::string(value)
}

fn id_of(value: &Value) -> String {
    text(field(value, "id"))
}

fn name_of(value: &Value) -> String {
    text(field(value, "name"))
}

/// What starts a workflow, read off its trigger node.
fn trigger_kind(workflow: &Value) -> String {
    let nodes = field(workflow, "nodes").and_then(Value::as_array);
    let trigger = nodes.and_then(|nodes| {
        nodes
            .iter()
            .find(|n| field(n, "type").and_then(Value::as_str) == Some("trigger"))
    });
    let Some(trigger) = trigger else {
        return "none".into();
    };
    match present(field(trigger, "config").and_then(|c| field(c, "triggerType"))) {
        Some(kind) => text(Some(kind)),
        None => "manual".into(),
    }
}

/// A workflow by id, by a prefix of one, or by name, and only when that names
/// exactly one of them.
///
/// A prefix that matches several is refused rather than resolved to the
/// first: seeded and imported workflows carry ids like `import:foo`, so shared
/// prefixes are ordinary, and acting on the wrong workflow is not a small
/// mistake.
fn find_workflow<'w>(workflows: &'w [Value], given: &str) -> Result<&'w Value, CommandError> {
    if let Some(exact) = workflows.iter().find(|w| id_of(w) == given) {
        return Ok(exact);
    }
    let by_prefix: Vec<&Value> = workflows
        .iter()
        .filter(|w| id_of(w).starts_with(given))
        .collect();
    match by_prefix.len() {
        1 => return Ok(by_prefix[0]),
        0 => {}
        n => {
            return Err(CommandError::Refused(format!(
                "\"{given}\" matches {n} workflows: {}",
                by_prefix
                    .iter()
                    .map(|w| name_of(w))
                    .collect::<Vec<_>>()
                    .join(", ")
            )))
        }
    }

    // Trimmed on both sides: a trailing space is invisible in the list, so two
    // rows that read the same must be ambiguous rather than quietly distinct.
    let wanted = crate::args::js_trim(given).to_lowercase();
    let by_name: Vec<&Value> = workflows
        .iter()
        .filter(|w| crate::args::js_trim(&name_of(w)).to_lowercase() == wanted)
        .collect();
    match by_name.len() {
        1 => Ok(by_name[0]),
        0 => Err(CommandError::Refused(format!(
            "no workflow matches \"{given}\""
        ))),
        n => Err(CommandError::Refused(format!(
            "\"{given}\" matches {n} workflows; use an id"
        ))),
    }
}

async fn list(ctx: &mut Context<'_>) -> Result<ExitCode, CommandError> {
    let workflows = ctx.call("workflow:list", None).await?;
    if ctx.args.json {
        ctx.io.write(&js::as_json(&workflows));
        return Ok(ExitCode::Ok);
    }
    let workflows = items(workflows);
    if workflows.is_empty() {
        ctx.io.write_err("No workflows.\n");
        return Ok(ExitCode::Ok);
    }
    // How it went and when are two columns: colour belongs on the status
    // word alone, and it cannot be picked out of "success 2h ago".
    const STATUS_COLUMN: usize = 4;
    let now = crate::time::now_ms();
    let rows: Vec<Vec<String>> = workflows
        .iter()
        .map(|w| {
            let ran = truthy(field(w, "lastRunAt"));
            vec![
                short_id(&id_of(w)).to_owned(),
                name_of(w),
                trigger_kind(w),
                if truthy(field(w, "enabled")) {
                    "yes"
                } else {
                    "no"
                }
                .to_owned(),
                if ran {
                    present(field(w, "lastRunStatus"))
                        .map_or_else(|| "ran".to_owned(), |s| text(Some(s)))
                } else {
                    "-".to_owned()
                },
                if ran {
                    time_ago(field(w, "lastRunAt"), now)
                } else {
                    "-".to_owned()
                },
            ]
        })
        .collect();
    let plain = ctx.plain;
    let paint = move |cell: &str, column: usize| {
        if column == STATUS_COLUMN {
            paint_status(cell, plain)
        } else {
            cell.to_owned()
        }
    };
    ctx.io.write(&table(
        &["ID", "NAME", "TRIGGER", "ENABLED", "LAST RUN", "WHEN"],
        &rows,
        Some(&paint),
    ));
    Ok(ExitCode::Ok)
}

async fn start_run(ctx: &mut Context<'_>, given: &str) -> Result<ExitCode, CommandError> {
    let workflows = items(ctx.call("workflow:list", None).await?);
    let workflow = find_workflow(&workflows, given)?;
    let mut params = Map::new();
    params.insert(
        "workflowId".into(),
        field(workflow, "id").cloned().unwrap_or(Value::Null),
    );
    if let Some(inputs) = &ctx.args.inputs {
        params.insert("context".into(), json!({ "inputs": inputs }));
    }
    let execution = ctx
        .call("workflow:run", Some(Value::Object(params)))
        .await?;
    if !truthy(Some(&execution)) {
        ctx.io.write_err(&format!(
            "vorn: \"{}\" did not start. Check the server log.\n",
            name_of(workflow)
        ));
        return Ok(ExitCode::Failure);
    }

    if ctx.args.json {
        ctx.io.write(&js::as_json(&execution));
        return Ok(ExitCode::Ok);
    }
    let pending = field(&execution, "nodeStates")
        .and_then(Value::as_array)
        .map_or(0, |states| {
            states
                .iter()
                .filter(|s| field(s, "status").and_then(Value::as_str) == Some("pending"))
                .count()
        });
    ctx.io.write(&format!(
        "run      {}\nworkflow {}\nsteps    {pending} to run\n",
        text(field(&execution, "runId")),
        name_of(workflow)
    ));
    ctx.io.write_err(&format!(
        "Follow it with: vorn workflow runs --workflow {}\n",
        short_id(&id_of(workflow))
    ));
    Ok(ExitCode::Ok)
}

/// A run by id, or by any prefix that names one.
///
/// Looked for among the runs that can be stopped rather than the most recent
/// fifty of everything: a run parked on a gate for a day is exactly the one
/// worth stopping, and it falls off the end of that listing.
async fn resolve_run_id(ctx: &Context<'_>, given: &str) -> Result<String, CommandError> {
    let (running, waiting) = ctx
        .call_both("workflowRun:listRunning", "workflowRun:listWaiting")
        .await?;
    let mut seen = HashSet::new();
    let runs: Vec<String> = items(running)
        .iter()
        .chain(items(waiting).iter())
        .map(|run| text(field(run, "runId")))
        .filter(|id| seen.insert(id.clone()))
        .collect();
    if runs.iter().any(|id| id == given) {
        return Ok(given.to_owned());
    }
    let mut matches: Vec<String> = runs
        .into_iter()
        .filter(|id| id.starts_with(given))
        .collect();
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err(CommandError::Refused(format!(
            "no run in flight matches \"{given}\""
        ))),
        n => Err(CommandError::Refused(format!(
            "\"{given}\" matches {n} runs: {}",
            matches
                .iter()
                .map(|id| short_id(id))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

async fn stop_run(ctx: &mut Context<'_>, given: &str) -> Result<ExitCode, CommandError> {
    let run_id = resolve_run_id(ctx, given).await?;
    ctx.call("workflow:stopRun", Some(json!({ "runId": run_id })))
        .await?;
    ctx.io
        .write_err(&format!("Stopped {}.\n", short_id(&run_id)));
    Ok(ExitCode::Ok)
}

async fn list_runs(ctx: &mut Context<'_>) -> Result<ExitCode, CommandError> {
    let workflows = items(ctx.call("workflow:list", None).await?);
    let names: HashMap<String, String> = workflows.iter().map(|w| (id_of(w), name_of(w))).collect();

    let limit = ctx.args.limit.map(crate::session::number);
    let runs = match ctx.args.workflow.as_deref().filter(|w| !w.is_empty()) {
        Some(given) => {
            let mut params = Map::new();
            params.insert(
                "workflowId".into(),
                field(find_workflow(&workflows, given)?, "id")
                    .cloned()
                    .unwrap_or(Value::Null),
            );
            if let Some(limit) = limit {
                params.insert("limit".into(), limit);
            }
            ctx.call("workflowRun:list", Some(Value::Object(params)))
                .await?
        }
        None => {
            let mut params = Map::new();
            if let Some(limit) = limit {
                params.insert("limit".into(), limit);
            }
            ctx.call("workflowRun:listAll", Some(Value::Object(params)))
                .await?
        }
    };

    if ctx.args.json {
        ctx.io.write(&js::as_json(&runs));
        return Ok(ExitCode::Ok);
    }
    let runs = items(runs);
    if runs.is_empty() {
        ctx.io.write_err("No runs.\n");
        return Ok(ExitCode::Ok);
    }

    const STATUS_COLUMN: usize = 2;
    let now = crate::time::now_ms();
    let rows: Vec<Vec<String>> = runs
        .iter()
        .map(|run| {
            let workflow_id = text(field(run, "workflowId"));
            vec![
                short_id(&text(field(run, "runId"))).to_owned(),
                names
                    .get(&workflow_id)
                    .cloned()
                    .unwrap_or_else(|| short_id(&workflow_id).to_owned()),
                text(field(run, "status")),
                time_ago(field(run, "startedAt"), now),
            ]
        })
        .collect();
    let plain = ctx.plain;
    let paint = move |cell: &str, column: usize| {
        if column == STATUS_COLUMN {
            paint_status(cell, plain)
        } else {
            cell.to_owned()
        }
    };
    ctx.io.write(&table(
        &["RUN", "WORKFLOW", "STATUS", "STARTED"],
        &rows,
        Some(&paint),
    ));
    Ok(ExitCode::Ok)
}

/// `vorn workflow ...`.
pub async fn run(ctx: &mut Context<'_>) -> ExitCode {
    let positionals = ctx.args.positionals.clone();
    let verb = positionals.get(1).map(String::as_str);
    let first = positionals
        .get(2)
        .map(String::as_str)
        .filter(|g| !g.is_empty());

    if ctx.args.help {
        ctx.io.write(WORKFLOW_USAGE);
        return ExitCode::Ok;
    }
    let Some(verb) = verb.filter(|v| !v.is_empty()) else {
        ctx.io.write_err(WORKFLOW_USAGE);
        return ExitCode::Usage;
    };
    if !["list", "run", "runs", "stop"].contains(&verb) {
        return ctx.usage(
            &format!("unknown workflow command \"{verb}\""),
            WORKFLOW_USAGE,
        );
    }
    if !ctx.server().await {
        return ExitCode::Unreachable;
    }

    let (what, outcome) = match verb {
        "list" => ("could not list workflows", list(ctx).await),
        "run" => {
            let Some(given) = first else {
                return ctx.usage("workflow run needs a workflow", WORKFLOW_USAGE);
            };
            ("could not start the workflow", start_run(ctx, given).await)
        }
        "stop" => {
            let Some(given) = first else {
                return ctx.usage("workflow stop needs a run", WORKFLOW_USAGE);
            };
            ("could not stop the run", stop_run(ctx, given).await)
        }
        _ => ("could not list runs", list_runs(ctx).await),
    };
    match outcome {
        Ok(code) => code,
        Err(err) => ctx.failed(what, err),
    }
}
