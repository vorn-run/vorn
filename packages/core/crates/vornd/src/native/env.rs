//! The environment vornd runs git, `which` and IDEs with, as the server does.
//!
//! The server runs these with its "safe" environment: the login shell's,
//! once the shell has answered (`$SHELL -ilc env`), else its own, and either
//! way without credential-shaped variables or the markers an agent CLI leaves
//! behind. A packaged app has only the system directories on its PATH, so
//! without the shell's answer git would be whatever `/usr/bin/git` is, not
//! the one the person uses in their terminal. vornd asks the shell the same
//! question in the background when it starts, and filters by the same lists
//! (`vorn_agents::launch::env`).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tracing::{info, warn};
// Which names are dropped, and how PATH is searched, are the launch
// builders' rules, shared with them.
use vorn_agents::launch::env::{launch_env, safe_env};
use vorn_agents::launch::{find_on_path as find_launchable, Platform};

/// How long the login shell has to print its environment.
const SHELL_TIMEOUT: Duration = Duration::from_secs(15);

/// How long a shell that did not answer is left alone before it is asked again.
const SHELL_RETRY: Duration = Duration::from_secs(30);

/// The most the shell may print; past it the answer is not kept.
const SHELL_MAX_OUTPUT: usize = 4 * 1024 * 1024;

/// An environment, as name and value pairs.
pub type Env = Vec<(String, String)>;

#[derive(Debug)]
enum Shell {
    /// Not asked yet, or asked and it failed: ask again from this instant.
    Waiting(Instant),
    Asking,
    Answered(Arc<Env>),
}

/// The safe environment, and the login shell's answer once it has one.
#[derive(Debug)]
pub struct SafeEnv {
    shell: Mutex<Shell>,
}

impl Default for SafeEnv {
    fn default() -> Self {
        SafeEnv {
            shell: Mutex::new(Shell::Waiting(Instant::now())),
        }
    }
}

impl SafeEnv {
    pub fn new() -> Arc<SafeEnv> {
        Arc::new(SafeEnv::default())
    }

    fn shell(&self) -> MutexGuard<'_, Shell> {
        self.shell.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The environment to run a program with: the shell's once it answered,
    /// this process's until then, filtered either way. Asks the shell again
    /// when it is time to.
    pub fn get(self: &Arc<Self>) -> Env {
        safe_env(self.source())
    }

    /// The server's `getLaunchEnv`, for what an agent runs: [`SafeEnv::get`]
    /// but for the names the person passed through (`envPassthrough`), and
    /// with `VORN_DATA_DIR` naming the server's data directory.
    pub fn launch(self: &Arc<Self>, passthrough: &[String], data_dir: Option<&Path>) -> Env {
        let dir = data_dir.map(|d| d.to_string_lossy().into_owned());
        launch_env(self.source(), passthrough, dir.as_deref())
    }

    /// The shell's environment once it answered, this process's until then,
    /// unfiltered. Asks the shell again when it is time to.
    fn source(self: &Arc<Self>) -> Env {
        let (answered, ask) = {
            let mut shell = self.shell();
            match &*shell {
                Shell::Answered(env) => (Some(Arc::clone(env)), false),
                Shell::Waiting(at) if Instant::now() >= *at => {
                    *shell = Shell::Asking;
                    (None, true)
                }
                _ => (None, false),
            }
        };
        // Asked with the lock released: on Windows the answer is stored
        // before `ask` returns, under the same lock.
        if ask {
            self.ask();
        }
        match answered {
            Some(env) => env.as_ref().clone(),
            None => std::env::vars().collect(),
        }
    }

    /// Whether the login shell has answered; on Windows there is none to ask.
    pub fn resolved(&self) -> bool {
        cfg!(windows) || matches!(*self.shell(), Shell::Answered(_))
    }

    /// Whether the login shell is being asked now.
    pub fn asking(&self) -> bool {
        matches!(*self.shell(), Shell::Asking)
    }

    /// Starts asking the login shell, once.
    pub fn prime(self: &Arc<Self>) {
        let mut shell = self.shell();
        if matches!(*shell, Shell::Waiting(_)) {
            *shell = Shell::Asking;
            drop(shell);
            self.ask();
        }
    }

    /// Asks on a thread of its own; nothing waits for it.
    fn ask(self: &Arc<Self>) {
        if cfg!(windows) {
            *self.shell() = Shell::Answered(Arc::new(std::env::vars().collect()));
            return;
        }
        let this = Arc::clone(self);
        std::thread::spawn(move || {
            let started = Instant::now();
            let next = match login_shell_env() {
                Ok(env) => {
                    info!(
                        ms = started.elapsed().as_millis() as u64,
                        "login shell environment resolved"
                    );
                    Shell::Answered(Arc::new(env))
                }
                Err(err) => {
                    warn!(%err, "the login shell did not answer; programs get vornd's PATH until it does");
                    Shell::Waiting(Instant::now() + SHELL_RETRY)
                }
            };
            *this.shell() = next;
        });
    }

    /// Where `name` is on the safe environment's PATH, as the server's
    /// `resolveExecutable` finds it.
    pub fn find(self: &Arc<Self>, name: &str) -> Option<PathBuf> {
        let env = self.get();
        let path = env
            .iter()
            .find(|(k, _)| k == "PATH" || k == "Path")
            .map(|(_, v)| v.clone())
            .or_else(|| std::env::var("PATH").ok())?;
        find_on_path(name, &path)
    }

    /// The git to run: found on PATH, else the bare name.
    pub fn git_bin(self: &Arc<Self>) -> String {
        self.find("git")
            .map_or_else(|| "git".to_owned(), |p| p.to_string_lossy().into_owned())
    }
}

/// `$SHELL -ilc env`, with this process's filtered environment.
fn login_shell_env() -> Result<Env, String> {
    use std::io::Read;
    use std::process::Stdio;
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/zsh".to_owned());
    let mut child = vorn_spawn::command(&shell)
        .args(["-ilc", "env"])
        .env_clear()
        .envs(filter(std::env::vars()))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{shell}: {e}"))?;
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut limited = (&mut stdout).take(SHELL_MAX_OUTPUT as u64 + 1);
        let _ = limited.read_to_end(&mut out);
        out
    });
    let deadline = Instant::now() + SHELL_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{shell} did not answer within {SHELL_TIMEOUT:?}"));
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    let out = reader.join().map_err(|_| "the reader stopped")?;
    if !status.success() {
        return Err(format!("{shell} exited with {status}"));
    }
    if out.len() > SHELL_MAX_OUTPUT {
        return Err(format!(
            "{shell} printed more than {SHELL_MAX_OUTPUT} bytes"
        ));
    }
    Ok(parse_env_output(&String::from_utf8_lossy(&out)))
}

/// `NAME=value` lines; a line without `=` past its first character is skipped.
fn parse_env_output(output: &str) -> Env {
    let mut env: Env = Vec::new();
    for line in output.split('\n') {
        if let Some(at) = line.find('=').filter(|&at| at > 0) {
            let (key, value) = (&line[..at], &line[at + 1..]);
            // A later line for the same name wins, as it does in an object.
            match env.iter_mut().find(|(k, _)| k == key) {
                Some(slot) => slot.1 = value.to_owned(),
                None => env.push((key.to_owned(), value.to_owned())),
            }
        }
    }
    env
}

/// The server's `filterEnv` with nothing passed through.
pub fn filter(source: impl Iterator<Item = (String, String)>) -> Env {
    safe_env(source)
}

/// The first entry of `path_env` holding `name` that can be run
/// (`findOnPath`).
pub fn find_on_path(name: &str, path_env: &str) -> Option<PathBuf> {
    find_launchable(name, Some(path_env), Platform::HOST).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vorn_agents::launch::env::filter_env;

    fn pairs(list: &[(&str, &str)]) -> impl Iterator<Item = (String, String)> {
        list.iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn drops_credentials_and_agent_markers_whatever_their_case() {
        let kept = filter(pairs(&[
            ("PATH", "/bin"),
            ("HOME", "/h"),
            ("GITHUB_TOKEN", "x"),
            ("github_token_extra", "x"),
            ("SECRET_VORN_BOOTSTRAP_TOKEN", "x"),
            ("ClaudeCode", "1"),
            ("CLAUDE_CODE_SSE_PORT", "1"),
            ("ANTHROPIC_BASE_URL", "u"),
            ("ANTHROPIC_API_KEY", "k"),
        ]));
        let names: Vec<&str> = kept.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["PATH", "HOME", "ANTHROPIC_BASE_URL"]);
    }

    #[test]
    fn passes_through_only_what_was_named_and_never_the_stripped() {
        let passthrough = vec!["ANTHROPIC_API_KEY".to_owned(), "CLAUDECODE".to_owned()];
        let kept = filter_env(
            pairs(&[
                ("anthropic_api_key", "k"),
                ("GITHUB_TOKEN", "x"),
                ("CLAUDECODE", "1"),
                ("PATH", "/bin"),
            ]),
            &passthrough,
        );
        let names: Vec<&str> = kept.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["anthropic_api_key", "PATH"]);
    }

    #[test]
    fn reads_env_output_as_the_server_does() {
        let env = parse_env_output("A=1\nB=x=y\n=skip\nnoequals\nA=2\nEMPTY=\n");
        assert_eq!(
            env,
            [
                ("A".to_owned(), "2".to_owned()),
                ("B".to_owned(), "x=y".to_owned()),
                ("EMPTY".to_owned(), String::new())
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn finds_a_runnable_file_on_path() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let tool = dir.path().join("tool");
        std::fs::write(&tool, "#!/bin/sh\n").unwrap();
        let path = format!("/nonexistent: {} ", dir.path().display());
        assert_eq!(find_on_path("tool", &path), None);
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(find_on_path("tool", &path), Some(tool));
        assert_eq!(find_on_path("absent", &path), None);
    }
}
