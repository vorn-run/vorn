//! What every command that talks to a server shares: its arguments, its
//! output, the client, and a server to talk to, started if there is none.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::args::ClientArgs;
use crate::exit::ExitCode;
use crate::output::{is_plain, Io};
use crate::rpc::{CallError, Client, DataDir};

/// The name the desktop app already writes its server output to.
const SERVER_LOG_FILENAME: &str = "server.log";
const READY_TIMEOUT: Duration = Duration::from_secs(20);
const POLL: Duration = Duration::from_millis(150);

/// Everything a client command needs.
pub struct Context<'a> {
    pub io: &'a mut dyn Io,
    pub rpc: Client,
    pub args: ClientArgs,
    /// No colour: piped, redirected, or asked to go without.
    pub plain: bool,
}

impl<'a> Context<'a> {
    pub fn new(io: &'a mut dyn Io, args: ClientArgs) -> Context<'a> {
        let dir = DataDir::resolve(args.data_dir.as_deref());
        // A ceiling beyond what a u64 of milliseconds holds is no ceiling.
        let timeout = args.timeout_ms.map(|ms| ms.min(u64::MAX as f64) as u64);
        let plain = is_plain(io.is_tty());
        Context {
            io,
            rpc: Client::new(dir, timeout),
            args,
            plain,
        }
    }

    /// A usage error: the message, then the usage it breaks.
    pub fn usage(&mut self, message: &str, usage: &str) -> ExitCode {
        self.io.write_err(&format!("vorn: {message}\n\n{usage}"));
        ExitCode::Usage
    }

    /// What the server threw, as one line a person can act on.
    pub fn failed(&mut self, what: &str, err: impl std::fmt::Display) -> ExitCode {
        self.io.write_err(&format!("vorn: {what}: {err}\n"));
        ExitCode::Failure
    }

    /// A command's error: an ambiguous target exits 3 with its candidates, anything else 1.
    pub fn error(&mut self, what: &str, err: CommandError) -> ExitCode {
        match err {
            CommandError::Ambiguous { given, found } => {
                self.io
                    .write_err(&format!("vorn: {}", crate::target::describe(&given, &found)));
                if self.args.json {
                    let data = crate::target::describe_json(&given, &found);
                    self.io.write(&crate::js::as_json(&data));
                }
                ExitCode::Ambiguous
            }
            other => self.failed(what, other),
        }
    }

    /// A server to talk to, started if there is not one already. False when
    /// it could not be.
    pub async fn server(&mut self) -> bool {
        if self.rpc.is_running() {
            return true;
        }
        self.io.write_err("No server running, starting one.\n");

        // The flag wins over what discovery resolved, so the log this names and
        // the directory the server is told to use cannot disagree.
        let flag = self.args.data_dir.clone();
        let dir = flag
            .as_deref()
            .map(Path::new)
            .unwrap_or(self.rpc.data_dir().path())
            .to_owned();
        let log = dir.join(SERVER_LOG_FILENAME);
        if let Err(err) = start_detached(&dir, &log, flag.as_deref()) {
            self.io.write_err(&format!("vorn: {err}\n"));
            return false;
        }

        let deadline = Instant::now() + READY_TIMEOUT;
        while Instant::now() < deadline {
            tokio::time::sleep(POLL).await;
            if self.rpc.is_running() {
                return true;
            }
        }
        self.io.write_err(&format!(
            "The server did not come up within {}s. Its output is in {}.\n",
            READY_TIMEOUT.as_secs(),
            log.display()
        ));
        false
    }

    /// One call, with `--timeout` applied.
    pub async fn call(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, CallError> {
        self.rpc.call(method, params).await
    }

    /// Two calls at once, as `Promise.all` makes them. When both fail, the
    /// first one's error is the one reported: they time out at the same
    /// moment, and Node's timers fire in the order they were set.
    pub async fn call_both(
        &self,
        first: &str,
        second: &str,
    ) -> Result<(serde_json::Value, serde_json::Value), CallError> {
        let (a, b) = tokio::join!(self.call(first, None), self.call(second, None));
        Ok((a?, b?))
    }
}

/// Why a command could not do what it was asked, after the arguments parsed.
#[derive(Debug)]
pub enum CommandError {
    /// The server did not answer, or answered with an error.
    Call(CallError),
    /// The answer did not name what was asked for: `no session matches "x"`.
    Refused(String),
    /// The target matched several sessions; [`Context::error`] lists them.
    Ambiguous {
        given: String,
        found: Vec<(usize, crate::target::Candidate)>,
    },
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandError::Call(err) => write!(f, "{err}"),
            CommandError::Refused(message) => f.write_str(message),
            CommandError::Ambiguous { given, found } => {
                f.write_str(crate::target::describe(given, found).trim_end())
            }
        }
    }
}

impl std::error::Error for CommandError {}

impl From<CallError> for CommandError {
    fn from(err: CallError) -> Self {
        CommandError::Call(err)
    }
}

/// Why a server could not be started.
#[derive(Debug)]
enum StartError {
    Io(std::io::Error),
    NotFound(crate::launch::NotFound),
    /// A debug build kept off the default data directory.
    Refused(String),
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartError::Io(err) => write!(f, "could not start a server: {err}"),
            StartError::NotFound(err) => write!(f, "{err}"),
            StartError::Refused(why) => f.write_str(why),
        }
    }
}

/// Starts vornd as the server detached, so it outlives this command, logging
/// to the server log, which it rotates itself; what it prints outside the log
/// (a panic) goes to `server.stderr`. A rival started by a second `vorn` at
/// the same moment stands down by itself, so no lock is held here.
fn start_detached(dir: &Path, log: &Path, data_dir_flag: Option<&str>) -> Result<(), StartError> {
    let data_dir = data_dir_flag.map_or_else(|| dir.to_owned(), PathBuf::from);
    crate::launch::refuse_default(&data_dir).map_err(StartError::Refused)?;
    std::fs::create_dir_all(dir).map_err(StartError::Io)?;
    let err = std::fs::File::create(dir.join("server.stderr")).map_err(StartError::Io)?;

    let server = crate::launch::locate().map_err(StartError::NotFound)?;
    let mut command = server.command(&data_dir, None, None);
    command
        .arg("--log-file")
        .arg(log)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(err)
        .current_dir(dir);
    detach(&mut command);
    // Not waited for: it is meant to keep running after this process exits.
    command.spawn().map_err(StartError::Io)?;
    Ok(())
}

#[cfg(unix)]
fn detach(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    // Its own process group, so a Ctrl-C meant for this command does not reach it.
    command.process_group(0);
}

#[cfg(windows)]
fn detach(command: &mut std::process::Command) {
    use vorn_spawn::{Hidden, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS};
    command.hidden_with(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}
