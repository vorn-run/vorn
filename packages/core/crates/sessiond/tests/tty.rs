//! A terminal's program on macOS: it has the terminal as its controlling one
//! whatever it is, so it hears SIGWINCH on a resize, it starts with default
//! signals, and one that cannot be executed fails the spawn.

#![cfg(target_os = "macos")]

use std::io::Read;
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

use vorn_sessiond::pty::{self, Master};
use vorn_sessiond::spawn::Program;

fn program(argv: &[&str]) -> Program {
    let argv: Vec<String> = argv.iter().map(|s| (*s).to_owned()).collect();
    Program::new(&argv, &[("PATH".into(), "/usr/bin:/bin".into())], None).unwrap()
}

/// What the terminal prints until `want` shows or `patience` passes.
fn read_until(master: &Master, want: &str, patience: Duration) -> String {
    let mut r = std::fs::File::from(master.dup().unwrap());
    let fd = r.as_raw_fd();
    // SAFETY: fcntl on a descriptor this test owns.
    unsafe {
        libc::fcntl(
            fd,
            libc::F_SETFL,
            libc::fcntl(fd, libc::F_GETFL) | libc::O_NONBLOCK,
        )
    };
    let until = Instant::now() + patience;
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    while Instant::now() < until && !String::from_utf8_lossy(&out).contains(want) {
        match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(_) => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[test]
fn a_program_that_does_not_claim_its_terminal_still_hears_a_resize() {
    // dash, like zsh, never asks for a controlling terminal on its own.
    let dash = program(&[
        "/bin/dash",
        "-c",
        "trap 'echo winch' WINCH; echo ready; while :; do sleep 0.05; done",
    ]);
    let (master, pid) = pty::spawn(&dash, 80, 24).unwrap();
    // SAFETY: a plain query on a master this test owns.
    let fg = unsafe { libc::tcgetpgrp(master.as_raw_fd()) };
    assert_eq!(fg, pid, "the terminal is the program's controlling one");
    let ready = read_until(&master, "ready", Duration::from_secs(5));
    assert!(ready.contains("ready"), "{ready:?}");
    master.resize(100, 30, 0, 0).unwrap();
    let heard = read_until(&master, "winch", Duration::from_secs(5));
    // SAFETY: ending the program this test started.
    unsafe { libc::kill(pid, libc::SIGKILL) };
    let mut status = 0;
    // SAFETY: reaping it.
    unsafe { libc::waitpid(pid, &mut status, 0) };
    assert!(
        heard.contains("winch"),
        "the program heard no SIGWINCH: {heard:?}"
    );
}

/// Wait for `pid` and give its exit code.
fn wait(pid: libc::pid_t) -> i32 {
    let mut status = 0;
    // SAFETY: reaping a child this test started.
    unsafe { libc::waitpid(pid, &mut status, 0) };
    libc::WEXITSTATUS(status)
}

#[test]
fn a_program_starts_with_sigpipe_at_its_default() {
    // With SIGPIPE ignored, `yes` would complain of a broken pipe.
    let sh = program(&["/bin/sh", "-c", "yes 2>&1 | head -1; echo done"]);
    let (master, pid) = pty::spawn(&sh, 80, 24).unwrap();
    let out = read_until(&master, "done", Duration::from_secs(5));
    assert_eq!(wait(pid), 0);
    assert!(out.contains("done"), "{out:?}");
    assert!(!out.to_lowercase().contains("broken pipe"), "{out:?}");
}

#[test]
fn a_program_that_cannot_be_executed_fails_the_spawn() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("garbage");
    std::fs::write(&bin, [0xde, 0xad, 0xbe, 0xef, 0, 1, 2, 3]).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let p = program(&[bin.to_str().unwrap()]);
    let err = pty::spawn(&p, 80, 24).err().expect("the spawn fails");
    assert_eq!(err.raw_os_error(), Some(libc::ENOEXEC), "{err}");
}
