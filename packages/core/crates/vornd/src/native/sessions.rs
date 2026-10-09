//! The terminal calls vornd answers itself:
//! `terminal:create`, `kill`, `rename`, `setGroup` and `reorder`, and
//! `shell:create`.
//!
//! Each answers as the server's handler did, from the session registry
//! vornd keeps ([`crate::registry`]), which then holds the record vornd made
//! or changed. Clients are told from the registry's notes
//! ([`super::session_events`]).
//!
//! A create prepares the session's workspace as the server's
//! `prepareSession` does: the worktree it names, or one it makes, or the
//! branch it checks out, under the repository's turn ([`super::Turns`]),
//! holding each directory involved until the record is in, so the server's
//! worktree calls see it as in use meanwhile. An agent that can be told
//! which conversation to start is given an id; one that names a
//! conversation already running is answered with the session running it,
//! and creates naming one conversation while one prepares share its answer
//! ([`crate::claims`]).
//!
//! While vornd owns the records between runs ([`crate::registry::SessionRegistry::owns`])
//! it also answers `sessions:restored`, `sessions:resume` and
//! `sessions:clear`, and a `terminal:kill` of a session of an earlier run,
//! as the server's handlers do: a resume takes the offered session once,
//! hands back the terminal already writing its conversation if one is, or
//! starts a shell where the session was, or the agent on the conversation
//! it had, under the same id ([`resume`]). The launch line is typed once
//! the shell has drawn its prompt ([`type_at`]).
//!
//! A call whose params are not the shape the server's handler read is
//! refused ([`super::bad_params`]), a terminal no record names in the
//! server's words, and a start before the records are read
//! ([`super::not_ready`]) or while the session holder is not connected
//! ([`NO_HOLDER`]). A start asked for as vornd starts waits for the holder
//! first ([`super::Native::await_holder`]).

use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};
use tracing::{debug, warn};
use vorn_agents::history::{recent_sessions_for, Homes, ProjectScope, RecentSession};
use vorn_agents::launch::shell as launch_shell;
use vorn_agents::launch::ssh as login;
use vorn_agents::launch::{
    display_name_from_prompt, launch_line, LaunchRequest, Machine, Platform, Quoting,
};
use vorn_agents::{paths, Agent};
use vorn_git::repo::{extract_worktree_name, node_basename, Git};
use vorn_sessiond_wire::{Io, Sig, SpawnSpec};

use super::ssh::{KeyFile, Remote, Secret};
use super::{agent, bad_params, not_ready, shell, Answer, Native};
use crate::claims::{Claims, OnePerKey};
use crate::registry::{AgentStatus, HeadlessStatus, Registry, Restored, TerminalSession};

/// How many of an agent's past sessions a resume looks through for the
/// conversation to continue (`getRecentSessionsFor`).
const RECENT_LIMIT: usize = 20;

/// The size a terminal starts at, before any client has fitted it.
pub const INITIAL_COLS: u16 = 80;
pub const INITIAL_ROWS: u16 = 24;

/// The terminal type programs are told they run in, off Windows.
pub(super) const PTY_TERM: &str = "xterm-256color";

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

    /// As [`Host::start`], handing what the program prints and how it ended
    /// to `watch`. A host that cannot watch drops it, which reads as lost.
    fn start_watched(&self, spec: SpawnSpec, name: String, input: Input, watch: Watch, then: Then) {
        drop(watch);
        self.start(spec, name, input, then);
    }

    /// Sends `sig` to session `id`'s program.
    fn signal(&self, id: &str, sig: Sig);

    /// Where session `id`'s output has got to, which a status set now is stamped with.
    fn head_stamp(&self, id: &str) -> Option<crate::registry::Stamp> {
        let _ = id;
        None
    }

    /// Sends `sig` to session `id`'s program `after` a while, if it still runs.
    fn signal_after(&self, id: &str, sig: Sig, after: Duration);

    /// Answers attaches for `id` although nothing runs under it: a session
    /// carried from the last run ([`crate::streams::Streams::expect`]).
    fn expect(&self, id: &str) {
        let _ = id;
    }

    /// `id` is being started: an attach meanwhile waits for it, and what is
    /// typed is kept for it ([`crate::streams::Streams::expect_start`]).
    fn expect_start(&self, id: &str) {
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
    /// A login to a remote host, typed into the shell as its output asks.
    Remote(Box<Remote>),
}

/// What a start's outcome is handed to.
pub type Then = Box<dyn FnOnce(Result<Started, String>) + Send>;

/// Where a watched program's output and exit code go ([`Host::start_watched`]).
#[derive(Debug)]
pub struct Watch {
    pub output: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    pub ended: tokio::sync::oneshot::Sender<i64>,
}

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

/// The conversations being started.
fn claims(native: &Native) -> Option<&Claims> {
    native.link.get().map(|l| l.claims())
}

/// Whether vornd owns the records between runs, and so answers the calls
/// about the sessions of earlier runs.
fn restores(native: &Native) -> bool {
    native.registry.get().is_some_and(|r| r.owns())
}

/// The error while the session holder is not connected.
pub(super) const NO_HOLDER: &str = "Terminals cannot start: the session holder is not connected";

/// Whether `method` starts a session, and so waits for the holder first.
pub fn starts(method: &str) -> bool {
    matches!(
        method,
        "terminal:create" | "shell:create" | "sessions:resume" | "headless:create"
    )
}

/// Answers `method` with `params`.
pub fn call(native: &Native, method: &str, params: &Value) -> Answer {
    match method {
        "terminal:create" => match CreateRequest::read(params) {
            Some(req) => create(native, &req),
            None => bad_params(method),
        },
        "sessions:restored" => restored(native),
        "sessions:resume" => match params.get("id").and_then(Value::as_str) {
            Some(id) => resume(native, id),
            None => bad_params(method),
        },
        "sessions:clear" => clear(native),
        "shell:create" => match params {
            Value::Null => shell_create(native, None),
            Value::String(cwd) if cwd.is_empty() => shell_create(native, None),
            Value::String(cwd) if Path::new(cwd).is_absolute() => shell_create(native, Some(cwd)),
            _ => bad_params(method),
        },
        _ => match asked(method, params) {
            Some(Asked::Kill(id)) => kill(native, &id),
            Some(Asked::Fields(id, fields)) => set_fields(native, &id, fields),
            Some(Asked::Order(ids)) => reorder(native, ids),
            None => bad_params(method),
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

/// `sessions:restored`: the sessions of earlier runs still offered.
fn restored(native: &Native) -> Answer {
    if !restores(native) {
        return not_ready();
    }
    match native.registry.get().and_then(|r| r.restored()) {
        Some(list) => Answer::Result(Value::Array(list)),
        None => not_ready(),
    }
}

/// `sessions:clear`: every offered session declined at once.
fn clear(native: &Native) -> Answer {
    if !restores(native) {
        return not_ready();
    }
    let Some(registry) = native.registry.get() else {
        return not_ready();
    };
    let declined = registry.change(|r| {
        let (all, note) = r.consume_all_restored();
        (all, vec![note])
    });
    let Some(declined) = declined else {
        return not_ready();
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
    if !restores(native) {
        return not_ready();
    }
    let Some(registry) = native.registry.get() else {
        return not_ready();
    };
    if !fed_and_held(native) {
        return Answer::Error(NO_HOLDER.into());
    }
    // Taken first: the second of two clients on one cold pane is told it is gone.
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
        return not_ready();
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
/// (`buildRestorePayload`), on `transcript` when it has one. `None` for an
/// agent this build does not know.
fn restore_request(
    previous: &TerminalSession,
    transcript: Option<String>,
) -> Option<CreateRequest> {
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
        remote_host_id: previous.remote_host_id.clone(),
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
    // A remote session's directories are on its host: it reconnects whatever this machine has.
    let remote = given(previous.remote_host_id.as_deref()).is_some();
    let previous = if remote {
        previous.clone()
    } else {
        let Some(cwd) = resume_cwd_for(previous) else {
            return workspace_gone(previous);
        };
        grounded(previous, &cwd)
    };
    let Some(claims) = claims(native) else {
        return not_ready();
    };
    // Scope read before the claim; claims do not lapse while the workspace is prepared.
    let scope = (!remote)
        .then(|| transcript_scope(native, &previous))
        .flatten();
    claims.preparing(id);
    let now = Instant::now();
    let free = if remote {
        pinned_free(native, &previous)
    } else {
        free_transcript_for(native, &previous, scope.as_ref())
    };
    let transcript = free.filter(|t| claims.claim(t, id, now).is_none());
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
        ssh: None,
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

/// The conversation a remote session's record names, when nothing holds it: this machine has no history of the host's.
fn pinned_free(native: &Native, session: &TerminalSession) -> Option<String> {
    let agent = Agent::from_id(&session.agent_type)?;
    let pinned = given(session.agent_session_id.as_deref()).filter(|_| agent.resumes_exactly())?;
    (!held_transcripts(native).iter().any(|h| h == pinned)).then(|| pinned.to_owned())
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
        ssh: None,
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
    /// The remote host the session runs on, over ssh.
    pub remote_host_id: Option<String>,
    /// Read by a headless create only: the workflow that asked for it.
    pub workflow_id: Option<String>,
    pub workflow_name: Option<String>,
}

impl CreateRequest {
    /// Reads `params` as the server's handler does. `None` for a call that
    /// is the server's: an agent this build does not know,
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
        let remote_host_id = text("remoteHostId")?;
        // A remote project is a path on the ssh host, POSIX whatever this host is.
        let absolute = match given(remote_host_id.as_deref()) {
            Some(_) => project_path.starts_with('/'),
            None => Path::new(&project_path).is_absolute(),
        };
        if !absolute {
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
            remote_host_id,
            workflow_id: text("workflowId")?,
            workflow_name: text("workflowName")?,
        })
    }

    /// The remote host the session runs on, when it names one.
    pub fn remote(&self) -> Option<&str> {
        given(self.remote_host_id.as_deref())
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

/// Whether the registry holds the server's records and decides, and the
/// session holder is connected.
pub(super) fn fed_and_held(native: &Native) -> bool {
    let fed = native
        .registry
        .get()
        .is_some_and(|r| r.read(|r| r.decides()) == Some(true));
    fed && native.host.get().is_some_and(|h| h.ready())
}

/// Why vornd cannot start a session now: the records are not read yet, or no holder.
fn unstartable(native: &Native) -> Option<Answer> {
    unread(native).or_else(|| unheld(native))
}

/// The error while the records are not read yet, if they are not.
fn unread(native: &Native) -> Option<Answer> {
    let decides = native
        .registry
        .get()
        .is_some_and(|r| r.read(|r| r.decides()) == Some(true));
    (!decides).then(not_ready)
}

/// The error while the session holder is not connected, if it is not.
fn unheld(native: &Native) -> Option<Answer> {
    (!native.host.get().is_some_and(|h| h.ready())).then(|| Answer::Error(NO_HOLDER.into()))
}

/// Tells clients a terminal with no program running has ended (`killPty`).
fn exited(native: &Native, id: &str) {
    native.broadcast_to(
        "terminal:exit",
        json!({ "id": id, "exitCode": 0 }),
        Some(id),
    );
}

/// `terminal:create` for a local agent.
fn create(native: &Native, req: &CreateRequest) -> Answer {
    if let Some(answer) = unread(native) {
        return answer;
    }
    // A host the settings do not name starts the session here, as the server started it.
    if let Some(h) = req.remote() {
        match remote_host(native, h) {
            Ok(Some(_)) => {}
            Ok(None) => {
                let local = CreateRequest {
                    remote_host_id: None,
                    ..req.clone()
                };
                return create(native, &local);
            }
            Err(answer) => return answer,
        }
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
        return not_ready();
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
    if let Some(answer) = unheld(native) {
        return answer;
    }
    let Some(settings) = agent::settings(native) else {
        return agent::no_settings();
    };
    let Some(config) = agent::command_of(&settings, req.agent) else {
        return agent::unreadable_command(req.agent);
    };
    if req.remote().is_some() {
        return match prepare_remote(native, req, &config, &settings, id, group_id) {
            Ok(start) => register(
                native,
                start.record,
                &start.cwd,
                start.argv,
                start.env,
                Input::Remote(Box::new(start.remote)),
                made,
            ),
            Err(answer) => answer,
        };
    }
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

/// Remote host `id` as the server keeps it, or `None` when vornd cannot read it.
fn remote_host(native: &Native, id: &str) -> Result<Option<vorn_protocol::RemoteHost>, Answer> {
    let db = native.db.get().ok_or_else(super::no_database)?;
    vorn_store::remote_host(db, id).map_err(|err| {
        debug!(%err, "could not read the remote host");
        Answer::Error("vornd could not read the remote host".to_owned())
    })
}

/// A terminal on a remote host, worked out: the local shell it starts in, and the login typed into it.
#[derive(Debug)]
struct RemoteStart {
    argv: Vec<String>,
    cwd: String,
    env: Vec<(String, String)>,
    record: TerminalSession,
    remote: Remote,
}

/// What `createRemotePty` starts for `req`: a login shell in the home directory, with the safe environment.
fn prepare_remote(
    native: &Native,
    req: &CreateRequest,
    config: &vorn_agents::AgentCommand,
    settings: &vorn_store::AgentSettings,
    id: &str,
    group_id: Option<String>,
) -> Result<RemoteStart, Answer> {
    let host_id = req.remote().unwrap_or_default();
    let host = remote_host(native, host_id)?.ok_or_else(not_ready)?;
    let env = native.env.get();
    let launch = LaunchRequest {
        agent: req.agent,
        args: req.args.clone(),
        model: req.model.clone(),
        remote_host_id: Some(host_id.to_owned()),
        resume_session_id: req.resume_session_id.clone(),
        session_id: req.session_id.clone(),
        initial_prompt: req.initial_prompt.clone(),
    };
    let default_shell = launch_shell::default_shell(None, Platform::HOST, var);
    let machine = Machine {
        platform: Platform::HOST,
        quoting: Quoting::local(Platform::HOST, &default_shell),
    };
    let launch_line = launch_line(&launch, Some(config), &env, &machine)
        .map_err(|e| Answer::Error(e.to_string()))?;
    let method = host.auth_method.as_ref().map(|m| m.0.as_str());
    // From the vault, for this one login: never in a create's params.
    let vaulted = |kind, id: &str| {
        native
            .secrets
            .item(kind, id)
            .map(|s| Secret::new(s.expose().to_owned()))
    };
    let stored_key = host
        .credential_id
        .as_deref()
        .filter(|c| method == Some("key-stored") && !c.is_empty())
        .and_then(|c| vaulted(vorn_vault::Kind::SshKey, c));
    let key = match (method, stored_key) {
        (Some("key-stored"), Some(content)) => Some(KeyFile::new(content)),
        (Some("key-stored"), None) => {
            warn!(host = %host.label, "key-stored auth selected but the vault has no key for it; falling back to agent");
            None
        }
        _ => None,
    };
    let key_path = key.as_ref().map(|k| k.path.to_string_lossy().into_owned());
    let target = login::Target {
        hostname: &host.hostname,
        user: &host.user,
        port: host.port,
        auth: login::Auth::from_stored(method, host.ssh_key_path.as_deref(), key_path.as_deref()),
        options: host.ssh_options.as_deref().filter(|o| !o.is_empty()),
    };
    let marker = login::marker(id);
    let remote = Remote {
        line: login::ssh_line(&target, &marker, Platform::HOST),
        command: login::remote_command(&req.project_path, &launch_line),
        marker,
        password: (target.auth == login::Auth::Password)
            .then(|| vaulted(vorn_vault::Kind::HostPassword, host_id))
            .flatten(),
        key,
    };
    let shell = launch_shell::default_shell(settings.shell.as_deref(), Platform::HOST, var);
    let mut argv = vec![shell];
    argv.extend(
        launch_shell::default_shell_args(Platform::HOST)
            .iter()
            .map(|a| (*a).to_owned()),
    );
    let record = TerminalSession {
        display_name: given(req.display_name.as_deref())
            .map(str::to_owned)
            .or_else(|| {
                given(req.initial_prompt.as_deref())
                    .and_then(|p| display_name_from_prompt(p, PROMPT_NAME_LEN))
            }),
        remote_host_id: Some(host.id.clone()),
        remote_host_label: Some(host.label.clone()),
        agent_session_id: req.named().map(str::to_owned),
        group_id,
        ..skeleton(id, req.agent.id(), &req.project_name, &req.project_path)
    };
    Ok(RemoteStart {
        argv,
        cwd: shell::home_dir(),
        env,
        record,
        remote,
    })
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
        ssh: None,
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
        ssh: None,
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
    if let Some(answer) = unstartable(native) {
        return answer;
    }
    let settings = agent::settings(native).unwrap_or_default();
    let (shell, setup) = match native.shells.setup(
        &native.env,
        settings.shell.as_deref(),
        settings.minimal_shell_prompt,
    ) {
        Ok(ready) => ready,
        Err(e) => return Answer::Error(e.to_string()),
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
        return not_ready();
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
        return not_ready();
    };
    let id = record.id.clone();
    // Set at the spawn and never in vornd's own environment, so no other
    // child inherits one session's id.
    set(&mut env, "VORN_SESSION_ID", id.clone());
    if !cfg!(windows) {
        set(&mut env, "TERM", PTY_TERM.to_owned());
    }
    let answer = record_json(&record);
    let mut created = json!({
        "agentType": record.agent_type,
        "projectName": record.project_name,
        "projectPath": record.project_path,
    });
    if let Some(branch) = record.branch.as_ref().filter(|b| !b.is_empty()) {
        created["branch"] = json!(branch);
    }
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
        Some(Ok(())) => super::tasks::log_event(native, &id, "created", Some(created)),
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
    // A client attaches as soon as it has the answer, and may type before the program is up.
    host.expect_start(&id);
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
                    host_after.forget(&id);
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
        return not_ready();
    };
    // An offered session has no program to hang up: closing it takes the offer once.
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
    let Some(closed) = closed else {
        return not_ready();
    };
    let Some((live, offer)) = closed else {
        exited(native, id);
        return Answer::Void;
    };
    if let Some(claims) = claims(native) {
        claims.release_for(id);
    }
    if let Some(offer) = offer {
        native.broadcast_to("worktree:confirmCleanup", offer, Some(id));
    }
    if !live {
        // No program to hang up: clients are told it ended, so a closing card finishes.
        exited(native, id);
    } else if !native.sessions.doom(id, Sig::Hup) {
        if let Some(host) = native.host.get() {
            host.signal(id, Sig::Hup);
        }
    }
    Answer::Void
}

/// `terminal:rename` and `terminal:setGroup`.
fn set_fields(native: &Native, id: &str, fields: Map<String, Value>) -> Answer {
    let Some(registry) = native.registry.get() else {
        return not_ready();
    };
    let renamed = fields.get("displayName").cloned();
    let done = registry.change(|r| match r.set_fields(id, fields) {
        Ok(note) => (Ok(()), note.into_iter().collect()),
        Err(e) => (Err(e), Vec::new()),
    });
    match done {
        Some(Ok(())) => {
            if let Some(name) = renamed {
                super::tasks::log_event(
                    native,
                    id,
                    "renamed",
                    Some(json!({ "displayName": name })),
                );
            }
            Answer::Void
        }
        Some(Err(e)) => Answer::Error(e.to_string()),
        None => not_ready(),
    }
}

/// `terminal:reorder`: told even when the order is the one there was, as
/// the server tells it.
fn reorder(native: &Native, ids: Vec<String>) -> Answer {
    let Some(registry) = native.registry.get() else {
        return not_ready();
    };
    let done = registry.change(|r| match r.reorder(ids) {
        Ok(note) => (Ok(()), vec![note]),
        Err(e) => (Err(e), Vec::new()),
    });
    match done {
        Some(Ok(())) => Answer::Void,
        // A duplicate or an unknown id, in the server's words.
        Some(Err(e)) => Answer::Error(e.to_string()),
        None => not_ready(),
    }
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
    fn reads_a_remote_project_as_a_posix_path_on_any_host() {
        let read = |path: &str| {
            CreateRequest::read(&json!({
                "agentType": "claude", "projectName": "far", "projectPath": path,
                "remoteHostId": "h",
            }))
        };
        assert!(read("/srv/far").is_some());
        assert!(read("srv/far").is_none());
        assert!(read(r"C:\srv\far").is_none());
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
        let read = |host: &str| {
            let path = if host.is_empty() { project() } else { "/srv/p" };
            CreateRequest::read(&json!({
                "agentType": "codex", "projectName": "p", "projectPath": path, "remoteHostId": host,
            }))
            .unwrap()
        };
        assert_eq!(read("").remote(), None);
        assert_eq!(read("h").remote(), Some("h"));
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
        streams: std::sync::Arc<crate::streams::Streams>,
        clients: std::sync::Arc<crate::serve::clients::Clients>,
    }

    impl Fed {
        /// A client that hears every note told from now on.
        pub(in crate::native) fn notes(&self) -> Notes {
            let conn = self.streams.connect();
            let forwarder = conn.forwarder();
            self.clients
                .add(conn.id(), forwarder, std::sync::Arc::default(), None, None);
            Notes {
                conn,
                held: std::collections::VecDeque::new(),
            }
        }
    }

    /// What one client is told.
    pub(in crate::native) struct Notes {
        conn: crate::streams::ClientConn,
        held: std::collections::VecDeque<Value>,
    }

    impl Notes {
        /// The next note, as `{method, params}`; an error when none is waiting.
        pub(in crate::native) fn try_recv(&mut self) -> Result<Value, &'static str> {
            if self.held.is_empty() {
                for msg in self.conn.drain_now() {
                    if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
                        let frame: Value = serde_json::from_str(text.as_str()).unwrap();
                        self.held.push_back(
                            json!({ "method": frame["method"], "params": frame["params"] }),
                        );
                    }
                }
            }
            self.held.pop_front().ok_or("nothing told")
        }
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
        native.set_link(std::sync::Arc::clone(&link));
        let clients = std::sync::Arc::new(crate::serve::clients::Clients::default());
        native.set_clients(std::sync::Arc::clone(&clients));
        Fed {
            native,
            registry,
            host,
            link,
            streams: crate::streams::Streams::new(),
            clients,
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
        let mut asks = fed.notes();
        assert_eq!(
            call(&fed.native, "terminal:kill", &json!("a")),
            Answer::Void
        );
        assert_eq!(ids(&fed), ["b", "sh"]);
        assert_eq!(fed.host.signalled(), ["a"]);
        // `b` is idle, so nothing is at work in the worktree any more.
        assert_eq!(
            asks.try_recv().unwrap(),
            json!({
                "method": "worktree:confirmCleanup",
                "params": { "id": "a", "projectPath": "/p", "worktreePath": "/w" },
            })
        );
        assert!(asks.try_recv().is_err());
        // One no record names: clients are told it ended, so a closing card finishes.
        assert_eq!(
            call(&fed.native, "terminal:kill", &json!("a")),
            Answer::Void
        );
        assert_eq!(
            asks.try_recv().unwrap(),
            json!({ "method": "terminal:exit", "params": { "id": "a", "exitCode": 0 } })
        );
        assert_eq!(
            call(&fed.native, "terminal:kill", &json!(5)),
            bad_params("terminal:kill")
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
    fn starts_a_shell_with_its_own_id_only_in_its_own_environment() {
        let fed = fed();
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_str().unwrap();
        let before = std::env::var("VORN_SESSION_ID").ok();
        let Answer::Result(record) = call(&fed.native, "shell:create", &json!(cwd)) else {
            panic!("no shell");
        };
        assert_eq!(record["agentType"], "shell");
        let (spec, _) = fed.host.last_start();
        assert_eq!(spec.cwd, cwd);
        let ids: Vec<_> = spec
            .env
            .iter()
            .filter(|(k, _)| k == "VORN_SESSION_ID")
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(ids, [record["id"].as_str().unwrap()]);
        // Set at the spawn only: no other child of vornd inherits it.
        assert_eq!(std::env::var("VORN_SESSION_ID").ok(), before);
        assert_eq!(
            call(&fed.native, "shell:create", &json!("relative")),
            bad_params("shell:create")
        );
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
        // What the server refused, refused in its words.
        for (method, params, why) in [
            (
                "terminal:reorder",
                json!(["a", "a"]),
                "Duplicate session IDs".to_owned(),
            ),
            (
                "terminal:reorder",
                json!(["a", "x"]),
                "Session not found: x".to_owned(),
            ),
            (
                "terminal:rename",
                json!({ "id": "x", "displayName": "n" }),
                "Session not found: x".to_owned(),
            ),
            (
                "terminal:setGroup",
                json!({ "id": "a", "groupId": 3 }),
                "terminal:setGroup cannot read the params it was given".to_owned(),
            ),
        ] {
            assert_eq!(
                call(&fed.native, method, &params),
                Answer::Error(why),
                "{method}"
            );
        }
    }

    #[test]
    fn answers_a_create_naming_a_running_conversation_with_its_session() {
        let fed = fed();
        // A conversation already running is shown rather than started again.
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

    /// vornd owning the records between runs, with a project directory
    /// that is there, so a resume has somewhere to start.
    fn owning() -> (Fed, tempfile::TempDir) {
        let fed = fed();
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
            not_ready()
        );
        assert_eq!(
            call(&other.native, "sessions:restored", &Value::Null),
            not_ready()
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

    /// The server's database beside `fed`, with remote host `h` logging in by `auth`.
    fn with_host(fed: &Fed, auth: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("vorn.db");
        let options = vorn_store::StoreOptions {
            default_shell: "/bin/sh".into(),
            default_agent_commands: Map::new(),
            default_workspace: serde_json::from_value(json!({
                "id": "personal", "name": "Personal", "icon": "User",
                "iconColor": "#6b7280", "order": 0,
            }))
            .unwrap(),
            owner_name: "owner".into(),
            seed_workflows: Vec::new(),
        };
        let (mut store, _) = vorn_store::Store::open(&db, options).unwrap();
        let host = json!({
            "id": "h", "label": "Box", "hostname": "box.example", "user": "me",
            "port": 2222, "authMethod": auth, "sshOptions": "-A", "credentialId": "k",
        });
        store
            .call(
                "saveConfig",
                json!([{ "version": 1, "defaults": {}, "projects": [], "remoteHosts": [host] }, []]),
            )
            .unwrap();
        fed.native.set_database(db);
        let secrets = &fed.native.secrets;
        secrets
            .put_item(vorn_vault::Kind::SshKey, "k", &KEY.into())
            .unwrap();
        secrets
            .put_item(vorn_vault::Kind::HostPassword, "h", &PASSWORD.into())
            .unwrap();
        dir
    }

    const PASSWORD: &str = "pw-hunter2-never-shown";
    const KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY----- never-shown";

    fn remote_create(extra: Value) -> Value {
        let mut params = json!({
            "agentType": "claude", "projectName": "far", "projectPath": "/srv/far",
            "remoteHostId": "h", "initialPrompt": "fix the build",
        });
        if let (Value::Object(p), Value::Object(extra)) = (&mut params, extra) {
            p.extend(extra);
        }
        params
    }

    fn remote_of(input: &Input) -> &Remote {
        match input {
            Input::Remote(remote) => remote,
            other => panic!("not a login: {other:?}"),
        }
    }

    #[test]
    fn starts_a_terminal_on_a_remote_host_in_a_local_shell_that_logs_in() {
        let fed = fed();
        let _db = with_host(&fed, "password");
        let answer = call(&fed.native, "terminal:create", &remote_create(json!({})));
        let Answer::Result(record) = answer else {
            panic!("not created: {answer:?}");
        };
        assert_eq!(
            (&record["remoteHostId"], &record["remoteHostLabel"]),
            (&json!("h"), &json!("Box"))
        );
        assert_eq!(record["projectPath"], "/srv/far");
        assert_eq!(record["displayName"], "fix the build");
        // Not pinned: the agent is told no id on a host whose history this machine cannot read.
        for absent in ["agentSessionId", "worktreePath", "branch", "headCommit"] {
            assert!(record.get(absent).is_none(), "{absent}: {record}");
        }
        let (spec, input) = fed.host.last_start();
        assert_eq!(spec.cwd, shell::home_dir());
        assert_eq!(
            spec.argv[1..],
            launch_shell::default_shell_args(Platform::HOST)
                .iter()
                .map(|a| (*a).to_owned())
                .collect::<Vec<_>>()[..]
        );
        let id = record["id"].as_str().unwrap();
        let remote = remote_of(&input);
        assert_eq!(remote.marker, login::marker(id));
        assert!(
            remote.line.starts_with(
                "ssh -t -p 2222 -o PreferredAuthentications=password -o PubkeyAuthentication=no -A me@box.example 'echo __VORN_READY_"
            ),
            "{}",
            remote.line
        );
        assert!(remote.command.starts_with("cd /srv/far && claude"));
        assert!(remote.command.contains("'fix the build'"));
        assert_eq!(remote.password.as_ref().map(Secret::expose), Some(PASSWORD));
        // A key goes with a stored-key login only.
        assert_eq!(remote.key, None);
        fed.host.up(7);
        assert_eq!(
            fed.registry.read(|r| r.terminal(id).unwrap().0.pid),
            Some(7)
        );
    }

    #[test]
    fn hands_a_stored_key_to_ssh_as_a_file_written_only_once_it_logs_in() {
        let fed = fed();
        let _db = with_host(&fed, "key-stored");
        let named = json!({ "resumeSessionId": "conv-far" });
        let Answer::Result(record) = call(&fed.native, "terminal:create", &remote_create(named))
        else {
            panic!("not created");
        };
        assert_eq!(record["agentSessionId"], "conv-far");
        let (_, input) = fed.host.last_start();
        let remote = remote_of(&input);
        let key = remote.key.as_ref().expect("the stored key");
        assert!(remote.line.starts_with(&format!(
            "ssh -t -p 2222 -i {} -A me@box.example",
            key.path.display()
        )));
        assert!(!key.path.exists());
        assert_eq!(remote.password, None);
        assert!(remote.command.contains("--resume conv-far"));

        // A stored key the vault does not hold: ssh falls back to the agent.
        fed.native
            .secrets
            .remove_item(vorn_vault::Kind::SshKey, "k");
        call(&fed.native, "terminal:create", &remote_create(json!({})));
        let (_, input) = fed.host.last_start();
        assert!(remote_of(&input)
            .line
            .starts_with("ssh -t -p 2222 -A me@box.example"));
    }

    #[test]
    fn starts_here_what_names_a_host_the_settings_do_not_have() {
        let fed = fed();
        // No database: vornd cannot tell the host.
        assert_eq!(
            call(&fed.native, "terminal:create", &remote_create(json!({}))),
            super::super::no_database()
        );
        assert!(fed.host.starts.lock().unwrap().is_empty());
        let _db = with_host(&fed, "agent");
        let other = json!({ "remoteHostId": "gone" });
        let made = call(&fed.native, "terminal:create", &remote_create(other));
        assert!(
            matches!(made, Answer::Result(ref r) if r.get("remoteHostId").is_none()),
            "{made:?}"
        );
        assert_eq!(fed.host.starts.lock().unwrap().len(), 1);
    }

    /// Everything tracing writes, at every level.
    #[derive(Clone, Default)]
    struct Captured(std::sync::Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
        type Writer = Captured;
        fn make_writer(&'a self) -> Captured {
            self.clone()
        }
    }

    #[test]
    fn credentials_never_reach_a_log_a_record_or_an_argv() {
        let captured = Captured::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(captured.clone())
            .with_max_level(tracing::Level::TRACE)
            .finish();
        let mut seen = Vec::new();
        tracing::subscriber::with_default(subscriber, || {
            for auth in ["password", "key-stored"] {
                let fed = fed();
                let _db = with_host(&fed, auth);
                let params = remote_create(json!({}));
                let answer = call(&fed.native, "terminal:create", &params);
                seen.push(format!("{answer:?}"));
                let (spec, input) = fed.host.last_start();
                seen.push(format!("{spec:?} {input:?}"));
                seen.push(format!("{:?}", CreateRequest::read(&params)));
                // A failed start is logged with its reason.
                fed.host.down("no holder");
                fed.registry.read(|r| {
                    seen.push(serde_json::to_string(&r.terminals()).unwrap());
                });
                seen.push(format!("{:?}", fed.registry.restored()));
                // The stored key without its content warns, naming the host only.
                fed.native
                    .secrets
                    .remove_item(vorn_vault::Kind::SshKey, "k");
                call(&fed.native, "terminal:create", &remote_create(json!({})));
            }
        });
        let logs = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
        assert!(
            logs.contains("vornd could not start this session"),
            "{logs}"
        );
        seen.push(logs);
        for text in &seen {
            assert!(
                !text.contains(PASSWORD) && !text.contains(KEY),
                "a credential in: {text}"
            );
        }
    }

    #[test]
    fn resumes_a_remote_session_by_logging_in_again_on_its_conversation() {
        let fed = fed();
        let _db = with_host(&fed, "agent");
        fed.registry.own_records();
        // Its project is on the host, not here: a resume reconnects anyway.
        let far = Path::new("/srv/far-not-here");
        offer(
            &fed,
            vec![carried_agent(
                "rr",
                far,
                json!({ "agentSessionId": "conv-rr", "remoteHostId": "h", "remoteHostLabel": "Box" }),
            )],
        );
        let answer = call(&fed.native, "sessions:resume", &json!({ "id": "rr" }));
        let Answer::Result(answer) = answer else {
            panic!("not resumed: {answer:?}");
        };
        assert_eq!(answer["ok"], true, "{answer}");
        let session = &answer["session"];
        assert_eq!(
            (
                &session["id"],
                &session["remoteHostId"],
                &session["agentSessionId"]
            ),
            (&json!("rr"), &json!("h"), &json!("conv-rr"))
        );
        assert_eq!(session["groupId"], "g");
        let (spec, input) = fed.host.last_start();
        assert_eq!(spec.cwd, shell::home_dir());
        let remote = remote_of(&input);
        assert!(remote.line.starts_with("ssh -t -p 2222 -A me@box.example"));
        assert!(remote.command.starts_with("cd /srv/far-not-here && claude"));
        assert!(remote.command.contains("--resume conv-rr"));
        assert_eq!(
            (remote.password.as_ref(), remote.key.as_ref()),
            (None, None)
        );
        fed.host.up(5);
        assert_eq!(ids(&fed), ["a", "b", "sh", "rr"]);
    }
}
