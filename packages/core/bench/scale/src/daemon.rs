//! Starting the daemons under test and taking them down with every program
//! they run, whatever state a phase left them in.

use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStdout, Command};

use crate::error::Error;
use crate::procfs;

/// How long a daemon has to say it is ready.
const READY_WITHIN: Duration = Duration::from_secs(60);

/// A daemon this bench started, and the pipe it said it was ready on.
#[derive(Debug)]
pub struct Daemon {
    name: &'static str,
    child: Child,
    pub pid: u32,
    // Kept open so a late write to stdout does not kill the daemon.
    _stdout: BufReader<ChildStdout>,
}

impl Daemon {
    /// Starts `cmd` and waits for the first line it prints.
    pub async fn start(name: &'static str, mut cmd: Command) -> Result<(Daemon, String), Error> {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn()?;
        let pid = child
            .id()
            .ok_or_else(|| Error::Failed(format!("{name} exited at once")))?;
        let stdout = child.stdout.take().expect("stdout was piped");
        let mut stdout = BufReader::new(stdout);
        let mut line = String::new();
        let read = tokio::time::timeout(READY_WITHIN, stdout.read_line(&mut line))
            .await
            .map_err(|_| Error::Timeout(format!("{name} to start")))??;
        if read == 0 {
            let status = child.wait().await?;
            return Err(Error::Failed(format!(
                "{name} exited before it was ready: {status}"
            )));
        }
        let daemon = Daemon {
            name,
            child,
            pid,
            _stdout: stdout,
        };
        Ok((daemon, line.trim_end().to_owned()))
    }

    /// How it ended, if it has.
    pub fn exited(&mut self) -> Option<String> {
        let status = self.child.try_wait().ok()??;
        Some(format!("{} exited: {status}", self.name))
    }

    /// Kills it and everything it started.
    pub async fn stop(mut self) {
        kill_tree(self.pid);
        let _ = tokio::time::timeout(Duration::from_secs(30), self.child.wait()).await;
    }
}

/// `e`, with which of `daemons` had died by then: a holder that ran out
/// of threads ends this way, and that is the finding.
pub fn explain(e: Error, daemons: &mut [Daemon]) -> Error {
    let gone: Vec<String> = daemons.iter_mut().filter_map(Daemon::exited).collect();
    if gone.is_empty() {
        return e;
    }
    Error::Failed(format!("{e} ({})", gone.join("; ")))
}

/// SIGKILLs `pid` and its descendants. The tree is read first: once the
/// root is gone its children move to init and cannot be told apart.
pub fn kill_tree(pid: u32) {
    let below = procfs::descendants(&[pid]);
    for p in std::iter::once(pid).chain(below) {
        let Ok(p) = libc::pid_t::try_from(p) else {
            continue;
        };
        // SAFETY: kill only sends a signal; a pid that is gone gives ESRCH.
        unsafe {
            libc::kill(p, libc::SIGKILL);
        }
    }
}
