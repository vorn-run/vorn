//! Programs started through the spawn helper keep every guarantee a
//! program sessiond starts itself has, with a small descriptor table.
#![cfg(target_os = "linux")]

use std::io::{Read, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::ExitStatus;
use std::sync::OnceLock;

use vorn_sessiond::pty;
use vorn_sessiond::spawn::helper::{self, Helper};
use vorn_sessiond::spawn::{Program, Stdio};

const SESSIOND: &str = env!("CARGO_BIN_EXE_vorn-sessiond");

/// Many descriptors open in this process, as in a sessiond holding
/// hundreds of sessions, and the helper installed. Shared by every test.
fn crowded() {
    static HELD: OnceLock<Vec<OwnedFd>> = OnceLock::new();
    HELD.get_or_init(|| {
        let null = std::fs::File::open("/dev/null").unwrap();
        let held = (0..700)
            .map(|_| OwnedFd::from(null.try_clone().unwrap()))
            .collect();
        helper::install(Path::new(SESSIOND)).unwrap();
        let ours = std::fs::read_to_string("/proc/self/status").unwrap();
        assert!(status_field(&ours, "FDSize:") >= 700, "{ours}");
        held
    });
}

fn sh(script: &str) -> Program {
    Program::new(&["sh".into(), "-c".into(), script.into()], &[], None).unwrap()
}

fn wait(pid: libc::pid_t) -> ExitStatus {
    let mut status = 0;
    // SAFETY: reaping a child of this process, which CLONE_PARENT made it.
    assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
    ExitStatus::from_raw(status)
}

fn read_all(master: &pty::Master) -> String {
    let mut r = std::fs::File::from(master.dup().unwrap());
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match r.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
        }
    }
    String::from_utf8_lossy(&out).replace("\r\n", "\n")
}

fn status_field(out: &str, name: &str) -> u64 {
    out.lines()
        .find_map(|l| l.strip_prefix(name)?.trim().parse().ok())
        .unwrap_or_else(|| panic!("no {name} in {out:?}"))
}

#[test]
fn a_terminal_program_gets_a_small_table_its_own_session_and_terminal() {
    crowded();
    let (master, pid) = pty::spawn(
        &sh("grep FDSize /proc/self/status; ls -l /proc/self/fd; read _; exit 5"),
        80,
        24,
    )
    .unwrap();
    // SAFETY: plain queries on a pid this test started and a master it owns.
    let (sid, fg) = unsafe { (libc::getsid(pid), libc::tcgetpgrp(master.as_raw_fd())) };
    assert_eq!(sid, pid, "its own session");
    assert_eq!(fg, pid, "the terminal is its controlling one");
    std::fs::File::from(master.dup().unwrap())
        .write_all(b"\n")
        .unwrap();
    let out = read_all(&master);
    assert_eq!(wait(pid).code(), Some(5));
    assert!(status_field(&out, "FDSize:") <= 256, "{out}");
    let fds: Vec<&str> = out
        .lines()
        .filter(|l| l.contains(" -> ") && !l.contains("/proc/"))
        .filter_map(|l| l.split(" -> ").next()?.split_whitespace().last())
        .collect();
    assert_eq!(fds, ["0", "1", "2"], "{out}");
}

#[test]
fn piped_programs_start_in_their_directory_with_default_signals() {
    crowded();
    let dir = tempfile::tempdir().unwrap();
    let (mut out_r, out_w) = std::io::pipe().unwrap();
    // The helper itself ignores SIGPIPE, as every Rust program does.
    let p = Program::new(
        &[
            "sh".into(),
            "-c".into(),
            "pwd; grep -E 'FDSize|SigBlk' /proc/self/status >&2; read x; echo stdin=$x; kill -PIPE $$".into(),
        ],
        &[],
        Some(dir.path()),
    )
    .unwrap();
    let pid = p
        .spawn(Stdio::Pipes {
            stdin: None,
            stdout: out_w.as_raw_fd(),
            stderr: out_w.as_raw_fd(),
        })
        .unwrap();
    drop(out_w);
    let mut out = String::new();
    out_r.read_to_string(&mut out).unwrap();
    assert_eq!(wait(pid).signal(), Some(libc::SIGPIPE), "{out}");
    let real = std::fs::canonicalize(dir.path()).unwrap();
    assert!(out.contains(&*real.to_string_lossy()), "{out}");
    assert!(out.contains("stdin=\n"), "stdin is /dev/null: {out}");
    assert!(out.contains("SigBlk:\t0000000000000000"), "{out}");
    assert!(status_field(&out, "FDSize:") <= 256, "{out}");
}

#[test]
fn a_program_that_cannot_start_says_why() {
    crowded();
    let (_r, w) = std::io::pipe().unwrap();
    let p = Program::new(&["sh".into()], &[], Some(Path::new("/nonexistent/dir"))).unwrap();
    let err = p
        .spawn(Stdio::Pipes {
            stdin: None,
            stdout: w.as_raw_fd(),
            stderr: w.as_raw_fd(),
        })
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound, "{err}");
}

#[test]
fn a_helper_that_died_is_started_again() {
    let h = Helper::start(Path::new(SESSIOND)).unwrap();
    let first = h.pid().unwrap();
    let run = |h: &Helper| {
        let (mut r, w) = std::io::pipe().unwrap();
        let pid = h
            .spawn(
                &sh("echo hi"),
                Stdio::Pipes {
                    stdin: None,
                    stdout: w.as_raw_fd(),
                    stderr: w.as_raw_fd(),
                },
            )
            .map(|r| r.unwrap());
        drop(w);
        let mut out = String::new();
        r.read_to_string(&mut out).unwrap();
        pid.map(|pid| (wait(pid), out))
    };
    let (status, out) = run(&h).unwrap();
    assert!(status.success() && out == "hi\n", "{out}");
    // SAFETY: killing the helper this test started.
    unsafe { libc::kill(first as libc::pid_t, libc::SIGKILL) };
    std::thread::sleep(std::time::Duration::from_millis(100));
    // The first request after its death finds it gone; the next starts a new one.
    assert!(run(&h).is_none());
    let (status, out) = run(&h).unwrap();
    assert!(status.success() && out == "hi\n", "{out}");
    assert_ne!(h.pid(), Some(first));
}
