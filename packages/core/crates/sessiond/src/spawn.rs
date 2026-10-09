//! Starting programs on macOS and Linux with `posix_spawn`, never `fork`.
//!
//! A fork copies the page tables of the whole process, which in a sessiond
//! holding thousands of sessions costs milliseconds per program, and the
//! child of a multithreaded process may then only make async-signal-safe
//! calls until it execs. `posix_spawn` does neither: glibc runs it as a
//! `vfork`-style clone, and macOS starts the program in the kernel.
//!
//! What a child must not inherit, it does not: every descriptor sessiond
//! holds is close-on-exec from the moment it exists ([`crate::pty`]), and on
//! macOS `POSIX_SPAWN_CLOEXEC_DEFAULT` closes anything else on top. Signals
//! sessiond ignores or blocks are back at their defaults, and the program
//! leads a session of its own; a terminal's program opens the terminal as
//! that session's leader, so it is its controlling one.

use std::ffi::{CStr, CString, OsStr, OsString};
use std::io;
use std::os::fd::RawFd;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;
use std::sync::OnceLock;

/// Signals whose disposition the child gets back at its default: one sessiond
/// ignores (Rust ignores SIGPIPE) would otherwise stay ignored across exec.
const DEFAULT_SIGNALS: [libc::c_int; 7] = [
    libc::SIGCHLD,
    libc::SIGHUP,
    libc::SIGINT,
    libc::SIGQUIT,
    libc::SIGTERM,
    libc::SIGALRM,
    libc::SIGPIPE,
];

/// Not in the `libc` crate for macOS; `<sys/spawn.h>` has it.
#[cfg(target_os = "macos")]
const POSIX_SPAWN_SETSID: libc::c_int = 0x0400;
#[cfg(target_os = "linux")]
const POSIX_SPAWN_SETSID: libc::c_int = libc::POSIX_SPAWN_SETSID as libc::c_int;

/// A program ready to start: its path resolved, its argv and environment in
/// the form exec takes.
#[derive(Debug, Clone)]
pub struct Program {
    path: CString,
    argv: Vec<CString>,
    env: Vec<CString>,
    cwd: Option<CString>,
}

/// What the program's stdin, stdout and stderr are.
#[derive(Debug, Clone, Copy)]
pub enum Stdio<'a> {
    /// All three are the terminal at this path, opened by the program as the
    /// leader of its session.
    Terminal(&'a CStr),
    /// Pipe ends sessiond made; no stdin reads `/dev/null`.
    Pipes {
        stdin: Option<RawFd>,
        stdout: RawFd,
        stderr: RawFd,
    },
}

impl Program {
    /// `argv[0]` found on the `PATH` of `env` (sessiond's own environment
    /// when `env` is empty, as the program then inherits it), started in
    /// `cwd` when given.
    pub fn new(argv: &[String], env: &[(String, String)], cwd: Option<&Path>) -> io::Result<Self> {
        let c = |s: &[u8]| CString::new(s).map_err(|_| io::Error::other("a NUL in the command"));
        let first = argv.first().ok_or_else(|| io::Error::other("empty argv"))?;
        let env: Vec<(OsString, OsString)> = if env.is_empty() {
            std::env::vars_os().collect()
        } else {
            env.iter().map(|(k, v)| (k.into(), v.into())).collect()
        };
        let path_var = env
            .iter()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.as_os_str());
        let path = resolve(OsStr::new(first), path_var, cwd)?;
        let mut envp = Vec::with_capacity(env.len());
        for (k, v) in env {
            let mut kv = k.into_vec();
            kv.push(b'=');
            kv.extend_from_slice(v.as_bytes());
            envp.push(c(&kv)?);
        }
        Ok(Program {
            path: c(path.as_bytes())?,
            argv: argv
                .iter()
                .map(|a| c(a.as_bytes()))
                .collect::<io::Result<_>>()?,
            env: envp,
            cwd: cwd.map(|d| c(d.as_os_str().as_bytes())).transpose()?,
        })
    }

    /// The environment's value for `key`, as the program will see it.
    pub fn env(&self, key: &str) -> Option<&[u8]> {
        self.env.iter().find_map(|kv| {
            let kv = kv.as_bytes();
            kv.strip_prefix(key.as_bytes())?.strip_prefix(b"=")
        })
    }

    /// Adds `key=value` to the environment.
    pub fn set_env(&mut self, key: &str, value: &OsStr) -> io::Result<()> {
        let mut kv = key.as_bytes().to_vec();
        kv.push(b'=');
        kv.extend_from_slice(value.as_bytes());
        self.env
            .push(CString::new(kv).map_err(|_| io::Error::other("a NUL in the environment"))?);
        Ok(())
    }

    /// Start it with `stdio`, leading a session of its own. The pid.
    pub fn spawn(&self, stdio: Stdio<'_>) -> io::Result<libc::pid_t> {
        let mut attr = Attr::new()?;
        let mut acts = Actions::new()?;
        match stdio {
            Stdio::Terminal(path) => {
                // No O_NOCTTY: as its session's leader, the program takes
                // the terminal as its controlling one by opening it.
                acts.open(0, path, libc::O_RDWR)?;
                acts.dup2(0, 1)?;
                acts.dup2(0, 2)?;
            }
            Stdio::Pipes {
                stdin,
                stdout,
                stderr,
            } => {
                match stdin {
                    Some(fd) => acts.dup2(fd, 0)?,
                    None => acts.open(0, c"/dev/null", libc::O_RDONLY)?,
                }
                acts.dup2(stdout, 1)?;
                acts.dup2(stderr, 2)?;
            }
        }
        let mut chdir_by_shell = false;
        if let Some(cwd) = &self.cwd {
            chdir_by_shell = !acts.chdir(cwd)?;
        }
        attr.setup()?;
        let mut pid: libc::pid_t = 0;
        // Only an older glibc lacks a chdir action; a shell changes into
        // the directory and execs the program there instead.
        let shell_argv;
        let (path, argv): (&CStr, Vec<*mut libc::c_char>) = if chdir_by_shell {
            shell_argv = [
                CString::from(c"sh"),
                CString::from(c"-c"),
                CString::from(c"cd -- \"$0\" && exec \"$@\""),
                self.cwd.clone().unwrap_or_default(),
            ];
            let argv = shell_argv
                .iter()
                .chain(std::iter::once(&self.path))
                .chain(self.argv.iter().skip(1))
                .map(|a| a.as_ptr().cast_mut())
                .chain(std::iter::once(std::ptr::null_mut()))
                .collect();
            (c"/bin/sh", argv)
        } else {
            (&self.path, ptrs(&self.argv))
        };
        let envp = ptrs(&self.env);
        // SAFETY: every pointer is to a NUL-terminated string or a
        // null-terminated array of them, alive for the call; the actions and
        // attributes were initialised above.
        let rc = unsafe {
            libc::posix_spawn(
                &mut pid,
                path.as_ptr(),
                &acts.0,
                &attr.0,
                argv.as_ptr(),
                envp.as_ptr(),
            )
        };
        if rc != 0 {
            return Err(io::Error::from_raw_os_error(rc));
        }
        Ok(pid)
    }
}

fn ptrs(strings: &[CString]) -> Vec<*mut libc::c_char> {
    strings
        .iter()
        .map(|s| s.as_ptr().cast_mut())
        .chain(std::iter::once(std::ptr::null_mut()))
        .collect()
}

/// `program` as exec finds it: a path when it has a slash, else the first
/// executable file of that name on `path_var`. Relative entries and paths
/// are taken from `cwd`, where the program starts.
fn resolve(program: &OsStr, path_var: Option<&OsStr>, cwd: Option<&Path>) -> io::Result<OsString> {
    if program.as_bytes().contains(&b'/') {
        return Ok(program.to_owned());
    }
    let default = OsStr::new("/usr/local/bin:/usr/bin:/bin");
    for dir in std::env::split_paths(path_var.unwrap_or(default)) {
        let dir = match cwd {
            Some(cwd) if dir.is_relative() => cwd.join(dir),
            _ if dir.as_os_str().is_empty() => ".".into(),
            _ => dir,
        };
        let candidate = dir.join(program);
        if is_executable(&candidate) {
            return Ok(candidate.into_os_string());
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("{} not found on PATH", program.to_string_lossy()),
    ))
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// `posix_spawnattr_t`, destroyed on drop.
struct Attr(libc::posix_spawnattr_t);

impl Attr {
    fn new() -> io::Result<Attr> {
        // SAFETY: init writes the whole attribute object before it is read.
        let mut a: libc::posix_spawnattr_t = unsafe { std::mem::zeroed() };
        check(unsafe { libc::posix_spawnattr_init(&mut a) })?;
        Ok(Attr(a))
    }

    fn setup(&mut self) -> io::Result<()> {
        // SAFETY: sigsets this frame owns, filled by their own init calls.
        unsafe {
            let mut defaults: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut defaults);
            for sig in DEFAULT_SIGNALS {
                libc::sigaddset(&mut defaults, sig);
            }
            let mut empty: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut empty);
            check(libc::posix_spawnattr_setsigdefault(&mut self.0, &defaults))?;
            check(libc::posix_spawnattr_setsigmask(&mut self.0, &empty))?;
        }
        let base = POSIX_SPAWN_SETSID | libc::POSIX_SPAWN_SETSIGDEF | libc::POSIX_SPAWN_SETSIGMASK;
        #[cfg(target_os = "macos")]
        let flags = base | libc::POSIX_SPAWN_CLOEXEC_DEFAULT;
        #[cfg(not(target_os = "macos"))]
        let flags = base;
        // `as _`: the flags are a c_short; every bit used fits.
        #[allow(clippy::cast_possible_truncation)]
        // SAFETY: an initialised attribute object.
        check(unsafe { libc::posix_spawnattr_setflags(&mut self.0, flags as _) })
    }
}

impl Drop for Attr {
    fn drop(&mut self) {
        // SAFETY: initialised in `new`, destroyed once.
        unsafe { libc::posix_spawnattr_destroy(&mut self.0) };
    }
}

/// `posix_spawn_file_actions_t`, destroyed on drop.
struct Actions(libc::posix_spawn_file_actions_t);

impl Actions {
    fn new() -> io::Result<Actions> {
        // SAFETY: init writes the whole object before it is read.
        let mut a: libc::posix_spawn_file_actions_t = unsafe { std::mem::zeroed() };
        check(unsafe { libc::posix_spawn_file_actions_init(&mut a) })?;
        Ok(Actions(a))
    }

    fn open(&mut self, fd: RawFd, path: &CStr, flags: libc::c_int) -> io::Result<()> {
        // SAFETY: an initialised object; the path is copied by the call.
        check(unsafe {
            libc::posix_spawn_file_actions_addopen(&mut self.0, fd, path.as_ptr(), flags, 0)
        })
    }

    /// `from` onto `to`, which is not close-on-exec in the program. `from`
    /// is never 0 to 2: sessiond keeps those open ([`crate::pty::stdio_open`]),
    /// so a descriptor it makes is never one of them.
    fn dup2(&mut self, from: RawFd, to: RawFd) -> io::Result<()> {
        // SAFETY: an initialised object.
        check(unsafe { libc::posix_spawn_file_actions_adddup2(&mut self.0, from, to) })
    }

    /// Change into `dir` before exec. False when this libc has no such action.
    fn chdir(&mut self, dir: &CStr) -> io::Result<bool> {
        let Some(f) = addchdir() else {
            return Ok(false);
        };
        // SAFETY: the symbol has this signature in every libc that has it.
        check(unsafe { f(&mut self.0, dir.as_ptr()) })?;
        Ok(true)
    }
}

impl Drop for Actions {
    fn drop(&mut self) {
        // SAFETY: initialised in `new`, destroyed once.
        unsafe { libc::posix_spawn_file_actions_destroy(&mut self.0) };
    }
}

type AddChdir =
    unsafe extern "C" fn(*mut libc::posix_spawn_file_actions_t, *const libc::c_char) -> libc::c_int;

/// `posix_spawn_file_actions_addchdir_np`, looked up at run time: glibc has
/// it from 2.29, and sessiond must still start on an older one.
fn addchdir() -> Option<AddChdir> {
    static F: OnceLock<Option<AddChdir>> = OnceLock::new();
    *F.get_or_init(|| {
        // SAFETY: dlsym with a NUL-terminated name; a found symbol is that
        // function, whose signature is fixed.
        unsafe {
            let p = libc::dlsym(
                libc::RTLD_DEFAULT,
                c"posix_spawn_file_actions_addchdir_np".as_ptr(),
            );
            (!p.is_null()).then(|| std::mem::transmute::<*mut libc::c_void, AddChdir>(p))
        }
    })
}

fn check(rc: libc::c_int) -> io::Result<()> {
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(rc))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_name_is_found_on_the_programs_path() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("tool");
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!("/nonexistent:{}", dir.path().display());
        let found = resolve(OsStr::new("tool"), Some(OsStr::new(&path)), None).unwrap();
        assert_eq!(found, bin.into_os_string());
        assert_eq!(
            resolve(OsStr::new("./x"), None, None).unwrap(),
            OsString::from("./x")
        );
        let err = resolve(OsStr::new("tool"), Some(OsStr::new("/nonexistent")), None).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn the_environment_is_the_specs_when_it_has_one() {
        let env = [
            ("PATH".to_owned(), "/bin:/usr/bin".to_owned()),
            ("A".to_owned(), "1".to_owned()),
        ];
        let mut p = Program::new(&["sh".into()], &env, None).unwrap();
        assert_eq!(p.env("A"), Some(&b"1"[..]));
        assert_eq!(p.env("HOME"), None);
        p.set_env("HOME", OsStr::new("/h")).unwrap();
        assert_eq!(p.env("HOME"), Some(&b"/h"[..]));
    }

    #[test]
    fn piped_output_and_the_directory_reach_the_program() {
        let dir = tempfile::tempdir().unwrap();
        let mut fds = [0; 2];
        // SAFETY: pipe2-alike on an array this frame owns.
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let p = Program::new(
            &["sh".into(), "-c".into(), "pwd; echo err >&2".into()],
            &[],
            Some(dir.path()),
        )
        .unwrap();
        let pid = p
            .spawn(Stdio::Pipes {
                stdin: None,
                stdout: fds[1],
                stderr: fds[1],
            })
            .unwrap();
        // SAFETY: closing the write end this test made, then reading the other.
        unsafe { libc::close(fds[1]) };
        use std::io::Read;
        use std::os::fd::FromRawFd;
        let mut out = String::new();
        unsafe { std::fs::File::from_raw_fd(fds[0]) }
            .read_to_string(&mut out)
            .unwrap();
        let mut status = 0;
        // SAFETY: reaping the child this test started.
        unsafe { libc::waitpid(pid, &mut status, 0) };
        let real = std::fs::canonicalize(dir.path()).unwrap();
        assert!(out.contains(&*real.to_string_lossy()), "{out:?}");
        assert!(out.contains("err"), "{out:?}");
    }
}
