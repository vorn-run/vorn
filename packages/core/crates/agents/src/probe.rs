//! Asking an installed CLI for its models (`probeProcess`).
//!
//! Each CLI has one invocation that lists models without starting a
//! conversation: Claude Code's stream-JSON initialize, Codex's app-server
//! `model/list`, Copilot's ACP `session/new` (never prompted) and
//! `opencode models`. [`Probe`] is that exchange as a state machine over
//! lines, so it can be tested without a process; [`probe`] runs it against
//! one, bounded in time and output, with stderr drained and never returned
//! (it can carry credentials).

use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use crate::models::{discovery_arguments, parse_choices, DiscoveryError, ModelChoice};
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
    agent: Agent,
    cwd: String,
    next_id: u64,
    collected: Vec<ModelChoice>,
    cursors: HashSet<String>,
}

impl Probe {
    /// The exchange for `agent`, run in `cwd`; `None` for an agent with no
    /// models to list.
    pub fn new(agent: Agent, cwd: &str) -> Option<Probe> {
        agent.selects_models().then(|| Probe {
            agent,
            cwd: cwd.to_owned(),
            next_id: 2,
            collected: Vec::new(),
            cursors: HashSet::new(),
        })
    }

    /// The arguments the CLI is started with, after `configured`'s.
    pub fn arguments(&self, configured: &[String]) -> Vec<String> {
        let mut args = discovery_arguments(self.agent, configured);
        let own: &[&str] = match self.agent {
            Agent::Claude => &[
                "-p",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                "--no-session-persistence",
                "--safe-mode",
                "--strict-mcp-config",
                "--tools",
                "",
            ],
            Agent::Codex => &["app-server", "--stdio"],
            Agent::Copilot => &["--acp", "--no-remote", "--no-remote-export"],
            Agent::OpenCode => {
                // Its listing takes none of the configured arguments.
                args.clear();
                &["models"]
            }
            Agent::Gemini => &[],
        };
        args.extend(own.iter().map(|s| (*s).to_owned()));
        args
    }

    /// The first message, or `None` for a CLI that is only to read (whose
    /// stdin is closed at once).
    pub fn opening(&self) -> Option<Value> {
        match self.agent {
            Agent::Claude => Some(json!({
                "type": "control_request",
                "request_id": "vorn-models",
                "request": { "subtype": "initialize" }
            })),
            Agent::Codex => Some(json!({
                "id": 1,
                "method": "initialize",
                "params": { "clientInfo": { "name": "vorn_model_catalog", "version": "1.0.0" } }
            })),
            Agent::Copilot => Some(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": 1,
                    "clientCapabilities": { "fs": { "readTextFile": false, "writeTextFile": false } }
                }
            })),
            Agent::OpenCode | Agent::Gemini => None,
        }
    }

    /// Whether the answer is the whole of stdout rather than JSON lines.
    pub fn reads_whole_output(&self) -> bool {
        self.agent == Agent::OpenCode
    }

    /// One line of stdout. Anything this cannot read, a refusal included,
    /// is [`DiscoveryError::Unsupported`], as every failure while reading a
    /// line is in the server.
    pub fn on_line(&mut self, line: &str) -> Result<Step, DiscoveryError> {
        let msg = match serde_json::from_str::<Value>(line) {
            Ok(Value::Object(msg)) => msg,
            Ok(_) => Map::new(),
            Err(_) => return Err(DiscoveryError::Unsupported),
        };
        self.read(&msg).map_err(|_| DiscoveryError::Unsupported)
    }

    fn read(&mut self, msg: &Map<String, Value>) -> Result<Step, DiscoveryError> {
        let id = msg.get("id").and_then(Value::as_f64);
        let failed = msg.get("error").is_some_and(truthy);
        let result = msg.get("result");
        match self.agent {
            Agent::Claude => {
                if msg.get("type").and_then(Value::as_str) != Some("control_response") {
                    return Ok(Step::Wait);
                }
                let response = msg.get("response");
                if field(response, "subtype").and_then(Value::as_str) != Some("success") {
                    return Err(DiscoveryError::Unsupported);
                }
                let models = field(field(response, "response"), "models").unwrap_or(&Value::Null);
                Ok(Step::Done(parse_choices(Agent::Claude, models)?))
            }
            Agent::Copilot => match id {
                Some(1.0) if failed => Err(DiscoveryError::Unsupported),
                Some(1.0) => Ok(Step::Send(vec![json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "session/new",
                    "params": { "cwd": self.cwd, "mcpServers": [] }
                })])),
                Some(2.0) if failed => Err(DiscoveryError::Unsupported),
                Some(2.0) => {
                    let models = field(field(result, "models"), "availableModels");
                    Ok(Step::Done(parse_choices(
                        Agent::Copilot,
                        models.unwrap_or(&Value::Null),
                    )?))
                }
                _ => Ok(Step::Wait),
            },
            Agent::Codex => {
                if id == Some(1.0) {
                    if failed {
                        return Err(DiscoveryError::Unsupported);
                    }
                    return Ok(Step::Send(vec![
                        json!({ "method": "initialized" }),
                        self.list_request(None),
                    ]));
                }
                if id != Some(self.next_id as f64) {
                    return Ok(Step::Wait);
                }
                if failed {
                    return Err(DiscoveryError::Unsupported);
                }
                let data = field(result, "data").unwrap_or(&Value::Null);
                self.collected.extend(parse_choices(Agent::Codex, data)?);
                match field(result, "nextCursor").and_then(Value::as_str) {
                    Some(cursor) if !cursor.is_empty() => {
                        if !self.cursors.insert(cursor.to_owned()) {
                            return Err(DiscoveryError::Unsupported);
                        }
                        self.next_id += 1;
                        Ok(Step::Send(vec![self.list_request(Some(cursor))]))
                    }
                    _ => Ok(Step::Done(dedupe(std::mem::take(&mut self.collected)))),
                }
            }
            Agent::OpenCode | Agent::Gemini => Ok(Step::Wait),
        }
    }

    fn list_request(&self, cursor: Option<&str>) -> Value {
        let mut params = json!({ "limit": 100, "includeHidden": false });
        if let Some(cursor) = cursor {
            params["cursor"] = Value::String(cursor.to_owned());
        }
        json!({ "id": self.next_id, "method": "model/list", "params": params })
    }

    /// The CLI ended before it answered: only `opencode models` answers by
    /// ending, and only when it succeeded.
    pub fn on_exit(&self, success: bool, output: &str) -> Result<Vec<ModelChoice>, DiscoveryError> {
        if self.agent == Agent::OpenCode && success {
            parse_choices(Agent::OpenCode, &Value::String(output.to_owned()))
        } else {
            Err(DiscoveryError::Failed)
        }
    }
}

fn field<'a>(value: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    value.and_then(Value::as_object).and_then(|o| o.get(key))
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
pub fn probe(context: &ProbeContext, agent: Agent) -> Result<Vec<ModelChoice>, DiscoveryError> {
    let cwd = context.cwd.to_string_lossy();
    let mut probe = Probe::new(agent, &cwd).ok_or(DiscoveryError::Failed)?;
    let mut child = Command::new(&context.command)
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
        Some(first) => send(&mut stdin, &[first])?,
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
    use super::*;

    fn line(v: Value) -> String {
        v.to_string()
    }

    #[test]
    fn initializes_codex_follows_pages_and_never_starts_a_thread() {
        let mut p = Probe::new(Agent::Codex, "/project").unwrap();
        assert_eq!(p.opening().unwrap()["method"], "initialize");
        let Step::Send(sent) = p.on_line(&line(json!({ "id": 1, "result": {} }))).unwrap() else {
            panic!("expected messages");
        };
        assert_eq!(sent[0], json!({ "method": "initialized" }));
        assert_eq!(
            sent[1],
            json!({ "id": 2, "method": "model/list", "params": { "limit": 100, "includeHidden": false } })
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
        let mut p = Probe::new(Agent::Codex, "/p").unwrap();
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
        let mut c = Probe::new(Agent::Claude, "/p").unwrap();
        assert_eq!(c.on_line(""), Err(DiscoveryError::Unsupported));
        assert_eq!(c.on_line("[1]"), Ok(Step::Wait));
        assert_eq!(
            c.on_line(&line(
                json!({ "type": "control_response", "response": { "subtype": "error" } })
            )),
            Err(DiscoveryError::Unsupported)
        );
    }

    #[test]
    fn reads_claude_and_copilot_models_without_prompting() {
        let mut c = Probe::new(Agent::Claude, "/p").unwrap();
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

        let mut p = Probe::new(Agent::Copilot, "/proj").unwrap();
        let Step::Send(sent) = p.on_line(&line(json!({ "id": 1, "result": {} }))).unwrap() else {
            panic!("expected session/new");
        };
        assert_eq!(sent[0]["method"], "session/new");
        assert_eq!(sent[0]["params"]["cwd"], "/proj");
        let done = p
            .on_line(&line(json!({ "id": 2, "result": { "models": { "availableModels": [{ "modelId": "gpt" }] } } })))
            .unwrap();
        assert!(matches!(done, Step::Done(ref m) if m[0].id == "gpt"));
        assert!(Probe::new(Agent::Gemini, "/p").is_none());
    }

    #[test]
    fn starts_each_cli_in_its_listing_mode() {
        let configured = vec!["--profile".to_owned(), "w".to_owned(), "--yolo".to_owned()];
        let codex = Probe::new(Agent::Codex, "/p").unwrap();
        assert_eq!(
            codex.arguments(&configured),
            ["--profile", "w", "app-server", "--stdio"]
        );
        let opencode = Probe::new(Agent::OpenCode, "/p").unwrap();
        assert_eq!(opencode.arguments(&configured), ["models"]);
        assert!(opencode.opening().is_none());
        assert!(opencode.on_exit(false, "a/b").is_err());
        assert_eq!(opencode.on_exit(true, "a/b\n").unwrap()[0].id, "a/b");
    }

    #[cfg(unix)]
    #[test]
    fn runs_a_cli_and_reads_its_answer() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("opencode");
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
        let ids: Vec<String> = probe(&context, Agent::OpenCode)
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(ids, ["p/m1", "p/m2"]);

        let failing = dir.path().join("claude");
        std::fs::write(&failing, "#!/bin/sh\nexit 3\n").unwrap();
        std::fs::set_permissions(&failing, std::fs::Permissions::from_mode(0o755)).unwrap();
        let context = ProbeContext {
            command: failing,
            ..context
        };
        let err = probe(&context, Agent::Claude).unwrap_err();
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
            probe(&missing, Agent::Codex),
            Err(DiscoveryError::CouldNotStart)
        );
    }
}
