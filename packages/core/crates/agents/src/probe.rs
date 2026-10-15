//! Asking an installed CLI for its models (`probeProcess`), as its adapter's
//! `[agent.models.list]` says.
//!
//! A listing is either an exchange of JSON lines (an initialize request, a
//! session or a page of models, never a prompt) or one run whose output
//! lists the models. [`Probe`] is that exchange as a state machine over
//! lines, so it can be tested without a process; [`probe`] runs it against
//! one, bounded in time and output, with stderr drained and never returned
//! (it can carry credentials).

use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use crate::models::{
    discovery_arguments, line_choices, parse_choices, same, DiscoveryError, ModelChoice,
};
use crate::spec::{at, ModelList};
use crate::Agent;

/// How long a CLI has to list its models.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// The most a CLI may print before its answer is refused.
pub const MAX_OUTPUT_BYTES: usize = 2 * 1024 * 1024;

/// The process to ask.
#[derive(Clone, Debug)]
pub struct ProbeContext {
    /// The executable, already found on PATH.
    pub command: PathBuf,
    /// The configured arguments; only those that select models are used.
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// The whole environment; nothing is inherited.
    pub env: Vec<(String, String)>,
}

/// What a line of the CLI's answer calls for.
#[derive(Debug, PartialEq)]
pub enum Step {
    /// Nothing yet.
    Wait,
    /// Write these messages, one per line.
    Send(Vec<Value>),
    Done(Vec<ModelChoice>),
}

/// One exchange with one CLI.
#[derive(Debug)]
pub struct Probe {
    list: ModelList,
    cwd: String,
    /// The step whose reply is awaited.
    step: usize,
    /// The last message sent, which a next page sends again.
    last: Option<Value>,
    collected: Vec<ModelChoice>,
    cursors: HashSet<String>,
    lines: Option<regex::Regex>,
}

impl Probe {
    /// The exchange for `agent`, run in `cwd`; `None` for an agent with no
    /// way to list its models.
    pub fn new(agent: &Agent, cwd: &str) -> Option<Probe> {
        let list = agent.spec().models.as_ref()?.list.clone()?;
        // Checked when the adapter was read.
        let lines = list.lines.as_deref().and_then(|p| regex::Regex::new(p).ok());
        Some(Probe {
            list,
            cwd: cwd.to_owned(),
            step: 0,
            last: None,
            collected: Vec::new(),
            cursors: HashSet::new(),
            lines,
        })
    }

    /// The arguments the CLI is started with, after `configured`'s.
    pub fn arguments(&self, configured: &[String]) -> Vec<String> {
        let mut args = discovery_arguments(&self.list.keep, configured);
        args.extend(self.list.args.iter().cloned());
        args
    }

    /// The first messages, or none for a CLI that is only to read (whose
    /// stdin is closed at once).
    pub fn opening(&mut self) -> Option<Vec<Value>> {
        let first = self.list.steps.first()?;
        let sent: Vec<Value> = first.send.iter().map(|m| fill(m, &self.cwd)).collect();
        self.last = sent.last().cloned();
        Some(sent)
    }

    /// Whether the answer is the whole of stdout rather than JSON lines.
    pub fn reads_whole_output(&self) -> bool {
        self.list.steps.is_empty()
    }

    /// One line of stdout. Anything this cannot read, a refusal included,
    /// is [`DiscoveryError::Unsupported`], as every failure while reading a
    /// line is in the server.
    pub fn on_line(&mut self, line: &str) -> Result<Step, DiscoveryError> {
        let msg = match serde_json::from_str::<Value>(line) {
            Ok(v @ Value::Object(_)) => v,
            Ok(_) => Value::Object(Map::new()),
            Err(_) => return Err(DiscoveryError::Unsupported),
        };
        self.read(&msg).map_err(|_| DiscoveryError::Unsupported)
    }

    fn read(&mut self, msg: &Value) -> Result<Step, DiscoveryError> {
        let Some(step) = self.list.steps.get(self.step) else {
            return Ok(Step::Wait);
        };
        let sent_id = self.last.as_ref().and_then(|m| m.get("id")).cloned();
        let answers = step.reply.iter().all(|(path, want)| {
            let want = match want {
                Value::String(s) if s == "{id}" => sent_id.as_ref().unwrap_or(&Value::Null),
                other => other,
            };
            at(msg, path).is_some_and(|v| same(v, want))
        });
        if !answers {
            return Ok(Step::Wait);
        }
        if step.fail.as_deref().and_then(|f| at(msg, f)).is_some_and(truthy) {
            return Err(DiscoveryError::Unsupported);
        }
        let required = step
            .require
            .iter()
            .all(|(path, want)| at(msg, path).is_some_and(|v| same(v, want)));
        if !required {
            return Err(DiscoveryError::Unsupported);
        }
        let Some(models) = &step.models else {
            self.step += 1;
            let next = self.list.steps.get(self.step);
            let sent: Vec<Value> = next
                .map(|n| n.send.iter().map(|m| fill(m, &self.cwd)).collect())
                .unwrap_or_default();
            self.last = sent.last().cloned().or(self.last.take());
            return Ok(Step::Send(sent));
        };
        let listed = at(msg, models).unwrap_or(&Value::Null);
        let paging = step.next.as_deref().zip(step.cursor.as_deref());
        let page = parse_choices(&self.list.choice, listed)?;
        let Some((next, cursor_at)) = paging else {
            return Ok(Step::Done(page));
        };
        self.collected.extend(page);
        match at(msg, next).and_then(Value::as_str) {
            Some(cursor) if !cursor.is_empty() => {
                if !self.cursors.insert(cursor.to_owned()) {
                    return Err(DiscoveryError::Unsupported);
                }
                let mut again = self.last.clone().unwrap_or(Value::Null);
                let id = again.get("id").and_then(Value::as_u64).unwrap_or(0) + 1;
                again["id"] = Value::from(id);
                set(&mut again, cursor_at, Value::String(cursor.to_owned()));
                self.last = Some(again.clone());
                Ok(Step::Send(vec![again]))
            }
            _ => Ok(Step::Done(dedupe(std::mem::take(&mut self.collected)))),
        }
    }

    /// The CLI ended before it answered: only a listing that reads the
    /// whole output answers by ending, and only when it succeeded.
    pub fn on_exit(&self, success: bool, output: &str) -> Result<Vec<ModelChoice>, DiscoveryError> {
        match (&self.lines, success) {
            (Some(pattern), true) => Ok(line_choices(pattern, output)),
            _ => Err(DiscoveryError::Failed),
        }
    }
}

/// `message` with each `"{cwd}"` string the directory.
fn fill(message: &Value, cwd: &str) -> Value {
    match message {
        Value::String(s) if s == "{cwd}" => Value::String(cwd.to_owned()),
        Value::Array(items) => Value::Array(items.iter().map(|v| fill(v, cwd)).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), fill(v, cwd))).collect()),
        other => other.clone(),
    }
}

/// Sets the dotted `path` in `value`, making objects on the way.
fn set(value: &mut Value, path: &str, to: Value) {
    let mut at = value;
    let mut keys = path.split('.').peekable();
    while let Some(key) = keys.next() {
        if !at.is_object() {
            *at = Value::Object(Map::new());
        }
        let map = at.as_object_mut().expect("made an object above");
        if keys.peek().is_none() {
            map.insert(key.to_owned(), to);
            return;
        }
        at = map.entry(key).or_insert_with(|| Value::Object(Map::new()));
    }
}

fn truthy(v: &Value) -> bool {
    !matches!(v, Value::Null | Value::Bool(false))
        && v.as_str() != Some("")
        && v.as_f64() != Some(0.0)
}

/// Pages can repeat a model: its first place, its last value.
fn dedupe(choices: Vec<ModelChoice>) -> Vec<ModelChoice> {
    let mut out: Vec<ModelChoice> = Vec::new();
    for choice in choices {
        match out.iter_mut().find(|c| c.id == choice.id) {
            Some(slot) => *slot = choice,
            None => out.push(choice),
        }
    }
    out
}

/// What the process sent, as the reader thread saw it.
enum Out {
    Bytes(Vec<u8>),
    End,
}

/// Asks the CLI in `context` for `agent`'s models, blocking this thread
/// for up to [`PROBE_TIMEOUT`].
pub fn probe(context: &ProbeContext, agent: &Agent) -> Result<Vec<ModelChoice>, DiscoveryError> {
    let cwd = context.cwd.to_string_lossy();
    let mut probe = Probe::new(agent, &cwd).ok_or(DiscoveryError::Failed)?;
    let mut child = vorn_spawn::command(&context.command)
        .args(probe.arguments(&context.args))
        .current_dir(&context.cwd)
        .env_clear()
        .envs(context.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| DiscoveryError::CouldNotStart)?;
    let answer = converse(&mut child, &mut probe);
    // Whatever the answer, the process has no more to say.
    let _ = child.kill();
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    answer
}

fn converse(child: &mut Child, probe: &mut Probe) -> Result<Vec<ModelChoice>, DiscoveryError> {
    let deadline = Instant::now() + PROBE_TIMEOUT;
    if let Some(mut stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            let _ = std::io::copy(&mut stderr, &mut std::io::sink());
        });
    }
    let mut stdout = child.stdout.take().ok_or(DiscoveryError::CouldNotStart)?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = [0u8; 16 * 1024];
        loop {
            match stdout.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(Out::Bytes(buf[..n].to_vec())).is_err() {
                        return;
                    }
                }
            }
        }
        let _ = tx.send(Out::End);
    });
    let mut stdin = child.stdin.take();
    match probe.opening() {
        Some(first) => send(&mut stdin, &first)?,
        None => drop(stdin.take()),
    }
    let mut pending: Vec<u8> = Vec::new();
    let mut total = 0usize;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(Out::Bytes(bytes)) => {
                total += bytes.len();
                if total > MAX_OUTPUT_BYTES {
                    return Err(DiscoveryError::TooMuchOutput);
                }
                pending.extend_from_slice(&bytes);
                if probe.reads_whole_output() {
                    continue;
                }
                while let Some(end) = pending.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = pending.drain(..=end).collect();
                    let line = String::from_utf8_lossy(&line[..line.len() - 1]);
                    match probe.on_line(&line)? {
                        Step::Wait => {}
                        Step::Send(messages) => send(&mut stdin, &messages)?,
                        Step::Done(choices) => return Ok(choices),
                    }
                }
            }
            Ok(Out::End) | Err(RecvTimeoutError::Disconnected) => {
                let success = wait_until(child, deadline)?;
                return probe.on_exit(success, &String::from_utf8_lossy(&pending));
            }
            Err(RecvTimeoutError::Timeout) => return Err(DiscoveryError::TimedOut),
        }
    }
}

fn send(stdin: &mut Option<ChildStdin>, messages: &[Value]) -> Result<(), DiscoveryError> {
    let Some(pipe) = stdin.as_mut() else {
        return Err(DiscoveryError::ConnectionClosed);
    };
    for message in messages {
        let mut line = message.to_string();
        line.push('\n');
        pipe.write_all(line.as_bytes())
            .map_err(|_| DiscoveryError::ConnectionClosed)?;
    }
    pipe.flush().map_err(|_| DiscoveryError::ConnectionClosed)
}

/// Whether the process, whose output has ended, exited with 0 before the
/// deadline.
fn wait_until(child: &mut Child, deadline: Instant) -> Result<bool, DiscoveryError> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.success()),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => return Err(DiscoveryError::TimedOut),
            Err(_) => return Err(DiscoveryError::Failed),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::testing::agent;

    fn line(v: Value) -> String {
        v.to_string()
    }

    /// Initializes, then lists pages until no cursor is left.
    fn paged() -> Agent {
        agent(
            r#"
            [agent]
            id = "paged"
            name = "Paged"
            [agent.launch]
            command = "paged"
            [agent.models.list]
            args = ["serve", "--stdio"]
            keep = { paired = ["--profile"], flags = ["--oss"] }
            [[agent.models.list.steps]]
            send = [{ id = 1, method = "initialize" }]
            reply = { id = "{id}" }
            fail = "error"
            [[agent.models.list.steps]]
            send = [{ method = "initialized" }, { id = 2, method = "model/list", params = { limit = 100 } }]
            reply = { id = "{id}" }
            fail = "error"
            models = "result.data"
            next = "result.nextCursor"
            cursor = "params.cursor"
            [agent.models.list.choice]
            id = ["model", "id"]
            "#,
        )
    }

    /// Opens a session in the directory and reads the models it offers.
    fn session() -> Agent {
        agent(
            r#"
            [agent]
            id = "session"
            name = "Session"
            [agent.launch]
            command = "session"
            [agent.models.list]
            args = ["--rpc"]
            [[agent.models.list.steps]]
            send = [{ id = 1, method = "initialize" }]
            reply = { id = "{id}" }
            fail = "error"
            [[agent.models.list.steps]]
            send = [{ id = 2, method = "session/new", params = { cwd = "{cwd}" } }]
            reply = { id = "{id}" }
            fail = "error"
            models = "result.models.available"
            [agent.models.list.choice]
            id = ["modelId"]
            "#,
        )
    }

    /// One request, a typed reply that must succeed.
    fn control() -> Agent {
        agent(
            r#"
            [agent]
            id = "control"
            name = "Control"
            [agent.launch]
            command = "control"
            [agent.models.list]
            args = ["-p"]
            [[agent.models.list.steps]]
            send = [{ type = "control_request", request = { subtype = "initialize" } }]
            reply = { type = "control_response" }
            require = { "response.subtype" = "success" }
            models = "response.response.models"
            [agent.models.list.choice]
            id = ["value"]
            "#,
        )
    }

    /// Prints its models, one per line among other text.
    fn printed() -> Agent {
        agent(
            r#"
            [agent]
            id = "printed"
            name = "Printed"
            [agent.launch]
            command = "printed"
            [agent.models.list]
            args = ["models"]
            lines = '^[^\s/]+/\S+$'
            "#,
        )
    }

    #[test]
    fn initializes_follows_pages_and_never_starts_a_conversation() {
        let mut p = Probe::new(&paged(), "/project").unwrap();
        assert_eq!(p.opening().unwrap()[0]["method"], "initialize");
        let Step::Send(sent) = p.on_line(&line(json!({ "id": 1, "result": {} }))).unwrap() else {
            panic!("expected messages");
        };
        assert_eq!(sent[0], json!({ "method": "initialized" }));
        assert_eq!(
            sent[1],
            json!({ "id": 2, "method": "model/list", "params": { "limit": 100 } })
        );
        assert_eq!(
            p.on_line(&line(json!({ "id": 9, "result": {} }))).unwrap(),
            Step::Wait
        );
        let Step::Send(next) = p
            .on_line(&line(
                json!({ "id": 2, "result": { "data": [{ "model": "first" }], "nextCursor": "c" } }),
            ))
            .unwrap()
        else {
            panic!("expected the next page");
        };
        assert_eq!(next[0]["id"], 3);
        assert_eq!(next[0]["params"]["cursor"], "c");
        let done = p
            .on_line(&line(json!({ "id": 3, "result": { "data": [{ "model": "second" }, { "model": "first", "displayName": "First" }], "nextCursor": null } })))
            .unwrap();
        assert_eq!(
            done,
            Step::Done(vec![
                ModelChoice {
                    id: "first".into(),
                    label: "First".into(),
                    description: None
                },
                ModelChoice {
                    id: "second".into(),
                    label: "second".into(),
                    description: None
                },
            ])
        );
    }

    #[test]
    fn refuses_a_repeated_cursor_and_anything_unreadable() {
        let mut p = Probe::new(&paged(), "/p").unwrap();
        p.opening();
        p.on_line(&line(json!({ "id": 1, "result": {} }))).unwrap();
        p.on_line(&line(
            json!({ "id": 2, "result": { "data": [], "nextCursor": "c" } }),
        ))
        .unwrap();
        assert_eq!(
            p.on_line(&line(
                json!({ "id": 3, "result": { "data": [], "nextCursor": "c" } })
            )),
            Err(DiscoveryError::Unsupported)
        );
        let mut c = Probe::new(&control(), "/p").unwrap();
        c.opening();
        assert_eq!(c.on_line(""), Err(DiscoveryError::Unsupported));
        assert_eq!(c.on_line("[1]"), Ok(Step::Wait));
        assert_eq!(
            c.on_line(&line(
                json!({ "type": "control_response", "response": { "subtype": "error" } })
            )),
            Err(DiscoveryError::Unsupported)
        );
        let mut s = Probe::new(&session(), "/p").unwrap();
        s.opening();
        assert_eq!(
            s.on_line(&line(json!({ "id": 1, "error": { "code": 1 } }))),
            Err(DiscoveryError::Unsupported)
        );
    }

    #[test]
    fn reads_models_without_prompting() {
        let mut c = Probe::new(&control(), "/p").unwrap();
        c.opening();
        assert_eq!(
            c.on_line(&line(json!({ "type": "system" }))).unwrap(),
            Step::Wait
        );
        let done = c
            .on_line(&line(
                json!({ "type": "control_response", "response": { "subtype": "success",
                "response": { "models": [{ "value": "opus", "displayName": "Opus" }] } } }),
            ))
            .unwrap();
        assert!(matches!(done, Step::Done(ref m) if m.len() == 1 && m[0].id == "opus"));

        let mut p = Probe::new(&session(), "/proj").unwrap();
        p.opening();
        let Step::Send(sent) = p.on_line(&line(json!({ "id": 1, "result": {} }))).unwrap() else {
            panic!("expected session/new");
        };
        assert_eq!(sent[0]["method"], "session/new");
        assert_eq!(sent[0]["params"]["cwd"], "/proj");
        let done = p
            .on_line(&line(json!({ "id": 2, "result": { "models": { "available": [{ "modelId": "gpt" }] } } })))
            .unwrap();
        assert!(matches!(done, Step::Done(ref m) if m[0].id == "gpt"));
        assert!(Probe::new(&crate::testing::bare("none"), "/p").is_none());
    }

    #[test]
    fn starts_each_cli_in_its_listing_mode() {
        let configured = vec!["--profile".to_owned(), "w".to_owned(), "--yolo".to_owned()];
        let paged = Probe::new(&paged(), "/p").unwrap();
        assert_eq!(
            paged.arguments(&configured),
            ["--profile", "w", "serve", "--stdio"]
        );
        let mut printed = Probe::new(&printed(), "/p").unwrap();
        assert_eq!(printed.arguments(&configured), ["models"]);
        assert!(printed.opening().is_none());
        assert!(printed.reads_whole_output());
        assert!(printed.on_exit(false, "a/b").is_err());
        assert_eq!(printed.on_exit(true, "a/b\n").unwrap()[0].id, "a/b");
    }

    #[cfg(unix)]
    #[test]
    fn runs_a_cli_and_reads_its_answer() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("printed");
        std::fs::write(
            &script,
            "#!/bin/sh\necho 'noise'\necho 'p/m1'\necho 'p/m2'\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let env = vec![("PATH".to_owned(), "/usr/bin:/bin".to_owned())];
        let context = ProbeContext {
            command: script.clone(),
            args: Vec::new(),
            cwd: dir.path().to_path_buf(),
            env: env.clone(),
        };
        let ids: Vec<String> = probe(&context, &printed())
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(ids, ["p/m1", "p/m2"]);

        let failing = dir.path().join("control");
        std::fs::write(&failing, "#!/bin/sh\nexit 3\n").unwrap();
        std::fs::set_permissions(&failing, std::fs::Permissions::from_mode(0o755)).unwrap();
        let context = ProbeContext {
            command: failing,
            ..context
        };
        let err = probe(&context, &control()).unwrap_err();
        assert!(
            matches!(
                err,
                DiscoveryError::Failed | DiscoveryError::ConnectionClosed
            ),
            "{err:?}"
        );
        let missing = ProbeContext {
            command: dir.path().join("absent"),
            args: Vec::new(),
            cwd: dir.path().to_path_buf(),
            env,
        };
        assert_eq!(
            probe(&missing, &paged()),
            Err(DiscoveryError::CouldNotStart)
        );
    }
}
