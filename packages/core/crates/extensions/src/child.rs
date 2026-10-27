//! One child spoken to in JSON-RPC, a message per line on its stdin and
//! stdout, its stderr kept as a short tail for the messages a failure gives.
//!
//! A child in the connector protocol only answers. One speaking the extension
//! API also asks: its requests and notifications go to the inbox it was
//! started with, and are answered through its [`Writer`].

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::ChildStdin;
use tokio::sync::{mpsc, oneshot, watch};
use tracing::{info, warn};

use crate::js;

/// The longest line a child may write before it is stopped.
pub const MAX_FRAME_BYTES: usize = 16 << 20;
const TAIL_LINES: usize = 40;
const MAX_STDERR_LINE: usize = 4096;
const TERM_AFTER: Duration = Duration::from_secs(2);
const KILL_AFTER: Duration = Duration::from_secs(5);
const GIVE_UP_AFTER: Duration = Duration::from_secs(6);
/// The code a child's error carries when it names none.
pub const CONNECTOR_ERROR: i64 = -32000;

/// What to run: the program, its arguments, where and with what.
#[derive(Debug, Clone)]
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// The whole environment; nothing of this process's is inherited.
    pub env: Vec<(String, String)>,
}

/// How a child ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit {
    Code(Option<i32>),
    Signal(String),
}

impl fmt::Display for Exit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Exit::Code(Some(code)) => write!(f, "code {code}"),
            Exit::Code(None) => f.write_str("code null"),
            Exit::Signal(name) => write!(f, "signal {name}"),
        }
    }
}

impl From<ExitStatus> for Exit {
    fn from(status: ExitStatus) -> Exit {
        #[cfg(unix)]
        if let Some(sig) = std::os::unix::process::ExitStatusExt::signal(&status) {
            return Exit::Signal(signal_name(sig));
        }
        Exit::Code(status.code())
    }
}

#[cfg(unix)]
fn signal_name(sig: i32) -> String {
    match sig {
        libc::SIGHUP => "SIGHUP".into(),
        libc::SIGINT => "SIGINT".into(),
        libc::SIGQUIT => "SIGQUIT".into(),
        libc::SIGABRT => "SIGABRT".into(),
        libc::SIGKILL => "SIGKILL".into(),
        libc::SIGSEGV => "SIGSEGV".into(),
        libc::SIGPIPE => "SIGPIPE".into(),
        libc::SIGTERM => "SIGTERM".into(),
        other => format!("{other}"),
    }
}

/// What a connector says of an error it answered with (`data` in protocol 1).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ErrorData {
    /// One of `validation`, `app-offline`, `signed-out`, `upstream`, `internal`.
    pub kind: Option<String>,
    pub retryable: Option<bool>,
    pub field: Option<String>,
    /// The extension API's own error code (`ext-error.code`).
    pub code: Option<String>,
}

/// The error kinds protocol 1 names; any other is dropped.
pub const ERROR_KINDS: &[&str] = &[
    "validation",
    "app-offline",
    "signed-out",
    "upstream",
    "internal",
];

/// Why a request got no result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallError {
    /// The child answered with an error.
    Answered {
        code: i64,
        message: String,
        data: ErrorData,
    },
    /// The child, or its pipe, failed before it answered.
    Transport(String),
    /// The child did not answer in time; it may still be running.
    Timeout(String),
}

impl CallError {
    pub fn code(&self) -> Option<i64> {
        match self {
            CallError::Answered { code, .. } => Some(*code),
            CallError::Transport(_) | CallError::Timeout(_) => None,
        }
    }
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallError::Answered { message, .. }
            | CallError::Transport(message)
            | CallError::Timeout(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for CallError {}

type Reply = oneshot::Sender<Result<Value, CallError>>;

/// A request or notification the child sent.
#[derive(Debug, Clone, PartialEq)]
pub struct Incoming {
    /// Absent for a notification, which is not answered.
    pub id: Option<Value>,
    pub method: String,
    pub params: Value,
}

/// Where a child's own requests go; bounded, so a flood waits on the bridge.
pub type Inbox = mpsc::Sender<Incoming>;

#[derive(Default)]
struct State {
    pending: HashMap<u64, (String, Reply)>,
    tail: VecDeque<String>,
    closing: bool,
}

impl State {
    /// The last line of the tail that names an error, or the last line, to
    /// say why a child failed.
    fn error_line(&self) -> Option<&str> {
        let kept = self
            .tail
            .iter()
            .map(|l| js::trim(l))
            .filter(|l| !l.is_empty());
        let mut last = None;
        let mut named = None;
        for line in kept {
            if names_error(line) {
                named = Some(line);
            }
            last = Some(line);
        }
        named.or(last)
    }

    fn with_line(&self, message: String) -> String {
        match self.error_line() {
            Some(line) if !message.contains(line) => format!("{message}: {line}"),
            _ => message,
        }
    }

    fn reject_all(&mut self, error: impl Fn(&str) -> String) {
        for (_, (method, reply)) in self.pending.drain() {
            let _ = reply.send(Err(CallError::Transport(error(&method))));
        }
    }
}

/// `/Error\b/`: the word ends at a boundary, as `TypeError:` does.
fn names_error(line: &str) -> bool {
    line.match_indices("Error").any(|(at, word)| {
        line[at + word.len()..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'))
    })
}

#[derive(Debug, Clone, Copy)]
enum Signal {
    Term,
    Kill,
}

/// A running child. Dropping it does not stop the child; [`Child::close`] does.
pub struct Child {
    name: String,
    pid: Option<u32>,
    stdin: Writer,
    state: Arc<Mutex<State>>,
    next_id: AtomicU64,
    signals: mpsc::UnboundedSender<Signal>,
    exit: watch::Receiver<Option<Exit>>,
}

impl fmt::Debug for Child {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Child").field("name", &self.name).finish()
    }
}

fn lock(state: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Child {
    /// Starts `launch`; `name` is how its messages and logs name it.
    pub fn start(launch: &Launch, name: String) -> Result<Child, String> {
        Child::start_with(launch, name, None)
    }

    /// Starts `launch` with an inbox for what the child asks; without one, a
    /// request from the child is logged and dropped, as protocol 1 has none.
    pub fn start_with(
        launch: &Launch,
        name: String,
        inbox: Option<Inbox>,
    ) -> Result<Child, String> {
        Child::start_redacting(launch, name, inbox, Vec::new())
    }

    /// [`Child::start_with`], with `secrets` (values the child was handed)
    /// taken out of every stderr line before it is logged or kept.
    pub fn start_redacting(
        launch: &Launch,
        name: String,
        inbox: Option<Inbox>,
        secrets: Vec<String>,
    ) -> Result<Child, String> {
        let spawn = |program: &Path| {
            vorn_spawn::tokio_command(program)
                .args(&launch.args)
                .current_dir(&launch.cwd)
                .env_clear()
                .envs(launch.env.iter().map(|(k, v)| (k, v)))
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(false)
                .spawn()
        };
        let spawned = match on_path(&launch.program, &launch.env, &launch.cwd) {
            Some(full) => spawn(&full).or_else(|err| {
                if bare_name_may_run(&err) {
                    spawn(&launch.program)
                } else {
                    Err(err)
                }
            }),
            None => spawn(&launch.program),
        };
        let mut child = spawned.map_err(|err| {
            format!(
                "{name} could not start {}: {}",
                launch.program.display(),
                spawn_reason(&launch.program, &err)
            )
        })?;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Err(format!(
                "{name} started {} without its pipes",
                launch.program.display()
            ));
        };
        let state = Arc::new(Mutex::new(State::default()));
        let (signals, mut signal_rx) = mpsc::unbounded_channel::<Signal>();
        let (exit_tx, exit) = watch::channel(None);

        let stderr_done = tokio::spawn(read_stderr(
            stderr,
            name.clone(),
            Arc::clone(&state),
            secrets,
        ));
        let stdout_done = tokio::spawn(read_stdout(
            stdout,
            name.clone(),
            Arc::clone(&state),
            signals.clone(),
            inbox,
        ));
        let pid = child.id();
        let waiter_name = name.clone();
        let waiter_state = Arc::clone(&state);
        tokio::spawn(async move {
            let status = loop {
                tokio::select! {
                    status = child.wait() => break status,
                    Some(sig) = signal_rx.recv() => send_signal(&mut child, pid, sig),
                }
            };
            // The pipes may hold the last lines it wrote, and its answers.
            let _ = stdout_done.await;
            let _ = stderr_done.await;
            let ended = status.map_or(Exit::Code(None), Exit::from);
            // Seen as ended before any waiting caller hears of it, so its next call starts a fresh one.
            let _ = exit_tx.send(Some(ended.clone()));
            let mut st = lock(&waiter_state);
            if st.closing {
                st.reject_all(|m| format!("{waiter_name} was stopped before it answered {m}"));
            } else {
                let tail = st.error_line().map(str::to_owned);
                st.reject_all(|m| {
                    let message = format!("{waiter_name} exited ({ended}) before it answered {m}");
                    match &tail {
                        Some(line) if !message.contains(line.as_str()) => {
                            format!("{message}: {line}")
                        }
                        _ => message,
                    }
                });
                info!("{waiter_name} exited ({ended})");
            }
            drop(st);
        });

        let stdin = Writer {
            name: Arc::from(name.as_str()),
            stdin: Arc::new(tokio::sync::Mutex::new(Some(stdin))),
        };
        Ok(Child {
            name,
            pid,
            stdin,
            state,
            next_id: AtomicU64::new(1),
            signals,
            exit,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether it has ended.
    pub fn exited(&self) -> bool {
        self.exit.borrow().is_some()
    }

    /// Waits for it to end, however it does.
    pub async fn wait(&self) -> Exit {
        let mut exit = self.exit.clone();
        let ended = match exit.wait_for(Option::is_some).await {
            Ok(ended) => ended.clone(),
            Err(_) => None,
        };
        ended.unwrap_or(Exit::Code(None))
    }

    /// Asks `method` and waits up to `timeout` for the answer.
    pub async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, CallError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        {
            let mut st = lock(&self.state);
            if st.closing || self.exited() {
                return Err(CallError::Transport(st.with_line(format!(
                    "{} is not running, so it cannot answer {method}",
                    self.name
                ))));
            }
            st.pending.insert(id, (method.to_owned(), tx));
        }
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if let Err(err) = self.stdin.write(&message).await {
            warn!("{err}");
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => Err(CallError::Transport(format!(
                "{} was stopped before it answered {method}",
                self.name
            ))),
            Err(_) => {
                lock(&self.state).pending.remove(&id);
                Err(CallError::Timeout(format!(
                    "{} did not answer {method} within {} s",
                    self.name,
                    timeout.as_secs_f64().round()
                )))
            }
        }
    }

    /// The process id, while it runs.
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// A handle that writes to its stdin, for whoever answers what it asks.
    pub fn writer(&self) -> Writer {
        self.stdin.clone()
    }

    /// Sends `method` without an id; nothing answers it.
    pub async fn notify(&self, method: &str, params: Value) -> Result<(), CallError> {
        self.stdin.notify(method, params).await
    }

    /// Ends its stdin, then `SIGTERM` and `SIGKILL` if it lingers; returns
    /// once it has ended or [`GIVE_UP_AFTER`] has passed.
    pub async fn close(&self) {
        {
            let mut st = lock(&self.state);
            st.closing = true;
            let name = &self.name;
            st.reject_all(|m| format!("{name} was stopped before it answered {m}"));
        }
        if self.exited() {
            return;
        }
        self.stdin.close().await;
        let started = tokio::time::Instant::now();
        let ended = self.wait();
        tokio::pin!(ended);
        for (at, sig) in [
            (TERM_AFTER, Some(Signal::Term)),
            (KILL_AFTER, Some(Signal::Kill)),
            (GIVE_UP_AFTER, None),
        ] {
            tokio::select! {
                _ = &mut ended => return,
                () = tokio::time::sleep_until(started + at) => {
                    if let Some(sig) = sig {
                        let _ = self.signals.send(sig);
                    }
                }
            }
        }
    }
}

/// Writes JSON-RPC messages to a child's stdin, a line each.
#[derive(Clone)]
pub struct Writer {
    name: Arc<str>,
    stdin: Arc<tokio::sync::Mutex<Option<ChildStdin>>>,
}

impl fmt::Debug for Writer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Writer").field("name", &self.name).finish()
    }
}

impl Writer {
    async fn write(&self, message: &Value) -> Result<(), CallError> {
        let mut line = serde_json::to_vec(message).unwrap_or_default();
        line.push(b'\n');
        match self.stdin.lock().await.as_mut() {
            Some(stdin) => stdin.write_all(&line).await.map_err(|err| {
                CallError::Transport(format!("{} stopped reading: {err}", self.name))
            }),
            None => Err(CallError::Transport(format!(
                "{} is not running",
                self.name
            ))),
        }
    }

    async fn close(&self) {
        drop(self.stdin.lock().await.take());
    }

    /// Sends `method` without an id; nothing answers it.
    pub async fn notify(&self, method: &str, params: Value) -> Result<(), CallError> {
        self.write(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await
    }

    /// Answers the child's request `id`: a result, or a code, message and data.
    pub async fn reply(&self, id: Value, answer: Result<Value, (i64, String, Value)>) {
        let message = match answer {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message, data)) => json!({
                "jsonrpc": "2.0", "id": id,
                "error": { "code": code, "message": message, "data": data }
            }),
        };
        if let Err(err) = self.write(&message).await {
            warn!("{err}");
        }
    }
}

fn send_signal(child: &mut tokio::process::Child, pid: Option<u32>, sig: Signal) {
    #[cfg(unix)]
    if let (Signal::Term, Some(pid)) = (sig, pid.and_then(|p| i32::try_from(p).ok())) {
        // SAFETY: kill(2) on our own child's pid, which `wait` has not reaped yet.
        unsafe { libc::kill(pid, libc::SIGTERM) };
        return;
    }
    let _ = (pid, sig);
    let _ = child.start_kill();
}

/// Where a bare program name (`node`) is on the `PATH` in `env`, which the
/// child runs with, rather than this process's.
///
/// Starting a bare name with its own `PATH` makes std fork the whole daemon
/// (copying its page tables while every other thread that touches memory
/// waits) to search that `PATH` in the child; a full path lets it use
/// `posix_spawn`, which shares the parent's memory until the child execs.
/// `None` when `program` is a path already, or is not found; the spawn then
/// searches as before, and says ENOENT for a missing one. Like `execvp`, an
/// entry this process may not execute is skipped.
#[cfg(unix)]
/// `program`'s full path on the child's own `PATH`, for a bare name.
pub(crate) fn on_path(program: &Path, env: &[(String, String)], cwd: &Path) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    if program.is_absolute() || program.components().count() != 1 {
        return None;
    }
    // The last PATH wins, as it does in the child's environment.
    let (_, path) = env.iter().rev().find(|(k, _)| k == "PATH")?;
    std::env::split_paths(path)
        .map(|dir| cwd.join(dir).join(program))
        .find(|candidate| {
            let Ok(c_path) = std::ffi::CString::new(candidate.as_os_str().as_bytes()) else {
                return false;
            };
            // SAFETY: access(2) reads a NUL-terminated path that outlives the call.
            let runnable = unsafe { libc::access(c_path.as_ptr(), libc::X_OK) } == 0;
            runnable && std::fs::metadata(candidate).is_ok_and(|m| m.is_file())
        })
}

/// Whether a full path from [`on_path`] failed where `execvp` on the bare
/// name could still run something: it skips an entry it may not execute
/// (EACCES) and runs a script with no `#!` line through `/bin/sh` (ENOEXEC).
/// Retrying with the bare name then behaves as it always did.
#[cfg(unix)]
fn bare_name_may_run(err: &std::io::Error) -> bool {
    matches!(err.raw_os_error(), Some(libc::EACCES | libc::ENOEXEC))
}

/// Windows starts no process by forking, so there is nothing to spare.
#[cfg(not(unix))]
pub(crate) fn on_path(_: &Path, _: &[(String, String)], _: &Path) -> Option<PathBuf> {
    None
}

#[cfg(not(unix))]
fn bare_name_may_run(_: &std::io::Error) -> bool {
    false
}

/// Why a start failed, as `spawn <program> ENOENT` and the like.
fn spawn_reason(program: &std::path::Path, err: &std::io::Error) -> String {
    let program = program.display();
    match err.kind() {
        std::io::ErrorKind::NotFound => format!("spawn {program} ENOENT"),
        std::io::ErrorKind::PermissionDenied => format!("spawn {program} EACCES"),
        _ => format!("spawn {program} {err}"),
    }
}

/// The shortest secret [`redact`] looks for: shorter values (`1`, `on`)
/// would blank ordinary words.
pub const MIN_REDACTED: usize = 4;

/// `line` with every secret in `secrets` replaced by `[redacted]`.
pub fn redact<'a>(line: &'a str, secrets: &[String]) -> std::borrow::Cow<'a, str> {
    let mut out = std::borrow::Cow::Borrowed(line);
    for secret in secrets.iter().filter(|s| s.len() >= MIN_REDACTED) {
        if out.contains(secret.as_str()) {
            out = std::borrow::Cow::Owned(out.replace(secret.as_str(), "[redacted]"));
        }
    }
    out
}

/// Takes secrets out of a stream read in pieces, so one split across two
/// reads is caught too: a piece that does not end its line holds back its
/// last bytes, as many as could start a secret, for the next piece.
#[derive(Debug)]
pub struct Redactor {
    secrets: Vec<Vec<u8>>,
    carry: Vec<u8>,
}

const REDACTED: &[u8] = b"[redacted]";

impl Redactor {
    pub fn new(secrets: &[String]) -> Redactor {
        Redactor {
            secrets: secrets
                .iter()
                .filter(|s| s.len() >= MIN_REDACTED)
                .map(|s| s.as_bytes().to_vec())
                .collect(),
            carry: Vec::new(),
        }
    }

    /// `piece` with every secret taken out, minus what is held back when
    /// `ends_line` is false; the held-back bytes come out with the next piece.
    pub fn push(&mut self, piece: &[u8], ends_line: bool) -> Vec<u8> {
        let mut buf = std::mem::take(&mut self.carry);
        buf.extend_from_slice(piece);
        for secret in &self.secrets {
            buf = replace_all(&buf, secret);
        }
        if !ends_line {
            // A secret that starts before this point ended inside `buf` and is gone.
            let hold = self.secrets.iter().map(Vec::len).max().unwrap_or(1) - 1;
            let at = buf.len().saturating_sub(hold);
            self.carry = buf.split_off(at);
        }
        buf
    }
}

/// `hay` with every `needle` replaced by `[redacted]`.
fn replace_all(hay: &[u8], needle: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(hay.len());
    let mut i = 0;
    while i < hay.len() {
        if hay[i..].starts_with(needle) {
            out.extend_from_slice(REDACTED);
            i += needle.len();
        } else {
            out.push(hay[i]);
            i += 1;
        }
    }
    out
}

async fn read_stderr(
    stderr: tokio::process::ChildStderr,
    name: String,
    state: Arc<Mutex<State>>,
    secrets: Vec<String>,
) {
    let mut redactor = Redactor::new(&secrets);
    let mut reader = BufReader::new(stderr);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        // Bounded, so a child that never ends a line cannot grow it for ever; the redactor joins the pieces.
        let read = (&mut reader)
            .take(MAX_STDERR_LINE as u64 + 1)
            .read_until(b'\n', &mut buf)
            .await;
        match read {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let piece = redactor.push(&buf, buf.ends_with(b"\n"));
        let text = String::from_utf8_lossy(&piece);
        let text = text.trim_end_matches('\n').trim_end_matches('\r');
        if js::trim(text).is_empty() {
            continue;
        }
        let kept = js::slice16(text, MAX_STDERR_LINE);
        info!("{name} stderr: {kept}");
        let mut st = lock(&state);
        st.tail.push_back(kept.to_owned());
        if st.tail.len() > TAIL_LINES {
            st.tail.pop_front();
        }
    }
}

async fn read_stdout(
    stdout: tokio::process::ChildStdout,
    name: String,
    state: Arc<Mutex<State>>,
    signals: mpsc::UnboundedSender<Signal>,
    inbox: Option<Inbox>,
) {
    let mut reader = BufReader::new(stdout);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let read = (&mut reader)
            .take(MAX_FRAME_BYTES as u64 + 1)
            .read_until(b'\n', &mut buf)
            .await;
        match read {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let line = buf.strip_suffix(b"\n").unwrap_or(&buf);
        if line.len() > MAX_FRAME_BYTES
            || (buf.last() != Some(&b'\n') && buf.len() > MAX_FRAME_BYTES)
        {
            warn!("{name} wrote a line over {MAX_FRAME_BYTES} bytes; stopping it");
            lock(&state).reject_all(|m| {
                format!("{name} answered {m} with a line over {MAX_FRAME_BYTES} bytes")
            });
            let _ = signals.send(Signal::Kill);
            // Drained, so the child is not left blocked on a full pipe.
            let _ = tokio::io::copy(&mut reader, &mut tokio::io::sink()).await;
            return;
        }
        let line = String::from_utf8_lossy(line);
        let line = line.strip_suffix('\r').unwrap_or(&line);
        if js::trim(line).is_empty() {
            continue;
        }
        if let Some(incoming) = on_line(&name, &state, line, inbox.is_some()) {
            if let Some(inbox) = &inbox {
                // Waits while the bridge is busy, so the child waits on its pipe.
                if inbox.send(incoming).await.is_err() {
                    return;
                }
            }
        }
    }
}

/// Settles an answer, or hands back a request the child made.
fn on_line(name: &str, state: &Mutex<State>, line: &str, asks: bool) -> Option<Incoming> {
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        warn!(
            "{name} wrote a line that is not JSON: {}",
            js::slice16(line, 200)
        );
        return None;
    };
    let Some(frame) = message.as_object() else {
        warn!(
            "{name} wrote a line that is not a message: {}",
            js::slice16(line, 200)
        );
        return None;
    };
    if let Some(method) = frame.get("method").and_then(Value::as_str) {
        if !asks {
            warn!("{name} sent {method}, which a connector does not send in protocol 1");
            return None;
        }
        return Some(Incoming {
            id: frame.get("id").cloned().filter(|id| !id.is_null()),
            method: method.to_owned(),
            params: frame.get("params").cloned().unwrap_or(Value::Null),
        });
    }
    let entry = frame
        .get("id")
        .and_then(Value::as_f64)
        .filter(|id| id.fract() == 0.0 && *id >= 0.0)
        .and_then(|id| lock(state).pending.remove(&(id as u64)));
    let Some((method, reply)) = entry else {
        let id = frame
            .get("id")
            .map_or_else(|| "undefined".to_owned(), |id| id.to_string());
        warn!("{name} answered a request nobody is waiting for: {id}");
        return None;
    };
    let answer = match frame.get("error") {
        Some(error) => Err(call_error(&method, error)),
        None => Ok(frame.get("result").cloned().unwrap_or(Value::Null)),
    };
    let _ = reply.send(answer);
    None
}

fn call_error(method: &str, error: &Value) -> CallError {
    let code = error
        .get("code")
        .and_then(Value::as_f64)
        .map_or(CONNECTOR_ERROR, |c| c as i64);
    let message = match error.get("message").and_then(Value::as_str) {
        Some(m) if !m.is_empty() => m.to_owned(),
        _ => format!("{method} failed"),
    };
    let data = error.get("data").filter(|d| d.is_object());
    let data = ErrorData {
        kind: data
            .and_then(|d| d.get("kind"))
            .and_then(Value::as_str)
            .filter(|k| ERROR_KINDS.contains(k))
            .map(str::to_owned),
        retryable: data
            .and_then(|d| d.get("retryable"))
            .and_then(Value::as_bool),
        field: data
            .and_then(|d| d.get("field"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        code: data
            .and_then(|d| d.get("code"))
            .and_then(Value::as_str)
            .map(str::to_owned),
    };
    CallError::Answered {
        code,
        message,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_line_that_names_an_error() {
        let mut st = State::default();
        st.tail.extend([
            "TypeError: x".to_owned(),
            "  at y".to_owned(),
            "Errors here".to_owned(),
            "done".to_owned(),
        ]);
        assert_eq!(st.error_line(), Some("TypeError: x"));
        assert_eq!(st.with_line("a".into()), "a: TypeError: x");
        assert_eq!(st.with_line("a TypeError: x".into()), "a TypeError: x");
        st.tail.clear();
        st.tail.push_back(" last ".into());
        assert_eq!(st.error_line(), Some("last"));
    }

    #[test]
    fn words_an_answered_error() {
        assert_eq!(
            call_error("m", &json!({ "code": -32601, "message": "nope" })),
            CallError::Answered {
                code: -32601,
                message: "nope".into(),
                data: ErrorData::default()
            }
        );
        assert_eq!(
            call_error("m", &json!("x")),
            CallError::Answered {
                code: CONNECTOR_ERROR,
                message: "m failed".into(),
                data: ErrorData::default()
            }
        );
        let CallError::Answered { data, .. } = call_error(
            "m",
            &json!({ "message": "x", "data": { "kind": "signed-out", "retryable": true, "field": "f" } }),
        ) else {
            panic!("an answered error");
        };
        assert_eq!(data.kind.as_deref(), Some("signed-out"));
        assert_eq!(
            (data.retryable, data.field.as_deref()),
            (Some(true), Some("f"))
        );
        let CallError::Answered { data, .. } =
            call_error("m", &json!({ "message": "x", "data": { "kind": "odd" } }))
        else {
            panic!("an answered error");
        };
        assert_eq!(data.kind, None);
    }

    #[test]
    fn describes_an_exit() {
        assert_eq!(Exit::Code(Some(3)).to_string(), "code 3");
        assert_eq!(Exit::Signal("SIGKILL".into()).to_string(), "signal SIGKILL");
        assert_eq!(Exit::Code(None).to_string(), "code null");
    }

    #[cfg(unix)]
    #[test]
    fn finds_a_bare_program_on_the_childs_path() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().expect("temp dir");
        let (first, second, cwd) = (
            root.path().join("a"),
            root.path().join("b"),
            root.path().join("c"),
        );
        for dir in [&first, &second, &cwd] {
            std::fs::create_dir(dir).expect("dir");
        }
        let put = |path: &Path, mode: u32| {
            std::fs::write(path, "#!/bin/sh\n").expect("write");
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
        };
        // Not executable in the first directory, so the second's is the one run.
        put(&first.join("tool"), 0o644);
        put(&second.join("tool"), 0o755);
        put(&cwd.join("local"), 0o755);
        let path = std::env::join_paths([&first, &second, Path::new(".")]).expect("PATH");
        let env = vec![
            ("PATH".to_owned(), "/nowhere".to_owned()),
            ("PATH".to_owned(), path.to_string_lossy().into_owned()),
        ];
        let found = |p: &str| on_path(Path::new(p), &env, &cwd);
        assert_eq!(found("tool"), Some(second.join("tool")));
        // A relative PATH entry is searched from the child's working directory.
        assert_eq!(found("local"), Some(cwd.join(".").join("local")));
        assert_eq!(found("missing"), None);
        assert_eq!(found("./tool"), None);
        assert_eq!(found("/bin/sh"), None);
        assert_eq!(on_path(Path::new("tool"), &[], &cwd), None);
    }

    #[cfg(unix)]
    #[test]
    fn skips_a_path_entry_this_user_may_not_execute() {
        use std::os::unix::fs::PermissionsExt;
        // SAFETY: geteuid(2) has no preconditions.
        if unsafe { libc::geteuid() } == 0 {
            return; // root may execute anything with an execute bit
        }
        let root = tempfile::tempdir().expect("temp dir");
        let (first, second) = (root.path().join("a"), root.path().join("b"));
        for (dir, mode) in [(&first, 0o601), (&second, 0o755)] {
            std::fs::create_dir(dir).expect("dir");
            let tool = dir.join("tool");
            std::fs::write(&tool, "#!/bin/sh\n").expect("write");
            std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(mode)).expect("chmod");
        }
        let path = std::env::join_paths([&first, &second]).expect("PATH");
        let env = vec![("PATH".to_owned(), path.to_string_lossy().into_owned())];
        // Others may execute the first, its owner may not, as execvp would find.
        assert_eq!(
            on_path(Path::new("tool"), &env, root.path()),
            Some(second.join("tool"))
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn starts_a_script_without_a_shebang_through_the_shell() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().expect("temp dir");
        let tool = root.path().join("tool");
        std::fs::write(
            &tool,
            "read line; echo '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"ok\"}'\n",
        )
        .expect("write");
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let path = std::env::join_paths([root.path(), Path::new("/usr/bin"), Path::new("/bin")])
            .expect("PATH");
        let launch = Launch {
            program: "tool".into(),
            args: vec![],
            cwd: root.path().to_owned(),
            env: vec![("PATH".into(), path.to_string_lossy().into_owned())],
        };
        let child = Child::start(&launch, "tool".into()).expect("starts");
        let answer = child
            .request("ping", json!({}), Duration::from_secs(10))
            .await
            .expect("answered");
        assert_eq!(answer, json!("ok"));
        child.close().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn starts_a_bare_program_from_the_childs_path() {
        let launch = Launch {
            program: "sh".into(),
            args: vec![
                "-c".into(),
                r#"read line; echo '{"jsonrpc":"2.0","id":1,"result":"ok"}'"#.into(),
            ],
            cwd: std::env::temp_dir(),
            env: vec![("PATH".into(), "/usr/bin:/bin".into())],
        };
        let child = Child::start(&launch, "sh".into()).expect("starts");
        let answer = child
            .request("ping", json!({}), Duration::from_secs(10))
            .await
            .expect("answered");
        assert_eq!(answer, json!("ok"));
        child.close().await;
    }

    #[test]
    fn a_secret_split_across_two_reads_is_still_taken_out() {
        let mut r = Redactor::new(&["s3cret-value".to_owned()]);
        let mut out = r.push(b"token s3cr", false);
        out.extend(r.push(b"et-value and more", false));
        out.extend(r.push(b" end\n", true));
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text, "token [redacted] and more end\n");
        // Nothing held once a line ends.
        assert_eq!(r.push(b"plain\n", true), b"plain\n");
    }

    #[test]
    fn redacts_every_secret_long_enough_to_mean_one() {
        let secrets = vec!["hunter22".to_owned(), "on".to_owned()];
        assert_eq!(
            redact("auth hunter22 then hunter22 on", &secrets),
            "auth [redacted] then [redacted] on"
        );
        assert!(matches!(
            redact("nothing here", &secrets),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_secret_the_child_prints_to_stderr_is_never_kept() {
        let launch = Launch {
            program: "sh".into(),
            args: vec![
                "-c".into(),
                "echo \"error: bad token s3cr3t-value\" >&2; exit 3".into(),
            ],
            cwd: std::env::temp_dir(),
            env: vec![("PATH".into(), "/usr/bin:/bin".into())],
        };
        let child = Child::start_redacting(&launch, "sh".into(), None, vec!["s3cr3t-value".into()])
            .expect("starts");
        let err = child
            .request("ping", json!({}), Duration::from_secs(10))
            .await
            .expect_err("it exits first");
        let message = format!("{err:?}");
        assert!(message.contains("[redacted]"), "{message}");
        assert!(!message.contains("s3cr3t-value"), "{message}");
    }
}
