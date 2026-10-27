//! Landlock on Linux: what [`super::wrap`] confines a program with, through
//! vornd started as a helper (`vornd --vorn-confine <rules> -- program`).
//! The helper confines itself, then becomes the program; a confined process
//! and everything it starts stay confined.
//!
//! The system's folders and the program's own folder are readable and
//! executable; its data folder and its projects are writable, each opened at
//! the real path vornd resolved and nowhere a link leads; no other file
//! is reachable. With
//! Landlock 4 (Linux 6.7) TCP connects are held to the ports in the rules,
//! on any host; UDP and Unix sockets are not held, so Vorn calls the
//! network not limited here.
//! Raw system calls, as the kernel documents them, rather than a binding
//! crate: three calls and two structs.

use std::ffi::{CString, OsString};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// Readable and executable on every system: where programs, their libraries
/// and the system's own files live.
pub const SYSTEM: &[&str] = &[
    "/usr", "/lib", "/lib32", "/lib64", "/bin", "/sbin", "/etc", "/opt", "/nix", "/snap", "/proc",
    "/sys", "/run",
];

const CREATE_RULESET_VERSION: u32 = 1;
const RULE_PATH_BENEATH: libc::c_int = 1;
const RULE_NET_PORT: libc::c_int = 2;

const EXECUTE: u64 = 1 << 0;
const WRITE_FILE: u64 = 1 << 1;
const READ_FILE: u64 = 1 << 2;
const READ_DIR: u64 = 1 << 3;
/// Removing and making entries, the rest of ABI 1.
const MAKE_AND_REMOVE: u64 = 0b1_1111_1111 << 4;
const REFER: u64 = 1 << 13;
const TRUNCATE: u64 = 1 << 14;
const IOCTL_DEV: u64 = 1 << 15;
const CONNECT_TCP: u64 = 1 << 1;
const BIND_TCP: u64 = 1 << 0;

const READ: u64 = EXECUTE | READ_FILE | READ_DIR;

#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
    handled_access_net: u64,
}

#[repr(C, packed)]
struct PathBeneathAttr {
    allowed_access: u64,
    parent_fd: i32,
}

#[repr(C)]
struct NetPortAttr {
    allowed_access: u64,
    port: u64,
}

/// The Landlock ABI this kernel speaks, `None` where it has none or it is off.
pub fn abi() -> Option<u32> {
    // SAFETY: with the version flag the kernel reads no attribute and returns a number.
    let v = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<RulesetAttr>(),
            0usize,
            CREATE_RULESET_VERSION,
        )
    };
    u32::try_from(v).ok().filter(|&v| v >= 1)
}

/// The file-system rights ABI `abi` handles: all of them, so none is left open by default.
fn handled_fs(abi: u32) -> u64 {
    let mut all = READ | WRITE_FILE | MAKE_AND_REMOVE;
    if abi >= 2 {
        all |= REFER;
    }
    if abi >= 3 {
        all |= TRUNCATE;
    }
    if abi >= 5 {
        all |= IOCTL_DEV;
    }
    all
}

/// What a confined program may reach, as [`super::wrap`] writes it.
#[derive(Debug, Default, serde::Deserialize)]
pub struct Rules {
    #[serde(default)]
    pub write: Vec<String>,
    #[serde(default)]
    pub read: Vec<String>,
    #[serde(default)]
    pub ports: Vec<u16>,
}

fn last_error(what: &str) -> io::Error {
    let err = io::Error::last_os_error();
    io::Error::new(err.kind(), format!("{what}: {err}"))
}

/// Confines this process to `rules`. Every later process it starts inherits it.
pub fn confine(rules: &Rules) -> io::Result<()> {
    let abi = abi().ok_or_else(|| io::Error::other("this kernel has no Landlock"))?;
    let fs = handled_fs(abi);
    let attr = RulesetAttr {
        handled_access_fs: fs,
        handled_access_net: if abi >= 4 { BIND_TCP | CONNECT_TCP } else { 0 },
    };
    // ABI 1 to 3 know only the first field.
    let size = if abi >= 4 {
        std::mem::size_of::<RulesetAttr>()
    } else {
        std::mem::size_of::<u64>()
    };
    // SAFETY: `attr` is a valid ruleset attribute of at least `size` bytes.
    let fd = unsafe { libc::syscall(libc::SYS_landlock_create_ruleset, &attr, size, 0u32) };
    let fd = i32::try_from(fd)
        .ok()
        .filter(|fd| *fd >= 0)
        .ok_or_else(|| last_error("landlock_create_ruleset"))?;
    // SAFETY: the kernel just returned this descriptor, and nothing else owns it.
    let ruleset = unsafe { OwnedFd::from_raw_fd(fd) };
    let system = SYSTEM.iter().map(|s| (*s, READ));
    let read = rules.read.iter().map(|s| (s.as_str(), READ));
    let dev = std::iter::once(("/dev", READ | WRITE_FILE | (fs & IOCTL_DEV)));
    for (path, access) in system.chain(read).chain(dev) {
        if let Some(fd) = open_path(Path::new(path)) {
            allow(&ruleset, &fd, Path::new(path), access & fs)?;
        }
    }
    // A writable folder is opened exactly as vornd resolved it, or not at all:
    // a program can swap a project it writes for a link between then and now.
    for path in rules.write.iter().map(Path::new) {
        match open_exact_dir(path) {
            Ok(fd) => allow(&ruleset, &fd, path, fs)?,
            Err(why) => eprintln!("vorn: not giving {}: {why}", path.display()),
        }
    }
    if abi >= 4 {
        for &port in &rules.ports {
            let rule = NetPortAttr {
                allowed_access: CONNECT_TCP,
                port: u64::from(port),
            };
            // SAFETY: `rule` is a valid net-port rule and the ruleset fd is open.
            let done = unsafe {
                libc::syscall(
                    libc::SYS_landlock_add_rule,
                    ruleset.as_raw_fd(),
                    RULE_NET_PORT,
                    &rule,
                    0u32,
                )
            };
            if done != 0 {
                return Err(last_error("landlock_add_rule (port)"));
            }
        }
    }
    // SAFETY: prctl with these arguments only sets a flag on this thread.
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(last_error("prctl(PR_SET_NO_NEW_PRIVS)"));
    }
    // SAFETY: the ruleset fd is open; restricting takes no other memory.
    if unsafe { libc::syscall(libc::SYS_landlock_restrict_self, ruleset.as_raw_fd(), 0u32) } != 0 {
        return Err(last_error("landlock_restrict_self"));
    }
    Ok(())
}

/// `path` opened for a rule, following links; `None` when it is not there.
fn open_path(path: &Path) -> Option<OwnedFd> {
    open_with(path, 0).ok()
}

/// The folder at `path`, which must be its real path: no link on the way,
/// checked through `/proc/self/fd` after opening so a swap cannot slip in.
fn open_exact_dir(path: &Path) -> io::Result<OwnedFd> {
    let fd = open_with(path, libc::O_NOFOLLOW | libc::O_DIRECTORY)?;
    let landed = std::fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd()))?;
    if landed != path {
        return Err(io::Error::other(format!("it is {} now", landed.display())));
    }
    Ok(fd)
}

fn open_with(path: &Path, flags: libc::c_int) -> io::Result<OwnedFd> {
    let c = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::other("a path with a NUL in it"))?;
    // SAFETY: `c` is a NUL-terminated path that outlives the call.
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_PATH | libc::O_CLOEXEC | flags) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: open just returned this descriptor, and nothing else owns it.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Allows `access` beneath `parent`, opened from `path`, a file or folder.
fn allow(ruleset: &OwnedFd, parent: &OwnedFd, path: &Path, access: u64) -> io::Result<()> {
    // A file takes only the rights a file has.
    let access = if path.is_dir() {
        access
    } else {
        access & (EXECUTE | WRITE_FILE | READ_FILE | TRUNCATE | IOCTL_DEV)
    };
    let rule = PathBeneathAttr {
        allowed_access: access,
        parent_fd: parent.as_raw_fd(),
    };
    // SAFETY: `rule` is a valid path-beneath rule and both descriptors are open.
    let done = unsafe {
        libc::syscall(
            libc::SYS_landlock_add_rule,
            ruleset.as_raw_fd(),
            RULE_PATH_BENEATH,
            &rule,
            0u32,
        )
    };
    if done != 0 {
        return Err(last_error(&format!(
            "landlock_add_rule ({})",
            path.display()
        )));
    }
    Ok(())
}

/// The helper: `args` are what follows [`super::CONFINE_FLAG`], `<rules> --
/// <program> <args>...`. Confines this process and becomes the program; it
/// returns only on failure, with why, and never runs the program unconfined.
pub fn confine_and_exec(args: &[OsString]) -> io::Error {
    let [rules, dashes, program, rest @ ..] = args else {
        return io::Error::other("usage: --vorn-confine <rules> -- <program> [args]");
    };
    if dashes != "--" {
        return io::Error::other("--vorn-confine wants -- before the program");
    }
    let rules: Rules = match serde_json::from_slice(rules.as_bytes()) {
        Ok(r) => r,
        Err(e) => return io::Error::other(format!("--vorn-confine rules: {e}")),
    };
    if let Err(e) = confine(&rules) {
        return e;
    }
    use std::os::unix::process::CommandExt;
    vorn_spawn::command(program).args(rest).exec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_every_right_its_abi_knows() {
        assert_eq!(handled_fs(1) & (REFER | TRUNCATE | IOCTL_DEV), 0);
        assert_ne!(handled_fs(2) & REFER, 0);
        assert_ne!(handled_fs(3) & TRUNCATE, 0);
        assert_ne!(handled_fs(5) & IOCTL_DEV, 0);
        assert_eq!(handled_fs(1) & READ, READ);
    }

    #[test]
    fn refuses_a_helper_line_it_cannot_read() {
        let s = |v: &[&str]| v.iter().map(OsString::from).collect::<Vec<_>>();
        assert!(confine_and_exec(&s(&["{}"])).to_string().contains("usage"));
        assert!(confine_and_exec(&s(&["{}", "x", "/bin/true"]))
            .to_string()
            .contains("--"));
        assert!(confine_and_exec(&s(&["nope", "--", "/bin/true"]))
            .to_string()
            .contains("rules"));
    }

    #[test]
    fn a_writable_folder_opens_only_at_its_real_path() {
        let root = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(root.path()).unwrap();
        let (real, other) = (root.join("real"), root.join("other"));
        std::fs::create_dir_all(real.join("in")).unwrap();
        std::fs::create_dir(&other).unwrap();
        assert!(open_exact_dir(&real).is_ok());
        assert!(open_exact_dir(&real.join("in")).is_ok());
        // The folder itself swapped for a link, or one above it.
        std::os::unix::fs::symlink(&other, root.join("app")).unwrap();
        assert!(open_exact_dir(&root.join("app")).is_err());
        std::os::unix::fs::symlink(&real, root.join("up")).unwrap();
        assert!(open_exact_dir(&root.join("up/in")).is_err());
        assert!(open_exact_dir(&root.join("gone")).is_err());
    }

    /// Run by [`a_confined_program_reads_only_what_it_was_given`] in a
    /// process of its own: confines itself to `VORN_LANDLOCK_PROBE`'s
    /// folder, then exits 0 when it can read `VORN_LANDLOCK_READ`, else 3.
    #[test]
    fn probe() {
        let (Ok(own), Ok(read)) = (
            std::env::var("VORN_LANDLOCK_PROBE"),
            std::env::var("VORN_LANDLOCK_READ"),
        ) else {
            return;
        };
        let rules = Rules {
            write: vec![own],
            ..Rules::default()
        };
        confine(&rules).expect("confines");
        std::process::exit(if std::fs::read(read).is_ok() { 0 } else { 3 });
    }

    /// A confined process reads its own folder, and nothing of another.
    #[test]
    fn a_confined_program_reads_only_what_it_was_given() {
        if abi().is_none() {
            eprintln!("skipped: this kernel has no Landlock");
            return;
        }
        let own = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        std::fs::write(other.path().join("secret"), "s3cret").unwrap();
        std::fs::write(own.path().join("mine"), "mine").unwrap();
        let reads = |file: &Path| {
            vorn_spawn::command(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "sandbox::landlock::tests::probe",
                    "--test-threads=1",
                ])
                .env("VORN_LANDLOCK_PROBE", own.path())
                .env("VORN_LANDLOCK_READ", file)
                .output()
                .unwrap()
                .status
                .success()
        };
        assert!(reads(&own.path().join("mine")));
        assert!(!reads(&other.path().join("secret")));
    }
}
