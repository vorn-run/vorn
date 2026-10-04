//! The subject process for real-kill tests: runs the reference engine for
//! the framed protocol on stdin and stdout (see `vorn_recovery::child`).
//! The driver kills it with SIGKILL or TerminateProcess at will.

use std::process::ExitCode;

fn main() -> ExitCode {
    match vorn_recovery::child::serve(std::io::stdin().lock(), std::io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("recovery-subject: {e}");
            ExitCode::FAILURE
        }
    }
}
