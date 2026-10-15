//! `vorn session start|list|logs|send|kill`: agent sessions in the running
//! server, terminal and headless alike.

use serde_json::{json, Map, Value};

use crate::client::{CommandError, Context};
use crate::exit::ExitCode;
use crate::js::{self, field, present, truthy};
use crate::output::{paint_status, short_id, table, time_ago};
use crate::paths;
use crate::render;
use crate::status;
use crate::target::{self, Candidate, Miss};

pub const SESSION_USAGE: &str = "Usage
  vorn session start --agent <agent> [--prompt <text>] [options]
  vorn session list [--recent] [--project <name>] [--json]
  vorn session read <target> [--lines <n>] [--scrollback] [--unwrap] [--trim]
                             [--format text|vt|html] [--json]
  vorn session send <target> <text> [--raw]
  vorn session wait <target> [--timeout <dur>] [--status idle|done|blocked] [--json]
  vorn session kill <target>

Targets
  Every command that takes a <target> reads it the same way. The first rule that
  matches decides:
    1. a whole session id
    2. a label (the session's name), exactly, then ignoring case
    3. a position: 3 is the third session in `vorn session list`
    4. the start or the end of an id
  A rule that matches more than one session is ambiguous: the candidates are
  listed on stderr (and on stdout as data with --json) and the exit code is 3.

Exit codes
  0 ok   1 failed   2 usage   3 ambiguous target   4 no server
  wait: the session's own exit code; 124 when --timeout passes first

Reading
  `read` prints the output the server kept. --scrollback, --unwrap and any
  --format but text read the terminal instead: its screen and scrollback, with
  soft-wrapped rows joined. --trim drops trailing spaces and trailing blank
  lines. vt keeps colours as SGR escapes; html draws them as spans in a <pre>.
  `logs` is the same command.

Waiting
  `wait` blocks until the session's program exits, then exits with its code.
  --timeout takes 30s, 5m, 1500ms or a bare number of seconds. --status
  waits instead for the program's status (as the app shows it) and exits 0.

Agents
  Each agent that is on in Vorn, by the name its adapter gives it

Options for start
  --prompt <text>     First thing the agent is told
  --project <name>    A project Vorn knows, or a name for this one
  --path <dir>        Project directory (default: the git root of this one)
  --branch <branch>   Branch to check out
  --worktree          Run in a new git worktree
  --headless          No terminal pane; the agent runs to completion
  --wait              With --headless, stay until it ends and show it to the terminal
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

type Addressed = Candidate;

/// Every session a target may name, in the order `vorn session list` shows them.
pub async fn candidates(ctx: &Context<'_>) -> Result<Vec<Candidate>, CommandError> {
    let (terminals, headless) = ctx
        .call_both("terminal:listActive", "headless:list")
        .await?;
    Ok(items(terminals)
        .iter()
        .map(|s| Candidate::from_record(s, false))
        .chain(
            items(headless)
                .iter()
                .filter(|s| field(s, "status").and_then(Value::as_str) == Some("running"))
                .map(|s| Candidate::from_record(s, true)),
        )
        .collect())
}

/// The session `given` names, by the rules in [`crate::target`].
pub async fn resolve_session(ctx: &Context<'_>, given: &str) -> Result<Addressed, CommandError> {
    let list = candidates(ctx).await?;
    match target::resolve(given, &list) {
        Ok(found) => Ok(found.clone()),
        Err(Miss::None) => Err(CommandError::Refused(format!(
            "no session matches \"{given}\""
        ))),
        Err(Miss::Ambiguous(found)) => Err(CommandError::Ambiguous {
            given: given.to_owned(),
            found,
        }),
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
    // The agents vornd runs come from their adapters; it names those that are on.
    let known: Vec<String> = match ctx.call("agent:list", None).await {
        Ok(listed) => items(listed)
            .iter()
            .filter_map(|a| field(a, "id").and_then(Value::as_str).map(str::to_owned))
            .collect(),
        Err(err) => return ctx.failed("could not start the session", CommandError::from(err)),
    };
    if !known.contains(&agent) {
        let names = if known.is_empty() {
            "none is on; switch one on in Settings".to_owned()
        } else {
            known.join(", ")
        };
        return ctx.usage(
            &format!("unknown agent \"{agent}\". Try: {names}"),
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
    let wait = args.headless && args.wait;

    if ctx.args.json {
        ctx.io.write(&js::as_json(&session));
        return if wait {
            wait_headless(ctx, &session).await
        } else {
            Ok(ExitCode::Ok)
        };
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
    if wait {
        return wait_headless(ctx, &session).await;
    }
    Ok(ExitCode::Ok)
}

/// Waits for headless `session` to end, reporting it to a terminal (OSC
/// 7501) as it goes; fails when its agent does.
async fn wait_headless(ctx: &mut Context<'_>, session: &Value) -> Result<ExitCode, CommandError> {
    let id = text(field(session, "id"));
    let name = [field(session, "displayName"), field(session, "agentType")]
        .into_iter()
        .map(text)
        .find(|n| !n.is_empty())
        .unwrap_or_else(|| "headless".to_owned());
    let report = ctx.io.is_tty() && !ctx.args.json;
    let mut reporter = status::Reporter::default();
    loop {
        let now = items(ctx.call("headless:list", None).await?)
            .into_iter()
            .find(|s| field(s, "id").and_then(Value::as_str) == Some(id.as_str()))
            .ok_or_else(|| CommandError::Refused(format!("session {} is gone", short_id(&id))))?;
        let record = status::headless_record(&now, &name);
        let state = record.state;
        if report {
            let out = reporter.changes(vec![record]);
            if !out.is_empty() {
                ctx.io.write(&out);
            }
        }
        match state {
            "done" => return Ok(ExitCode::Ok),
            "error" => return Ok(ExitCode::Failure),
            _ => tokio::time::sleep(status::POLL).await,
        }
    }
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

    let (terminals, headless) = ctx
        .call_both("terminal:listActive", "headless:list")
        .await?;
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
        return ctx.usage("session read needs a session", SESSION_USAGE);
    };
    match read_logs(ctx, given).await {
        Ok(code) => code,
        Err(err) => ctx.error("could not read the session", err),
    }
}

async fn read_logs(ctx: &mut Context<'_>, given: &str) -> Result<ExitCode, CommandError> {
    let target = resolve_session(ctx, given).await?;
    if target.headless {
        return Ok(no_terminal(ctx, &target.id, "read"));
    }
    let format = ctx.args.format.clone().unwrap_or_else(|| "text".to_owned());
    if format != "text" || ctx.args.unwrap || ctx.args.scrollback {
        return read_screen(ctx, &target, &format).await;
    }
    let mut params = Map::new();
    params.insert("id".into(), target.id.clone().into());
    if let Some(lines) = ctx.args.lines {
        params.insert("lines".into(), number(lines));
    }
    let lines = ctx
        .call("terminal:readOutput", Some(Value::Object(params)))
        .await?;
    let mut lines: Vec<Value> = items(lines);
    if ctx.args.trim {
        for line in &mut lines {
            *line = Value::String(text(Some(line)).trim_end_matches(' ').to_owned());
        }
        while lines
            .last()
            .is_some_and(|l| text(Some(l)).trim().is_empty())
        {
            lines.pop();
        }
    }
    if ctx.args.json {
        ctx.io.write(&js::as_json(&Value::Array(lines)));
        return Ok(ExitCode::Ok);
    }
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

/// The terminal's screen and scrollback, drawn as `format`.
async fn read_screen(
    ctx: &mut Context<'_>,
    target: &Addressed,
    format: &str,
) -> Result<ExitCode, CommandError> {
    let answer = ctx
        .call("terminal:readScrollback", Some(json!({ "id": target.id })))
        .await?;
    let mut lines = render::parse(&text(field(&answer, "data")));
    if ctx.args.trim {
        render::trim(&mut lines);
    }
    if let Some(n) = ctx.args.lines {
        render::tail(&mut lines, n as usize);
    }
    let out = match format {
        "vt" => render::vt(&lines),
        "html" => render::html(&lines),
        _ => render::text(&lines),
    };
    if ctx.args.json {
        let data = json!({ "session": target.id, "format": format, "data": out });
        ctx.io.write(&js::as_json(&data));
    } else {
        ctx.io.write(&out);
    }
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

/// The program's own state in a terminal record: the root record's, if it reports one.
fn program_state(record: &Value) -> Option<String> {
    let records = field(field(record, "programStatus")?, "records")?.as_array()?;
    let root = records
        .iter()
        .find(|r| field(r, "id").and_then(Value::as_str) == Some(""))?;
    field(root, "state")?.as_str().map(str::to_owned)
}

/// Whether a session whose status is `status` and whose program says `program` has reached `want`.
fn reached(want: &str, status: Option<&str>, program: Option<&str>) -> bool {
    match want {
        "idle" => status == Some("idle") || program == Some("idle"),
        "blocked" => status == Some("waiting") || program == Some("blocked"),
        _ => program == Some("done"),
    }
}

/// The code a `wait` ends with for a program that exited with `code`.
fn exit_of(code: Option<i64>) -> ExitCode {
    match code {
        Some(c @ 0..=255) => ExitCode::Passed(c as i32),
        _ => ExitCode::Failure,
    }
}

async fn wait(ctx: &mut Context<'_>, given: Option<&str>) -> ExitCode {
    let Some(given) = given.filter(|g| !g.is_empty()) else {
        return ctx.usage("session wait needs a session", SESSION_USAGE);
    };
    match wait_for(ctx, given).await {
        Ok(code) => code,
        Err(err) => ctx.error("could not wait for the session", err),
    }
}

/// Blocks on the server's events, not on polls: the queue is opened before the
/// session is looked at, so an exit in between is still heard.
async fn wait_for(ctx: &mut Context<'_>, given: &str) -> Result<ExitCode, CommandError> {
    let mut events = ctx
        .rpc
        .events(json!({ "events": ["session.exited", "session.status", "session.program"] }))
        .await?;
    let target = resolve_session(ctx, given).await?;
    let want = ctx.args.status.clone();

    // Where it is now.
    let (terminals, headless) = ctx
        .call_both("terminal:listActive", "headless:list")
        .await?;
    let now = items(if target.headless { headless } else { terminals })
        .into_iter()
        .find(|s| field(s, "id").and_then(Value::as_str) == Some(target.id.as_str()));
    let Some(now) = now else {
        return Err(CommandError::Refused(format!(
            "session {} is gone",
            short_id(&target.id)
        )));
    };
    let state = |status: Option<&str>, program: Option<&str>| {
        want.as_deref().is_some_and(|w| reached(w, status, program))
    };
    if state(
        field(&now, "status").and_then(Value::as_str),
        program_state(&now).as_deref(),
    ) {
        return Ok(reported(
            ctx,
            &target,
            json!({ "status": want }),
            ExitCode::Ok,
        ));
    }
    let ended = field(&now, "ended").and_then(Value::as_bool) == Some(true);
    if ended || field(&now, "status").and_then(Value::as_str) == Some("exited") {
        // A headless run keeps its code; a shell its last one; another terminal's is not kept.
        let code = field(&now, "exitCode")
            .or_else(|| field(&now, "shellExitCode"))
            .and_then(Value::as_i64);
        return Ok(finished(
            ctx,
            &target,
            want.as_deref(),
            json!({ "exitCode": code }),
            code,
        ));
    }

    let deadline = ctx
        .args
        .wait_timeout
        .map(|d| tokio::time::Instant::now() + d);
    let mut status: Option<String> = None;
    loop {
        let next = events.next();
        let event = match deadline {
            Some(at) => match tokio::time::timeout_at(at, next).await {
                Ok(event) => event?,
                Err(_) => {
                    let data = json!({ "session": target.id, "timedOut": true });
                    if ctx.args.json {
                        ctx.io.write(&js::as_json(&data));
                    }
                    ctx.io.write_err(&format!(
                        "vorn: {} did not finish in time\n",
                        short_id(&target.id)
                    ));
                    return Ok(ExitCode::Passed(124));
                }
            },
            None => next.await?,
        };
        if field(&event, "session").and_then(Value::as_str) != Some(target.id.as_str()) {
            continue;
        }
        let data = field(&event, "data").cloned().unwrap_or(Value::Null);
        match field(&event, "event").and_then(Value::as_str) {
            Some("session.exited") => {
                let code = field(&data, "exitCode").and_then(Value::as_i64);
                return Ok(finished(ctx, &target, want.as_deref(), data, code));
            }
            Some("session.status") => {
                status = field(&data, "status")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            Some("session.program") => {}
            _ => continue,
        }
        let program = field(&data, "state").and_then(Value::as_str);
        let program = if field(&event, "event").and_then(Value::as_str) == Some("session.program") {
            program
        } else {
            None
        };
        if state(status.as_deref(), program) {
            return Ok(reported(
                ctx,
                &target,
                json!({ "status": want }),
                ExitCode::Ok,
            ));
        }
    }
}

/// The session's program ended: the exit record, and the code to exit with.
fn finished(
    ctx: &mut Context<'_>,
    target: &Addressed,
    want: Option<&str>,
    record: Value,
    code: Option<i64>,
) -> ExitCode {
    match want {
        // A program that ended is done; for any other status it never got there.
        Some("done") => reported(ctx, target, record, ExitCode::Ok),
        Some(w) => {
            ctx.io.write_err(&format!(
                "vorn: {} ended before it was {w}\n",
                short_id(&target.id)
            ));
            ExitCode::Failure
        }
        None => reported(ctx, target, record, exit_of(code)),
    }
}

/// With `--json`, the record on stdout; the code is the command's.
fn reported(ctx: &mut Context<'_>, target: &Addressed, record: Value, code: ExitCode) -> ExitCode {
    if ctx.args.json {
        let mut out = Map::new();
        out.insert("session".into(), target.id.clone().into());
        if let Value::Object(fields) = record {
            out.extend(fields);
        }
        ctx.io.write(&js::as_json(&Value::Object(out)));
    }
    code
}

async fn send(ctx: &mut Context<'_>, given: Option<&str>, input: Option<String>) -> ExitCode {
    let (Some(given), Some(input)) = (given.filter(|g| !g.is_empty()), input) else {
        return ctx.usage("session send needs a session and text", SESSION_USAGE);
    };
    match send_input(ctx, given, &input).await {
        Ok(code) => code,
        Err(err) => ctx.error("could not send to the session", err),
    }
}

async fn send_input(
    ctx: &mut Context<'_>,
    given: &str,
    input: &str,
) -> Result<ExitCode, CommandError> {
    let target = resolve_session(ctx, given).await?;
    if target.headless {
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
        return ctx.usage("session kill needs a session", SESSION_USAGE);
    };
    match kill_session(ctx, given).await {
        Ok(code) => code,
        Err(err) => ctx.error("could not kill the session", err),
    }
}

async fn kill_session(ctx: &mut Context<'_>, given: &str) -> Result<ExitCode, CommandError> {
    let target = resolve_session(ctx, given).await?;
    // Two registries, two kill methods: a headless run is not a pty.
    let method = if target.headless {
        "headless:kill"
    } else {
        "terminal:kill"
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
    if !["start", "list", "logs", "read", "wait", "send", "kill"].contains(&verb) {
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
        "logs" | "read" => logs(ctx, first).await,
        "wait" => wait(ctx, first).await,
        "send" => {
            let joined = rest.get(1..).unwrap_or_default().join(" ");
            send(ctx, first, (!joined.is_empty()).then_some(joined)).await
        }
        _ => kill(ctx, first).await,
    }
}
