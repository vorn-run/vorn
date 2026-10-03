//! Git for Vorn's server, off its event loop.
//!
//! [`run`] takes the arguments the server would hand to `git` and returns what
//! git would print. A few read-only commands are answered in-process by gix,
//! but only where the answer is byte-for-byte what git prints and only when
//! nothing about the repository or the environment could make git answer
//! differently; everything else, and every failure, runs git itself. Either
//! way the call blocks the thread it runs on, which is the point: the caller is
//! a worker thread, never Node's.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

mod exec;
mod fast;

/// One git command, as the server would have run it with `execFileSync`.
#[derive(Clone, Debug)]
pub struct Request {
    /// The git executable, already resolved by the caller.
    pub bin: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// The whole environment git runs with; nothing is inherited.
    pub env: Vec<(String, String)>,
    pub timeout: Duration,
    /// Stdout past this many bytes is an error rather than a truncated answer.
    pub max_buffer: usize,
}

/// Who answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Gix,
    Git,
}

#[derive(Debug)]
pub struct Reply {
    pub stdout: String,
    pub engine: Engine,
}

#[derive(Debug)]
pub enum Error {
    /// git exited non-zero. Worded as Node words it, so a message the server
    /// shows (a failed checkout, a rejected push) reads the same on either path.
    Failed {
        command: String,
        stderr: String,
    },
    /// git could not be started at all, or its `cwd` does not exist.
    Spawn {
        bin: String,
        error: std::io::Error,
    },
    TimedOut {
        bin: String,
        after: Duration,
    },
    TooLarge {
        bin: String,
        limit: usize,
    },
}

/// Worded as `execFileSync` words its errors, which the server shows as they are.
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Failed { command, stderr } if stderr.is_empty() => {
                write!(f, "Command failed: {command}")
            }
            Error::Failed { command, stderr } => write!(f, "Command failed: {command}\n{stderr}"),
            Error::Spawn { bin, error } => match errno_name(error) {
                Some(code) => write!(f, "spawnSync {bin} {code}"),
                None => write!(f, "spawnSync {bin}: {error}"),
            },
            Error::TimedOut { bin, .. } => write!(f, "spawnSync {bin} ETIMEDOUT"),
            Error::TooLarge { bin, .. } => write!(f, "spawnSync {bin} ENOBUFS"),
        }
    }
}

fn errno_name(error: &std::io::Error) -> Option<&'static str> {
    use std::io::ErrorKind::*;
    Some(match error.kind() {
        NotFound => "ENOENT",
        PermissionDenied => "EACCES",
        NotADirectory => "ENOTDIR",
        _ => return None,
    })
}

impl std::error::Error for Error {}

/// Answers `req` as git would, from gix when that is exact, else from git.
pub fn run(req: &Request) -> Result<Reply, Error> {
    if let Some(stdout) = fast::answer(req) {
        if stdout.len() <= req.max_buffer {
            return Ok(Reply {
                stdout,
                engine: Engine::Gix,
            });
        }
    }
    exec::run(req).map(|stdout| Reply {
        stdout,
        engine: Engine::Git,
    })
}

/// Whether gix answers anything in this process. False on Windows, and when the
/// process environment steers git (a `GIT_DIR`, injected config, ...), which
/// gix would read too and git might read differently.
pub fn gix_answers_here() -> bool {
    fast::available()
}

/// Runs `req` with git itself, never gix. For comparing the two.
pub fn run_git(req: &Request) -> Result<String, Error> {
    exec::run(req)
}

/// `bin args...`, as Node prints a failed command.
fn command_line(req: &Request) -> String {
    std::iter::once(req.bin.as_str())
        .chain(req.args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}
