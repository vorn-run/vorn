//! Terminals on macOS and Linux: the PTY pair is opened here and the program
//! started on it with [`crate::spawn`], which never forks.
//!
//! Every descriptor sessiond opens is close-on-exec from the moment it
//! exists, and the ones it inherited are made so once at startup
//! ([`cloexec_inherited`]), so no program inherits another's terminal.

use std::ffi::{CStr, CString};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::Mutex;

use crate::spawn::{Program, Stdio};

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

    /// A second handle on the master, close-on-exec like the first: input
    /// is written through it, so it is polled for room apart from output.
    pub fn dup(&self) -> io::Result<OwnedFd> {
        self.fd.try_clone()
    }
}

/// A master another sessiond passed over.
impl From<OwnedFd> for Master {
    fn from(fd: OwnedFd) -> Self {
        Master { fd }
    }
}

impl AsRawFd for Master {
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        self.fd.as_raw_fd()
    }
}

/// Start `program` on a new terminal of `cols` x `rows`, as the leader of
/// its own session with the terminal as its controlling one. Its pid.
pub fn spawn(program: &Program, cols: u16, rows: u16) -> io::Result<(Master, libc::pid_t)> {
    let (master, slave, name) = open()?;
    set_size(&master, cols, rows, 0, 0)?;
    let pid = program.spawn(Stdio::Terminal(&name))?;
    // Held until the program has the terminal open, so the master never
    // reads the end of a terminal nobody opened yet. Now only the program
    // and what it starts hold it, and the master reads EOF once they all
    // closed it.
    drop(slave);
    #[cfg(target_os = "macos")]
    claimed(&master, pid);
    Ok((Master { fd: master }, pid))
}

/// Wait, briefly, until `pid` holds the terminal as its controlling one.
/// macOS returns from posix_spawn before the new session claims it (a few
/// ms later); a sessiond that died in between would leave the program
/// without the hangup that ends it.
#[cfg(target_os = "macos")]
fn claimed(master: &OwnedFd, pid: libc::pid_t) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    // SAFETY: tcgetpgrp on a master this module owns.
    while unsafe { libc::tcgetpgrp(master.as_raw_fd()) } != pid {
        if std::time::Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(std::time::Duration::from_micros(250));
    }
}

/// Open a PTY pair, both ends close-on-exec from the start, so a program
/// another thread starts meanwhile inherits neither.
fn open() -> io::Result<(OwnedFd, OwnedFd, CString)> {
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
    Ok((master, slave, name))
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

/// Open `/dev/null` on whichever of stdin, stdout and stderr sessiond was
/// started without, so no descriptor it makes later lands on one of them and
/// is mistaken for a program's stdio ([`crate::spawn`]).
pub fn stdio_open() {
    for fd in 0..3 {
        // SAFETY: fcntl on a descriptor number, and open of a fixed path,
        // which takes the lowest free number: the one found missing.
        unsafe {
            if libc::fcntl(fd, libc::F_GETFD) == -1 {
                libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
            }
        }
    }
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

    fn sh(script: &str) -> Program {
        Program::new(&["sh".into(), "-c".into(), script.into()], &[], None).unwrap()
    }

    fn spawn(p: Program, cols: u16, rows: u16) -> io::Result<(Master, Child)> {
        super::spawn(&p, cols, rows).map(|(m, pid)| (m, Child(pid)))
    }

    struct Child(libc::pid_t);

    impl Child {
        fn id(&self) -> u32 {
            self.0 as u32
        }

        fn wait(&mut self) -> io::Result<std::process::ExitStatus> {
            use std::os::unix::process::ExitStatusExt;
            let mut status = 0;
            // SAFETY: reaping the child this test started.
            if unsafe { libc::waitpid(self.0, &mut status, 0) } == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(std::process::ExitStatus::from_raw(status))
        }
    }

    fn writer(m: &Master) -> std::fs::File {
        std::fs::File::from(m.dup().unwrap())
    }

    /// Everything the terminal prints until the program and all it started
    /// have closed it.
    fn read_all(master: &Master) -> String {
        let mut r = writer(master);
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
        writer(&master).write_all(b"\n").unwrap();
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
        writer(&master).write_all(b"\n").unwrap();
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
