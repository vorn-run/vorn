//! The environment vornd runs git, `which` and IDEs with, as the server does.
//!
//! The server runs these with its "safe" environment: the login shell's,
//! once the shell has answered (`$SHELL -ilc env`), else its own, and either
//! way without credential-shaped variables or the markers an agent CLI leaves
//! behind. A packaged app has only the system directories on its PATH, so
//! without the shell's answer git would be whatever `/usr/bin/git` is, not
//! the one the person uses in their terminal. vornd asks the shell the same
//! question in the background when it starts, and filters by the same lists.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tracing::{info, warn};

/// How long the login shell has to print its environment.
const SHELL_TIMEOUT: Duration = Duration::from_secs(15);

/// How long a shell that did not answer is left alone before it is asked again.
const SHELL_RETRY: Duration = Duration::from_secs(30);

/// The most the shell may print; past it the answer is not kept.
const SHELL_MAX_OUTPUT: usize = 4 * 1024 * 1024;

/// Names stripped whatever the configuration says (`NEVER_BORROWED_ENV` and
/// the desktop's launch credential, `BOOTSTRAP_ENV_VAR`).
const STRIP_KEYS: &[&str] = &["CLAUDECODE"];
const STRIP_PREFIXES: &[&str] = &["CLAUDE_CODE_", "SECRET_VORN_BOOTSTRAP_TOKEN"];

/// Credential-shaped names (`SENSITIVE_ENV_PREFIXES`), never handed to git.
const SENSITIVE_PREFIXES: &[&str] = &[
    "AWS_SECRET",
    "AWS_SESSION",
    "GITHUB_TOKEN",
    "GH_TOKEN",
    "OPENAI_API",
    "ANTHROPIC_API",
    "GOOGLE_API",
    "STRIPE_",
    "DATABASE_URL",
    "DB_PASSWORD",
    "SECRET_",
    "PRIVATE_KEY",
    "NPM_TOKEN",
    "NODE_AUTH_TOKEN",
];

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
            Some(env) => filter(env.iter().cloned()),
            None => filter(std::env::vars()),
        }
    }

    /// Whether the login shell has answered; on Windows there is none to ask.
    pub fn resolved(&self) -> bool {
        cfg!(windows) || matches!(*self.shell(), Shell::Answered(_))
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
    use std::process::{Command, Stdio};
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/zsh".to_owned());
    let mut child = Command::new(&shell)
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
    source
        .filter(|(key, _)| {
            let upper = key.to_uppercase();
            !(STRIP_KEYS.contains(&upper.as_str())
                || STRIP_PREFIXES.iter().any(|p| upper.starts_with(p))
                || SENSITIVE_PREFIXES.iter().any(|p| upper.starts_with(p)))
        })
        .collect()
}

/// The first entry of `path_env` holding `name` that can be run.
fn find_on_path(name: &str, path_env: &str) -> Option<PathBuf> {
    let sep = if cfg!(windows) { ';' } else { ':' };
    let candidates: Vec<String> = if cfg!(windows) {
        vec![
            format!("{name}.exe"),
            format!("{name}.cmd"),
            name.to_owned(),
        ]
    } else {
        vec![name.to_owned()]
    };
    path_env
        .split(sep)
        .map(str::trim)
        .filter(|dir| !dir.is_empty())
        .flat_map(|dir| candidates.iter().map(move |c| Path::new(dir).join(c)))
        .find(|full| runnable(full))
}

#[cfg(unix)]
fn runnable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn runnable(path: &Path) -> bool {
    path.exists()
}

#[cfg(test)]
mod tests {
    use super::*;

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
