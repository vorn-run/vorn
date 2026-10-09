//! Helper processes that never open a console window.
//!
//! vornd and sessiond run without a console of their own on Windows, so every
//! console program they start (git, a shell, an agent's CLI, ssh) would get a
//! fresh, visible console window unless asked not to. Node's `windowsHide`
//! did this for the server it replaced; [`command`] and `tokio_command` do
//! it here. Everywhere else it is a no-op, so call sites need no `cfg`.
//!
//! Every helper process starts through them: a test in this crate fails on a
//! bare `Command::new` elsewhere. Not for ConPTY sessions or anything the user
//! is meant to see.

use std::ffi::OsStr;

/// No console window for a console program (`CREATE_NO_WINDOW`).
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// No console at all, not even an inherited one (`DETACHED_PROCESS`).
pub const DETACHED_PROCESS: u32 = 0x0000_0008;
/// A process group of its own, so a Ctrl-C meant for the parent misses it.
pub const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
/// Out of the parent's job, so closing the job does not end it.
pub const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

/// A [`std::process::Command`] for `program` that opens no console window.
pub fn command(program: impl AsRef<OsStr>) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    cmd.hidden();
    cmd
}

/// A [`tokio::process::Command`] for `program` that opens no console window.
#[cfg(feature = "tokio")]
pub fn tokio_command(program: impl AsRef<OsStr>) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(program);
    cmd.hidden();
    cmd
}

/// Start a command without a console window on Windows.
pub trait Hidden {
    /// Sets `CREATE_NO_WINDOW`, replacing any creation flags set before.
    fn hidden(&mut self) -> &mut Self {
        self.hidden_with(0)
    }

    /// [`Hidden::hidden`] plus the Windows creation `flags` given, since
    /// creation flags are set at once rather than added to. Windows ignores
    /// `CREATE_NO_WINDOW` beside [`DETACHED_PROCESS`], which has no console
    /// to show either.
    fn hidden_with(&mut self, flags: u32) -> &mut Self;
}

impl Hidden for std::process::Command {
    #[cfg(windows)]
    fn hidden_with(&mut self, flags: u32) -> &mut Self {
        use std::os::windows::process::CommandExt;
        self.creation_flags(CREATE_NO_WINDOW | flags)
    }

    #[cfg(not(windows))]
    fn hidden_with(&mut self, _: u32) -> &mut Self {
        self
    }
}

#[cfg(feature = "tokio")]
impl Hidden for tokio::process::Command {
    #[cfg(windows)]
    fn hidden_with(&mut self, flags: u32) -> &mut Self {
        self.creation_flags(CREATE_NO_WINDOW | flags)
    }

    #[cfg(not(windows))]
    fn hidden_with(&mut self, _: u32) -> &mut Self {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hidden_child_still_pipes_its_output() {
        let out = if cfg!(windows) {
            command("cmd").args(["/d", "/c", "echo hidden"]).output()
        } else {
            command("sh").args(["-c", "echo hidden"]).output()
        }
        .expect("the shell starts");
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hidden");
    }
}
