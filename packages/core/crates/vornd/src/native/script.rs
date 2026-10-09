//! Project scripts, run on pipes in vornd's session holder (`script:execute`, [`execute`]).
//!
//! bash (but on Windows) and PowerShell read their program as they go, so
//! it is written to a file under the data directory, which goes when the
//! session ends ([`Scripts::ended`]); python and node read it whole from
//! stdin, which then closes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use serde_json::{json, Value};
use tracing::{info, warn};
use vorn_sessiond_wire::{Io, Sig, SpawnSpec, Stdin};

use super::headless::FORCE_KILL_DELAY;
use super::sessions::{set, Input, Started, Then, Watch};
use super::{agent, Answer, Native};

/// The call that runs a script.
pub const METHOD: &str = "script:execute";

/// How a script type is run, as the server's `interpreterFor` runs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Interpreter {
    Bash,
    PowerShell,
    Python,
    Node,
}

impl Interpreter {
    fn parse(script_type: &str) -> Option<Interpreter> {
        match script_type {
            "bash" => Some(Interpreter::Bash),
            "powershell" => Some(Interpreter::PowerShell),
            "python" => Some(Interpreter::Python),
            "node" => Some(Interpreter::Node),
            _ => None,
        }
    }

    /// The file the program is written to, where it must be one. On Windows
    /// bash.exe is usually the WSL launcher, which cannot open a Windows
    /// path, so bash keeps stdin there.
    fn file(self, windows: bool) -> Option<&'static str> {
        match self {
            Interpreter::Bash if !windows => Some("script.sh"),
            Interpreter::PowerShell => Some("script.ps1"),
            _ => None,
        }
    }

    /// The program and its arguments, with `file` where the program is read
    /// from, then `extra`.
    fn argv(self, windows: bool, file: &str, extra: &[String]) -> Vec<String> {
        let mut argv: Vec<String> = match self {
            Interpreter::Bash if windows => vec!["bash.exe".into(), "-s".into()],
            Interpreter::Bash => vec!["bash".into(), file.into()],
            Interpreter::PowerShell => vec![
                "pwsh".into(),
                "-NoProfile".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-File".into(),
                file.into(),
            ],
            Interpreter::Python if windows => vec!["python".into(), "-".into()],
            Interpreter::Python => vec!["python3".into(), "-".into()],
            Interpreter::Node => vec!["node".into(), "-".into()],
        };
        argv.extend(extra.iter().cloned());
        argv
    }
}

/// One script to run.
#[derive(Debug)]
struct Request {
    interpreter: Interpreter,
    /// Where it runs, resolved by [`execute`].
    cwd: String,
    args: Vec<String>,
    /// The step's secrets: values to run with.
    secrets: Vec<(String, String)>,
}

impl Request {
    fn read(params: &Value) -> Result<Request, String> {
        let text = |k| params.get(k).and_then(Value::as_str);
        let script_type = text("scriptType").unwrap_or_default();
        let interpreter = Interpreter::parse(script_type)
            .ok_or_else(|| format!("Unsupported script type: {script_type}"))?;
        let cwd = text("cwd")
            .filter(|c| !c.is_empty())
            .ok_or("a script needs a cwd")?
            .to_owned();
        let args = match params.get("args") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(a)) => a
                .iter()
                .map(|v| v.as_str().map(str::to_owned))
                .collect::<Option<_>>()
                .ok_or("a script's args are strings")?,
            Some(_) => return Err("a script's args are strings".to_owned()),
        };
        let secrets = match params.get("secretEnv") {
            Some(Value::Object(m)) => m
                .iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                .collect(),
            _ => Vec::new(),
        };
        Ok(Request {
            interpreter,
            cwd,
            args,
            secrets,
        })
    }
}

/// A script's own directory under the data directory, removed with it.
#[derive(Debug)]
struct ScriptDir(PathBuf);

impl ScriptDir {
    /// Writes `contents` to `name` in a new directory of `root`, readable by
    /// this account only.
    fn write(root: &Path, name: &str, contents: &str) -> std::io::Result<(ScriptDir, PathBuf)> {
        private_dir(root, true)?;
        let dir = ScriptDir(root.join(format!("vorn-script-{}", uuid::Uuid::new_v4().simple())));
        private_dir(&dir.0, false)?;
        let file = dir.0.join(name);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        std::io::Write::write_all(&mut options.open(&file)?, contents.as_bytes())?;
        Ok((dir, file))
    }
}

impl Drop for ScriptDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn private_dir(path: &Path, recursive: bool) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(recursive);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(path)
}

/// The scripts vornd runs.
#[derive(Debug)]
pub struct Scripts {
    /// Weak: the native side holds the link, which holds this.
    native: Weak<Native>,
    /// The files of the scripts running, by session id.
    files: Mutex<HashMap<String, Option<ScriptDir>>>,
}

impl Scripts {
    pub fn new(native: &Arc<Native>) -> Arc<Scripts> {
        Arc::new(Scripts {
            native: Arc::downgrade(native),
            files: Mutex::default(),
        })
    }

    fn lock_files(&self) -> std::sync::MutexGuard<'_, HashMap<String, Option<ScriptDir>>> {
        self.files.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Starts the script in `params` and hands `then` its start; an error is why nothing started.
    fn start(
        self: &Arc<Self>,
        params: &Value,
        watch: Option<Watch>,
        then: Then,
    ) -> Result<(), String> {
        let native = self.native.upgrade().ok_or("vornd is stopping")?;
        let host = native
            .host
            .get()
            .filter(|h| h.ready())
            .ok_or("vornd's session holder is not connected")?;
        let id = params
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or("a script needs an id")?
            .to_owned();
        let mut req = Request::read(params)?;
        if let Some(from) = params
            .get("secretsFrom")
            .and_then(Value::as_str)
            .filter(|f| !f.is_empty())
        {
            req.secrets = native.script_secrets(from);
        }
        let content = params
            .get("scriptContent")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let settings = agent::settings(&native).ok_or("vornd cannot read the settings")?;
        let data_dir = native.db.get().and_then(|db| db.parent());
        let mut env = native.env.launch(&settings.env_passthrough, data_dir);
        for (k, v) in req.secrets {
            set(&mut env, &k, v);
        }
        let windows = cfg!(windows);
        let (dir, argv, input) = match req.interpreter.file(windows) {
            Some(name) => {
                let root = data_dir.map_or_else(std::env::temp_dir, |d| d.join("scripts"));
                let (dir, file) = ScriptDir::write(&root, name, content)
                    .map_err(|e| format!("could not write the script: {e}"))?;
                let argv = req
                    .interpreter
                    .argv(windows, &file.to_string_lossy(), &req.args);
                (Some(dir), argv, Input::None)
            }
            None => (
                None,
                req.interpreter.argv(windows, "", &req.args),
                Input::Prompt(Some(content.as_bytes().to_vec())),
            ),
        };
        {
            let mut files = self.lock_files();
            if files.contains_key(&id) {
                return Err(format!("a script runs as {id} already"));
            }
            files.insert(id.clone(), dir);
        }
        let stdin = if input == Input::None {
            Stdin::Null
        } else {
            Stdin::Pipe
        };
        info!(%id, cwd = %req.cwd, line = %argv.join(" "), "running a script");
        let spec = SpawnSpec {
            argv,
            cwd: req.cwd,
            env,
            io: Io::Piped { stdin },
            ring_bytes: None,
        };
        native.sessions.lock_starting().insert(id.clone(), None);
        let (sessions, host_after) = (Arc::clone(&native.sessions), Arc::clone(host));
        let (files, name) = (Arc::downgrade(self), id.clone());
        let then: Then = Box::new(move |outcome: Result<Started, String>| {
            let pending = sessions.lock_starting().remove(&id).flatten();
            match &outcome {
                Ok(_) => {
                    if let Some(sig) = pending {
                        stop(host_after.as_ref(), &id, sig);
                    }
                }
                Err(why) => {
                    warn!(%id, %why, "vornd could not start this script");
                    if let Some(files) = files.upgrade() {
                        files.ended(&id);
                    }
                }
            }
            then(outcome);
        });
        match watch {
            Some(watch) => host.start_watched(spec, name, input, watch, then),
            None => host.start(spec, name, input, then),
        }
        Ok(())
    }

    /// Session `id` ended: a script's file goes with it.
    pub fn ended(&self, id: &str) {
        self.lock_files().remove(id);
    }

    /// Whether a script runs, or is starting, as `id`.
    pub fn runs(&self, id: &str) -> bool {
        self.lock_files().contains_key(id)
    }
}

/// Sends `sig`, and `SIGKILL` after [`FORCE_KILL_DELAY`] when it was `SIGTERM`.
fn stop(host: &dyn super::sessions::Host, id: &str, sig: Sig) {
    host.signal(id, sig);
    if sig == Sig::Term {
        host.signal_after(id, Sig::Kill, FORCE_KILL_DELAY);
    }
}

/// `script:execute`: runs a script in the session holder, tells its
/// output and end to every client by its `runId` (`script:data`,
/// `script:exit`), and answers `{success, output, error?, exitCode}` once
/// it ended.
pub async fn execute(native: &Arc<Native>, params: Value) -> Answer {
    let run = params
        .get("runId")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let tell = |method: &str, fields: Value| {
        if let Some(run) = &run {
            let mut note = json!({ "runId": run });
            if let (Some(n), Some(f)) = (note.as_object_mut(), fields.as_object()) {
                n.extend(f.clone());
            }
            native.broadcast(method, note);
        }
    };
    let fail = |message: String| {
        warn!(%message, "a script did not run");
        tell(
            "script:data",
            json!({ "data": format!("Error: {message}\n") }),
        );
        tell("script:exit", json!({ "exitCode": 1 }));
        Answer::Result(json!({ "success": false, "output": "", "error": message }))
    };
    let script_type = params
        .get("scriptType")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if Interpreter::parse(script_type).is_none() {
        let message = format!("Unsupported script type: {script_type}");
        return Answer::Result(json!({ "success": false, "output": "", "error": message }));
    }
    let Some(scripts) = native.link.get().and_then(|l| l.scripts()).cloned() else {
        return fail("vornd does not run scripts".to_owned());
    };
    let cwd = ["cwd", "projectPath"]
        .iter()
        .find_map(|k| {
            params
                .get(*k)
                .and_then(Value::as_str)
                .filter(|c| !c.is_empty())
        })
        .map(str::to_owned)
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|d| d.to_string_lossy().into_owned())
        })
        .unwrap_or_default();
    let id = format!("script-{}", uuid::Uuid::new_v4());
    let mut asked = json!({
        "id": id,
        "scriptType": script_type,
        "scriptContent": params.get("scriptContent").cloned().unwrap_or(json!("")),
        "cwd": cwd,
        "args": params.get("args").filter(|a| !a.is_null()).cloned().unwrap_or(json!([])),
    });
    if let Some(from) = params.get("secretsFrom").filter(|f| f.is_string()) {
        asked["secretsFrom"] = from.clone();
    }
    let (output_tx, mut output) = tokio::sync::mpsc::unbounded_channel();
    let (ended_tx, ended) = tokio::sync::oneshot::channel();
    let (started_tx, started) = tokio::sync::oneshot::channel();
    let watch = Watch {
        output: output_tx,
        ended: ended_tx,
    };
    let then: Then = Box::new(move |outcome| {
        let _ = started_tx.send(outcome);
    });
    // A run that resumes as vornd starts waits for the session holder, as the server's did.
    native.await_holder().await;
    let s = Arc::clone(&scripts);
    let ran = tokio::task::spawn_blocking(move || s.start(&asked, Some(watch), then)).await;
    match ran {
        Ok(Ok(())) => {}
        Ok(Err(why)) => return fail(why),
        Err(err) => return fail(err.to_string()),
    }
    match started.await {
        Ok(Ok(_)) => {}
        Ok(Err(why)) => return fail(why),
        Err(_) => return fail("vornd's session holder went away".to_owned()),
    }
    let mut text = Utf8::default();
    let mut printed = String::new();
    while let Some(bytes) = output.recv().await {
        let chunk = text.push(&bytes);
        if !chunk.is_empty() {
            tell("script:data", json!({ "data": chunk }));
            printed.push_str(&chunk);
        }
    }
    let rest = text.finish();
    if !rest.is_empty() {
        tell("script:data", json!({ "data": rest }));
        printed.push_str(&rest);
    }
    let code = ended.await.unwrap_or(1);
    scripts.ended(&id);
    tell("script:exit", json!({ "exitCode": code }));
    let mut answer = json!({ "success": code == 0, "output": printed, "exitCode": code });
    if code != 0 {
        answer["error"] = json!(if printed.is_empty() {
            format!("Exited with code {code}")
        } else {
            printed
        });
    }
    Answer::Result(answer)
}

/// Text from bytes that arrive in pieces: a character split between two
/// pieces is told whole with the second.
#[derive(Debug, Default)]
pub(crate) struct Utf8 {
    pending: Vec<u8>,
}

impl Utf8 {
    pub(crate) fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let whole = match std::str::from_utf8(&self.pending) {
            Ok(_) => self.pending.len(),
            // Only an incomplete character at the end waits; anything else is replaced now.
            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            Err(_) => self.pending.len(),
        };
        let rest = self.pending.split_off(whole);
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending = rest;
        text
    }

    fn finish(&mut self) -> String {
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        text
    }
}

#[cfg(test)]
mod tests {
    use super::super::sessions::tests::{fed, Fed};
    use super::*;

    #[test]
    fn tells_a_character_split_between_pieces_whole() {
        let mut text = Utf8::default();
        let e_acute = "é".as_bytes();
        assert_eq!(text.push(&[b'a', e_acute[0]]), "a");
        assert_eq!(text.push(&[e_acute[1], b'b']), "éb");
        assert_eq!(text.push(&[0xff, b'c']), "\u{fffd}c");
        assert_eq!(text.push(&[e_acute[0]]), "");
        assert_eq!(text.finish(), "\u{fffd}");
    }

    fn ready() -> (Fed, Arc<Scripts>, tempfile::TempDir) {
        let fed = fed();
        let data = tempfile::tempdir().unwrap();
        fed.native.set_database(data.path().join("vorn.db"));
        let scripts = Scripts::new(&fed.native);
        fed.link.set_scripts(Arc::clone(&scripts));
        (fed, scripts, data)
    }

    fn cwd() -> String {
        std::env::temp_dir().to_string_lossy().into_owned()
    }

    type Outcomes = Arc<Mutex<Vec<Result<Started, String>>>>;

    fn kept() -> (Outcomes, Then) {
        let outcomes: Outcomes = Arc::default();
        let into = Arc::clone(&outcomes);
        (outcomes, Box::new(move |o| into.lock().unwrap().push(o)))
    }

    #[test]
    fn a_bash_script_runs_from_a_private_file_that_goes_when_it_ends() {
        let (fed, scripts, data) = ready();
        let (outcomes, then) = kept();
        let params = json!({
            "id": "s1", "scriptType": "bash", "scriptContent": "echo hi", "cwd": cwd(),
            "args": ["a b"], "secretEnv": { "API_KEY": "k" },
        });
        scripts.start(&params, None, then).unwrap();
        let (spec, input) = fed.host.last_start();
        let (fed_in, stdin) = if cfg!(windows) {
            (Input::Prompt(Some(b"echo hi".to_vec())), Stdin::Pipe)
        } else {
            (Input::None, Stdin::Null)
        };
        assert_eq!(input, fed_in);
        assert_eq!(spec.io, Io::Piped { stdin });
        assert_eq!(spec.cwd, cwd());
        assert!(spec.env.contains(&("API_KEY".into(), "k".into())));
        let file = PathBuf::from(&spec.argv[1]);
        if cfg!(windows) {
            assert_eq!(spec.argv, ["bash.exe", "-s", "a b"]);
        } else {
            assert_eq!(spec.argv[0], "bash");
            assert_eq!(spec.argv[2], "a b");
            assert!(file.starts_with(data.path().join("scripts")), "{file:?}");
            assert_eq!(std::fs::read_to_string(&file).unwrap(), "echo hi");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
                assert_eq!(mode(&file), 0o600);
                assert_eq!(mode(file.parent().unwrap()), 0o700);
            }
        }
        fed.host.up(42);
        assert_eq!(
            outcomes.lock().unwrap().as_slice(),
            [Ok(Started { pid: 42, epoch: 1 })]
        );
        assert!(scripts.runs("s1"));
        scripts.ended("s1");
        assert!(!scripts.runs("s1"));
        assert!(!file.exists() || cfg!(windows));
        assert_eq!(fed.native.sessions.lock_starting().len(), 0);
    }

    #[test]
    fn python_and_node_read_the_script_from_stdin() {
        let (fed, scripts, _data) = ready();
        for (kind, program) in [("python", "python3"), ("node", "node")] {
            let id = format!("s-{kind}");
            let params = json!({
                "id": id, "scriptType": kind, "scriptContent": "print(1)", "cwd": cwd(),
            });
            scripts.start(&params, None, kept().1).unwrap();
            let (spec, input) = fed.host.last_start();
            let program = if cfg!(windows) && kind == "python" {
                "python"
            } else {
                program
            };
            assert_eq!(spec.argv, [program, "-"]);
            assert_eq!(spec.io, Io::Piped { stdin: Stdin::Pipe });
            assert_eq!(input, Input::Prompt(Some(b"print(1)".to_vec())));
        }
    }

    #[test]
    fn a_script_that_cannot_start_is_refused_and_leaves_nothing() {
        let (fed, scripts, data) = ready();
        let (outcomes, then) = kept();
        let params =
            json!({ "id": "s1", "scriptType": "bash", "scriptContent": "x", "cwd": cwd() });
        scripts.start(&params, None, then).unwrap();
        fed.host.down("no such directory");
        assert_eq!(
            outcomes.lock().unwrap().as_slice(),
            [Err("no such directory".to_owned())]
        );
        assert!(!scripts.runs("s1"));
        // Windows feeds bash on stdin and writes no file at all.
        let left = std::fs::read_dir(data.path().join("scripts")).map_or(0, |d| d.count());
        assert_eq!(left, 0);
    }

    #[test]
    fn what_cannot_run_is_refused_before_anything_starts() {
        let (fed, scripts, _data) = ready();
        let base = json!({ "id": "s1", "scriptType": "bash", "scriptContent": "x", "cwd": cwd() });
        let with = |k: &str, v: Value| {
            let mut p = base.clone();
            p[k] = v;
            p
        };
        for (params, why) in [
            (
                with("scriptType", json!("ruby")),
                "Unsupported script type: ruby",
            ),
            (with("cwd", json!("")), "a script needs a cwd"),
            (with("id", json!("")), "a script needs an id"),
            (with("args", json!([1])), "a script's args are strings"),
        ] {
            assert_eq!(scripts.start(&params, None, kept().1), Err(why.to_owned()));
        }
        scripts.start(&base, None, kept().1).unwrap();
        assert_eq!(
            scripts.start(&base, None, kept().1),
            Err("a script runs as s1 already".to_owned())
        );
        assert_eq!(fed.host.starts.lock().unwrap().len(), 1);
    }
}
