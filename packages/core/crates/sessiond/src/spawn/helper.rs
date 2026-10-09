//! The spawn helper: a process sessiond starts at launch, from its own
//! binary, that starts every program for it on Linux.
//!
//! A child gets a descriptor table sized by its parent's highest open
//! descriptor, and exec closes the descriptors but keeps the table. The
//! helper holds a handful, so a program started from it costs the kernel a
//! few hundred bytes of table instead of sessiond's half a megabyte.
//!
//! sessiond sends each program over a socket, with the terminal's slave or
//! the pipe ends beside it (`SCM_RIGHTS`). The helper is single-threaded, so
//! it can fork; it forks with `CLONE_PARENT`, which makes sessiond the
//! program's parent: sessiond reaps it and watches it with a pidfd exactly as
//! it does a program it started itself. The child leads a new session, takes
//! the terminal as its controlling one, puts back the default signals, and
//! execs; an exec that fails says why over a close-on-exec pipe.
//!
//! The helper exits when sessiond's end of the socket closes, which happens
//! when sessiond does: nothing else holds it.

use std::ffi::{CStr, CString};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{Program, Stdio, DEFAULT_SIGNALS};
use crate::fdpass::{self, Receiver};
use crate::wire::{Message, WireError};

/// The argument that makes the sessiond binary the helper.
pub const ARG: &str = "--spawn-helper";

/// How long sessiond waits for the helper to start one program before it
/// gives up on that helper. An exec reads the program from disk, so this is
/// generous.
const PATIENCE: Duration = Duration::from_secs(30);

static INSTALLED: OnceLock<Helper> = OnceLock::new();

/// Start programs through a helper run from `exe` from now on. Call once, at
/// launch; a later call keeps the first helper.
pub fn install(exe: &Path) -> io::Result<()> {
    if INSTALLED.get().is_none() {
        let _ = INSTALLED.set(Helper::start(exe)?);
    }
    Ok(())
}

/// Start `program` through the installed helper. `None` when there is none
/// or it was gone before it heard the request: start it some other way.
pub(super) fn spawn(program: &Program, stdio: Stdio<'_>) -> Option<io::Result<libc::pid_t>> {
    INSTALLED.get()?.spawn(program, stdio)
}

/// A helper process and the socket to it, started again after it fails.
pub struct Helper {
    exe: PathBuf,
    link: Mutex<Option<Link>>,
}

struct Link {
    rx: Receiver,
    child: Child,
}

impl Drop for Link {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Link {
    fn start(exe: &Path) -> io::Result<Link> {
        let (ours, theirs) = UnixStream::pair()?;
        ours.set_read_timeout(Some(PATIENCE))?;
        let child = vorn_spawn::command(exe)
            .arg(ARG)
            .stdin(std::process::Stdio::from(OwnedFd::from(theirs)))
            .stdout(std::process::Stdio::null())
            .spawn()?;
        Ok(Link {
            rx: Receiver::new(ours),
            child,
        })
    }
}

impl Helper {
    /// Start a helper from `exe`, the sessiond binary.
    pub fn start(exe: &Path) -> io::Result<Helper> {
        Ok(Helper {
            exe: exe.to_owned(),
            link: Mutex::new(Some(Link::start(exe)?)),
        })
    }

    /// The helper's pid, while one runs.
    pub fn pid(&self) -> Option<u32> {
        let link = self.link.lock().unwrap_or_else(PoisonError::into_inner);
        link.as_ref().map(|l| l.child.id())
    }

    /// Start `program` with `stdio`, leading a session of its own, as a child
    /// of this process. `None` when no helper could hear the request.
    pub fn spawn(&self, program: &Program, stdio: Stdio<'_>) -> Option<io::Result<libc::pid_t>> {
        let mut link = self.link.lock().unwrap_or_else(PoisonError::into_inner);
        if link.is_none() {
            *link = Link::start(&self.exe)
                .inspect_err(|e| eprintln!("vorn-sessiond: the spawn helper did not start: {e}"))
                .ok();
        }
        let l = link.as_mut()?;
        let (fds, kind) = match stdio {
            Stdio::Terminal { slave, .. } => (vec![slave], Fds::Terminal),
            Stdio::Pipes {
                stdin,
                stdout,
                stderr,
            } => {
                let mut fds: Vec<RawFd> = stdin.into_iter().collect();
                fds.extend([stdout, stderr]);
                let stdin = stdin.is_some();
                (fds, Fds::Pipes { stdin })
            }
        };
        let req = Request {
            path: program.path.as_bytes().to_vec(),
            argv: program.argv.iter().map(|a| a.as_bytes().to_vec()).collect(),
            env: program.env.iter().map(|e| e.as_bytes().to_vec()).collect(),
            cwd: program.cwd.as_ref().map(|d| d.as_bytes().to_vec()),
            fds: kind,
        };
        if let Err(e) = fdpass::send(l.rx.socket(), &req, &fds) {
            eprintln!("vorn-sessiond: the spawn helper is gone: {e}");
            *link = None;
            return None;
        }
        Some(match l.rx.recv::<Reply>() {
            Ok(Reply::Started(pid)) => Ok(pid),
            Ok(Reply::Failed { pid, errno }) => {
                if pid > 0 {
                    reap(pid);
                }
                Err(io::Error::from_raw_os_error(errno))
            }
            // It may have started the program, so it is not started again.
            Err(e) => {
                *link = None;
                Err(io::Error::other(format!("the spawn helper failed: {e}")))
            }
        })
    }
}

/// Collect a child whose exec failed; it exits at once.
fn reap(pid: libc::pid_t) {
    let mut status = 0;
    // SAFETY: waitpid on a child of this process that nothing else waits for.
    while unsafe { libc::waitpid(pid, &mut status, 0) } == -1 && errno() == libc::EINTR {}
}

/// One program to start, in the form exec takes, without the NULs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Request {
    path: Vec<u8>,
    argv: Vec<Vec<u8>>,
    env: Vec<Vec<u8>>,
    cwd: Option<Vec<u8>>,
    fds: Fds,
}

/// The descriptors beside a request, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Fds {
    /// The terminal's slave.
    Terminal,
    /// stdin when there is one, then stdout and stderr.
    Pipes { stdin: bool },
}

impl Fds {
    fn count(self) -> usize {
        match self {
            Fds::Terminal => 1,
            Fds::Pipes { stdin } => 2 + usize::from(stdin),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Reply {
    Started(libc::pid_t),
    /// `pid` is the child that could not exec, for sessiond to reap, or 0
    /// when there was none.
    Failed {
        pid: libc::pid_t,
        errno: i32,
    },
}

macro_rules! framed {
    ($($ty:ty = $kind:literal),*) => {$(
        impl Message for $ty {
            fn kind(&self) -> u8 {
                $kind
            }
            fn body(&self) -> postcard::Result<Vec<u8>> {
                postcard::to_stdvec(self)
            }
            fn read(kind: u8, body: &[u8]) -> Result<Self, WireError> {
                if kind != $kind {
                    return Err(WireError::UnknownType(kind));
                }
                postcard::from_bytes(body).map_err(|_| WireError::BadBody(kind))
            }
        }
    )*};
}

framed!(Request = 1, Reply = 2);

/// The helper's whole life: start what sessiond asks, on the socket it was
/// given as stdin, until sessiond hangs up.
pub fn serve() -> io::Result<()> {
    crate::pty::cloexec_inherited();
    // SAFETY: stdin is the socket sessiond started the helper with, owned
    // by nothing else in this process.
    let sock = UnixStream::from(unsafe { OwnedFd::from_raw_fd(0) });
    let mut rx = Receiver::new(sock);
    loop {
        let req = match rx.recv::<Request>() {
            Ok(req) => req,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        let fds = rx.take_fds(req.fds.count())?;
        let reply = start(&req, &fds).unwrap_or_else(|errno| Reply::Failed { pid: 0, errno });
        drop(fds);
        fdpass::send(rx.socket(), &reply, &[])?;
    }
}

/// Fork, as sessiond's child, and exec `req` in the child.
fn start(req: &Request, fds: &[OwnedFd]) -> Result<Reply, i32> {
    let c = |b: &[u8]| CString::new(b).map_err(|_| libc::EINVAL);
    let path = c(&req.path)?;
    let argv = req
        .argv
        .iter()
        .map(|a| c(a))
        .collect::<Result<Vec<_>, _>>()?;
    let env = req
        .env
        .iter()
        .map(|e| c(e))
        .collect::<Result<Vec<_>, _>>()?;
    let cwd = req.cwd.as_deref().map(c).transpose()?;
    let raw: Vec<RawFd> = fds.iter().map(AsRawFd::as_raw_fd).collect();
    let (stdin, stdout, stderr, terminal) = match (req.fds, raw.as_slice()) {
        (Fds::Terminal, &[t]) => (Some(t), t, t, true),
        (Fds::Pipes { stdin: true }, &[i, o, e]) => (Some(i), o, e, false),
        (Fds::Pipes { stdin: false }, &[o, e]) => (None, o, e, false),
        _ => return Err(libc::EINVAL),
    };
    let mut report = [0; 2];
    // SAFETY: pipe2 on an array this frame owns.
    if unsafe { libc::pipe2(report.as_mut_ptr(), libc::O_CLOEXEC) } == -1 {
        return Err(errno());
    }
    // SAFETY: both ends were just made and are owned by nothing else.
    let (read_end, write_end) = unsafe {
        (
            OwnedFd::from_raw_fd(report[0]),
            OwnedFd::from_raw_fd(report[1]),
        )
    };
    let exec = Exec {
        path: &path,
        argv: super::ptrs(&argv),
        envp: super::ptrs(&env),
        cwd: cwd.as_deref(),
        stdin,
        stdout,
        stderr,
        terminal,
        report: write_end.as_raw_fd(),
    };
    // SAFETY: a fork (no shared memory, no new stack) whose child execs or
    // exits without returning. CLONE_PARENT makes sessiond its parent and
    // SIGCHLD what sessiond hears when it exits. The helper has one thread,
    // so the child inherits no lock another thread holds.
    let pid = unsafe {
        libc::syscall(
            libc::SYS_clone,
            (libc::CLONE_PARENT | libc::SIGCHLD) as libc::c_ulong,
            0usize,
            0usize,
            0usize,
            0usize,
        )
    };
    match pid {
        -1 => return Err(errno()),
        // SAFETY: in the child, with everything `exec` points at alive.
        0 => unsafe { exec.run() },
        _ => {}
    }
    let pid = libc::pid_t::try_from(pid).map_err(|_| libc::EIO)?;
    drop(write_end);
    let mut why = [0u8; 4];
    let mut got = 0;
    while got < why.len() {
        // SAFETY: a read into the rest of a buffer this frame owns.
        let n = unsafe {
            libc::read(
                read_end.as_raw_fd(),
                why[got..].as_mut_ptr().cast(),
                why.len() - got,
            )
        };
        match usize::try_from(n) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(_) if errno() == libc::EINTR => {}
            Err(_) => break,
        }
    }
    Ok(match got {
        0 => Reply::Started(pid),
        4 => Reply::Failed {
            pid,
            errno: i32::from_ne_bytes(why),
        },
        _ => Reply::Failed {
            pid,
            errno: libc::EIO,
        },
    })
}

/// What the child needs, all built before the fork, so it allocates nothing.
struct Exec<'a> {
    path: &'a CStr,
    argv: Vec<*mut libc::c_char>,
    envp: Vec<*mut libc::c_char>,
    cwd: Option<&'a CStr>,
    stdin: Option<RawFd>,
    stdout: RawFd,
    stderr: RawFd,
    terminal: bool,
    report: RawFd,
}

impl Exec<'_> {
    /// Become the program, or write errno to `report` and exit.
    ///
    /// # Safety
    /// Only in the child of [`start`]'s fork.
    unsafe fn run(&self) -> ! {
        let fail = || -> ! {
            let e = errno().to_ne_bytes();
            libc::write(self.report, e.as_ptr().cast(), e.len());
            libc::_exit(127)
        };
        if libc::setsid() == -1 {
            fail();
        }
        #[allow(clippy::cast_lossless)]
        if self.terminal && libc::ioctl(self.stdout, libc::TIOCSCTTY as _, 0) == -1 {
            fail();
        }
        // Every source is above 2 (the helper keeps 0 to 2 open), so no dup2
        // here overwrites one still to come.
        let stdin = match self.stdin {
            Some(fd) => fd,
            None => libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC),
        };
        if stdin == -1
            || libc::dup2(stdin, 0) == -1
            || libc::dup2(self.stdout, 1) == -1
            || libc::dup2(self.stderr, 2) == -1
        {
            fail();
        }
        for sig in DEFAULT_SIGNALS {
            libc::signal(sig, libc::SIG_DFL);
        }
        let mut empty: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut empty);
        libc::sigprocmask(libc::SIG_SETMASK, &empty, std::ptr::null_mut());
        if let Some(dir) = self.cwd {
            if libc::chdir(dir.as_ptr()) == -1 {
                fail();
            }
        }
        libc::execve(
            self.path.as_ptr(),
            self.argv.as_ptr().cast(),
            self.envp.as_ptr().cast(),
        );
        fail()
    }
}

fn errno() -> i32 {
    io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or(libc::EIO)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_and_replies_cross_the_socket_whole() {
        let req = Request {
            path: b"/bin/sh".to_vec(),
            argv: vec![b"sh".to_vec(), b"-c".to_vec(), b"exit 3".to_vec()],
            env: vec![b"A=1".to_vec()],
            cwd: Some(b"/tmp".to_vec()),
            fds: Fds::Pipes { stdin: true },
        };
        let frame = req.encode();
        assert_eq!(Request::read(frame[4], &frame[5..]).unwrap(), req);
        let reply = Reply::Failed { pid: 7, errno: 2 };
        let frame = reply.encode();
        assert_eq!(Reply::read(frame[4], &frame[5..]).unwrap(), reply);
        assert_eq!(Reply::read(1, &frame[5..]), Err(WireError::UnknownType(1)));
        assert_eq!(req.fds.count(), 3);
        assert_eq!(Fds::Terminal.count(), 1);
    }
}
