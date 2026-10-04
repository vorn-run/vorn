//! Installing and starting sessiond so it outlives the app (RC §6 flow C,
//! §10), and finding the instances already running.
//!
//! - Binaries are copied out of the app bundle to
//!   `$VORN_HOME/bin/vorn-sessiond-<version>`, so replacing the bundle never
//!   touches a running binary.
//! - On Linux sessiond gets its own systemd user scope: desktop launchers put
//!   apps in an `app-*.scope`, and stopping that scope kills every process in
//!   it. On macOS it starts in its own session. On Windows it breaks away
//!   from the app's job object, so closing the app does not end sessions.
//! - Each running instance announces itself in `run/` with a small text file
//!   that vornd lists to find it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A running sessiond, as its announcement says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    pub endpoint: String,
    pub pid: u32,
    pub proto: u16,
    pub build: String,
    pub instance: u128,
}

fn info_path(home: &Path, instance: u128) -> PathBuf {
    home.join("run").join(format!("sessiond-{instance:x}.info"))
}

/// Write this instance's announcement.
pub fn announce(home: &Path, i: &Instance) -> io::Result<()> {
    fs::create_dir_all(home.join("run"))?;
    let body = format!(
        "endpoint={}\npid={}\nproto={}\nbuild={}\ninstance={:x}\n",
        i.endpoint, i.pid, i.proto, i.build, i.instance
    );
    let path = info_path(home, i.instance);
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, body)?;
    fs::rename(tmp, path)
}

/// Remove this instance's announcement.
pub fn withdraw(home: &Path, instance: u128) {
    let _ = fs::remove_file(info_path(home, instance));
}

fn parse(text: &str) -> Option<Instance> {
    let mut endpoint = None;
    let mut pid = None;
    let mut proto = None;
    let mut build = None;
    let mut instance = None;
    for line in text.lines() {
        let (k, v) = line.split_once('=')?;
        match k {
            "endpoint" => endpoint = Some(v.to_owned()),
            "pid" => pid = v.parse().ok(),
            "proto" => proto = v.parse().ok(),
            "build" => build = Some(v.to_owned()),
            "instance" => instance = u128::from_str_radix(v, 16).ok(),
            _ => {}
        }
    }
    Some(Instance {
        endpoint: endpoint?,
        pid: pid?,
        proto: proto?,
        build: build?,
        instance: instance?,
    })
}

/// The instances announced under `home` whose process is still alive. An
/// announcement left by one that crashed is removed.
pub fn running(home: &Path) -> Vec<Instance> {
    let Ok(dir) = fs::read_dir(home.join("run")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in dir.flatten() {
        let path = e.path();
        if path.extension().is_none_or(|x| x != "info") {
            continue;
        }
        let Some(i) = fs::read_to_string(&path).ok().as_deref().and_then(parse) else {
            continue;
        };
        if alive(i.pid) {
            out.push(i);
        } else {
            let _ = fs::remove_file(&path);
        }
    }
    out.sort_by_key(|i| i.instance);
    out
}

/// RC §6 flow D: whether installing a vornd whose oldest supported protocol
/// is `vornd_min` would end sessions on a running sessiond, so the updater
/// must ask first.
pub fn update_ends_sessions(running: &[Instance], vornd_min: u16) -> bool {
    running.iter().any(|i| i.proto < vornd_min)
}

/// The installed name for a version.
pub fn installed_path(home: &Path, version: &str) -> PathBuf {
    let name = format!("vorn-sessiond-{version}{}", std::env::consts::EXE_SUFFIX);
    home.join("bin").join(name)
}

/// Copy the bundled binary to its versioned name, unless that version is
/// already installed. The copy is renamed into place, so a reader never sees
/// half a binary, and an installed version is never overwritten while it may
/// be running.
pub fn install(bundled: &Path, home: &Path, version: &str) -> io::Result<PathBuf> {
    let dest = installed_path(home, version);
    if dest.exists() {
        return Ok(dest);
    }
    fs::create_dir_all(home.join("bin"))?;
    let tmp = dest.with_extension(format!("tmp-{}", std::process::id()));
    fs::copy(bundled, &tmp)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))?;
    }
    match fs::rename(&tmp, &dest) {
        // Another launcher installed it first.
        Err(_) if dest.exists() => {
            let _ = fs::remove_file(&tmp);
            Ok(dest)
        }
        Err(e) => Err(e),
        Ok(()) => Ok(dest),
    }
}

/// Start `binary` detached from the caller and wait for it to announce
/// itself. Readiness is the announcement, not its stdout: a scope wrapper may
/// not hand the pipe through.
pub fn start(binary: &Path, home: &Path, timeout: Duration) -> io::Result<Instance> {
    fs::create_dir_all(home.join("log"))?;
    let log_path = home.join("log").join("sessiond.log");
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let mut cmd = detached(binary, home);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log));
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        // A job that forbids breakaway refuses the spawn; start inside it.
        #[cfg(windows)]
        Err(e) if e.raw_os_error() == Some(5) => {
            let mut cmd = detached_flags(binary, home, false);
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            cmd.spawn()?
        }
        Err(e) => return Err(e),
    };
    // The scope wrapper and setsid both exec in place, so the child's pid is
    // sessiond's.
    let pid = child.id();
    let t = Instant::now();
    let found = loop {
        if let Some(i) = running(home).into_iter().find(|i| i.pid == pid) {
            break Ok(i);
        }
        if let Some(status) = child.try_wait()? {
            break Err(io::Error::other(format!(
                "sessiond exited ({status}) before it started: {}",
                log_tail(&log_path)
            )));
        }
        if t.elapsed() >= timeout {
            break Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("sessiond did not start: {}", log_tail(&log_path)),
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // It lives on its own; a thread only reaps it if it exits while the
    // launcher still runs, so it never lingers as a zombie.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    found
}

fn log_tail(path: &Path) -> String {
    let text = fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().rev().take(5).collect();
    lines.into_iter().rev().collect::<Vec<_>>().join(" | ")
}

#[cfg(target_os = "linux")]
fn detached(binary: &Path, home: &Path) -> Command {
    // Its own transient user scope, when a user manager is there to make one.
    if systemd_scope_available() {
        return scope("systemd-run", binary, home);
    }
    setsid(binary, home)
}

/// Start `binary` in a transient user scope through `runner` (systemd-run).
/// The scope only moves it out of the launcher's cgroup; systemd-run then
/// execs `binary` in place, so without its own session sessiond would stay
/// in the launcher's process group and die with a Ctrl-C or hangup aimed
/// at the launcher's terminal.
#[cfg(target_os = "linux")]
fn scope(runner: impl AsRef<std::ffi::OsStr>, binary: &Path, home: &Path) -> Command {
    let mut cmd = Command::new(runner);
    cmd.args(["--user", "--scope", "--quiet", "--collect"])
        .arg(format!("--unit={}", unit_name()))
        .arg(binary)
        .arg("--home")
        .arg(home);
    new_session(&mut cmd);
    cmd
}

/// A scope name no other start shares, even two from one process.
#[cfg(target_os = "linux")]
fn unit_name() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!(
        "vorn-sessiond-{}-{}-{nanos:x}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(target_os = "linux")]
fn systemd_scope_available() -> bool {
    Command::new("systemd-run")
        .args(["--user", "--scope", "--quiet", "--collect", "true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(all(unix, not(target_os = "linux")))]
fn detached(binary: &Path, home: &Path) -> Command {
    setsid(binary, home)
}

#[cfg(unix)]
fn setsid(binary: &Path, home: &Path) -> Command {
    let mut cmd = Command::new(binary);
    cmd.arg("--home").arg(home);
    new_session(&mut cmd);
    cmd
}

/// Make the child the leader of a new session, out of the launcher's process
/// group and away from its controlling terminal.
#[cfg(unix)]
fn new_session(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: setsid is async-signal-safe and touches only the child.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(windows)]
fn detached(binary: &Path, home: &Path) -> Command {
    detached_flags(binary, home, true)
}

#[cfg(windows)]
fn detached_flags(binary: &Path, home: &Path, breakaway: bool) -> Command {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    let mut cmd = Command::new(binary);
    cmd.arg("--home").arg(home);
    let mut flags = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
    if breakaway {
        flags |= CREATE_BREAKAWAY_FROM_JOB;
    }
    cmd.creation_flags(flags);
    cmd
}

/// Whether a process is still running (a zombie counts as gone).
pub fn alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    if fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|s| {
            s.rsplit_once(')')
                .map(|(_, rest)| rest.trim_start().starts_with('Z'))
        })
        .unwrap_or(false)
    {
        return false;
    }
    #[cfg(unix)]
    {
        // SAFETY: signal 0 only checks that the process exists.
        let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
        r == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
        use windows_sys::Win32::System::Threading::{
            OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
        };
        // SAFETY: the handle is checked and closed.
        unsafe {
            let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if h.is_null() {
                return false;
            }
            let r = WaitForSingleObject(h, 0);
            CloseHandle(h);
            r == WAIT_TIMEOUT
        }
    }
}

/// End a process at once: SIGKILL, or TerminateProcess.
pub fn kill(pid: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        // SAFETY: kill(2) on a pid the caller names.
        if unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, TerminateProcess, PROCESS_TERMINATE,
        };
        // SAFETY: the handle is checked and closed.
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if h.is_null() {
                return Err(io::Error::last_os_error());
            }
            let ok = TerminateProcess(h, 1);
            CloseHandle(h);
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inst(proto: u16, instance: u128) -> Instance {
        Instance {
            endpoint: format!("e{instance}"),
            pid: std::process::id(),
            proto,
            build: "0.8.0".into(),
            instance,
        }
    }

    #[test]
    fn announcements_round_trip_and_dead_ones_are_dropped() {
        let home = tempfile::tempdir().unwrap();
        announce(home.path(), &inst(1, 7)).unwrap();
        announce(home.path(), &inst(1, 3)).unwrap();
        let mut dead = inst(1, 9);
        dead.pid = u32::MAX - 1;
        announce(home.path(), &dead).unwrap();
        assert_eq!(running(home.path()), vec![inst(1, 3), inst(1, 7)]);
        assert!(!info_path(home.path(), 9).exists());
        withdraw(home.path(), 3);
        assert_eq!(running(home.path()), vec![inst(1, 7)]);
    }

    /// RC-T12: a vornd that drops proto 1 ends sessions on a running proto-1
    /// sessiond, so the updater must ask; one that keeps it need not.
    #[test]
    fn the_compatibility_gate() {
        let running = vec![inst(1, 1), inst(2, 2)];
        assert!(update_ends_sessions(&running, 2));
        assert!(!update_ends_sessions(&running, 1));
        assert!(!update_ends_sessions(&[], 3));
    }

    /// The scope wrapper starts sessiond in a session of its own, as the
    /// plain start does, so a signal to the launcher's terminal misses it.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_scope_wrapper_starts_in_its_own_session() {
        let home = tempfile::tempdir().unwrap();
        // `true` stands in for systemd-run: what matters is the process it
        // starts as, which the wrapper turns into sessiond by exec.
        let mut child = scope("true", Path::new("vorn-sessiond"), home.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        // Readable until it is reaped, even once it has exited.
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        child.wait().unwrap();
        // After the command name: state, ppid, pgrp, session.
        let (_, rest) = stat.rsplit_once(')').unwrap();
        let fields: Vec<&str> = rest.split_whitespace().collect();
        assert_eq!(fields[2], pid.to_string(), "its own process group");
        assert_eq!(fields[3], pid.to_string(), "its own session");
    }

    #[test]
    fn install_copies_once_per_version() {
        let home = tempfile::tempdir().unwrap();
        let src = home.path().join("bundled");
        fs::write(&src, b"v1").unwrap();
        let a = install(&src, home.path(), "0.8.0").unwrap();
        assert_eq!(fs::read(&a).unwrap(), b"v1");
        // The bundle changes; the installed version stays as it was.
        fs::write(&src, b"v2").unwrap();
        assert_eq!(install(&src, home.path(), "0.8.0").unwrap(), a);
        assert_eq!(fs::read(&a).unwrap(), b"v1");
        let b = install(&src, home.path(), "0.8.1").unwrap();
        assert_ne!(a, b);
        assert_eq!(fs::read(&b).unwrap(), b"v2");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&b).unwrap().permissions().mode() & 0o777,
                0o755
            );
        }
    }
}
