//! The quick tier: the manifest's argv, run as the person, over stdio.
//!
//! One-shot: a process per call, the request as one JSON object on stdin
//! (`{"method","args","ctx"}`), the answer as JSON on stdout. An error is a
//! non-zero exit (its stderr the message) or `{"error":{"code","message"}}`.
//!
//! Service: one long-lived process speaking JSON-RPC 2.0, a message per line,
//! both ways. Vorn calls `guest.handle`; the extension may call `net.fetch`,
//! `host.call` and `host.log` back while it answers, and those go through the
//! same [`Gate`] a component's imports do. The names are the WIT's
//! (`interface.function`) and the params its parameter names.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::bridge::{FetchRequest, Gate};
use crate::ExtError;

/// What one call is about: the WIT's `call-ctx`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CallCtx {
    pub project: String,
    pub caller: String,
}

/// What to run, where, and with which environment.
#[derive(Debug, Clone)]
pub struct Program {
    pub argv: Vec<String>,
    /// The extension's folder.
    pub dir: PathBuf,
    /// Added to the environment the process inherits.
    pub env: Vec<(String, String)>,
}

impl Program {
    fn spawn(&self, ctx: &CallCtx) -> Result<Child, ExtError> {
        let (program, args) = self
            .argv
            .split_first()
            .ok_or_else(|| ExtError::new("manifest", "runtime.command is empty"))?;
        let mut cmd = vorn_spawn::command(program);
        cmd.args(args)
            // The extension's own folder, so `bash run.sh` and `cargo run` find their files.
            .current_dir(&self.dir)
            .env("VORN_PROJECT", &ctx.project)
            .env(
                "VORN_CONTEXT_JSON",
                serde_json::to_string(ctx).unwrap_or_default(),
            )
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd.spawn()
            .map_err(|e| ExtError::new("spawn", format!("cannot run {program}: {e}")))
    }
}

/// Runs the argv once for one call and kills it at `deadline`.
pub fn one_shot(
    program: &Program,
    method: &str,
    args: &Value,
    ctx: &CallCtx,
    deadline: Duration,
) -> Result<Value, ExtError> {
    let mut child = program.spawn(ctx)?;
    let request = json!({ "method": method, "args": args, "ctx": ctx });
    if let Some(mut stdin) = child.stdin.take() {
        // A script that never reads its stdin closes the pipe; that is not an error.
        let _ = stdin.write_all(request.to_string().as_bytes());
    }
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    let status = wait(&mut child, deadline)?;
    let stdout = out.recv().unwrap_or_default();
    let stderr = err.recv().unwrap_or_default();
    if !status.success() {
        let tail = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("");
        return Err(ExtError::new(
            "failed",
            format!("{} exited with {status}: {tail}", program.argv.join(" ")),
        ));
    }
    let value: Value = serde_json::from_str(stdout.trim())
        .map_err(|e| ExtError::new("bad-output", format!("stdout is not one JSON value: {e}")))?;
    match value
        .as_object()
        .and_then(|o| o.get("error"))
        .filter(|_| value.as_object().is_some_and(|o| o.len() == 1))
    {
        Some(e) => Err(serde_json::from_value(e.clone())
            .unwrap_or_else(|_| ExtError::new("failed", e.to_string()))),
        None => Ok(value),
    }
}

fn drain(pipe: Option<impl Read + Send + 'static>) -> Receiver<String> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut s = String::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_string(&mut s);
        }
        let _ = tx.send(s);
    });
    rx
}

fn wait(child: &mut Child, deadline: Duration) -> Result<std::process::ExitStatus, ExtError> {
    let start = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| ExtError::new("failed", e.to_string()))?
        {
            return Ok(status);
        }
        if start.elapsed() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ExtError::new(
                "timeout",
                format!("no answer within {deadline:?}"),
            ));
        }
        thread::sleep(Duration::from_millis(2));
    }
}

/// A long-lived extension process: one call at a time, any number of bridge
/// calls back while it answers.
pub struct Service {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    next: u64,
}

impl Service {
    /// Starts the process; `ctx` names the project it starts in.
    pub fn start(program: &Program, ctx: &CallCtx) -> Result<Service, ExtError> {
        let mut child = program.spawn(ctx)?;
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        if let Some(stderr) = child.stderr.take() {
            // The extension's own logging; kept off vornd's stdout.
            thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    eprintln!("[ext] {line}");
                }
            });
        }
        let (tx, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Service {
            child,
            stdin,
            lines,
            next: 1,
        })
    }

    /// Whether the process is still there.
    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// `guest.handle`, answering the extension's bridge calls through `gate`.
    pub fn call(
        &mut self,
        method: &str,
        args: &Value,
        ctx: &CallCtx,
        gate: &Gate<'_>,
        deadline: Duration,
    ) -> Result<Value, ExtError> {
        let id = self.next;
        self.next += 1;
        self.send(&json!({
            "jsonrpc": "2.0", "id": id, "method": "guest.handle",
            "params": { "method": method, "args": args, "ctx": ctx }
        }))?;
        let until = Instant::now() + deadline;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            let line = match self.lines.recv_timeout(left) {
                Ok(line) => line,
                Err(RecvTimeoutError::Timeout) => {
                    return Err(ExtError::new(
                        "timeout",
                        format!("no answer within {deadline:?}"),
                    ))
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(ExtError::new("exited", "the extension's process ended"))
                }
            };
            let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                return Err(ExtError::new("bad-output", format!("not JSON-RPC: {line}")));
            };
            match msg.get("method").and_then(Value::as_str) {
                // A bridge call from the extension, answered before its own answer arrives.
                Some(m) => {
                    let answer = bridge(gate, m, &msg["params"]);
                    if let Some(rid) = msg.get("id") {
                        self.send(&reply(rid, answer))?;
                    }
                }
                None if msg["id"] == json!(id) => {
                    return match msg.get("error") {
                        Some(e) => Err(ExtError::new(
                            e["data"]["code"].as_str().unwrap_or("failed"),
                            e["message"].as_str().unwrap_or("the extension failed"),
                        )),
                        None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                    };
                }
                None => {}
            }
        }
    }

    fn send(&mut self, msg: &Value) -> Result<(), ExtError> {
        let mut line = msg.to_string();
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|()| self.stdin.flush())
            .map_err(|e| ExtError::new("exited", format!("cannot write to the extension: {e}")))
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// One bridge call from a service, by its WIT name.
fn bridge(gate: &Gate<'_>, method: &str, params: &Value) -> Result<Value, ExtError> {
    match method {
        "net.fetch" => {
            let req: FetchRequest = serde_json::from_value(params["req"].clone())
                .map_err(|e| ExtError::new("bad-request", format!("net.fetch: {e}")))?;
            let res = gate.fetch(&req)?;
            Ok(serde_json::to_value(res).unwrap_or(Value::Null))
        }
        "host.call" => {
            let args: Value = match &params["args"] {
                Value::String(s) => serde_json::from_str(s).unwrap_or(Value::Null),
                other => other.clone(),
            };
            gate.call(params["method"].as_str().unwrap_or(""), &args)
        }
        "host.log" => {
            eprintln!(
                "[ext {}] {}",
                gate.manifest.id,
                params["message"].as_str().unwrap_or("")
            );
            Ok(Value::Null)
        }
        other => Err(ExtError::new(
            "unknown-method",
            format!("the bridge has no {other}"),
        )),
    }
}

fn reply(id: &Value, answer: Result<Value, ExtError>) -> Value {
    match answer {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id,
            "error": { "code": -32000, "message": e.message, "data": { "code": e.code } } }),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::bridge::{FetchResponse, World};
    use crate::manifest::Manifest;

    struct Echo;
    impl World for Echo {
        fn fetch(&self, req: &FetchRequest) -> Result<FetchResponse, ExtError> {
            Ok(FetchResponse {
                status: 200,
                headers: vec![],
                body: req.url.clone(),
            })
        }
        fn call(&self, _: &str, _: &Value, _: &str) -> Result<Value, ExtError> {
            Ok(json!("main"))
        }
        fn credential(&self, _: &Manifest) -> Result<Option<String>, ExtError> {
            Ok(None)
        }
    }

    fn sh(script: &str) -> Program {
        Program {
            argv: vec!["sh".into(), "-c".into(), script.into()],
            dir: std::env::temp_dir(),
            env: vec![],
        }
    }

    fn ctx() -> CallCtx {
        CallCtx {
            project: String::new(),
            caller: "test".into(),
        }
    }

    #[test]
    fn a_one_shot_reads_its_request_and_answers_json() {
        let p = sh(r#"read req; echo "[{\"id\":\"$(echo "$req" | cut -c12-15)\"}]""#);
        let v = one_shot(&p, "rows", &json!({}), &ctx(), Duration::from_secs(5)).unwrap();
        assert_eq!(v, json!([{ "id": "rows" }]));
    }

    #[test]
    fn a_one_shot_failure_carries_stderr_and_a_hang_times_out() {
        let e = one_shot(
            &sh("echo 'gh: not logged in' >&2; exit 4"),
            "rows",
            &json!({}),
            &ctx(),
            Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(e.message.ends_with("gh: not logged in"), "{e}");
        let e = one_shot(
            &sh("sleep 5"),
            "rows",
            &json!({}),
            &ctx(),
            Duration::from_millis(100),
        )
        .unwrap_err();
        assert_eq!(e.code, "timeout");
        let e = one_shot(
            &sh(r#"echo '{"error":{"code":"denied","message":"no"}}'"#),
            "rows",
            &json!({}),
            &ctx(),
            Duration::from_secs(5),
        )
        .unwrap_err();
        assert_eq!(e.code, "denied");
    }

    #[test]
    fn a_service_calls_the_bridge_while_it_answers() {
        let m = Manifest::parse(
            "id = \"x\"\nname = \"X\"\nversion = \"1\"\napi = \"vorn:extension@0.9\"\nnetwork = [\"api.github.com\"]\nruntime = { command = [\"sh\"], service = true }\n",
        )
        .unwrap();
        let gate = Gate {
            manifest: &m,
            world: &Echo,
            project: "",
        };
        // Asks for one fetch, then answers with what came back.
        let script = r#"while read req; do
            echo '{"jsonrpc":"2.0","id":"g1","method":"net.fetch","params":{"req":{"method":"GET","url":"https://api.github.com/x"}}}'
            read fetched
            body=$(echo "$fetched" | sed 's/.*"body":"\([^"]*\)".*/\1/')
            echo "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"$body\"}"
        done"#;
        let mut s = Service::start(&sh(script), &ctx()).unwrap();
        let v = s
            .call("rows", &json!({}), &ctx(), &gate, Duration::from_secs(5))
            .unwrap();
        assert_eq!(v, json!("https://api.github.com/x"));
    }
}
