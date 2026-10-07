//! The terminal calls vornd answers itself with the Native server switch on:
//! `terminal:create`, `kill`, `rename`, `setGroup` and `reorder`, and
//! `shell:create`.
//!
//! Each answers as the server's handler does, from the copy of the session
//! registry vornd keeps ([`crate::registry`]), which then holds the record
//! vornd made or changed and tells the server so (`native: true`). The
//! server stays the one that tells clients (`session:created`, `updated`,
//! `reordered`), saves the records and runs what hangs off a new session
//! (extensions, a copilot's hooks, the agent's conversation captured); it
//! does that from the registry's notes.
//!
//! A create prepares the session's workspace as the server's
//! `prepareSession` does: the worktree it names, or one it makes, or the
//! branch it checks out, under the repository's turn ([`super::Turns`]),
//! holding each directory involved until the record is in, so the server's
//! worktree calls see it as in use meanwhile. An agent that can be told
//! which conversation to start is given an id; one that names a
//! conversation already running is answered with the session running it,
//! and creates naming one conversation while one prepares share its answer
//! ([`crate::claims`]). Nothing new is started while the server is winding
//! down (`vornd:draining`).
//!
//! While vornd owns the records between runs ([`crate::applink::AppLink::restores`])
//! it also answers `sessions:restored`, `sessions:resume` and
//! `sessions:clear`, and a `terminal:kill` of a session of an earlier run,
//! as the server's handlers do: a resume takes the offered session once,
//! hands back the terminal already writing its conversation if one is, or
//! starts a shell where the session was, or the agent on the conversation
//! it had, under the same id ([`resume`]). The launch line is typed once
//! the shell has drawn its prompt ([`type_at`]).
//!
//! The server keeps a call vornd cannot answer as it would: one for a
//! remote host, one whose params are not the shape its handler reads, one
//! naming a terminal the registry does not hold, and every call while the
//! registry does not hold the server's records or vornd's session holder
//! is not connected.
//!
//! In shadow mode nothing here changes anything: a create or a resume is
//! compared with the server's as the spawn each would ask for ([`plan`]),
//! and the other calls as what each would answer ([`foresee`]), read from
//! the copy.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};
use tracing::warn;
use vorn_agents::history::{recent_sessions_for, Homes, ProjectScope, RecentSession};
use vorn_agents::launch::shell as launch_shell;
use vorn_agents::launch::{
    display_name_from_prompt, launch_line, LaunchRequest, Machine, Platform, Quoting,
};
use vorn_agents::{paths, Agent};
use vorn_git::repo::{extract_worktree_name, node_basename, Git};
use vorn_sessiond_wire::{Io, Sig, SpawnSpec};

use super::{agent, headless, shell, Answer, Native};
use crate::claims::{Claims, OnePerKey};
use crate::registry::{AgentStatus, HeadlessStatus, Registry, Restored, TerminalSession};

/// How many of an agent's past sessions a resume looks through for the
/// conversation to continue (`getRecentSessionsFor`).
const RECENT_LIMIT: usize = 20;

/// The size a terminal starts at, before any client has fitted it.
pub const INITIAL_COLS: u16 = 80;
pub const INITIAL_ROWS: u16 = 24;

/// The terminal type programs are told they run in, off Windows.
const PTY_TERM: &str = "xterm-256color";

/// How long a create naming a conversation that a start of the server's
/// holds waits for it before it says the conversation is busy, and how
/// often it looks meanwhile.
const HOLDER_WAIT: Duration = Duration::from_secs(60);
const HOLDER_POLL: Duration = Duration::from_millis(50);

/// The longest an agent's display name taken from its prompt runs.
const PROMPT_NAME_LEN: usize = 60;

/// The params of `terminal:create` the server's handler reads; any other
/// is a call for the server.
const CREATE_KEYS: [&str; 18] = [
    "agentType",
    "model",
    "projectName",
    "projectPath",
    "resumeSessionId",
    "sessionId",
    "displayName",
    "branch",
    "useWorktree",
    "existingWorktreePath",
    "worktreeName",
    "remoteHostId",
    "initialPrompt",
    "promptDelayMs",
    "headless",
    "workflowId",
    "workflowName",
    "args",
];

/// What starts and signals the sessions vornd creates: the engine, which
/// runs them in its session holder.
pub trait Host: Send + Sync + fmt::Debug {
    /// Whether a session can be started now: the holder is connected.
    fn ready(&self) -> bool;

    /// Starts `spec` under `name`, then calls `then` with how it went, and
    /// gives it `input` once it is up. Returns at once.
    fn start(&self, spec: SpawnSpec, name: String, input: Input, then: Then);

    /// Sends `sig` to session `id`'s program.
    fn signal(&self, id: &str, sig: Sig);

    /// Sends `sig` to session `id`'s program `after` a while, if it still runs.
    fn signal_after(&self, id: &str, sig: Sig, after: Duration);

    /// Answers attaches for `id` although nothing runs under it: a session
    /// carried from the last run ([`crate::streams::Streams::expect`]).
    fn expect(&self, id: &str) {
        let _ = id;
    }

    /// `id` was let go of without starting ([`crate::streams::Streams::forget`]).
    fn forget(&self, id: &str) {
        let _ = id;
    }
}

/// The shortest wait before an agent's launch line is typed: the shell
/// has to be reading its terminal.
pub const TYPE_AFTER: Duration = Duration::from_millis(300);

/// How long after the shell's first output the line is typed: the prompt
/// is drawn, and the line editor that echoes the line once is up.
pub const TYPE_SETTLE: Duration = Duration::from_millis(100);

/// The longest wait for a shell that prints nothing before the line is
/// typed anyway.
pub const TYPE_AT_MOST: Duration = Duration::from_millis(1500);

/// When to type the launch line of a session started at `asked`, given
/// when its shell first printed, if it has: once the shell has settled
/// after its prompt, never sooner than [`TYPE_AFTER`], and at
/// [`TYPE_AT_MOST`] if it never prints.
pub fn type_at(
    asked: tokio::time::Instant,
    printed: Option<tokio::time::Instant>,
) -> tokio::time::Instant {
    match printed {
        Some(at) => (at + TYPE_SETTLE).max(asked + TYPE_AFTER),
        None => asked + TYPE_AT_MOST,
    }
}

/// What a session's program is given once it is up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    None,
    /// Typed into a terminal, no sooner than [`TYPE_AFTER`] after the
    /// start was asked for: an agent's launch line, for its shell.
    Typed(Vec<u8>),
    /// Written to a piped program's stdin, which then closes: a headless
    /// agent's prompt, or nothing when it has none.
    Prompt(Option<Vec<u8>>),
}

/// What a start's outcome is handed to.
pub type Then = Box<dyn FnOnce(Result<Started, String>) + Send>;

/// A session's program, started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Started {
    pub pid: u32,
    /// The epoch its records start in.
    pub epoch: u32,
}

/// What the sessions vornd starts keep between calls.
#[derive(Debug, Default)]
pub struct Sessions {
    /// Sessions whose program is being started, and the signal a close or
    /// a stop meanwhile asked for, sent once it is up. One lock for both,
    /// so a close cannot fall between the start's look and its answer.
    starting: Mutex<HashMap<String, Option<Sig>>>,
    /// Creates naming a conversation, while they prepare.
    creating: OnePerKey<Answer>,
}

/// The refusal for a new session while the server winds down
/// (`refuseWhileClosing`), if it is.
fn refusal(native: &Native) -> Option<&'static str> {
    native.link.get()?.closing().refusal()
}

/// How the server is winding down, if it is.
pub(super) fn closing(native: &Native) -> Option<crate::applink::Closing> {
    native.link.get().map(|l| l.closing())
}

/// The conversations being started, which vornd's creates and the server's
/// starts claim alike.
fn claims(native: &Native) -> Option<&Claims> {
    native.link.get().map(|l| l.claims())
}

/// Whether vornd owns the records between runs, and so answers the calls
/// about the sessions of earlier runs.
fn restores(native: &Native) -> bool {
    native.link.get().is_some_and(|l| l.restores())
}

/// Answers `method` with `params`.
pub fn call(native: &Native, method: &str, params: &Value) -> Answer {
    match method {
        "terminal:create" => match CreateRequest::read(params) {
            Some(req) => create(native, &req),
            None => Answer::Forward,
        },
        "sessions:restored" => restored(native),
        "sessions:resume" => match params.get("id").and_then(Value::as_str) {
            Some(id) if restores(native) => resume(native, id),
            _ => Answer::Forward,
        },
        "sessions:clear" => clear(native),
        "shell:create" => match params {
            Value::Null => shell_create(native, None),
            Value::String(cwd) if cwd.is_empty() => shell_create(native, None),
            Value::String(cwd) if Path::new(cwd).is_absolute() => shell_create(native, Some(cwd)),
            _ => Answer::Forward,
        },
        _ => match asked(method, params) {
            Some(Asked::Kill(id)) => kill(native, &id),
            Some(Asked::Fields(id, fields)) => set_fields(native, &id, fields),
            Some(Asked::Order(ids)) => reorder(native, ids),
            None => Answer::Forward,
        },
    }
}

/// What a call that changes a terminal held asks for, as the server's
/// handler reads it.
#[derive(Debug)]
enum Asked {
    Kill(String),
    /// Fields of [`crate::registry::PATCHABLE`] to set on a terminal.
    Fields(String, Map<String, Value>),
    Order(Vec<String>),
}

/// Reads `params` for `method`; `None` for a shape the server's handler
/// would not expect, which is then its to answer.
fn asked(method: &str, params: &Value) -> Option<Asked> {
    match method {
        "terminal:kill" => params.as_str().map(|id| Asked::Kill(id.to_owned())),
        "terminal:rename" => {
            let id = params.get("id").and_then(Value::as_str)?;
            let name = params.get("displayName").and_then(Value::as_str)?;
            let mut fields = Map::new();
            fields.insert("displayName".into(), json!(name));
            // Asked for by a person: an extension may not overrule it.
            fields.insert("renamedByPerson".into(), json!(true));
            Some(Asked::Fields(id.to_owned(), fields))
        }
        "terminal:setGroup" => {
            let id = params.get("id").and_then(Value::as_str)?;
            let group = match params.get("groupId") {
                Some(Value::Null) => Value::Null,
                // An empty id takes it out of its group, as the server reads it.
                Some(Value::String(g)) if g.is_empty() => Value::Null,
                Some(Value::String(g)) => json!(g),
                _ => return None,
            };
            let mut fields = Map::new();
            fields.insert("groupId".into(), group);
            Some(Asked::Fields(id.to_owned(), fields))
        }
        "terminal:reorder" => strings(params).map(Asked::Order),
        _ => None,
    }
}

/// Whether [`foresee`] can say what `method` would answer.
pub fn foresees(method: &str) -> bool {
    matches!(
        method,
        "terminal:kill"
            | "terminal:rename"
            | "terminal:setGroup"
            | "terminal:reorder"
            | "sessions:clear"
    ) || headless::foresees(method)
        || super::worktree_move::foresees(method)
}

/// What vornd would answer `method`, read from the copy of the registry
/// without changing it, for the comparison with the server's answer in
/// shadow mode. `None` when the answer is the server's own: a terminal the
/// copy does not hold, params of a shape its handler does not read, or no
/// copy of its records yet.
pub fn foresee(native: &Native, method: &str, params: &Value) -> Option<Answer> {
    if headless::foresees(method) {
        return headless::foresee(native, params);
    }
    if super::worktree_move::foresees(method) {
        return super::worktree_move::foresee(native, method, params);
    }
    if method == "sessions:clear" {
        return native.registry.get()?.read(|_| Answer::Void);
    }
    let asked = asked(method, params)?;
    native.registry.get()?.read(|r| match &asked {
        // A session of an earlier run closes without an exit, as the
        // server closes one.
        Asked::Kill(id) => r
            .terminal(id)
            .map(|_| ())
            .or_else(|| {
                r.restored()
                    .iter()
                    .find(|o| o["session"]["id"] == *id)
                    .map(|_| ())
            })
            .map(|()| Answer::Void),
        Asked::Fields(id, _) => Some(refused(r.check_terminal(id))),
        Asked::Order(ids) => Some(refused(r.check_order(ids))),
    })?
}

/// `sessions:restored`: the sessions of earlier runs still offered.
fn restored(native: &Native) -> Answer {
    if !restores(native) {
        return Answer::Forward;
    }
    match native.registry.get().and_then(|r| r.restored()) {
        Some(list) => Answer::Result(Value::Array(list)),
        None => Answer::Forward,
    }
}

/// `sessions:clear`: every offered session declined at once.
fn clear(native: &Native) -> Answer {
    if !restores(native) {
        return Answer::Forward;
    }
    let Some(registry) = native.registry.get() else {
        return Answer::Forward;
    };
    let declined = registry.change(|r| {
        let (all, note) = r.consume_all_restored();
        (all, vec![note])
    });
    let Some(declined) = declined else {
        return Answer::Forward;
    };
    if let Some(host) = native.host.get() {
        for r in &declined {
            host.forget(&r.session.id);
        }
    }
    Answer::Void
}

/// What `sessions:resume` found under an id.
#[derive(Debug)]
enum Taken {
    /// A session of an earlier run, taken once.
    Offered(Restored),
    /// A terminal whose program ended during this run.
    Ended(TerminalSession),
}

impl Taken {
    fn session(&self) -> &TerminalSession {
        match self {
            Taken::Offered(r) => &r.session,
            Taken::Ended(s) => s,
        }
    }
}

/// `sessions:resume {id}`: the session the id names, offered from an
/// earlier run or ended during this one, started again under its id. One
/// whose conversation is already being written is not started: the
/// terminal writing it is handed back (`boundTo`).
fn resume(native: &Native, id: &str) -> Answer {
    let Some(registry) = native.registry.get() else {
        return Answer::Forward;
    };
    if !fed_and_held(native) {
        return Answer::Error("Terminals cannot start: the session holder is not connected".into());
    }
    // Taken before anything is started: the second of two clients looking
    // at one cold pane is told it is gone.
    let taken = registry.change(|r| match r.consume_restored(id) {
        Some((offered, note)) => (Some(Taken::Offered(offered)), vec![note]),
        None => (r.ended_terminal(id).cloned().map(Taken::Ended), Vec::new()),
    });
    let Some(taken) = taken else {
        return Answer::Error("The Vorn server is not connected to vornd".to_owned());
    };
    let Some(taken) = taken else {
        return Answer::Result(json!({ "ok": false, "reason": "gone" }));
    };
    let previous = taken.session();
    let holder = given(previous.agent_session_id.as_deref()).and_then(|t| running_on(native, t));
    if let Some(holder) = holder {
        if let Taken::Ended(_) = &taken {
            registry.change(|r| ((), r.release_for_resume(id)));
        }
        let bound = holder["id"].clone();
        return Answer::Result(json!({ "ok": true, "session": holder, "boundTo": bound }));
    }
    let answer = if previous.agent_type == "shell" {
        resume_shell(native, id, previous)
    } else {
        resume_agent(native, id, previous)
    };
    let started = matches!(&answer, Answer::Result(v) if v["ok"] == true);
    if !started {
        // Offered again: a resume that did not start is not the end of it.
        if let Taken::Offered(offered) = taken {
            registry.change(|r| ((), vec![r.restore_held(offered)]));
        }
    }
    answer
}

/// The directory a session starts again in: the most specific of where
/// its shell was, its worktree and its project that is still a directory
/// (`resumeCwdFor`).
pub fn resume_cwd_for(previous: &TerminalSession) -> Option<String> {
    [
        previous.shell_cwd.as_deref(),
        previous.worktree_path.as_deref(),
        Some(previous.project_path.as_str()),
    ]
    .into_iter()
    .flatten()
    .find(|p| Path::new(p).is_dir())
    .map(str::to_owned)
}

fn workspace_gone(previous: &TerminalSession) -> Answer {
    Answer::Result(json!({
        "ok": false,
        "reason": "workspace-gone",
        "message": format!("{} is gone", previous.project_path),
    }))
}

/// The record a session starts again with: a shell's as `createShellPty`
/// makes it, under the id and with the fields carried over from the one
/// before (`sessions:resume`).
fn resumed_shell(previous: &TerminalSession, cwd: &str, count: usize) -> TerminalSession {
    let project_name = match node_basename(cwd) {
        "" => "shell".to_owned(),
        name => name.to_owned(),
    };
    TerminalSession {
        display_name: previous
            .display_name
            .clone()
            .or_else(|| Some(format!("Shell {}", count + 1))),
        project_name: previous.project_name.clone(),
        project_path: previous.project_path.clone(),
        worktree_path: previous.worktree_path.clone(),
        worktree_name: previous.worktree_name.clone(),
        branch: previous.branch.clone(),
        is_worktree: previous.is_worktree,
        group_id: previous.group_id.clone(),
        shell_cwd: Some(cwd.to_owned()),
        ..skeleton(&previous.id, "shell", &project_name, cwd)
    }
}

/// How many shells a resumed one is numbered after: those there are, less
/// the one it replaces, which the server lets go of first.
fn shells_before(native: &Native, id: &str) -> Option<usize> {
    native.registry.get()?.read(|r| {
        let replaced = r
            .ended_terminal(id)
            .is_some_and(|t| t.agent_type == "shell");
        r.shells() - usize::from(replaced)
    })
}

/// A shell started again where it was.
fn resume_shell(native: &Native, id: &str, previous: &TerminalSession) -> Answer {
    let Some(cwd) = resume_cwd_for(previous) else {
        return workspace_gone(previous);
    };
    let settings = agent::settings(native).unwrap_or_default();
    let setup = native.shells.setup(
        &native.env,
        settings.shell.as_deref(),
        settings.minimal_shell_prompt,
    );
    let (shell, setup) = match setup {
        Ok(ready) => ready,
        Err(e) => return failed(e.to_string()),
    };
    let mut argv = vec![shell];
    match setup.args {
        Some(args) => argv.extend(args),
        None => argv.extend(
            launch_shell::default_shell_args(Platform::HOST)
                .iter()
                .map(|a| (*a).to_owned()),
        ),
    }
    let mut env = native.env.get();
    for (k, v) in setup.env {
        set(&mut env, &k, v);
    }
    let Some(count) = shells_before(native, id) else {
        return Answer::Forward;
    };
    let record = resumed_shell(previous, &cwd, count);
    resumed(register(
        native,
        record,
        &cwd,
        argv,
        env,
        Input::None,
        Made::Resumed,
    ))
}

/// A resume's answer from the start's: the record, or why it failed.
fn resumed(answer: Answer) -> Answer {
    match answer {
        Answer::Result(session) => Answer::Result(json!({ "ok": true, "session": session })),
        Answer::Error(message) => failed(message),
        other => other,
    }
}

fn failed(message: String) -> Answer {
    Answer::Result(json!({ "ok": false, "reason": "failed", "message": message }))
}

/// The create a session of an earlier run starts again as
/// (`buildRestorePayload`), on `transcript` when it has one. `None` for a
/// remote host's session, or an agent this build does not know.
fn restore_request(
    previous: &TerminalSession,
    transcript: Option<String>,
) -> Option<CreateRequest> {
    if given(previous.remote_host_id.as_deref()).is_some() {
        return None;
    }
    let in_worktree = previous.is_worktree == Some(true);
    Some(CreateRequest {
        agent: Agent::from_id(&previous.agent_type)?,
        model: None,
        project_name: previous.project_name.clone(),
        project_path: previous.project_path.clone(),
        resume_session_id: transcript,
        session_id: None,
        display_name: previous.display_name.clone(),
        branch: previous.branch.clone().filter(|_| in_worktree),
        existing_worktree_path: previous.worktree_path.clone().filter(|_| in_worktree),
        worktree_name: previous.worktree_name.clone(),
        use_worktree: in_worktree && previous.worktree_path.is_none(),
        initial_prompt: None,
        args: None,
        workflow_id: None,
        workflow_name: None,
    })
}

/// The session as it starts again when its worktree is gone: in the
/// project, as no worktree session.
fn grounded(previous: &TerminalSession, cwd: &str) -> TerminalSession {
    if previous.worktree_path.is_some() && cwd == previous.project_path {
        TerminalSession {
            worktree_path: None,
            is_worktree: Some(false),
            ..previous.clone()
        }
    } else {
        previous.clone()
    }
}

/// An agent started again on the conversation it had.
fn resume_agent(native: &Native, id: &str, previous: &TerminalSession) -> Answer {
    let Some(cwd) = resume_cwd_for(previous) else {
        return workspace_gone(previous);
    };
    let previous = grounded(previous, &cwd);
    if given(previous.remote_host_id.as_deref()).is_some() {
        return failed("Resuming a session on a remote host is the server's; it is not available while vornd owns the session records".into());
    }
    let Some(claims) = claims(native) else {
        return Answer::Forward;
    };
    // Read before the claim, so the claim and what it is checked against
    // are one step; and not lapsing while the workspace is prepared.
    let scope = transcript_scope(native, &previous);
    claims.preparing(id);
    let now = Instant::now();
    let transcript = free_transcript_for(native, &previous, scope.as_ref())
        .filter(|t| claims.claim(t, id, now).is_none());
    let Some(req) = restore_request(&previous, transcript.clone()) else {
        claims.prepared(id, Instant::now());
        return failed(format!(
            "{} is not an agent this build can start",
            previous.agent_type
        ));
    };
    let answer = start_agent(native, &req, id, previous.group_id.clone(), Made::Resumed);
    claims.prepared(id, Instant::now());
    match &answer {
        // The record names the conversation now: the claim standing in for it is spent.
        Answer::Result(record) if record.get("agentSessionId").is_some() => claims.release_for(id),
        Answer::Result(_) => {}
        _ => {
            if let Some(t) = &transcript {
                claims.release(t, id);
            }
        }
    }
    resumed(answer)
}

/// The project and its worktrees, where the agent may have recorded the
/// session's conversation (`transcriptScope`).
fn transcript_scope(native: &Native, session: &TerminalSession) -> Option<ProjectScope> {
    let git = Git {
        bin: native.env.git_bin(),
        env: native.env.get(),
    };
    let project = Path::new(&session.project_path);
    let worktrees: Vec<String> = git
        .list_worktrees(project)
        .into_iter()
        .map(|w| w.path)
        .collect();
    Some(ProjectScope::new(
        &session.project_path,
        worktrees.iter().map(String::as_str),
    ))
}

/// The conversations being written now: by the terminals and headless
/// agents running, and by the starts claiming one (`heldTranscripts`,
/// `spawningTranscripts`).
fn held_transcripts(native: &Native) -> Vec<String> {
    let mut held: Vec<String> = native
        .registry
        .get()
        .and_then(|r| {
            r.read(|r| {
                r.live_terminals()
                    .filter_map(|t| t.agent_session_id.clone())
                    .chain(
                        r.headless()
                            .filter(|h| h.status == HeadlessStatus::Running)
                            .filter_map(|h| h.agent_session_id.clone()),
                    )
                    .collect::<Vec<_>>()
            })
        })
        .unwrap_or_default();
    if let Some(claims) = claims(native) {
        held.extend(claims.held(Instant::now()));
    }
    held
}

/// The conversation a resume continues, among those nothing holds
/// (`freeTranscriptFor`): the one the record names, else the agent's most
/// recent in the project, preferring its worktree and project directories.
/// `None` lets the agent choose.
fn free_transcript_for(
    native: &Native,
    session: &TerminalSession,
    scope: Option<&ProjectScope>,
) -> Option<String> {
    let agent = Agent::from_id(&session.agent_type)?;
    if !agent.resumes_exactly() {
        return None;
    }
    let held = held_transcripts(native);
    if let Some(pinned) = given(session.agent_session_id.as_deref()) {
        if !held.iter().any(|h| h == pinned) {
            return Some(pinned.to_owned());
        }
    }
    let homes = Homes::from_env()?;
    let wanted: Vec<String> = [
        session.worktree_path.as_deref(),
        Some(&session.project_path),
    ]
    .into_iter()
    .flatten()
    .map(paths::comparable)
    .collect();
    let available = |c: &RecentSession| c.agent == agent && !held.contains(&c.session_id);
    let at_preferred = |candidates: &[RecentSession]| {
        wanted.iter().find_map(|path| {
            candidates
                .iter()
                .find(|c| available(c) && paths::comparable(&c.project_path) == *path)
                .map(|c| c.session_id.clone())
        })
    };
    let scoped = recent_sessions_for(agent, &homes, scope, RECENT_LIMIT);
    if let Some(found) = at_preferred(&scoped).or_else(|| {
        scoped
            .iter()
            .find(|c| available(c))
            .map(|c| c.session_id.clone())
    }) {
        return Some(found);
    }
    // Unscoped matches by path only: a loose match here would cross projects.
    at_preferred(&recent_sessions_for(agent, &homes, None, RECENT_LIMIT))
}

/// What is there now for each offered session against what its record
/// says (`verifyRestored`), for the local ones: whether its directory is
/// still there, and the branch and commit it stands at. Each answer is
/// kept on the record, for `sessions:restored`.
pub fn verify_restored(native: &Native) {
    let Some(registry) = native.registry.get() else {
        return;
    };
    let Some(offered) = registry.restored() else {
        return;
    };
    let git = Git {
        bin: native.env.git_bin(),
        env: native.env.get(),
    };
    let mut answers: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
    for one in offered {
        let session = &one["session"];
        let Some(id) = session.get("id").and_then(Value::as_str) else {
            continue;
        };
        if given(session.get("remoteHostId").and_then(Value::as_str)).is_some() {
            continue;
        }
        let cwd = session
            .get("worktreePath")
            .and_then(Value::as_str)
            .or_else(|| session.get("projectPath").and_then(Value::as_str))
            .unwrap_or_default()
            .to_owned();
        let present = Path::new(&cwd).is_dir();
        let (branch, head) = if present {
            // One git answer per directory: records share them.
            answers
                .entry(cwd.clone())
                .or_insert_with(|| (git.branch(Path::new(&cwd)), git.head(Path::new(&cwd))))
                .clone()
        } else {
            (None, None)
        };
        let environment = json!({
            "worktree": if present { "ok" } else { "missing" },
            "branch": { "recorded": session.get("branch").cloned().unwrap_or(Value::Null), "actual": branch },
            "head": { "recorded": session.get("headCommit").cloned().unwrap_or(Value::Null), "actual": head },
        });
        registry.set_environment(id, environment);
    }
}

/// A check's outcome as the server answers it: nothing, or its refusal.
pub(super) fn refused(check: Result<(), crate::registry::RegistryError>) -> Answer {
    match check {
        Ok(()) => Answer::Void,
        Err(e) => Answer::Error(e.to_string()),
    }
}

/// An array of strings, or `None`.
fn strings(v: &Value) -> Option<Vec<String>> {
    v.as_array()?
        .iter()
        .map(|s| s.as_str().map(str::to_owned))
        .collect()
}

/// A `terminal:create` payload vornd can answer: a local agent session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateRequest {
    pub agent: Agent,
    pub model: Option<String>,
    pub project_name: String,
    pub project_path: String,
    pub resume_session_id: Option<String>,
    pub session_id: Option<String>,
    pub display_name: Option<String>,
    pub branch: Option<String>,
    pub use_worktree: bool,
    pub existing_worktree_path: Option<String>,
    pub worktree_name: Option<String>,
    pub initial_prompt: Option<String>,
    pub args: Option<Vec<String>>,
    /// Read by a headless create only: the workflow that asked for it.
    pub workflow_id: Option<String>,
    pub workflow_name: Option<String>,
}

impl CreateRequest {
    /// Reads `params` as the server's handler does. `None` for a call that
    /// is the server's: a remote host's, an agent this build does not know,
    /// a param it does not read or one of a shape it would not expect.
    pub fn read(params: &Value) -> Option<CreateRequest> {
        let p = params.as_object()?;
        if p.keys().any(|k| !CREATE_KEYS.contains(&k.as_str())) {
            return None;
        }
        let text = |k: &str| -> Option<Option<String>> {
            match p.get(k) {
                None | Some(Value::Null) => Some(None),
                Some(Value::String(s)) => Some(Some(s.clone())),
                Some(_) => None,
            }
        };
        let flag = |k: &str| -> Option<bool> {
            match p.get(k) {
                None | Some(Value::Null) => Some(false),
                Some(Value::Bool(b)) => Some(*b),
                Some(_) => None,
            }
        };
        if text("remoteHostId")?.is_some_and(|h| !h.is_empty()) {
            return None;
        }
        // Read for its shape only: the server's handlers ignore it.
        flag("headless")?;
        match p.get("promptDelayMs") {
            None | Some(Value::Null | Value::Number(_)) => {}
            Some(_) => return None,
        }
        let args = match p.get("args") {
            None | Some(Value::Null) => None,
            Some(v) => Some(strings(v)?),
        };
        let project_path = text("projectPath")??;
        if !Path::new(&project_path).is_absolute() {
            return None;
        }
        Some(CreateRequest {
            agent: Agent::from_id(&text("agentType")??)?,
            model: text("model")?,
            project_name: text("projectName")??,
            project_path,
            resume_session_id: text("resumeSessionId")?,
            session_id: text("sessionId")?,
            display_name: text("displayName")?,
            branch: text("branch")?,
            use_worktree: flag("useWorktree")?,
            existing_worktree_path: text("existingWorktreePath")?,
            worktree_name: text("worktreeName")?,
            initial_prompt: text("initialPrompt")?,
            args,
            workflow_id: text("workflowId")?,
            workflow_name: text("workflowName")?,
        })
    }

    /// The conversation the create names, when its agent can be sent back
    /// to one (`transcriptNamedOnCreate`).
    pub fn named(&self) -> Option<&str> {
        given(self.resume_session_id.as_deref()).filter(|_| self.agent.resumes_exactly())
    }
}

/// JavaScript's truthiness for an optional string.
pub(super) fn given(s: Option<&str>) -> Option<&str> {
    s.filter(|s| !s.is_empty())
}

pub(super) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// A record as the server's handler answers it.
fn record_json(record: &TerminalSession) -> Value {
    serde_json::to_value(record).unwrap_or(Value::Null)
}

/// Whether vornd can start a session now: it creates terminals (as
/// `vornd:hello` told the server, which follows them only then), the
/// registry holds the server's records and decides, and the session holder
/// is connected. When not, the server does it.
fn can_start(native: &Native) -> bool {
    let creates = native.link.get().is_some_and(|l| l.creates_terminals());
    creates && fed_and_held(native)
}

/// Whether the registry holds the server's records and decides, and the
/// session holder is connected.
pub(super) fn fed_and_held(native: &Native) -> bool {
    let fed = native
        .registry
        .get()
        .is_some_and(|r| r.read(|r| r.decides()) == Some(true));
    fed && native.host.get().is_some_and(|h| h.ready())
}

/// `terminal:create` for a local agent.
fn create(native: &Native, req: &CreateRequest) -> Answer {
    if !can_start(native) {
        return Answer::Forward;
    }
    let Some(named) = req.named().map(str::to_owned) else {
        return start_agent(native, req, &new_id(), None, Made::Created);
    };
    // Naming a conversation that is already running: show what is writing
    // it rather than starting a second agent on it.
    if let Some(running) = running_on(native, &named) {
        return Answer::Result(running);
    }
    let Some(claims) = claims(native) else {
        return Answer::Forward;
    };
    // Preparing may make a worktree or check out a branch: a second create
    // for the same conversation meanwhile gets the first one's session.
    native
        .sessions
        .creating
        .run(&named, || create_named(native, claims, req, &named))
}

/// A create naming conversation `named`, claimed under the id the session
/// will have before anything is prepared.
fn create_named(native: &Native, claims: &Claims, req: &CreateRequest, named: &str) -> Answer {
    let id = new_id();
    if let Some(holder) = claims.claim(named, &id, Instant::now()) {
        // A start of the server's (a resume) got there first: wait for it,
        // then show what it started.
        let deadline = Instant::now() + HOLDER_WAIT;
        while claims.holder(named, Instant::now()).as_deref() == Some(holder.as_str())
            && Instant::now() < deadline
            && live_record(native, &holder).is_none()
        {
            std::thread::sleep(HOLDER_POLL);
        }
        if let Some(bound) = running_on(native, named).or_else(|| live_record(native, &holder)) {
            return Answer::Result(bound);
        }
        if claims.claim(named, &id, Instant::now()).is_some() {
            return Answer::Error("This conversation is already starting in another pane".into());
        }
    }
    claims.preparing(&id);
    let answer = start_agent(native, req, &id, None, Made::Created);
    claims.prepared(&id, Instant::now());
    let pinned = match &answer {
        Answer::Result(record) => record.get("agentSessionId").is_some(),
        _ => false,
    };
    // An agent told the id names the conversation itself; one that cannot
    // be keeps the claim until it reports, seconds later. A create that
    // failed holds nothing.
    if pinned || !matches!(answer, Answer::Result(_)) {
        claims.release(named, &id);
    }
    answer
}

/// The live terminal writing conversation `transcript`, as a record.
fn running_on(native: &Native, transcript: &str) -> Option<Value> {
    native.registry.get()?.read(|r| {
        r.live_terminals()
            .find(|t| t.agent_session_id.as_deref() == Some(transcript))
            .map(record_json)
    })?
}

/// Terminal `id`, while its program runs.
fn live_record(native: &Native, id: &str) -> Option<Value> {
    native
        .registry
        .get()?
        .read(|r| r.live_terminals().find(|t| t.id == id).map(record_json))?
}

pub(super) fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// What `prepareSession` works out for a local agent before it starts.
#[derive(Debug, Default)]
struct Prepared {
    agent_session_id: Option<String>,
    launch_line: String,
    /// Where the shell starts: the worktree when there is one.
    cwd: String,
    worktree_path: Option<String>,
    worktree_name: Option<String>,
    branch: Option<String>,
    head_commit: Option<String>,
}

/// The workspaces one preparation holds, let go of when it is dropped.
pub(super) struct Holds<'a> {
    native: &'a Native,
    dirs: Vec<String>,
}

impl<'a> Holds<'a> {
    pub(super) fn new(native: &'a Native) -> Holds<'a> {
        Holds {
            native,
            dirs: Vec::new(),
        }
    }

    pub(super) fn hold(&mut self, dir: &str) {
        let key = paths::normalize(dir);
        if let Some(registry) = self.native.registry.get() {
            registry.change(|r| ((), vec![r.hold(&key)]));
        }
        self.dirs.push(key);
    }
}

impl Drop for Holds<'_> {
    fn drop(&mut self) {
        let Some(registry) = self.native.registry.get() else {
            return;
        };
        for dir in self.dirs.drain(..) {
            registry.change(|r| ((), r.release(&dir).into_iter().collect()));
        }
    }
}

/// How a record comes to be in the registry: made new, or started again
/// under an id it had.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Made {
    Created,
    Resumed,
}

/// Prepares and starts agent session `id`, in `group_id` when it is filed
/// under one (a resume carries the group over).
fn start_agent(
    native: &Native,
    req: &CreateRequest,
    id: &str,
    group_id: Option<String>,
    made: Made,
) -> Answer {
    if let Some(why) = refusal(native) {
        return Answer::Error(why.to_owned());
    }
    let Some(settings) = agent::settings(native) else {
        return Answer::Forward;
    };
    let Some(config) = agent::command_of(&settings, req.agent) else {
        return Answer::Forward;
    };
    let mut holds = Holds::new(native);
    // Held from here until the record is in, so a worktree action in between
    // sees it as in use: the worktree it names, and one it creates.
    if let Some(existing) = given(req.existing_worktree_path.as_deref()) {
        holds.hold(existing);
    }
    let prepared = match prepare(native, req, &config, &mut holds) {
        Ok(p) => p,
        Err(answer) => return answer,
    };
    // Again: closing may have begun while the workspace was prepared.
    if let Some(why) = refusal(native) {
        return Answer::Error(why.to_owned());
    }
    let shell = launch_shell::default_shell(settings.shell.as_deref(), Platform::HOST, var);
    let mut argv = vec![shell];
    argv.extend(
        launch_shell::default_shell_args(Platform::HOST)
            .iter()
            .map(|a| (*a).to_owned()),
    );
    let data_dir = native.db.get().and_then(|db| db.parent());
    let env = native.env.launch(&settings.env_passthrough, data_dir);
    let record = TerminalSession {
        display_name: given(req.display_name.as_deref())
            .map(str::to_owned)
            .or_else(|| {
                given(req.initial_prompt.as_deref())
                    .and_then(|p| display_name_from_prompt(p, PROMPT_NAME_LEN))
            }),
        branch: prepared.branch.filter(|b| !b.is_empty()),
        head_commit: prepared.head_commit.filter(|h| !h.is_empty()),
        is_worktree: prepared.worktree_path.as_ref().map(|_| true),
        worktree_name: prepared
            .worktree_path
            .as_ref()
            .and(prepared.worktree_name.clone()),
        worktree_path: prepared.worktree_path,
        agent_session_id: prepared.agent_session_id.filter(|a| !a.is_empty()),
        group_id,
        ..skeleton(id, req.agent.id(), &req.project_name, &req.project_path)
    };
    let typed = Input::Typed(format!("{}\r", prepared.launch_line).into_bytes());
    let answer = register(native, record, &prepared.cwd, argv, env, typed, made);
    drop(holds);
    answer
}

/// A record with what every new terminal has.
fn skeleton(id: &str, agent: &str, project_name: &str, project_path: &str) -> TerminalSession {
    TerminalSession {
        id: id.to_owned(),
        agent_type: agent.to_owned(),
        project_name: project_name.to_owned(),
        project_path: project_path.to_owned(),
        status: AgentStatus::Running,
        created_at: now_ms(),
        pid: 0,
        display_name: None,
        branch: None,
        worktree_path: None,
        worktree_name: None,
        is_worktree: None,
        remote_host_id: None,
        remote_host_label: None,
        hook_session_id: None,
        agent_session_id: None,
        status_source: None,
        group_id: None,
        cols: Some(u32::from(INITIAL_COLS)),
        rows: Some(u32::from(INITIAL_ROWS)),
        renamed_by_person: None,
        shell_cwd: None,
        head_commit: None,
        shell_exit_code: None,
        saved_at: None,
        rev: None,
        status_at: None,
        exit_at: None,
        other: Map::new(),
    }
}

pub(super) fn var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// The workspace a session runs in, as `prepareSession` and
/// `createHeadless` work it out: the worktree it names, or one made for
/// it, or the branch checked out in the project.
#[derive(Debug, Default)]
pub(super) struct Workspace {
    /// Where the program starts: the worktree when there is one.
    pub cwd: String,
    /// The branch the call asked for, as it was taken; `None` leaves it to
    /// what git says of `cwd`.
    pub branch: Option<String>,
    /// The worktree the call named, when it is there.
    pub reused: Option<String>,
    /// The worktree made for the call: its path and name.
    pub made: Option<(String, String)>,
}

/// Works out the workspace for `req`, making a worktree or checking out a
/// branch as the server does, holding each directory made in `holds`.
pub(super) fn workspace(
    native: &Native,
    req: &CreateRequest,
    holds: &mut Holds<'_>,
) -> Result<Workspace, Answer> {
    let git = Git {
        bin: native.env.git_bin(),
        env: native.env.get(),
    };
    let project = req.project_path.as_str();
    let mut out = Workspace {
        cwd: project.to_owned(),
        ..Workspace::default()
    };
    let branch = given(req.branch.as_deref());
    let existing = given(req.existing_worktree_path.as_deref());
    if let Some(existing) = existing.filter(|e| Path::new(e).exists()) {
        out.cwd = existing.to_owned();
        out.branch = req.branch.clone();
        out.reused = Some(existing.to_owned());
    } else if let Some(branch) = branch.filter(|_| req.use_worktree || existing.is_some()) {
        if git.is_git_repo(Path::new(project)) {
            if let Some(gone) = existing {
                warn!(path = gone, "the worktree is gone; making a new one");
            }
            let made = native.turns.take(Path::new(project), || {
                git.create_worktree_at(project, branch, req.worktree_name.as_deref(), |p| {
                    holds.hold(p)
                })
            });
            let made = made.map_err(|e| Answer::Error(e.to_string()))?;
            out.cwd.clone_from(&made.worktree_path);
            out.made = Some((made.worktree_path, made.name));
            out.branch = Some(made.branch);
        } else {
            warn!(project, "not a git repository; no worktree for it");
        }
    } else if let Some(branch) = branch {
        let dir = Path::new(project);
        if git.is_git_repo(dir) {
            if git.branch(dir).as_deref() != Some(branch) {
                // What git says is not the call's: the session starts on
                // whatever is checked out, as the server's does.
                let _ = native.turns.take(dir, || git.checkout(dir, branch));
            }
            out.branch = Some(branch.to_owned());
        }
    }
    if out.branch.as_deref().is_none_or(str::is_empty) {
        out.branch = git.branch(Path::new(&out.cwd));
    }
    Ok(out)
}

/// The commit `dir` stands at.
pub(super) fn head_of(native: &Native, dir: &str) -> Option<String> {
    let git = Git {
        bin: native.env.git_bin(),
        env: native.env.get(),
    };
    git.head(Path::new(dir))
}

/// The pinned id, the launch line and the workspace (`prepareLocal`). The
/// line is built first, so a line that cannot be built creates nothing.
fn prepare(
    native: &Native,
    req: &CreateRequest,
    config: &vorn_agents::AgentCommand,
    holds: &mut Holds<'_>,
) -> Result<Prepared, Answer> {
    let resume = given(req.resume_session_id.as_deref()).map(str::to_owned);
    // Agents that take one are given a fresh id to start the conversation
    // under, so it can be resumed exactly later.
    let mut agent_session_id = if req.agent.resumes_exactly() {
        req.resume_session_id.clone()
    } else {
        None
    };
    let mut session_id = req.session_id.clone();
    if req.agent.pins_session_ids() {
        agent_session_id = Some(resume.clone().unwrap_or_else(|| {
            let minted = new_id();
            session_id = Some(minted.clone());
            minted
        }));
    }
    let launch = LaunchRequest {
        agent: req.agent,
        args: req.args.clone(),
        model: req.model.clone(),
        remote_host_id: None,
        resume_session_id: req.resume_session_id.clone(),
        session_id,
        initial_prompt: req.initial_prompt.clone(),
    };
    let default_shell = launch_shell::default_shell(None, Platform::HOST, var);
    let machine = Machine {
        platform: Platform::HOST,
        quoting: Quoting::local(Platform::HOST, &default_shell),
    };
    let safe = native.env.get();
    let launch_line = launch_line(&launch, Some(config), &safe, &machine)
        .map_err(|e| Answer::Error(e.to_string()))?;

    let space = workspace(native, req, holds)?;
    let mut out = Prepared {
        agent_session_id,
        launch_line,
        head_commit: head_of(native, &space.cwd),
        branch: space.branch,
        cwd: space.cwd,
        ..Prepared::default()
    };
    // The project itself, named as a worktree, is no worktree.
    if let Some(existing) = space
        .reused
        .filter(|e| paths::normalize(e) != paths::normalize(&req.project_path))
    {
        out.worktree_name = Some(
            given(req.worktree_name.as_deref())
                .map_or_else(|| extract_worktree_name(&existing), str::to_owned),
        );
        out.worktree_path = Some(existing);
    } else if let Some((path, name)) = space.made {
        out.worktree_path = Some(path);
        out.worktree_name = Some(name);
    }
    Ok(out)
}

/// `shell:create`: a login shell in `cwd`, or the home directory, with its
/// integration, numbered after the shells there are.
fn shell_create(native: &Native, cwd: Option<&str>) -> Answer {
    if !can_start(native) {
        return Answer::Forward;
    }
    let settings = agent::settings(native).unwrap_or_default();
    // Shims of a version this build does not know, or not written yet: the
    // server starts it.
    let Ok((shell, setup)) = native.shells.setup(
        &native.env,
        settings.shell.as_deref(),
        settings.minimal_shell_prompt,
    ) else {
        return Answer::Forward;
    };
    let dir = cwd.map_or_else(shell::home_dir, str::to_owned);
    let mut argv = vec![shell];
    match setup.args {
        Some(args) => argv.extend(args),
        None => argv.extend(
            launch_shell::default_shell_args(Platform::HOST)
                .iter()
                .map(|a| (*a).to_owned()),
        ),
    }
    let mut env = native.env.get();
    for (k, v) in setup.env {
        set(&mut env, &k, v);
    }
    let id = new_id();
    let project_name = match node_basename(&dir) {
        "" => "shell".to_owned(),
        name => name.to_owned(),
    };
    let Some(count) = native.registry.get().and_then(|r| r.read(Registry::shells)) else {
        return Answer::Forward;
    };
    let record = TerminalSession {
        display_name: Some(format!("Shell {}", count + 1)),
        shell_cwd: Some(dir.clone()),
        ..skeleton(&id, "shell", &project_name, &dir)
    };
    register(native, record, &dir, argv, env, Input::None, Made::Created)
}

/// Sets `key` in `env`, replacing a value it had, as an object spread does.
pub(super) fn set(env: &mut Vec<(String, String)>, key: &str, value: String) {
    match env.iter_mut().find(|(k, _)| k == key) {
        Some(slot) => slot.1 = value,
        None => env.push((key.to_owned(), value)),
    }
}

/// Puts `record` in the registry and starts its program in `cwd`. The
/// answer is the record, as the server answers before its program is up.
fn register(
    native: &Native,
    record: TerminalSession,
    cwd: &str,
    argv: Vec<String>,
    mut env: Vec<(String, String)>,
    input: Input,
    made: Made,
) -> Answer {
    let (Some(registry), Some(host)) = (native.registry.get(), native.host.get()) else {
        return Answer::Forward;
    };
    let id = record.id.clone();
    // Set at the spawn and never in vornd's own environment, so no other
    // child inherits one session's id.
    set(&mut env, "VORN_SESSION_ID", id.clone());
    if !cfg!(windows) {
        set(&mut env, "TERM", PTY_TERM.to_owned());
    }
    let answer = record_json(&record);
    let made = registry.change(|r| {
        let put = match made {
            Made::Created => r.create(record),
            Made::Resumed => r.resume(record),
        };
        match put {
            Ok(notes) => (Ok(()), notes),
            Err(e) => (Err(e), Vec::new()),
        }
    });
    match made {
        Some(Ok(())) => {}
        Some(Err(e)) => return Answer::Error(e.to_string()),
        // The server went while the session was prepared.
        None => return Answer::Error("The Vorn server is not connected to vornd".to_owned()),
    }
    let spec = SpawnSpec {
        argv,
        cwd: cwd.to_owned(),
        env,
        io: Io::Pty {
            cols: INITIAL_COLS,
            rows: INITIAL_ROWS,
        },
        ring_bytes: None,
    };
    native.sessions.lock_starting().insert(id.clone(), None);
    let (registry, host_after) = (std::sync::Arc::clone(registry), std::sync::Arc::clone(host));
    let sessions = std::sync::Arc::clone(&native.sessions);
    let name = id.clone();
    host.start(
        spec,
        name,
        input,
        Box::new(move |outcome| {
            let doomed = sessions.lock_starting().remove(&id).flatten();
            match outcome {
                Ok(s) => {
                    // A record gone meanwhile (the server let go of it) holds
                    // the program no more than a close does.
                    let held = registry
                        .change(|r| match r.started(&id, s.pid, s.epoch) {
                            Some(note) => (true, vec![note]),
                            None => (false, Vec::new()),
                        })
                        .unwrap_or(false);
                    if let Some(sig) = doomed.or((!held).then_some(Sig::Hup)) {
                        host_after.signal(&id, sig);
                    }
                }
                Err(why) => {
                    warn!(%id, %why, "vornd could not start this session");
                    registry.change(|r| ((), r.failed(&id, &why).into_iter().collect()));
                }
            }
        }),
    );
    Answer::Result(answer)
}

impl Sessions {
    pub(super) fn lock_starting(&self) -> std::sync::MutexGuard<'_, HashMap<String, Option<Sig>>> {
        self.starting.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Asks for `sig` to be sent to session `id` once its program is up,
    /// if it is still starting: answers whether it was, under the one lock
    /// the start's answer takes.
    pub(super) fn doom(&self, id: &str, sig: Sig) -> bool {
        match self.lock_starting().get_mut(id) {
            Some(pending) => {
                *pending = Some(sig);
                true
            }
            None => false,
        }
    }
}

/// `terminal:kill`: the card closes and its program, if it still runs, is
/// hung up. The last session in a worktree to close offers to clean it up.
fn kill(native: &Native, id: &str) -> Answer {
    let Some(registry) = native.registry.get() else {
        return Answer::Forward;
    };
    // A session of an earlier run has no program to hang up: closing it
    // is a decision about the record, as a resume is, taken once.
    let offered = registry.change(
        |r| match r.owns().then(|| r.consume_restored(id)).flatten() {
            Some((_, note)) => (true, vec![note]),
            None => (false, Vec::new()),
        },
    );
    if offered == Some(true) {
        if let Some(host) = native.host.get() {
            host.forget(id);
        }
        return Answer::Void;
    }
    let closed = registry.change(|r| match r.close(id) {
        Ok((record, live, notes)) => {
            let offer = record
                .worktree_path
                .as_deref()
                .filter(|w| r.active_in_worktree(w).is_empty())
                .map(|w| {
                    json!({
                        "id": record.id,
                        "projectPath": record.project_path,
                        "worktreePath": w,
                    })
                });
            (Some((live, offer)), notes)
        }
        Err(_) => (None, Vec::new()),
    });
    // Not a terminal vornd's copy holds: one carried over from a previous
    // run, which the server keeps.
    let Some(Some((live, offer))) = closed else {
        return Answer::Forward;
    };
    if let Some(claims) = claims(native) {
        claims.release_for(id);
    }
    if live && !native.sessions.doom(id, Sig::Hup) {
        if let Some(host) = native.host.get() {
            host.signal(id, Sig::Hup);
        }
    }
    if let (Some(offer), Some(link)) = (offer, native.link.get()) {
        link.tell("vornd:cleanupOffer", offer);
    }
    Answer::Void
}

/// `terminal:rename` and `terminal:setGroup`.
fn set_fields(native: &Native, id: &str, fields: Map<String, Value>) -> Answer {
    let Some(registry) = native.registry.get() else {
        return Answer::Forward;
    };
    let done = registry.change(|r| match r.set_fields(id, fields) {
        Ok(note) => (true, note.into_iter().collect()),
        Err(_) => (false, Vec::new()),
    });
    match done {
        Some(true) => Answer::Void,
        // No such terminal here: the server says what it makes of it.
        _ => Answer::Forward,
    }
}

/// `terminal:reorder`: told even when the order is the one there was, as
/// the server tells it.
fn reorder(native: &Native, ids: Vec<String>) -> Answer {
    let Some(registry) = native.registry.get() else {
        return Answer::Forward;
    };
    let done = registry.change(|r| match r.reorder(ids) {
        Ok(note) => (true, vec![note]),
        Err(_) => (false, Vec::new()),
    });
    match done {
        Some(true) => Answer::Void,
        // A duplicate or an unknown id: refused by the server, in its words.
        _ => Answer::Forward,
    }
}

/// Whether [`plan`] works out what `method` would start.
pub fn plans(method: &str) -> bool {
    matches!(
        method,
        "terminal:create" | "shell:create" | "headless:create" | "sessions:resume"
    )
}

/// What a create or a shell would start, worked out without starting or
/// changing anything, for the spawn-plan comparison in shadow mode:
/// `{argv, cwd, envKeys, record}`, the record as [`plan_record`] keeps it.
/// `shells` is how many shells there were when the call came. `None` when
/// it cannot be worked out without a change (a worktree to make, a branch
/// to check out, a conversation to claim) or vornd could not start it
/// itself.
pub fn plan(native: &Native, method: &str, params: &Value, shells: usize) -> Option<Value> {
    match method {
        "terminal:create" => plan_agent(native, &CreateRequest::read(params)?),
        "headless:create" => headless::plan(native, &CreateRequest::read(params)?),
        "shell:create" => {
            let cwd = match params {
                Value::Null => None,
                Value::String(c) if c.is_empty() => None,
                Value::String(c) if Path::new(c).is_absolute() => Some(c.as_str()),
                _ => return None,
            };
            plan_shell(native, cwd, shells)
        }
        "sessions:resume" => plan_resume(native, params.get("id")?.as_str()?),
        _ => None,
    }
}

/// What a resume would start: a shell where the session was, or the agent
/// on the conversation the record names, under its id. `None` when the id
/// names no session to resume, or the start needs a change (a worktree to
/// make) or a conversation looked up in the agent's history.
fn plan_resume(native: &Native, id: &str) -> Option<Value> {
    let previous = native.registry.get()?.read(|r| {
        r.restored()
            .iter()
            .find(|o| o["session"]["id"] == id)
            .and_then(|o| serde_json::from_value::<TerminalSession>(o["session"].clone()).ok())
            .or_else(|| r.ended_terminal(id).cloned())
    })??;
    let cwd = resume_cwd_for(&previous)?;
    if previous.agent_type == "shell" {
        let settings = agent::settings(native).unwrap_or_default();
        let (shell, setup) = native
            .shells
            .setup(
                &native.env,
                settings.shell.as_deref(),
                settings.minimal_shell_prompt,
            )
            .ok()?;
        let mut argv = vec![shell];
        match setup.args {
            Some(args) => argv.extend(args),
            None => argv.extend(
                launch_shell::default_shell_args(Platform::HOST)
                    .iter()
                    .map(|a| (*a).to_owned()),
            ),
        }
        let mut env = native.env.get();
        for (k, v) in setup.env {
            set(&mut env, &k, v);
        }
        let count = shells_before(native, id)?;
        return Some(plan_json(
            argv,
            &cwd,
            env,
            &resumed_shell(&previous, &cwd, count),
        ));
    }
    let previous = grounded(&previous, &cwd);
    // Only a conversation the record names: a lookup in the agent's
    // history is the server's to make, and the plan would guess at it.
    let transcript = given(previous.agent_session_id.as_deref()).map(str::to_owned);
    let req = restore_request(&previous, transcript)?;
    let settings = agent::settings(native)?;
    let config = agent::command_of(&settings, req.agent)?;
    if given(req.existing_worktree_path.as_deref()).is_none_or(|e| !Path::new(e).exists())
        && req.use_worktree
    {
        return None;
    }
    let mut holds = Holds::new(native);
    let prepared = prepare(native, &req, &config, &mut holds).ok()?;
    let shell = launch_shell::default_shell(settings.shell.as_deref(), Platform::HOST, var);
    let mut argv = vec![shell];
    argv.extend(
        launch_shell::default_shell_args(Platform::HOST)
            .iter()
            .map(|a| (*a).to_owned()),
    );
    let data_dir = native.db.get().and_then(|db| db.parent());
    let env = native.env.launch(&settings.env_passthrough, data_dir);
    let record = TerminalSession {
        display_name: given(req.display_name.as_deref()).map(str::to_owned),
        branch: prepared.branch.filter(|b| !b.is_empty()),
        head_commit: prepared.head_commit.filter(|h| !h.is_empty()),
        is_worktree: prepared.worktree_path.as_ref().map(|_| true),
        worktree_name: prepared
            .worktree_path
            .as_ref()
            .and(prepared.worktree_name.clone()),
        worktree_path: prepared.worktree_path,
        agent_session_id: prepared.agent_session_id.filter(|a| !a.is_empty()),
        group_id: previous.group_id.clone(),
        ..skeleton(id, req.agent.id(), &req.project_name, &req.project_path)
    };
    Some(plan_json(argv, &prepared.cwd, env, &record))
}

fn plan_agent(native: &Native, req: &CreateRequest) -> Option<Value> {
    let settings = agent::settings(native)?;
    let config = agent::command_of(&settings, req.agent)?;
    let existing = given(req.existing_worktree_path.as_deref());
    // Making a worktree or checking out a branch is a change.
    let reuses = existing.is_some_and(|e| Path::new(e).exists());
    if !reuses && given(req.branch.as_deref()).is_some() {
        return None;
    }
    if req.named().is_some() {
        return None;
    }
    let mut holds = Holds::new(native);
    let prepared = prepare(native, req, &config, &mut holds).ok()?;
    let shell = launch_shell::default_shell(settings.shell.as_deref(), Platform::HOST, var);
    let mut argv = vec![shell];
    argv.extend(
        launch_shell::default_shell_args(Platform::HOST)
            .iter()
            .map(|a| (*a).to_owned()),
    );
    let data_dir = native.db.get().and_then(|db| db.parent());
    let env = native.env.launch(&settings.env_passthrough, data_dir);
    let record = TerminalSession {
        display_name: given(req.display_name.as_deref())
            .map(str::to_owned)
            .or_else(|| {
                given(req.initial_prompt.as_deref())
                    .and_then(|p| display_name_from_prompt(p, PROMPT_NAME_LEN))
            }),
        branch: prepared.branch.filter(|b| !b.is_empty()),
        head_commit: prepared.head_commit.filter(|h| !h.is_empty()),
        is_worktree: prepared.worktree_path.as_ref().map(|_| true),
        worktree_name: prepared
            .worktree_path
            .as_ref()
            .and(prepared.worktree_name.clone()),
        worktree_path: prepared.worktree_path,
        agent_session_id: prepared.agent_session_id.filter(|a| !a.is_empty()),
        ..skeleton("", req.agent.id(), &req.project_name, &req.project_path)
    };
    Some(plan_json(argv, &prepared.cwd, env, &record))
}

fn plan_shell(native: &Native, cwd: Option<&str>, count: usize) -> Option<Value> {
    let settings = agent::settings(native).unwrap_or_default();
    let (shell, setup) = native
        .shells
        .setup(
            &native.env,
            settings.shell.as_deref(),
            settings.minimal_shell_prompt,
        )
        .ok()?;
    let dir = cwd.map_or_else(shell::home_dir, str::to_owned);
    let mut argv = vec![shell];
    match setup.args {
        Some(args) => argv.extend(args),
        None => argv.extend(
            launch_shell::default_shell_args(Platform::HOST)
                .iter()
                .map(|a| (*a).to_owned()),
        ),
    }
    let mut env = native.env.get();
    for (k, v) in setup.env {
        set(&mut env, &k, v);
    }
    let project_name = match node_basename(&dir) {
        "" => "shell".to_owned(),
        name => name.to_owned(),
    };
    let record = TerminalSession {
        display_name: Some(format!("Shell {}", count + 1)),
        shell_cwd: Some(dir.clone()),
        ..skeleton("", "shell", &project_name, &dir)
    };
    Some(plan_json(argv, &dir, env, &record))
}

/// A plan as it is compared: the environment by its names, sorted, with
/// the two vornd adds at the spawn.
fn plan_json(
    argv: Vec<String>,
    cwd: &str,
    env: Vec<(String, String)>,
    record: &TerminalSession,
) -> Value {
    let mut keys: Vec<String> = env.into_iter().map(|(k, _)| k).collect();
    keys.push("VORN_SESSION_ID".to_owned());
    if !cfg!(windows) {
        keys.push("TERM".to_owned());
    }
    plan_of(argv, cwd, keys, &record_json(record))
}

/// What a minted conversation id reads as in a plan, wherever it appears.
const MINTED: &str = "<agentSessionId>";

/// The conversation id `record` was given, when it was given one.
fn minted(record: &Value) -> Option<&str> {
    given(record.get("agentSessionId").and_then(Value::as_str))
}

/// A plan from its parts, the environment's names sorted, and the minted
/// conversation id the same on both sides.
pub(super) fn plan_of(
    argv: Vec<String>,
    cwd: &str,
    mut keys: Vec<String>,
    record: &Value,
) -> Value {
    keys.sort();
    keys.dedup();
    let argv: Vec<String> = match minted(record) {
        Some(id) => argv.iter().map(|a| a.replace(id, MINTED)).collect(),
        None => argv,
    };
    json!({
        "argv": argv,
        "cwd": cwd,
        "envKeys": keys,
        "record": plan_record(record),
    })
}

/// A record as a plan is compared: without what differs from one start to
/// the next (the id, when it was made, the pid, a minted conversation id)
/// and the hook session the server links copilot to once it has the record.
pub fn plan_record(record: &Value) -> Value {
    let mut kept = BTreeMap::new();
    if let Value::Object(fields) = record {
        for (k, v) in fields {
            if matches!(
                k.as_str(),
                "id" | "createdAt"
                    | "startedAt"
                    | "pid"
                    | "rev"
                    | "statusAt"
                    | "exitAt"
                    | "hookSessionId"
            ) {
                continue;
            }
            // Minted for an agent that pins one: only that there is one,
            // and the same wherever the launch names it.
            let v = match (k.as_str(), v.as_str(), minted(record)) {
                ("agentSessionId", _, _) => json!(true),
                ("launchCommand", Some(line), Some(id)) => json!(line.replace(id, MINTED)),
                _ => v.clone(),
            };
            kept.insert(k.clone(), v);
        }
    }
    json!(kept)
}

/// A spawn as the server asked vornd for it, as a plan is compared.
pub fn spawn_plan(spawn: &Value, reply: &Value) -> Value {
    let argv = spawn.get("argv").and_then(strings).unwrap_or_default();
    let cwd = spawn.get("cwd").and_then(Value::as_str).unwrap_or_default();
    let keys: Vec<String> = spawn
        .get("env")
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    plan_of(argv, cwd, keys, reply)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// A project path that is absolute where the test runs.
    fn project() -> &'static str {
        if cfg!(windows) {
            "C:\\p"
        } else {
            "/p"
        }
    }

    #[test]
    fn reads_a_local_create_and_leaves_the_rest_to_the_server() {
        let req = CreateRequest::read(&json!({
            "agentType": "claude", "projectName": "p", "projectPath": project(),
            "useWorktree": true, "branch": "b", "args": ["--x"], "promptDelayMs": 5,
        }))
        .unwrap();
        assert_eq!(req.agent, Agent::Claude);
        assert!(req.use_worktree);
        assert_eq!(req.args.as_deref(), Some(&["--x".to_owned()][..]));
        for theirs in [
            // A remote host's session is started over SSH, by the server.
            json!({ "agentType": "claude", "projectName": "p", "projectPath": project(), "remoteHostId": "h" }),
            // A param the handler does not read.
            json!({ "agentType": "claude", "projectName": "p", "projectPath": project(), "extra": 1 }),
            // Shapes it would not expect.
            json!({ "agentType": "claude", "projectName": "p", "projectPath": project(), "useWorktree": "yes" }),
            json!({ "agentType": "claude", "projectName": "p", "projectPath": "rel" }),
            json!({ "agentType": "shell", "projectName": "p", "projectPath": project() }),
            json!({ "agentType": "claude", "projectPath": project() }),
            json!("claude"),
        ] {
            assert_eq!(CreateRequest::read(&theirs), None, "{theirs}");
        }
        // An empty remote host is none, as the handler reads it.
        assert!(CreateRequest::read(&json!({
            "agentType": "codex", "projectName": "p", "projectPath": project(), "remoteHostId": "",
        }))
        .is_some());
    }

    #[test]
    fn names_a_conversation_only_for_an_agent_that_can_be_sent_back_to_one() {
        let named = |agent: &str, resume: Value| {
            CreateRequest::read(&json!({
                "agentType": agent, "projectName": "p", "projectPath": project(),
                "resumeSessionId": resume,
            }))
            .unwrap()
            .named()
            .map(str::to_owned)
        };
        assert_eq!(named("claude", json!("c1")).as_deref(), Some("c1"));
        assert_eq!(named("codex", json!("c2")).as_deref(), Some("c2"));
        assert_eq!(named("gemini", json!("c3")), None);
        assert_eq!(named("claude", json!("")), None);
        assert_eq!(named("claude", Value::Null), None);
    }

    /// A host that starts nothing until told, and keeps the signals sent.
    #[derive(Default)]
    pub(in crate::native) struct FakeHost {
        pub(in crate::native) starts: Mutex<Vec<(String, SpawnSpec, Input, Then)>>,
        signals: Mutex<Vec<(String, Sig)>>,
        forgotten: Mutex<Vec<String>>,
    }

    impl fmt::Debug for FakeHost {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("FakeHost")
        }
    }

    impl Host for FakeHost {
        fn ready(&self) -> bool {
            true
        }
        fn start(&self, spec: SpawnSpec, name: String, input: Input, then: Then) {
            self.starts.lock().unwrap().push((name, spec, input, then));
        }
        fn signal(&self, id: &str, sig: Sig) {
            self.signals.lock().unwrap().push((id.to_owned(), sig));
        }
        fn signal_after(&self, id: &str, sig: Sig, _: Duration) {
            self.signals.lock().unwrap().push((id.to_owned(), sig));
        }
        fn forget(&self, id: &str) {
            self.forgotten.lock().unwrap().push(id.to_owned());
        }
    }

    impl FakeHost {
        /// The oldest start is up as `pid`.
        pub(in crate::native) fn up(&self, pid: u32) {
            let (_, _, _, then) = self.starts.lock().unwrap().remove(0);
            then(Ok(Started { pid, epoch: 1 }));
        }
        /// The oldest start failed.
        pub(in crate::native) fn down(&self, why: &str) {
            let (_, _, _, then) = self.starts.lock().unwrap().remove(0);
            then(Err(why.to_owned()));
        }
        /// What the latest start was asked for.
        pub(in crate::native) fn last_start(&self) -> (SpawnSpec, Input) {
            let starts = self.starts.lock().unwrap();
            let (_, spec, input, _) = starts.last().expect("a start was asked for");
            (spec.clone(), input.clone())
        }
        pub(in crate::native) fn signalled(&self) -> Vec<String> {
            self.signals
                .lock()
                .unwrap()
                .iter()
                .map(|(id, _)| id.clone())
                .collect()
        }
        pub(in crate::native) fn signals(&self) -> Vec<(String, Sig)> {
            self.signals.lock().unwrap().clone()
        }
    }

    pub(in crate::native) struct Fed {
        pub(in crate::native) native: std::sync::Arc<Native>,
        pub(in crate::native) registry: std::sync::Arc<crate::registry::SessionRegistry>,
        pub(in crate::native) host: std::sync::Arc<FakeHost>,
        pub(in crate::native) link: std::sync::Arc<crate::applink::AppLink>,
    }

    /// vornd holding the server's records: two agents in one worktree, one
    /// of them idle, and a shell.
    pub(in crate::native) fn fed() -> Fed {
        let native = Native::new();
        let registry = crate::registry::SessionRegistry::new();
        native.set_registry(std::sync::Arc::clone(&registry));
        registry.decide_statuses();
        let agent = |id: &str, status: &str| {
            json!({
                "id": id, "agentType": "claude", "projectName": "p", "projectPath": "/p",
                "status": status, "createdAt": 1, "pid": 9, "worktreePath": "/w",
            })
        };
        let shell = json!({
            "id": "sh", "agentType": "shell", "projectName": "p", "projectPath": "/p",
            "status": "running", "createdAt": 1, "pid": 9,
        });
        registry
            .feed(
                1,
                &json!({
                    "op": "snapshot", "headless": [], "order": ["a", "b", "sh"],
                    "terminals": [agent("a", "running"), agent("b", "idle"), shell],
                }),
            )
            .unwrap();
        let host = std::sync::Arc::new(FakeHost::default());
        native.set_host(std::sync::Arc::clone(&host) as std::sync::Arc<dyn Host>);
        let link = std::sync::Arc::new(crate::applink::AppLink::default());
        link.set_creates_terminals();
        native.set_link(std::sync::Arc::clone(&link));
        Fed {
            native,
            registry,
            host,
            link,
        }
    }

    fn ids(fed: &Fed) -> Vec<String> {
        fed.registry
            .read(|r| r.terminals().iter().map(|t| t.id.clone()).collect())
            .unwrap()
    }

    #[test]
    fn closes_a_terminal_hangs_it_up_and_offers_the_worktree_once_none_is_at_work() {
        let fed = fed();
        let (mut asks, _listening) = fed.link.listen();
        assert_eq!(
            call(&fed.native, "terminal:kill", &json!("a")),
            Answer::Void
        );
        assert_eq!(ids(&fed), ["b", "sh"]);
        assert_eq!(fed.host.signalled(), ["a"]);
        // `b` is idle, so nothing is at work in the worktree any more.
        let offer = asks.try_recv().unwrap();
        assert_eq!(offer["method"], "vornd:cleanupOffer");
        assert_eq!(
            offer["params"],
            json!({ "id": "a", "projectPath": "/p", "worktreePath": "/w" })
        );
        assert!(asks.try_recv().is_err());
        // One the registry does not hold is the server's: it may be one
        // carried over from a previous run.
        assert_eq!(
            call(&fed.native, "terminal:kill", &json!("a")),
            Answer::Forward
        );
        assert_eq!(
            call(&fed.native, "terminal:kill", &json!(5)),
            Answer::Forward
        );
    }

    #[test]
    fn a_terminal_closed_while_it_starts_is_hung_up_once_it_is_up() {
        let fed = fed();
        let record = skeleton("n", "shell", "p", "/p");
        let answer = register(
            &fed.native,
            record,
            "/p",
            vec!["sh".into()],
            Vec::new(),
            Input::None,
            Made::Created,
        );
        assert!(matches!(answer, Answer::Result(ref r) if r["id"] == "n" && r["pid"] == 0));
        assert_eq!(ids(&fed), ["a", "b", "sh", "n"]);
        assert_eq!(
            call(&fed.native, "terminal:kill", &json!("n")),
            Answer::Void
        );
        assert!(fed.host.signalled().is_empty());
        fed.host.up(77);
        assert_eq!(fed.host.signalled(), ["n"]);

        // One whose record the server let go of while it started: nothing
        // holds the program, so it is hung up too.
        let record = skeleton("m", "shell", "p", "/p");
        register(
            &fed.native,
            record,
            "/p",
            vec!["sh".into()],
            Vec::new(),
            Input::None,
            Made::Created,
        );
        fed.registry
            .feed(1, &json!({ "op": "remove", "kind": "terminal", "id": "m" }))
            .unwrap();
        fed.host.up(78);
        assert_eq!(fed.host.signalled(), ["n", "m"]);
    }

    #[test]
    fn creates_nothing_unless_vornd_creates_terminals() {
        let fed = fed();
        let link = std::sync::Arc::new(crate::applink::AppLink::default());
        let native = Native::new();
        native.set_registry(std::sync::Arc::clone(&fed.registry));
        native.set_host(std::sync::Arc::clone(&fed.host) as std::sync::Arc<dyn Host>);
        native.set_link(link);
        // The shell group alone native: the server would not follow it.
        assert_eq!(call(&native, "shell:create", &Value::Null), Answer::Forward);
        assert!(fed.host.starts.lock().unwrap().is_empty());
    }

    #[test]
    fn renames_regroups_and_reorders_in_the_registry() {
        let fed = fed();
        let rename = json!({ "id": "a", "displayName": "mine" });
        assert_eq!(call(&fed.native, "terminal:rename", &rename), Answer::Void);
        let group = json!({ "id": "a", "groupId": "g" });
        assert_eq!(call(&fed.native, "terminal:setGroup", &group), Answer::Void);
        let a = fed
            .registry
            .read(|r| r.terminal("a").unwrap().0.clone())
            .unwrap();
        assert_eq!(
            (
                a.display_name.as_deref(),
                a.renamed_by_person,
                a.group_id.as_deref()
            ),
            (Some("mine"), Some(true), Some("g"))
        );
        // An empty group takes it out of its group, as the server reads it.
        let ungroup = json!({ "id": "a", "groupId": "" });
        assert_eq!(
            call(&fed.native, "terminal:setGroup", &ungroup),
            Answer::Void
        );
        assert_eq!(
            fed.registry
                .read(|r| r.terminal("a").unwrap().0.group_id.clone()),
            Some(None)
        );
        assert_eq!(
            call(&fed.native, "terminal:reorder", &json!(["sh", "b", "a"])),
            Answer::Void
        );
        assert_eq!(ids(&fed), ["sh", "b", "a"]);
        // What the server refuses, it refuses in its own words.
        for (method, params) in [
            ("terminal:reorder", json!(["a", "a"])),
            ("terminal:rename", json!({ "id": "x", "displayName": "n" })),
            ("terminal:setGroup", json!({ "id": "a", "groupId": 3 })),
        ] {
            assert_eq!(
                call(&fed.native, method, &params),
                Answer::Forward,
                "{method}"
            );
        }
    }

    #[test]
    fn refuses_a_new_agent_while_the_server_winds_down() {
        let fed = fed();
        fed.link.set_closing(crate::applink::Closing::Draining);
        let req = CreateRequest::read(&json!({
            "agentType": "claude", "projectName": "p", "projectPath": project(),
        }))
        .unwrap();
        assert_eq!(
            create(&fed.native, &req),
            Answer::Error(crate::applink::DRAINING_MESSAGE.to_owned())
        );
        fed.link.set_closing(crate::applink::Closing::HandingOver);
        assert_eq!(
            create(&fed.native, &req),
            Answer::Error(crate::applink::HANDOVER_MESSAGE.to_owned())
        );
        // A conversation already running is shown, closing or not.
        let mut fields = Map::new();
        fields.insert("agentSessionId".into(), json!("conv"));
        fed.registry
            .change(|r| ((), r.set_fields("a", fields).unwrap().into_iter().collect()));
        let named = CreateRequest::read(&json!({
            "agentType": "claude", "projectName": "p", "projectPath": project(),
            "resumeSessionId": "conv",
        }))
        .unwrap();
        assert!(matches!(create(&fed.native, &named), Answer::Result(r) if r["id"] == "a"));
    }

    #[test]
    fn compares_a_plan_without_what_differs_between_starts() {
        let reply = json!({
            "id": "a", "agentType": "claude", "projectName": "p", "projectPath": "/p",
            "status": "running", "createdAt": 5, "cols": 80, "rows": 24, "pid": 0,
            "agentSessionId": "minted",
        });
        let spawn = json!({
            "argv": ["/bin/sh", "-l"], "cwd": "/p",
            "env": { "TERM": "x", "PATH": "/bin", "VORN_SESSION_ID": "a" },
        });
        let mut record = skeleton("b", "claude", "p", "/p");
        record.agent_session_id = Some("other".into());
        let ours = plan_json(
            vec!["/bin/sh".into(), "-l".into()],
            "/p",
            vec![("PATH".into(), "/bin".into())],
            &record,
        );
        if !cfg!(windows) {
            assert_eq!(ours, spawn_plan(&spawn, &reply));
        }
        assert_eq!(plan_record(&reply)["agentSessionId"], true);
        assert!(plan_record(&reply).get("pid").is_none());
        // A minted id reads the same wherever the launch names it.
        let mut headless = reply.clone();
        headless["launchCommand"] = json!("claude --session-id minted -p");
        let spawn = json!({ "argv": ["claude", "--session-id", "minted", "-p"], "cwd": "/p" });
        let planned = spawn_plan(&spawn, &headless);
        assert_eq!(planned["argv"][2], "<agentSessionId>");
        assert_eq!(
            planned["record"]["launchCommand"],
            "claude --session-id <agentSessionId> -p"
        );
        let mut linked = reply.clone();
        linked["hookSessionId"] = json!("copilot-hook");
        assert_eq!(plan_record(&linked), plan_record(&reply));
    }

    #[test]
    fn foresees_what_the_server_answers_a_change_without_making_it() {
        let fed = fed();
        let foreseen = |method: &str, params: Value| foresee(&fed.native, method, &params);
        assert_eq!(foreseen("terminal:kill", json!("a")), Some(Answer::Void));
        assert_eq!(
            foreseen("terminal:rename", json!({ "id": "a", "displayName": "n" })),
            Some(Answer::Void)
        );
        assert_eq!(
            foreseen("terminal:rename", json!({ "id": "x", "displayName": "n" })),
            Some(Answer::Error("Session not found: x".into()))
        );
        assert_eq!(
            foreseen("terminal:setGroup", json!({ "id": "b", "groupId": null })),
            Some(Answer::Void)
        );
        assert_eq!(
            foreseen("terminal:reorder", json!(["sh", "b", "a"])),
            Some(Answer::Void)
        );
        assert_eq!(
            foreseen("terminal:reorder", json!(["a", "a"])),
            Some(Answer::Error("Duplicate session IDs".into()))
        );
        assert_eq!(
            foreseen("terminal:reorder", json!(["a", "x"])),
            Some(Answer::Error("Session not found: x".into()))
        );
        // The server's own: a terminal the copy does not hold, a shape its
        // handler does not read.
        assert_eq!(foreseen("terminal:kill", json!("x")), None);
        assert_eq!(
            foreseen("terminal:setGroup", json!({ "id": "a", "groupId": 3 })),
            None
        );
        // Nothing moved.
        assert_eq!(ids(&fed), ["a", "b", "sh"]);
        assert!(fed.host.signalled().is_empty());
        assert_eq!(
            fed.registry
                .read(|r| r.terminal("a").unwrap().0.display_name.clone()),
            Some(None)
        );
    }

    /// vornd owning the records between runs, with a project directory
    /// that is there, so a resume has somewhere to start.
    fn owning() -> (Fed, tempfile::TempDir) {
        let fed = fed();
        fed.link.set_creates_headless();
        fed.native.set_database(
            std::env::temp_dir()
                .join("vornd-no-such-db")
                .join("vorn.db"),
        );
        fed.registry.own_records();
        (fed, tempfile::tempdir().unwrap())
    }

    fn carried_agent(id: &str, dir: &Path, extra: Value) -> TerminalSession {
        let mut v = json!({
            "id": id, "agentType": "claude", "projectName": "proj", "projectPath": dir,
            "status": "idle", "createdAt": 1, "pid": 9, "groupId": "g", "savedAt": 5,
        });
        if let (Value::Object(v), Value::Object(extra)) = (&mut v, extra) {
            v.extend(extra);
        }
        serde_json::from_value(v).unwrap()
    }

    fn offer(fed: &Fed, records: Vec<TerminalSession>) {
        fed.registry.carry(
            crate::registry::Carried {
                terminals: records,
                ..Default::default()
            },
            10,
        );
    }

    #[test]
    fn resumes_an_offered_agent_on_its_conversation_under_its_id_once() {
        let (fed, dir) = owning();
        offer(
            &fed,
            vec![carried_agent(
                "r",
                dir.path(),
                json!({ "agentSessionId": "conv-r" }),
            )],
        );
        let listed = call(&fed.native, "sessions:restored", &Value::Null);
        let Answer::Result(Value::Array(offered)) = listed else {
            panic!("not listed: {listed:?}");
        };
        assert_eq!(offered[0]["session"]["id"], "r");

        let answer = call(&fed.native, "sessions:resume", &json!({ "id": "r" }));
        let Answer::Result(answer) = answer else {
            panic!("not resumed: {answer:?}");
        };
        assert_eq!(answer["ok"], true);
        let session = &answer["session"];
        assert_eq!(
            (session["id"].clone(), session["agentType"].clone()),
            (json!("r"), json!("claude"))
        );
        assert_eq!(
            (
                session["agentSessionId"].clone(),
                session["groupId"].clone()
            ),
            (json!("conv-r"), json!("g"))
        );
        assert_eq!(session["status"], "running");
        let (spec, input) = fed.host.last_start();
        assert_eq!(spec.cwd, dir.path().to_str().unwrap());
        let Input::Typed(line) = input else {
            panic!("not typed: {input:?}");
        };
        assert!(String::from_utf8_lossy(&line).contains("conv-r"));
        // Live again in the registry, last in the order; no longer offered.
        assert_eq!(ids(&fed), ["a", "b", "sh", "r"]);
        assert!(fed.registry.restored().unwrap().is_empty());
        fed.host.up(42);
        assert_eq!(
            fed.registry
                .read(|r| r.terminal("r").unwrap().0.pid)
                .unwrap(),
            42
        );
        // The second client to ask is told it is gone.
        assert_eq!(
            call(&fed.native, "sessions:resume", &json!({ "id": "r" })),
            Answer::Result(json!({ "ok": false, "reason": "gone" }))
        );
        // Nothing while vornd does not own the records.
        let other = self::fed();
        assert_eq!(
            call(&other.native, "sessions:resume", &json!({ "id": "r" })),
            Answer::Forward
        );
        assert_eq!(
            call(&other.native, "sessions:restored", &Value::Null),
            Answer::Forward
        );
    }

    #[test]
    fn a_resume_whose_workspace_is_gone_keeps_the_offer() {
        let (fed, _dir) = owning();
        let gone = std::env::temp_dir().join("vornd-no-such-project");
        offer(&fed, vec![carried_agent("g", &gone, json!({}))]);
        let answer = call(&fed.native, "sessions:resume", &json!({ "id": "g" }));
        let Answer::Result(answer) = answer else {
            panic!("{answer:?}");
        };
        assert_eq!(
            (answer["ok"].clone(), answer["reason"].clone()),
            (json!(false), json!("workspace-gone"))
        );
        assert!(answer["message"].as_str().unwrap().ends_with("is gone"));
        assert_eq!(fed.registry.restored().unwrap().len(), 1);
    }

    #[test]
    fn hands_back_the_terminal_already_writing_the_conversation() {
        let (fed, dir) = owning();
        let mut fields = Map::new();
        fields.insert("agentSessionId".into(), json!("conv"));
        fed.registry
            .change(|r| ((), r.set_fields("a", fields).unwrap().into_iter().collect()));
        offer(
            &fed,
            vec![carried_agent(
                "o",
                dir.path(),
                json!({ "agentSessionId": "conv" }),
            )],
        );
        let answer = call(&fed.native, "sessions:resume", &json!({ "id": "o" }));
        let Answer::Result(answer) = answer else {
            panic!("{answer:?}");
        };
        assert_eq!(
            (answer["ok"].clone(), answer["boundTo"].clone()),
            (json!(true), json!("a"))
        );
        assert_eq!(answer["session"]["id"], "a");
        assert!(fed.host.starts.lock().unwrap().is_empty());
        // An ended terminal on the same conversation is let go of quietly.
        let mut ended = json!({
            "id": "b", "agentType": "claude", "projectName": "p", "projectPath": "/p",
            "status": "idle", "createdAt": 1, "pid": 9, "agentSessionId": "conv",
        });
        ended["worktreePath"] = json!("/w");
        fed.registry
            .feed(
                1,
                &json!({ "op": "upsert", "kind": "terminal", "record": ended, "ended": true }),
            )
            .unwrap();
        // The conversation it names is the registry's to set, not an upsert's.
        let mut fields = Map::new();
        fields.insert("agentSessionId".into(), json!("conv"));
        fed.registry
            .change(|r| ((), r.set_fields("b", fields).unwrap().into_iter().collect()));
        let answer = call(&fed.native, "sessions:resume", &json!({ "id": "b" }));
        let Answer::Result(answer) = answer else {
            panic!("{answer:?}");
        };
        assert_eq!(answer["boundTo"], "a", "{answer}");
        assert_eq!(ids(&fed), ["a", "sh"]);
    }

    #[test]
    fn closes_an_offered_session_without_a_program_and_declines_them_all() {
        let (fed, dir) = owning();
        offer(
            &fed,
            vec![
                carried_agent("x", dir.path(), json!({})),
                carried_agent("y", dir.path(), json!({})),
            ],
        );
        assert_eq!(
            call(&fed.native, "terminal:kill", &json!("x")),
            Answer::Void
        );
        assert!(fed.host.signalled().is_empty());
        assert_eq!(fed.host.forgotten.lock().unwrap().as_slice(), ["x"]);
        assert_eq!(fed.registry.restored().unwrap().len(), 1);
        assert_eq!(
            call(&fed.native, "sessions:clear", &Value::Null),
            Answer::Void
        );
        assert!(fed.registry.restored().unwrap().is_empty());
        assert_eq!(fed.host.forgotten.lock().unwrap().as_slice(), ["x", "y"]);
        // In shadow, a close of an offered session is foreseen as nothing.
        let other = self::fed();
        other.registry.feed(1, &json!({ "op": "restored", "restored": [{ "session": carried_agent("q", dir.path(), json!({})), "endedAt": 1 }] })).unwrap();
        assert_eq!(
            foresee(&other.native, "terminal:kill", &json!("q")),
            Some(Answer::Void)
        );
        assert_eq!(
            foresee(&other.native, "terminal:kill", &json!("nope")),
            None
        );
        assert_eq!(
            foresee(&other.native, "sessions:clear", &Value::Null),
            Some(Answer::Void)
        );
    }

    #[test]
    fn plans_a_resume_as_the_start_it_would_make_without_taking_the_offer() {
        let (fed, dir) = owning();
        offer(
            &fed,
            vec![carried_agent(
                "p",
                dir.path(),
                json!({ "agentSessionId": "conv-p" }),
            )],
        );
        let planned = plan(&fed.native, "sessions:resume", &json!({ "id": "p" }), 0).unwrap();
        assert_eq!(planned["cwd"], dir.path().to_str().unwrap());
        assert_eq!(planned["record"]["agentType"], "claude");
        assert_eq!(planned["record"]["groupId"], "g");
        assert_eq!(planned["record"]["agentSessionId"], true);
        assert!(planned["argv"].as_array().unwrap().len() >= 2);
        assert_eq!(fed.registry.restored().unwrap().len(), 1);
        assert!(plan(&fed.native, "sessions:resume", &json!({ "id": "nope" }), 0).is_none());
    }

    #[test]
    fn types_the_launch_line_once_the_shell_has_drawn_its_prompt() {
        let asked = tokio::time::Instant::now();
        // The prompt came quickly: no sooner than the shortest wait.
        assert_eq!(
            type_at(asked, Some(asked + Duration::from_millis(50))),
            asked + TYPE_AFTER
        );
        // It came late: a settle after it.
        let late = asked + Duration::from_millis(800);
        assert_eq!(type_at(asked, Some(late)), late + TYPE_SETTLE);
        // It never came: typed anyway, at the longest wait.
        assert_eq!(type_at(asked, None), asked + TYPE_AT_MOST);
    }
}
