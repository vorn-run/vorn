//! What the process leaves behind, so a script can branch on it.
//!
//! 3 is missing deliberately: the server exits with it when another server
//! holds its endpoint, and `vorn server serve` passes that code through.

/// How a command ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitCode {
    Ok,
    Failure,
    /// The command line was wrong.
    Usage,
    /// No server could be reached or started.
    Unreachable,
    /// The server process's own code, passed through.
    Passed(i32),
}

impl ExitCode {
    pub fn code(self) -> i32 {
        match self {
            ExitCode::Ok => 0,
            ExitCode::Failure => 1,
            ExitCode::Usage => 2,
            ExitCode::Unreachable => 4,
            ExitCode::Passed(code) => code,
        }
    }
}
