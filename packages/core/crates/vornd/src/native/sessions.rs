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
//! The server keeps a call vornd cannot answer as it would: one for a
//! remote host, one whose params are not the shape its handler reads, one
//! naming a terminal the registry does not hold (a session carried over from
//! a previous run), and every call while the registry does not hold the
//! server's records or vornd's session holder is not connected.
//!
//! In shadow mode nothing here changes anything: a create is compared with
//! the server's as the spawn each would ask for ([`plan`]), and the other
//! calls as what each would answer ([`foresee`]), read from the copy.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};
use tracing::warn;
use vorn_agents::launch::shell as launch_shell;
use vorn_agents::launch::{
    display_name_from_prompt, launch_line, LaunchRequest, Machine, Platform, Quoting,
};
use vorn_agents::{paths, Agent};
use vorn_git::repo::{extract_worktree_name, node_basename, Git};
use vorn_sessiond_wire::{Io, Sig, SpawnSpec};

use super::{agent, headless, shell, Answer, Native};
use crate::claims::{Claims, OnePerKey};
use crate::registry::{AgentStatus, Registry, TerminalSession};

/// The size a terminal starts at, before any client has fitted it.
pub const INITIAL_COLS: u16 = 80;
pub const INITIAL_ROWS: u16 = 24;

/// The terminal type programs are told they run in, off Windows.
const PTY_TERM: &str = "xterm-256color";

/// How long after a create the agent's launch line is typed into its
/// shell, as the server waits.
pub const TYPE_AFTER: Duration = Duration::from_millis(300);

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

/// Answers `method` with `params`.
pub fn call(native: &Native, method: &str, params: &Value) -> Answer {
    match method {
        "terminal:create" => match CreateRequest::read(params) {
            Some(req) => create(native, &req),
            None => Answer::Forward,
        },
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
        "terminal:kill" | "terminal:rename" | "terminal:setGroup" | "terminal:reorder"
    ) || headless::foresees(method)
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
    let asked = asked(method, params)?;
    native.registry.get()?.read(|r| match &asked {
        Asked::Kill(id) => r.terminal(id).map(|_| Answer::Void),
        Asked::Fields(id, _) => Some(refused(r.check_terminal(id))),
        Asked::Order(ids) => Some(refused(r.check_order(ids))),
    })?
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
        return start_agent(native, req, &new_id());
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
    let answer = start_agent(native, req, &id);
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

/// Prepares and starts agent session `id`.
fn start_agent(native: &Native, req: &CreateRequest, id: &str) -> Answer {
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
        ..skeleton(id, req.agent.id(), &req.project_name, &req.project_path)
    };
    let typed = Input::Typed(format!("{}\r", prepared.launch_line).into_bytes());
    let answer = register(native, record, &prepared.cwd, argv, env, typed);
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
    register(native, record, &dir, argv, env, Input::None)
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
    let made = registry.change(|r| match r.create(record) {
        Ok(notes) => (Ok(()), notes),
        Err(e) => (Err(e), Vec::new()),
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
        "terminal:create" | "shell:create" | "headless:create"
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
        _ => None,
    }
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

/// A plan from its parts, the environment's names sorted.
pub(super) fn plan_of(
    argv: Vec<String>,
    cwd: &str,
    mut keys: Vec<String>,
    record: &Value,
) -> Value {
    keys.sort();
    keys.dedup();
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
            // Minted for an agent that pins one: only that there is one.
            let v = if k == "agentSessionId" {
                json!(true)
            } else {
                v.clone()
            };
            kept.insert(k.clone(), v);
        }
    }
    json!(kept)
}

/// A spawn as the server asked vornd for it, as a plan is compared.
pub fn spawn_plan(spawn: &Value, reply: &Value) -> Value {
    let argv = spawn.get("argv").cloned().unwrap_or(Value::Null);
    let cwd = spawn.get("cwd").cloned().unwrap_or(Value::Null);
    let mut keys: Vec<String> = spawn
        .get("env")
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    keys.sort();
    json!({
        "argv": argv,
        "cwd": cwd,
        "envKeys": keys,
        "record": plan_record(reply),
    })
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
}
