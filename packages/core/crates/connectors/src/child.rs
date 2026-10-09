//! One child spoken to in Vorn's connector protocol: a JSON-RPC message per
//! line on its stdin and stdout, its stderr kept as a short tail for the
//! messages a failure gives.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::path::PathBuf;
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
}

impl CallError {
    pub fn code(&self) -> Option<i64> {
        match self {
            CallError::Answered { code, .. } => Some(*code),
            CallError::Transport(_) => None,
        }
    }
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallError::Answered { message, .. } | CallError::Transport(message) => {
                f.write_str(message)
            }
        }
    }
}

impl std::error::Error for CallError {}

type Reply = oneshot::Sender<Result<Value, CallError>>;

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
    stdin: tokio::sync::Mutex<Option<ChildStdin>>,
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
        let mut command = vorn_spawn::tokio_command(&launch.program);
        command
            .args(&launch.args)
            .current_dir(&launch.cwd)
            .env_clear()
            .envs(launch.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(false);
        let mut child = command.spawn().map_err(|err| {
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

        let stderr_done = tokio::spawn(read_stderr(stderr, name.clone(), Arc::clone(&state)));
        let stdout_done = tokio::spawn(read_stdout(
            stdout,
            name.clone(),
            Arc::clone(&state),
            signals.clone(),
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
            let _ = exit_tx.send(Some(ended));
        });

        Ok(Child {
            name,
            stdin: tokio::sync::Mutex::new(Some(stdin)),
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
        let mut line = serde_json::to_vec(
            &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        )
        .unwrap_or_default();
        line.push(b'\n');
        if let Some(stdin) = self.stdin.lock().await.as_mut() {
            if let Err(err) = stdin.write_all(&line).await {
                warn!("{} stdin: {err}", self.name);
            }
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => Err(CallError::Transport(format!(
                "{} was stopped before it answered {method}",
                self.name
            ))),
            Err(_) => {
                lock(&self.state).pending.remove(&id);
                Err(CallError::Transport(format!(
                    "{} did not answer {method} within {} s",
                    self.name,
                    timeout.as_secs_f64().round()
                )))
            }
        }
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
        drop(self.stdin.lock().await.take());
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

/// Why a start failed, worded as Node words it.
fn spawn_reason(program: &std::path::Path, err: &std::io::Error) -> String {
    let program = program.display();
    match err.kind() {
        std::io::ErrorKind::NotFound => format!("spawn {program} ENOENT"),
        std::io::ErrorKind::PermissionDenied => format!("spawn {program} EACCES"),
        _ => format!("spawn {program} {err}"),
    }
}

async fn read_stderr(stderr: tokio::process::ChildStderr, name: String, state: Arc<Mutex<State>>) {
    let mut reader = BufReader::new(stderr);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        // Bounded, so a child that never ends a line cannot grow it for ever.
        let read = (&mut reader)
            .take(MAX_STDERR_LINE as u64 + 1)
            .read_until(b'\n', &mut buf)
            .await;
        match read {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let text = String::from_utf8_lossy(&buf);
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
        on_line(&name, &state, line);
    }
}

fn on_line(name: &str, state: &Mutex<State>, line: &str) {
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        warn!(
            "{name} wrote a line that is not JSON: {}",
            js::slice16(line, 200)
        );
        return;
    };
    let Some(frame) = message.as_object() else {
        warn!(
            "{name} wrote a line that is not a message: {}",
            js::slice16(line, 200)
        );
        return;
    };
    if let Some(method) = frame.get("method").and_then(Value::as_str) {
        warn!("{name} sent {method}, which a connector does not send in protocol 1");
        return;
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
        return;
    };
    let answer = match frame.get("error") {
        Some(error) => Err(call_error(&method, error)),
        None => Ok(frame.get("result").cloned().unwrap_or(Value::Null)),
    };
    let _ = reply.send(answer);
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
}
