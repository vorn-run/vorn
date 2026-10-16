//! What the process leaves behind, so a script can branch on it.
//!
//! 3 means a target matched more than one thing. The server also exits with 3
//! when another server holds its endpoint, and `vorn server serve` passes that
//! code through; the two never come from the same command.

/// How a command ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitCode {
    Ok,
    Failure,
    /// The command line was wrong.
    Usage,
    /// No server could be reached or started.
    Unreachable,
    /// A session target matched more than one session.
    Ambiguous,
    /// The server process's own code, passed through.
    Passed(i32),
}

impl ExitCode {
    pub fn code(self) -> i32 {
        match self {
            ExitCode::Ok => 0,
            ExitCode::Failure => 1,
            ExitCode::Usage => 2,
            ExitCode::Ambiguous => 3,
            ExitCode::Unreachable => 4,
            ExitCode::Passed(code) => code,
        }
    }
}
