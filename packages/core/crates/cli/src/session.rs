//! `vorn session start|list|logs|send|kill`: agent sessions in the running
//! server, terminal and headless alike.

use serde_json::{json, Map, Value};

use crate::client::{CommandError, Context};
use crate::exit::ExitCode;
use crate::js::{self, field, present, truthy};
use crate::output::{paint_status, short_id, table, time_ago};
use crate::paths;

const AGENTS: [&str; 5] = ["claude", "copilot", "codex", "opencode", "gemini"];

pub const SESSION_USAGE: &str = "Usage
  vorn session start --agent <agent> [--prompt <text>] [options]
  vorn session list [--recent] [--project <name>] [--json]
  vorn session logs <id> [--lines <n>] [--json]
  vorn session send <id> <text> [--raw]
  vorn session kill <id>

Agents
  claude, copilot, codex, opencode, gemini

Options for start
  --prompt <text>     First thing the agent is told
  --project <name>    A project Vorn knows, or a name for this one
  --path <dir>        Project directory (default: the git root of this one)
  --branch <branch>   Branch to check out
  --worktree          Run in a new git worktree
  --headless          No terminal pane; the agent runs to completion
  --name <label>      Display name for the session
";

/// A list the server answered with; anything else reads as empty.
fn items(value: Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items,
        _ => Vec::new(),
    }
}

fn text(value: Option<&Value>) -> String {
    js::string(value)
}

/// The projects in `config:load`'s answer.
fn projects(config: &Value) -> Vec<Value> {
    present(field(config, "projects"))
        .cloned()
        .map(items)
        .unwrap_or_default()
}

/// The project a session belongs to, registered if Vorn has not seen it.
///
/// A directory is identified by its path, never its name: two checkouts of
/// one repository are two projects.
async fn resolve_project(ctx: &Context<'_>, agent: &str) -> Result<(String, String), CommandError> {
    let config = ctx.call("config:load", None).await?;
    let known = projects(&config);

    // A project named with no directory beside it is one Vorn already knows,
    // so this works from anywhere rather than only inside that checkout.
    let wanted = ctx.args.project.as_deref().filter(|p| !p.is_empty());
    let path_given = ctx.args.path.as_deref().filter(|p| !p.is_empty());
    if let (Some(wanted), None) = (wanted, path_given) {
        if let Some(named) = known
            .iter()
            .find(|p| field(p, "name").and_then(Value::as_str) == Some(wanted))
        {
            return Ok((text(field(named, "name")), text(field(named, "path"))));
        }
    }

    let project_path = match path_given {
        Some(path) => paths::resolve(path),
        None => {
            let cwd = std::env::current_dir().unwrap_or_default();
            match paths::repo_root(&cwd).await {
                Some(root) => root,
                None => cwd.to_string_lossy().into_owned(),
            }
        }
    };
    let normalized = paths::normalize(&project_path);
    if let Some(project) = known.iter().find(|p| {
        field(p, "path")
            .and_then(Value::as_str)
            .is_some_and(|path| paths::normalize(path) == normalized)
    }) {
        return Ok((text(field(project, "name")), project_path));
    }

    let project_name = ctx
        .args
        .project
        .clone()
        .unwrap_or_else(|| paths::basename(&project_path).to_owned());
    let registered = json!({
        "name": project_name,
        "path": project_path,
        "preferredAgents": [agent],
    });
    let mut next = match config {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    let mut all = known;
    all.push(registered);
    next.insert("projects".into(), Value::Array(all));
    ctx.call("config:save", Some(Value::Object(next))).await?;
    Ok((project_name, project_path))
}

/// Which half of the server owns a session, because they are killed differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Terminal,
    Headless,
}

struct Addressed {
    id: String,
    kind: Kind,
}

/// The session an id names, accepting any prefix that names only one.
///
/// Every list prints eight characters, so those eight have to be enough to
/// act on afterwards, in either half of the server.
async fn resolve_session(ctx: &Context<'_>, given: &str) -> Result<Addressed, CommandError> {
    let (terminals, headless) = tokio::try_join!(
        ctx.call("terminal:listActive", None),
        ctx.call("headless:list", None)
    )?;
    let candidates: Vec<Addressed> = items(terminals)
        .iter()
        .map(|s| Addressed {
            id: text(field(s, "id")),
            kind: Kind::Terminal,
        })
        .chain(items(headless).iter().map(|s| Addressed {
            id: text(field(s, "id")),
            kind: Kind::Headless,
        }))
        .collect();

    let mut matches = Vec::new();
    for candidate in candidates {
        if candidate.id == given {
            return Ok(candidate);
        }
        if candidate.id.starts_with(given) {
            matches.push(candidate);
        }
    }
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err(CommandError::Refused(format!(
            "no session matches \"{given}\""
        ))),
        n => Err(CommandError::Refused(format!(
            "\"{given}\" matches {n} sessions: {}",
            matches
                .iter()
                .map(|c| short_id(&c.id))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// A headless run has no terminal behind it, so two of the verbs cannot reach one.
fn no_terminal(ctx: &mut Context<'_>, id: &str, verb: &str) -> ExitCode {
    let short = short_id(id);
    ctx.io.write_err(&format!(
        "vorn: {short} is a headless session, which has no terminal to {verb}. Its output streams to Vorn while it runs; kill it with: vorn session kill {short}\n"
    ));
    ExitCode::Failure
}

async fn start(ctx: &mut Context<'_>) -> ExitCode {
    let Some(agent) = ctx.args.agent.clone() else {
        return ctx.usage("session start needs --agent", SESSION_USAGE);
    };
    if !AGENTS.contains(&agent.as_str()) {
        return ctx.usage(
            &format!("unknown agent \"{agent}\". Try: {}", AGENTS.join(", ")),
            SESSION_USAGE,
        );
    }
    match start_session(ctx, &agent).await {
        Ok(code) => code,
        Err(err) => ctx.failed("could not start the session", err),
    }
}

async fn start_session(ctx: &mut Context<'_>, agent: &str) -> Result<ExitCode, CommandError> {
    let (project_name, project_path) = resolve_project(ctx, agent).await?;
    let mut payload = Map::new();
    payload.insert("agentType".into(), agent.into());
    payload.insert("projectName".into(), project_name.into());
    payload.insert("projectPath".into(), project_path.into());
    let args = &ctx.args;
    if let Some(prompt) = args.prompt.as_deref().filter(|p| !p.is_empty()) {
        payload.insert("initialPrompt".into(), prompt.into());
    }
    if let Some(branch) = args.branch.as_deref().filter(|b| !b.is_empty()) {
        payload.insert("branch".into(), branch.into());
    }
    if args.worktree {
        payload.insert("useWorktree".into(), true.into());
    }
    if let Some(name) = args.name.as_deref().filter(|n| !n.is_empty()) {
        payload.insert("displayName".into(), name.into());
    }
    let method = if args.headless {
        "headless:create"
    } else {
        "terminal:create"
    };
    let session = ctx.call(method, Some(Value::Object(payload))).await?;

    if ctx.args.json {
        ctx.io.write(&js::as_json(&session));
        return Ok(ExitCode::Ok);
    }

    let path = match field(&session, "worktreePath") {
        Some(worktree) if truthy(Some(worktree)) => text(Some(worktree)),
        _ => text(field(&session, "projectPath")),
    };
    let mut lines = vec![
        format!("session  {}", text(field(&session, "id"))),
        format!("agent    {}", text(field(&session, "agentType"))),
        format!("project  {}", text(field(&session, "projectName"))),
        format!("path     {path}"),
    ];
    let branch = field(&session, "branch");
    if truthy(branch) {
        lines.push(format!("branch   {}", text(branch)));
    }
    lines.push(String::new());
    ctx.io.write(&lines.join("\n"));
    Ok(ExitCode::Ok)
}

/// Where a project by that name lives, so `--project` means the same thing everywhere.
async fn path_for_project(ctx: &Context<'_>, name: &str) -> Result<Value, CommandError> {
    let config = ctx.call("config:load", None).await?;
    projects(&config)
        .into_iter()
        .find(|p| field(p, "name").and_then(Value::as_str) == Some(name))
        .map(|p| field(&p, "path").cloned().unwrap_or(Value::Null))
        .ok_or_else(|| CommandError::Refused(format!("no project named \"{name}\"")))
}

async fn list(ctx: &mut Context<'_>) -> ExitCode {
    match list_sessions(ctx).await {
        Ok(code) => code,
        Err(err) => ctx.failed("could not list sessions", err),
    }
}

/// Headless sessions are sessions; a list that hid them would hide `--headless`.
async fn list_sessions(ctx: &mut Context<'_>) -> Result<ExitCode, CommandError> {
    if ctx.args.recent {
        // Recent sessions are filtered by where they ran, so a project name
        // has to become a path.
        let project = ctx.args.project.clone().filter(|p| !p.is_empty());
        let within = match (project, ctx.args.path.as_deref().filter(|p| !p.is_empty())) {
            (Some(project), _) => Some(path_for_project(ctx, &project).await?),
            (None, Some(path)) => Some(Value::String(paths::resolve(path))),
            (None, None) => None,
        };
        let recent = ctx.call("sessions:getRecent", within).await?;
        if ctx.args.json {
            ctx.io.write(&js::as_json(&recent));
            return Ok(ExitCode::Ok);
        }
        let recent = items(recent);
        if recent.is_empty() {
            ctx.io.write_err("No recent sessions.\n");
            return Ok(ExitCode::Ok);
        }
        let now = crate::time::now_ms();
        let rows: Vec<Vec<String>> = recent
            .iter()
            .map(|s| {
                vec![
                    short_id(&text(field(s, "sessionId"))).to_owned(),
                    text(field(s, "agentType")),
                    paths::basename(&text(field(s, "projectPath"))).to_owned(),
                    time_ago(field(s, "timestamp"), now),
                    text(field(s, "activityLabel")),
                ]
            })
            .collect();
        ctx.io.write(&table(
            &["ID", "AGENT", "PROJECT", "WHEN", "ACTIVITY"],
            &rows,
            None,
        ));
        return Ok(ExitCode::Ok);
    }

    let (terminals, headless) = tokio::try_join!(
        ctx.call("terminal:listActive", None),
        ctx.call("headless:list", None)
    )?;
    let wanted = ctx.args.project.as_deref().filter(|p| !p.is_empty());
    let sessions: Vec<Value> = items(terminals)
        .into_iter()
        .chain(
            items(headless)
                .into_iter()
                .filter(|s| field(s, "status").and_then(Value::as_str) == Some("running")),
        )
        .filter(|s| wanted.is_none() || field(s, "projectName").and_then(Value::as_str) == wanted)
        .collect();

    if ctx.args.json {
        ctx.io.write(&js::as_json(&Value::Array(sessions)));
        return Ok(ExitCode::Ok);
    }
    if sessions.is_empty() {
        ctx.io.write_err("No sessions running.\n");
        return Ok(ExitCode::Ok);
    }

    const STATUS_COLUMN: usize = 4;
    let rows: Vec<Vec<String>> = sessions
        .iter()
        .map(|s| {
            vec![
                short_id(&text(field(s, "id"))).to_owned(),
                text(field(s, "agentType")),
                text(field(s, "projectName")),
                present(field(s, "branch")).map_or_else(|| "-".to_owned(), |b| text(Some(b))),
                text(field(s, "status")),
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
        &["ID", "AGENT", "PROJECT", "BRANCH", "STATUS"],
        &rows,
        Some(&paint),
    ));
    Ok(ExitCode::Ok)
}

async fn logs(ctx: &mut Context<'_>, given: Option<&str>) -> ExitCode {
    let Some(given) = given.filter(|g| !g.is_empty()) else {
        return ctx.usage("session logs needs a session id", SESSION_USAGE);
    };
    match read_logs(ctx, given).await {
        Ok(code) => code,
        Err(err) => ctx.failed("could not read the session", err),
    }
}

async fn read_logs(ctx: &mut Context<'_>, given: &str) -> Result<ExitCode, CommandError> {
    let target = resolve_session(ctx, given).await?;
    if target.kind == Kind::Headless {
        return Ok(no_terminal(ctx, &target.id, "read"));
    }
    let mut params = Map::new();
    params.insert("id".into(), target.id.clone().into());
    if let Some(lines) = ctx.args.lines {
        params.insert("lines".into(), number(lines));
    }
    let lines = ctx
        .call("terminal:readOutput", Some(Value::Object(params)))
        .await?;
    if ctx.args.json {
        ctx.io.write(&js::as_json(&lines));
        return Ok(ExitCode::Ok);
    }
    let lines = items(lines);
    // Empty is an answer, and silence reads as a broken command.
    if lines.is_empty() {
        ctx.io
            .write_err(&format!("Nothing kept for {} yet.\n", short_id(&target.id)));
        return Ok(ExitCode::Ok);
    }
    let joined: Vec<String> = lines.iter().map(|l| text(Some(l))).collect();
    ctx.io.write(&format!("{}\n", joined.join("\n")));
    Ok(ExitCode::Ok)
}

/// A count from the command line as a JSON number.
pub fn number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_992.0 {
        Value::from(value as i64)
    } else {
        serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number)
    }
}

async fn send(ctx: &mut Context<'_>, given: Option<&str>, input: Option<String>) -> ExitCode {
    let (Some(given), Some(input)) = (given.filter(|g| !g.is_empty()), input) else {
        return ctx.usage("session send needs a session id and text", SESSION_USAGE);
    };
    match send_input(ctx, given, &input).await {
        Ok(code) => code,
        Err(err) => ctx.failed("could not send to the session", err),
    }
}

async fn send_input(
    ctx: &mut Context<'_>,
    given: &str,
    input: &str,
) -> Result<ExitCode, CommandError> {
    let target = resolve_session(ctx, given).await?;
    if target.kind == Kind::Headless {
        return Ok(no_terminal(ctx, &target.id, "send to"));
    }
    // Enter is what submits a prompt; --raw is for sending control sequences instead.
    let data = if ctx.args.raw {
        input.to_owned()
    } else {
        format!("{}\r", input.trim_end_matches(['\r', '\n']))
    };
    ctx.rpc
        .notify(
            "terminal:write",
            Some(json!({ "id": target.id, "data": data })),
        )
        .await?;
    ctx.io
        .write_err(&format!("Sent to {}.\n", short_id(&target.id)));
    Ok(ExitCode::Ok)
}

async fn kill(ctx: &mut Context<'_>, given: Option<&str>) -> ExitCode {
    let Some(given) = given.filter(|g| !g.is_empty()) else {
        return ctx.usage("session kill needs a session id", SESSION_USAGE);
    };
    match kill_session(ctx, given).await {
        Ok(code) => code,
        Err(err) => ctx.failed("could not kill the session", err),
    }
}

async fn kill_session(ctx: &mut Context<'_>, given: &str) -> Result<ExitCode, CommandError> {
    let target = resolve_session(ctx, given).await?;
    // Two registries, two kill methods: a headless run is not a pty.
    let method = match target.kind {
        Kind::Headless => "headless:kill",
        Kind::Terminal => "terminal:kill",
    };
    ctx.call(method, Some(Value::String(target.id.clone())))
        .await?;
    ctx.io
        .write_err(&format!("Killed {}.\n", short_id(&target.id)));
    Ok(ExitCode::Ok)
}

/// `vorn session ...`.
pub async fn run(ctx: &mut Context<'_>) -> ExitCode {
    let positionals = ctx.args.positionals.clone();
    let verb = positionals.get(1).map(String::as_str);
    let rest = positionals.get(2..).unwrap_or_default();

    if ctx.args.help {
        ctx.io.write(SESSION_USAGE);
        return ExitCode::Ok;
    }
    let Some(verb) = verb.filter(|v| !v.is_empty()) else {
        ctx.io.write_err(SESSION_USAGE);
        return ExitCode::Usage;
    };
    if !["start", "list", "logs", "send", "kill"].contains(&verb) {
        return ctx.usage(
            &format!("unknown session command \"{verb}\""),
            SESSION_USAGE,
        );
    }
    if !ctx.server().await {
        return ExitCode::Unreachable;
    }

    let first = rest.first().map(String::as_str);
    match verb {
        "start" => start(ctx).await,
        "list" => list(ctx).await,
        "logs" => logs(ctx, first).await,
        "send" => {
            let joined = rest.get(1..).unwrap_or_default().join(" ");
            send(ctx, first, (!joined.is_empty()).then_some(joined)).await
        }
        _ => kill(ctx, first).await,
    }
}
