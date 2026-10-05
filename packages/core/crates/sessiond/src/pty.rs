//! Terminals on macOS and Linux: the PTY pair is opened here and the program
//! started with a plain `Command`, so nothing runs in the child between fork
//! and exec but a few async-signal-safe calls.
//!
//! sessiond is multithreaded, and a forked child of a multithreaded process
//! may only make async-signal-safe calls until it execs: another thread may
//! have held the allocator's lock at the fork. So no descriptor is closed by
//! listing them in the child. Every descriptor sessiond opens is close-on-exec
//! from the moment it exists, and the ones it inherited are made so once at
//! startup ([`cloexec_inherited`]).

use std::ffi::{CStr, CString};
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

/// Signals whose disposition the child gets back at its default: one sessiond
/// ignores would otherwise stay ignored across exec.
const DEFAULT_SIGNALS: [libc::c_int; 6] = [
    libc::SIGCHLD,
    libc::SIGHUP,
    libc::SIGINT,
    libc::SIGQUIT,
    libc::SIGTERM,
    libc::SIGALRM,
];

/// `ptsname` answers in a buffer shared by the whole process.
static PTSNAME: Mutex<()> = Mutex::new(());

/// A terminal's master end. Dropping it closes the terminal.
pub struct Master {
    fd: OwnedFd,
}

impl Master {
    /// Set the terminal's size; the program hears SIGWINCH.
    pub fn resize(&self, cols: u16, rows: u16, px_w: u16, px_h: u16) -> io::Result<()> {
        set_size(&self.fd, cols, rows, px_w, px_h)
    }

    /// A second handle on the master for reading, close-on-exec like the first.
    pub fn reader(&self) -> io::Result<File> {
        Ok(File::from(self.fd.try_clone()?))
    }

    /// A second handle on the master for writing.
    pub fn writer(&self) -> io::Result<File> {
        Ok(File::from(self.fd.try_clone()?))
    }
}

/// Start `cmd` on a new terminal of `cols` x `rows`, as the leader of its own
/// session with the terminal as its controlling one. Its stdio is replaced.
pub fn spawn(mut cmd: Command, cols: u16, rows: u16) -> io::Result<(Master, Child)> {
    let (master, slave) = open()?;
    set_size(&master, cols, rows, 0, 0)?;
    cmd.stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave));
    // SAFETY: the closure runs in the child between fork and exec, and makes
    // only async-signal-safe calls: signal, setsid and ioctl. It allocates
    // nothing and takes no lock.
    unsafe {
        cmd.pre_exec(|| {
            for sig in DEFAULT_SIGNALS {
                libc::signal(sig, libc::SIG_DFL);
            }
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            // `as _`: the request's type differs between platforms.
            #[allow(clippy::cast_lossless)]
            if libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    // The child's copies of the slave are dropped with `cmd`, so only the
    // child holds it open and the master reads EOF once it and everything it
    // started have closed it.
    let child = cmd.spawn()?;
    Ok((Master { fd: master }, child))
}

/// Open a PTY pair, both ends close-on-exec from the start, so a program
/// another thread starts meanwhile inherits neither.
fn open() -> io::Result<(OwnedFd, OwnedFd)> {
    let flags = libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC;
    // SAFETY: a NUL-terminated path; the result is checked before use.
    let fd = unsafe { libc::open(c"/dev/ptmx".as_ptr(), flags) };
    if fd == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` was just opened and is owned by nothing else.
    let master = unsafe { OwnedFd::from_raw_fd(fd) };
    // SAFETY: grantpt and unlockpt on a master this function opened.
    if unsafe { libc::grantpt(master.as_raw_fd()) } == -1
        || unsafe { libc::unlockpt(master.as_raw_fd()) } == -1
    {
        return Err(io::Error::last_os_error());
    }
    let name = slave_name(&master)?;
    // SAFETY: a NUL-terminated path; the result is checked before use.
    let fd = unsafe { libc::open(name.as_ptr(), flags) };
    if fd == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: as above.
    let slave = unsafe { OwnedFd::from_raw_fd(fd) };
    Ok((master, slave))
}

fn slave_name(master: &OwnedFd) -> io::Result<CString> {
    let _only = PTSNAME.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the answer is copied out before the lock is let go, and no
    // other code in sessiond calls ptsname.
    let p = unsafe { libc::ptsname(master.as_raw_fd()) };
    if p.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: non-null, so a NUL-terminated string in ptsname's buffer.
    Ok(unsafe { CStr::from_ptr(p) }.to_owned())
}

fn set_size(fd: &OwnedFd, cols: u16, rows: u16, px_w: u16, px_h: u16) -> io::Result<()> {
    let ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: px_w,
        ws_ypixel: px_h,
    };
    // SAFETY: TIOCSWINSZ reads one winsize, which outlives the call.
    #[allow(clippy::cast_lossless)]
    if unsafe { libc::ioctl(fd.as_raw_fd(), libc::TIOCSWINSZ as _, &ws) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Make every descriptor above stderr that sessiond inherited close-on-exec,
/// so no program it starts inherits it. Call once, first thing in `main`,
/// while sessiond has one thread.
pub fn cloexec_inherited() {
    // macOS and Linux both list the process's descriptors here.
    let Ok(dir) = std::fs::read_dir("/dev/fd") else {
        return;
    };
    let fds: Vec<libc::c_int> = dir
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
        .filter(|&fd| fd > 2)
        .collect();
    for fd in fds {
        // SAFETY: fcntl on a descriptor number; one that was the listing's
        // own and is gone by now fails with EBADF, which is ignored.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags != -1 {
                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn sh(script: &str) -> Command {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(script);
        cmd
    }

    /// Everything the terminal prints until the program and all it started
    /// have closed it.
    fn read_all(master: &Master) -> String {
        let mut r = master.reader().unwrap();
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match r.read(&mut buf) {
                // Linux reports a closed slave as EIO.
                Ok(0) | Err(_) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
            }
        }
        String::from_utf8_lossy(&out).replace("\r\n", "\n")
    }

    #[test]
    fn the_program_leads_a_session_on_its_own_terminal() {
        let (master, mut child) = spawn(sh("read _; exit 5"), 80, 24).unwrap();
        let pid = child.id() as libc::pid_t;
        // SAFETY: plain queries on a pid this test started and a master it owns.
        let (sid, fg) = unsafe { (libc::getsid(pid), libc::tcgetpgrp(master.fd.as_raw_fd())) };
        assert_eq!(sid, pid, "its own session");
        assert_eq!(fg, pid, "the terminal is its controlling one");
        use std::io::Write;
        master.writer().unwrap().write_all(b"\n").unwrap();
        // macOS holds the program's exit until the terminal's echo of that
        // line is read.
        read_all(&master);
        assert_eq!(child.wait().unwrap().code(), Some(5));
    }

    #[test]
    fn stdio_is_the_terminal_at_its_size() {
        let (master, mut child) = spawn(
            sh("[ -t 0 ] && [ -t 1 ] && [ -t 2 ] && echo tty; stty size"),
            100,
            30,
        )
        .unwrap();
        let out = read_all(&master);
        child.wait().unwrap();
        assert!(out.contains("tty\n"), "{out:?}");
        assert!(out.contains("30 100"), "{out:?}");
    }

    #[test]
    fn resize_reaches_the_program() {
        let (master, mut child) = spawn(sh("read _; stty size"), 80, 24).unwrap();
        master.resize(132, 50, 0, 0).unwrap();
        use std::io::Write;
        master.writer().unwrap().write_all(b"\n").unwrap();
        let out = read_all(&master);
        child.wait().unwrap();
        assert!(out.contains("50 132"), "{out:?}");
    }

    #[test]
    fn no_terminal_descriptor_reaches_the_program() {
        // Another terminal open while this one starts: neither end may leak.
        let (other, mut other_child) = spawn(sh("read _"), 80, 24).unwrap();
        let theirs = other.fd.as_raw_fd();
        let (master, mut child) = spawn(
            sh(&format!(
                "for f in {theirs} $(( {theirs} + 1 )); do [ -e /dev/fd/$f ] && echo leaked $f; done; echo done"
            )),
            80,
            24,
        )
        .unwrap();
        let out = read_all(&master);
        child.wait().unwrap();
        assert!(out.contains("done") && !out.contains("leaked"), "{out:?}");
        drop(other);
        let _ = other_child.wait();
    }

    #[test]
    fn signals_sessiond_ignores_are_default_again() {
        // SAFETY: ignoring SIGHUP in this test process only for the spawn.
        let old = unsafe { libc::signal(libc::SIGHUP, libc::SIG_IGN) };
        let spawned = spawn(sh("kill -HUP $$; sleep 5"), 80, 24);
        // SAFETY: putting back what was there.
        unsafe { libc::signal(libc::SIGHUP, old) };
        let (_master, mut child) = spawned.unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGHUP));
    }
}
