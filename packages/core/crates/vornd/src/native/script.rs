//! The server's project scripts, run in vornd's session holder
//! (`vornd:script`).
//!
//! The server stays the entry point (`script:execute`, which its workflows
//! call in process), and asks vornd on the app's channel to run each script
//! under a session id it chose and already follows. vornd builds the
//! process as the server's `executeScript` does, with the environment its
//! agents get ([`super::env::SafeEnv::launch`]) and the step's secrets
//! the server sends, and starts it on pipes in the session holder: its
//! output is the session's records and its end the session's exit effect,
//! which the server reads as it reads a headless agent's and tells clients
//! and the workflow waiting on it. The answer is the program's start, or
//! why vornd could not start it: then nothing ran, and the server runs it
//! itself.
//!
//! bash (but on Windows) and PowerShell read their program as they go, so
//! it is written to a file under the data directory, which goes when the
//! session ends ([`Scripts::ended`]); python and node read it whole from
//! stdin, which then closes. A cancel sends `SIGTERM`, and `SIGKILL`
//! [`FORCE_KILL_DELAY`] later if the script still runs.
//!
//! In shadow mode the server runs every script itself and sends vornd what
//! it started (`vornd:scriptPlan`): the arguments, the working directory and
//! the environment's names, which are compared with what vornd would have
//! started ([`Scripts::compare`]) and counted under `script:execute`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use serde_json::{json, Value};
use tracing::{info, warn};
use vorn_sessiond_wire::{Io, Sig, SpawnSpec, Stdin};

use super::headless::FORCE_KILL_DELAY;
use super::sessions::{set, Input, Started, Then};
use super::{agent, first_difference, Native};
use crate::groups::{Counted, Groups, Mode};

/// What a script's call is counted as, wherever the server was asked.
pub const METHOD: &str = "script:execute";

/// What stands for the script's file in a compared plan: each side writes
/// its own.
const FILE: &str = "<script>";

/// Whether a client's `method` is compared by the server's plan
/// (`vornd:scriptPlan`) rather than here as it passes.
pub fn compared_by_server(method: &str) -> bool {
    method == METHOD
}

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

/// One `vornd:script` or `vornd:scriptPlan`, as the server sends it.
#[derive(Debug)]
struct Request {
    interpreter: Interpreter,
    /// Where it runs: the server resolved it, as `executeScript` does.
    cwd: String,
    args: Vec<String>,
    /// The step's secrets: values to run with, or only their names for a
    /// plan.
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
            _ => strings(params.get("secretKeys"))
                .into_iter()
                .map(|k| (k, String::new()))
                .collect(),
        };
        Ok(Request {
            interpreter,
            cwd,
            args,
            secrets,
        })
    }
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// A script's own directory under the data directory, removed with it.
#[derive(Debug)]
struct ScriptDir(PathBuf);

impl ScriptDir {
    /// Writes `contents` to `name` in a new directory of `root`, readable by
    /// this account only, as the server's `scriptFileFor` does.
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

/// The scripts vornd runs for the server, in the mode its `script` group
/// is in.
#[derive(Debug)]
pub struct Scripts {
    mode: Mode,
    /// Weak: the native side holds the app's channel, which holds this.
    native: Weak<Native>,
    groups: Arc<Groups>,
    /// The files of the scripts running, by session id.
    files: Mutex<HashMap<String, Option<ScriptDir>>>,
}

impl Scripts {
    pub fn new(mode: Mode, native: &Arc<Native>, groups: Arc<Groups>) -> Arc<Scripts> {
        Arc::new(Scripts {
            mode,
            native: Arc::downgrade(native),
            groups,
            files: Mutex::default(),
        })
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    fn lock_files(&self) -> std::sync::MutexGuard<'_, HashMap<String, Option<ScriptDir>>> {
        self.files.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `vornd:script`: starts the script under the session id the server
    /// gave, then hands `then` its start or why it failed. An error is why
    /// nothing was started. Blocks while the environment is read.
    pub fn run(self: &Arc<Self>, params: &Value, then: Then) -> Result<(), String> {
        if self.mode != Mode::Native {
            return Err("vornd does not run scripts".to_owned());
        }
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
        let req = Request::read(params)?;
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
        self.groups.count(METHOD, Counted::Native);
        let (sessions, host_after) = (Arc::clone(&native.sessions), Arc::clone(host));
        let files = Arc::downgrade(self);
        host.start(
            spec,
            id.clone(),
            input,
            Box::new(move |outcome: Result<Started, String>| {
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
            }),
        );
        Ok(())
    }

    /// `vornd:scriptCancel`: the script is asked to stop, and made to a
    /// while later; one still starting is once it is up.
    pub fn cancel(&self, id: &str) -> Result<(), String> {
        if !self.lock_files().contains_key(id) {
            return Err(format!("no script runs as {id}"));
        }
        let native = self.native.upgrade().ok_or("vornd is stopping")?;
        if native.sessions.doom(id, Sig::Term) {
            return Ok(());
        }
        if let Some(host) = native.host.get() {
            stop(host.as_ref(), id, Sig::Term);
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

    /// `vornd:scriptPlan`: what the server started, compared with what vornd
    /// would have, and counted. Answers whether they are the same.
    pub fn compare(&self, params: &Value) -> Result<Value, String> {
        if self.mode != Mode::Shadow {
            return Err("vornd does not compare scripts".to_owned());
        }
        let theirs = params.get("plan").ok_or("a plan to compare")?;
        let ours = self.plan(params)?;
        if &ours == theirs {
            self.groups.count(METHOD, Counted::ShadowMatched);
            return Ok(json!({ "same": true }));
        }
        let at = first_difference(&ours, theirs);
        warn!(method = METHOD, differs_at = %at, "shadow plan differs from the server's");
        self.groups.count(METHOD, Counted::ShadowMismatched);
        Ok(json!({ "same": false, "differsAt": at }))
    }

    /// What vornd would start for a script, as `{argv, cwd, envKeys}`, its
    /// file [`FILE`] and the names sorted. Blocks while the environment is
    /// read.
    fn plan(&self, params: &Value) -> Result<Value, String> {
        let native = self.native.upgrade().ok_or("vornd is stopping")?;
        let req = Request::read(params)?;
        let settings = agent::settings(&native).ok_or("vornd cannot read the settings")?;
        let data_dir = native.db.get().and_then(|db| db.parent());
        let env = native.env.launch(&settings.env_passthrough, data_dir);
        Ok(plan_of(req, env))
    }
}

fn plan_of(req: Request, env: Vec<(String, String)>) -> Value {
    let mut keys: Vec<String> = env.into_iter().chain(req.secrets).map(|(k, _)| k).collect();
    keys.sort();
    keys.dedup();
    let argv = req.interpreter.argv(cfg!(windows), FILE, &req.args);
    json!({ "argv": argv, "cwd": req.cwd, "envKeys": keys })
}

/// Sends `sig`, and `SIGKILL` after [`FORCE_KILL_DELAY`] when it was `SIGTERM`.
fn stop(host: &dyn super::sessions::Host, id: &str, sig: Sig) {
    host.signal(id, sig);
    if sig == Sig::Term {
        host.signal_after(id, Sig::Kill, FORCE_KILL_DELAY);
    }
}

/// Answers the app's `vornd:script`, `vornd:scriptCancel` and
/// `vornd:scriptPlan`; a start is answered once its program is up, and its
/// file goes when the session's exit effect is in.
#[cfg(feature = "engine")]
pub fn call(
    engine: &Arc<crate::engine::Engine>,
    scripts: Option<&Arc<Scripts>>,
    fwd: &crate::streams::Forwarder,
    rpc: Option<Value>,
    method: &str,
    params: Value,
) {
    use crate::streams::{answer, refuse};
    let reply = {
        let fwd = fwd.clone();
        move |done: Result<Value, String>| {
            if let Some(rpc) = &rpc {
                match done {
                    Ok(v) => fwd.send_now(&answer(rpc, v)),
                    Err(e) => fwd.send_now(&refuse(rpc, &e)),
                }
            }
        }
    };
    let Some(scripts) = scripts.cloned() else {
        return reply(Err("vornd does not run scripts".to_owned()));
    };
    match method {
        "vornd:scriptCancel" => {
            let id = params.get("id").and_then(Value::as_str).unwrap_or_default();
            reply(scripts.cancel(id).map(|()| Value::Null));
        }
        "vornd:scriptPlan" => {
            tokio::task::spawn_blocking(move || reply(scripts.compare(&params)));
        }
        _ => {
            let events = engine.subscribe();
            let (engine, runtime) = (Arc::clone(engine), tokio::runtime::Handle::current());
            tokio::task::spawn_blocking(move || {
                let id = params
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let reply = Arc::new(Mutex::new(Some(reply)));
                let answered = Arc::clone(&reply);
                let after = Arc::clone(&scripts);
                let started = scripts.run(
                    &params,
                    Box::new(move |outcome| {
                        let reply = answered.lock().unwrap_or_else(|e| e.into_inner()).take();
                        let done = outcome.map(|s| {
                            runtime.spawn(clean_up(engine, events, after, id.clone()));
                            json!({ "id": id, "pid": s.pid, "epoch": s.epoch })
                        });
                        if let Some(reply) = reply {
                            reply(done);
                        }
                    }),
                );
                if let Err(why) = started {
                    if let Some(reply) = reply.lock().unwrap_or_else(|e| e.into_inner()).take() {
                        reply(Err(why));
                    }
                }
            });
        }
    }
}

/// Waits for script `id` to end, then lets its file go.
#[cfg(feature = "engine")]
async fn clean_up(
    engine: Arc<crate::engine::Engine>,
    mut events: tokio::sync::broadcast::Receiver<crate::engine::Event>,
    scripts: Arc<Scripts>,
    id: String,
) {
    use crate::engine::Event;
    use tokio::sync::broadcast::error::RecvError;
    loop {
        match events.recv().await {
            Ok(Event::Effect(fx, vorn_engine::Effect::Exit { .. })) if fx.session == id => break,
            Ok(Event::Closed(s)) if s.brief.session == id => break,
            Ok(_) => {}
            Err(RecvError::Lagged(_)) => {
                let held = engine.journal().held();
                if !held.iter().any(|h| h.session == id && h.exit.is_none()) {
                    break;
                }
            }
            Err(RecvError::Closed) => break,
        }
    }
    scripts.ended(&id);
}

#[cfg(test)]
mod tests {
    use super::super::sessions::tests::{fed, Fed};
    use super::*;

    fn ready(mode: Mode) -> (Fed, Arc<Scripts>, tempfile::TempDir) {
        let fed = fed();
        let data = tempfile::tempdir().unwrap();
        fed.native.set_database(data.path().join("vorn.db"));
        let groups = Arc::new(Groups::parse(&format!("script={mode}")).unwrap());
        let scripts = Scripts::new(mode, &fed.native, groups);
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
        let (fed, scripts, data) = ready(Mode::Native);
        let (outcomes, then) = kept();
        let params = json!({
            "id": "s1", "scriptType": "bash", "scriptContent": "echo hi", "cwd": cwd(),
            "args": ["a b"], "secretEnv": { "API_KEY": "k" },
        });
        scripts.run(&params, then).unwrap();
        let (spec, input) = fed.host.last_start();
        assert_eq!(input, Input::None);
        assert_eq!(spec.io, Io::Piped { stdin: Stdin::Null });
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
        let (fed, scripts, _data) = ready(Mode::Native);
        for (kind, program) in [("python", "python3"), ("node", "node")] {
            let id = format!("s-{kind}");
            let params = json!({
                "id": id, "scriptType": kind, "scriptContent": "print(1)", "cwd": cwd(),
            });
            scripts.run(&params, kept().1).unwrap();
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
        let (fed, scripts, data) = ready(Mode::Native);
        let (outcomes, then) = kept();
        let params =
            json!({ "id": "s1", "scriptType": "bash", "scriptContent": "x", "cwd": cwd() });
        scripts.run(&params, then).unwrap();
        fed.host.down("no such directory");
        assert_eq!(
            outcomes.lock().unwrap().as_slice(),
            [Err("no such directory".to_owned())]
        );
        assert!(!scripts.runs("s1"));
        let left = std::fs::read_dir(data.path().join("scripts"))
            .unwrap()
            .count();
        assert_eq!(left, 0);
    }

    #[test]
    fn what_cannot_run_is_refused_before_anything_starts() {
        let (fed, scripts, _data) = ready(Mode::Native);
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
            assert_eq!(scripts.run(&params, kept().1), Err(why.to_owned()));
        }
        scripts.run(&base, kept().1).unwrap();
        assert_eq!(
            scripts.run(&base, kept().1),
            Err("a script runs as s1 already".to_owned())
        );
        assert_eq!(fed.host.starts.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_cancel_stops_the_script_then_kills_it() {
        let (fed, scripts, _data) = ready(Mode::Native);
        let params = json!({ "id": "s1", "scriptType": "node", "scriptContent": "", "cwd": cwd() });
        scripts.run(&params, kept().1).unwrap();
        // Cancelled while it starts: told once it is up.
        scripts.cancel("s1").unwrap();
        assert!(fed.host.signals().is_empty());
        fed.host.up(7);
        assert_eq!(
            fed.host.signals(),
            [("s1".to_owned(), Sig::Term), ("s1".to_owned(), Sig::Kill)]
        );
        scripts.cancel("s1").unwrap();
        assert_eq!(fed.host.signals().len(), 4);
        scripts.ended("s1");
        assert_eq!(scripts.cancel("s1"), Err("no script runs as s1".to_owned()));
    }

    #[test]
    fn a_shadow_plan_is_compared_and_counted() {
        let (fed, _, _data) = ready(Mode::Native);
        let native_mode = Scripts::new(Mode::Native, &fed.native, Arc::new(Groups::all_forward()));
        assert!(native_mode.compare(&json!({})).is_err());
        let (_fed, scripts, _data) = ready(Mode::Shadow);
        assert_eq!(
            scripts.run(&json!({}), kept().1),
            Err("vornd does not run scripts".to_owned())
        );
        let params = json!({
            "scriptType": "python", "cwd": "/p", "args": ["x"], "secretKeys": ["TOKEN"],
        });
        let ours = scripts.plan(&params).unwrap();
        let program = if cfg!(windows) { "python" } else { "python3" };
        assert_eq!(ours["argv"], json!([program, "-", "x"]));
        let keys = strings(ours.get("envKeys"));
        assert!(keys.contains(&"TOKEN".to_owned()), "{keys:?}");
        assert!(keys.contains(&"VORN_DATA_DIR".to_owned()), "{keys:?}");
        assert!(keys.windows(2).all(|w| w[0] < w[1]), "{keys:?}");

        let mut same = params.clone();
        same["plan"] = ours.clone();
        assert_eq!(scripts.compare(&same).unwrap(), json!({ "same": true }));
        let mut other = params;
        let mut theirs = ours;
        theirs["cwd"] = json!("/q");
        other["plan"] = theirs;
        assert_eq!(
            scripts.compare(&other).unwrap(),
            json!({ "same": false, "differsAt": "/cwd" })
        );
        let counts = &scripts.groups.counts()["script"];
        assert_eq!((counts.shadow_matched, counts.shadow_mismatched), (1, 1));
    }

    #[test]
    fn a_bash_plan_names_its_file_the_same_on_both_sides() {
        let req = Request::read(&json!({ "scriptType": "bash", "cwd": "/p" })).unwrap();
        let plan = plan_of(req, vec![("PATH".into(), "/bin".into())]);
        let argv = if cfg!(windows) {
            json!(["bash.exe", "-s"])
        } else {
            json!(["bash", FILE])
        };
        assert_eq!(
            plan,
            json!({ "argv": argv, "cwd": "/p", "envKeys": ["PATH"] })
        );
        let ps = Request::read(&json!({ "scriptType": "powershell", "cwd": "/p" })).unwrap();
        assert_eq!(
            plan_of(ps, Vec::new())["argv"],
            json!([
                "pwsh",
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                FILE
            ])
        );
        assert!(compared_by_server(METHOD));
        assert!(!compared_by_server("script:other"));
    }
}
