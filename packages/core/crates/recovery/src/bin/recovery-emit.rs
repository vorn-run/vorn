//! `recovery-emit <seed> <bytes> <chunk> <pause_ms>`: prints seeded terminal
//! output (`vorn_recovery::emit::output`) in pieces of about `chunk` bytes,
//! `pause_ms` apart, then stays alive until it is killed, so the session it
//! runs in neither ends nor goes quiet for a reason of its own. End-to-end
//! recovery tests run it in sessiond and compare what was recorded with
//! what it printed.

use std::process::ExitCode;
use std::time::Duration;

use vorn_recovery::emit;

fn main() -> ExitCode {
    let args = match emit::Args::parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("recovery-emit: {e}");
            return ExitCode::FAILURE;
        }
    };
    emit::enable_vt();
    emit::drain_stdin();
    if let Err(e) = emit::run(args, &mut std::io::stdout().lock()) {
        eprintln!("recovery-emit: {e}");
        return ExitCode::FAILURE;
    }
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
