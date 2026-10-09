//! Installing and starting sessiond so it outlives the app (RC §6 flow C,
//! §10), and finding the instances already running.
//!
//! - Each version is copied out of the app bundle into a directory of its
//!   own, `$VORN_HOME/bin/sessiond-<version>/`, so replacing the bundle never
//!   touches a running binary. A bundle whose binary differs from the one
//!   installed for its version (a local rebuild) goes beside it, in
//!   `sessiond-<version>+<fingerprint>/`. The files that must sit beside the binary go
//!   with it ([`COMPANIONS`]): on Windows, the ConPTY that sessiond loads in
//!   place of the system's (`conpty.dll`) and the console host it starts
//!   (`<arch>/OpenConsole.exe`).
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
    /// The binary it runs, so a launcher can tell two builds of one version
    /// apart. Absent from announcements written before it was added.
    pub exe: Option<PathBuf>,
    /// The newest handoff protocol it speaks ([`crate::wire::HANDOFF`]):
    /// whether it can hand its sessions to a newer holder rather than be
    /// drained. Absent from holders that cannot.
    pub handoff: Option<u16>,
}

impl Instance {
    /// Whether it runs `binary`, comparing resolved paths. One that did not
    /// say what it runs is taken to run something else.
    pub fn runs(&self, binary: &Path) -> bool {
        let resolve = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_owned());
        self.exe
            .as_deref()
            .is_some_and(|exe| resolve(exe) == resolve(binary))
    }
}

fn info_path(home: &Path, instance: u128) -> PathBuf {
    home.join("run").join(format!("sessiond-{instance:x}.info"))
}

/// Write this instance's announcement.
pub fn announce(home: &Path, i: &Instance) -> io::Result<()> {
    fs::create_dir_all(home.join("run"))?;
    let mut body = format!(
        "endpoint={}\npid={}\nproto={}\nbuild={}\ninstance={:x}\n",
        i.endpoint, i.pid, i.proto, i.build, i.instance
    );
    // A path with a line break cannot be announced; leaving it out reads as
    // another build, which is only drained.
    if let Some(exe) = i.exe.as_deref().and_then(Path::to_str) {
        if !exe.contains(['\n', '\r']) {
            body.push_str(&format!("exe={exe}\n"));
        }
    }
    if let Some(v) = i.handoff {
        body.push_str(&format!("handoff={v}\n"));
    }
    let path = info_path(home, i.instance);
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, body)?;
    fs::rename(tmp, path)
}

/// Remove this instance's announcement.
pub fn withdraw(home: &Path, instance: u128) {
    let _ = fs::remove_file(info_path(home, instance));
}

pub(crate) fn parse(text: &str) -> Option<Instance> {
    let mut endpoint = None;
    let mut pid = None;
    let mut proto = None;
    let mut build = None;
    let mut instance = None;
    let mut exe = None;
    let mut handoff = None;
    for line in text.lines() {
        let (k, v) = line.split_once('=')?;
        match k {
            "endpoint" => endpoint = Some(v.to_owned()),
            "pid" => pid = v.parse().ok(),
            "proto" => proto = v.parse().ok(),
            "build" => build = Some(v.to_owned()),
            "instance" => instance = u128::from_str_radix(v, 16).ok(),
            "exe" => exe = Some(PathBuf::from(v)),
            "handoff" => handoff = v.parse().ok(),
            _ => {}
        }
    }
    Some(Instance {
        endpoint: endpoint?,
        pid: pid?,
        proto: proto?,
        build: build?,
        instance: instance?,
        exe,
        handoff,
    })
}

/// The instances announced under `home` whose process is still alive. An
/// announcement left by one that crashed is removed, with its socket.
pub fn running(home: &Path) -> Vec<Instance> {
    let run = home.join("run");
    let Ok(dir) = fs::read_dir(&run) else {
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
            crate::rundir::forget(&run, &path, &i);
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

/// Files that go with the bundled binary when they sit beside it, as paths
/// relative to its directory with `/` between components. On Windows these
/// are the ConPTY sideload: the binary's directory is where `conpty.dll` is
/// found first, and that DLL starts the console host from the subdirectory
/// named for the machine's architecture. A file the bundle does not have is
/// skipped, so other platforms install the binary alone.
pub const COMPANIONS: &[&str] = &["conpty.dll", "x64/OpenConsole.exe", "arm64/OpenConsole.exe"];

/// The directory a version is installed in. Named differently from the
/// single files older builds installed (`vorn-sessiond-<version>`), so the
/// two layouts never collide.
pub fn installed_dir(home: &Path, version: &str) -> PathBuf {
    home.join("bin").join(format!("sessiond-{version}"))
}

/// The binary installed first for a version.
pub fn installed_path(home: &Path, version: &str) -> PathBuf {
    installed_dir(home, version).join(exe_name())
}

fn exe_name() -> String {
    format!("vorn-sessiond{}", std::env::consts::EXE_SUFFIX)
}

/// Where a bundle whose binary differs from the one installed for its
/// version goes, named for its contents so the same rebuild is found again.
fn rebuilt_dir(home: &Path, version: &str, print: Fingerprint) -> PathBuf {
    home.join("bin").join(format!("sessiond-{version}+{print}"))
}

/// A binary's length and CRC-32: enough to tell a rebuild from the build it
/// replaces without keeping a second copy to compare against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Fingerprint {
    len: u64,
    crc: u32,
}

impl Fingerprint {
    fn of(path: &Path) -> io::Result<Fingerprint> {
        let mut file = fs::File::open(path)?;
        let mut hasher = crc32fast::Hasher::new();
        let mut buf = vec![0u8; 64 << 10];
        let mut len = 0u64;
        loop {
            let n = io::Read::read(&mut file, &mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            len += n as u64;
        }
        Ok(Fingerprint {
            len,
            crc: hasher.finalize(),
        })
    }

    /// Whether `path` has these contents; a different length answers
    /// without reading it.
    fn matches(self, path: &Path) -> io::Result<bool> {
        if fs::metadata(path)?.len() != self.len {
            return Ok(false);
        }
        Ok(Fingerprint::of(path)? == self)
    }
}

impl std::fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:x}-{:08x}", self.len, self.crc)
    }
}

/// Install the bundled binary and its [`COMPANIONS`] under their version,
/// unless the same binary is already installed, and answer the installed
/// binary. A version is installed in [`installed_dir`]; a bundle of that
/// version with a different binary, as a local rebuild has, is installed
/// beside it rather than over it. The directory is built under a temporary
/// name and renamed into place, so a reader never sees half an install, and
/// an install is never overwritten while it may be running. When another
/// launcher installs the same build first, its install is used.
pub fn install(bundled: &Path, home: &Path, version: &str) -> io::Result<PathBuf> {
    let first = installed_path(home, version);
    if !first.exists() {
        return install_in(bundled, home, version, &installed_dir(home, version));
    }
    let print = Fingerprint::of(bundled)?;
    if print.matches(&first)? {
        return Ok(first);
    }
    let dir = rebuilt_dir(home, version, print);
    let dest = dir.join(exe_name());
    if dest.exists() {
        return Ok(dest);
    }
    install_in(bundled, home, version, &dir)
}

/// Stage the bundle and rename it to `dir`, unless another launcher has.
fn install_in(bundled: &Path, home: &Path, version: &str, dir: &Path) -> io::Result<PathBuf> {
    let dest = dir.join(exe_name());
    let bin = home.join("bin");
    fs::create_dir_all(&bin)?;
    let tmp = bin.join(format!(".sessiond-{version}.{}.tmp", unique()));
    let built = stage(bundled, &tmp, &tmp.join(exe_name()));
    let done = built.and_then(|()| rename_dir(&tmp, dir, &dest));
    match done {
        Ok(()) => Ok(dest),
        Err(e) => {
            let _ = fs::remove_dir_all(&tmp);
            // Another launcher installed it first: renaming onto its
            // directory fails, and its binary is there.
            if dest.exists() {
                Ok(dest)
            } else {
                Err(e)
            }
        }
    }
}

/// Renames the staged directory `from` to `to`, unless `done` shows another
/// launcher got there first. On Windows a scanner still reading a file just
/// copied into `from` makes the rename fail for a moment, so it is tried
/// again for about a second there.
fn rename_dir(from: &Path, to: &Path, done: &Path) -> io::Result<()> {
    let mut retries = if cfg!(windows) { 20 } else { 0 };
    loop {
        match fs::rename(from, to) {
            Err(e)
                if retries > 0 && e.kind() == io::ErrorKind::PermissionDenied && !done.exists() =>
            {
                retries -= 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            r => return r,
        }
    }
}

/// Fill `dir` with the bundled binary, as `exe`, and the companions found
/// beside it.
fn stage(bundled: &Path, dir: &Path, exe: &Path) -> io::Result<()> {
    fs::create_dir(dir)?;
    fs::copy(bundled, exe)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(exe, fs::Permissions::from_mode(0o755))?;
    }
    let Some(from) = bundled.parent() else {
        return Ok(());
    };
    for rel in COMPANIONS {
        let src = rel.split('/').fold(from.to_path_buf(), |p, c| p.join(c));
        if !src.is_file() {
            continue;
        }
        let to = rel.split('/').fold(dir.to_path_buf(), |p, c| p.join(c));
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&src, &to)?;
    }
    Ok(())
}

/// A name no other install or scope shares, even two at once from one
/// process.
fn unique() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!(
        "{}-{}-{nanos:x}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// Start `binary` detached from the caller and wait for it to announce
/// itself. Readiness is the announcement, not its stdout: a scope wrapper may
/// not hand the pipe through.
pub fn start(binary: &Path, home: &Path, timeout: Duration) -> io::Result<Instance> {
    spawn(binary, home)?.wait(timeout)
}

/// A sessiond started by this process that may not have announced itself yet.
/// Dropping it leaves the process running.
#[derive(Debug)]
pub struct Starting {
    /// The scope wrapper and setsid both exec in place, so this is sessiond's.
    pid: u32,
    /// Taken only by `drop`, which hands it to a thread that reaps it.
    child: Option<std::process::Child>,
    home: PathBuf,
    log_path: PathBuf,
}

/// Start `binary` detached from the caller, without waiting for it.
pub fn spawn(binary: &Path, home: &Path) -> io::Result<Starting> {
    fs::create_dir_all(home.join("log"))?;
    let log_path = home.join("log").join("sessiond.log");
    // Its stderr is a plain file it keeps open, so the log is rotated here, at each start.
    let _ = vorn_logfile::rotate_if_full(&log_path, vorn_logfile::Rotation::DEFAULT);
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let mut cmd = detached(binary, home);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log));
    let child = match cmd.spawn() {
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
    Ok(Starting {
        pid: child.id(),
        child: Some(child),
        home: home.to_owned(),
        log_path,
    })
}

impl Starting {
    /// Its process id.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Wait up to `timeout` for it to announce itself. A `TimedOut` error
    /// leaves it running, to be waited on again; any other means it is gone.
    pub fn wait(&mut self, timeout: Duration) -> io::Result<Instance> {
        let pid = self.pid;
        let Some(child) = self.child.as_mut() else {
            return Err(io::Error::other("sessiond is no longer this launcher's"));
        };
        let t = Instant::now();
        loop {
            if let Some(i) = running(&self.home).into_iter().find(|i| i.pid == pid) {
                return Ok(i);
            }
            if let Some(status) = child.try_wait()? {
                return Err(io::Error::other(format!(
                    "sessiond exited ({status}) before it started: {}",
                    log_tail(&self.log_path)
                )));
            }
            if t.elapsed() >= timeout {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("sessiond did not start: {}", log_tail(&self.log_path)),
                ));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Starting {
    fn drop(&mut self) {
        // It lives on its own; a thread reaps it if it exits while the launcher runs.
        if let Some(mut child) = self.child.take() {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
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
    let mut cmd = vorn_spawn::command(runner);
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
    format!("vorn-sessiond-{}", unique())
}

#[cfg(target_os = "linux")]
fn systemd_scope_available() -> bool {
    vorn_spawn::command("systemd-run")
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
    let mut cmd = vorn_spawn::command(binary);
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
    use vorn_spawn::{
        Hidden, CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS,
    };
    let mut cmd = vorn_spawn::command(binary);
    cmd.arg("--home").arg(home);
    let mut flags = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
    if breakaway {
        flags |= CREATE_BREAKAWAY_FROM_JOB;
    }
    cmd.hidden_with(flags);
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
            exe: Some(PathBuf::from(format!("/bin/s{instance}"))),
            handoff: (instance % 2 == 1).then_some(1),
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
        assert_eq!(a, installed_path(home.path(), "0.8.0"));
        assert_eq!(
            a.parent(),
            Some(installed_dir(home.path(), "0.8.0").as_path())
        );
        assert_eq!(fs::read(&a).unwrap(), b"v1");
        assert_eq!(install(&src, home.path(), "0.8.0").unwrap(), a);
        fs::write(&src, b"v2").unwrap();
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
        // No temporary directory is left behind.
        let mut left: Vec<String> = fs::read_dir(home.path().join("bin"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["sessiond-0.8.0", "sessiond-0.8.1"]);
    }

    /// A bundle of an installed version with the same binary, from wherever
    /// it is, reuses the install.
    #[test]
    fn the_same_build_of_a_version_is_reused() {
        let home = tempfile::tempdir().unwrap();
        let src = home.path().join("bundled");
        fs::write(&src, b"build").unwrap();
        let a = install(&src, home.path(), "0.8.0").unwrap();
        let moved = home.path().join("moved");
        fs::write(&moved, b"build").unwrap();
        assert_eq!(install(&moved, home.path(), "0.8.0").unwrap(), a);
        assert_eq!(fs::read_dir(home.path().join("bin")).unwrap().count(), 1);
    }

    /// A rebuild of an installed version goes beside the install, which
    /// stays as it was, and is found again by its contents.
    #[test]
    fn a_rebuild_of_a_version_is_installed_beside_it() {
        let home = tempfile::tempdir().unwrap();
        let src = home.path().join("bundled");
        fs::write(&src, b"v1").unwrap();
        let a = install(&src, home.path(), "0.8.0").unwrap();
        // Same length, other bytes: only the checksum tells them apart.
        fs::write(&src, b"v2").unwrap();
        let b = install(&src, home.path(), "0.8.0").unwrap();
        assert_ne!(a, b);
        assert_eq!(fs::read(&a).unwrap(), b"v1");
        assert_eq!(fs::read(&b).unwrap(), b"v2");
        let dir = b.parent().unwrap().file_name().unwrap().to_string_lossy();
        assert!(dir.starts_with("sessiond-0.8.0+2-"), "{dir}");
        assert_eq!(install(&src, home.path(), "0.8.0").unwrap(), b);
        fs::write(&src, b"longer").unwrap();
        let c = install(&src, home.path(), "0.8.0").unwrap();
        assert!(c != a && c != b);
        // Going back to the first build finds its install.
        fs::write(&src, b"v1").unwrap();
        assert_eq!(install(&src, home.path(), "0.8.0").unwrap(), a);
        assert_eq!(fs::read_dir(home.path().join("bin")).unwrap().count(), 3);
    }

    #[test]
    fn an_instance_runs_the_binary_it_announced() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let exe = bin.join("vorn-sessiond");
        fs::write(&exe, b"x").unwrap();
        let mut i = inst(1, 1);
        // Announced through another spelling of the same path.
        i.exe = Some(bin.join("..").join("bin").join("vorn-sessiond"));
        assert!(i.runs(&exe));
        assert!(!i.runs(&bin.join("other")));
        i.exe = None;
        assert!(!i.runs(&exe));
    }

    /// Announcements from sessionds that did not name their binary still
    /// list; one whose path cannot be written on a line leaves it out.
    #[test]
    fn the_announced_binary_is_optional() {
        let home = tempfile::tempdir().unwrap();
        let older = "endpoint=e\npid=1\nproto=1\nbuild=0.8.0\ninstance=5\n";
        assert_eq!(parse(older).unwrap().exe, None);
        let mut i = inst(1, 2);
        i.exe = Some(PathBuf::from("/a\nb"));
        announce(home.path(), &i).unwrap();
        assert_eq!(running(home.path())[0].exe, None);
        let i = inst(1, 2);
        announce(home.path(), &i).unwrap();
        assert_eq!(running(home.path()), vec![i]);
    }

    /// An older build's single-file install of the same version is a
    /// different path, so it neither stops the install nor is touched.
    #[test]
    fn the_old_single_file_layout_does_not_collide() {
        let home = tempfile::tempdir().unwrap();
        let old = home.path().join("bin").join(format!(
            "vorn-sessiond-0.8.0{}",
            std::env::consts::EXE_SUFFIX
        ));
        fs::create_dir_all(old.parent().unwrap()).unwrap();
        fs::write(&old, b"old").unwrap();
        let src = home.path().join("bundled");
        fs::write(&src, b"new").unwrap();
        let a = install(&src, home.path(), "0.8.0").unwrap();
        assert_ne!(a, old);
        assert_eq!(fs::read(&a).unwrap(), b"new");
        assert_eq!(fs::read(&old).unwrap(), b"old");
    }

    /// The files beside the bundled binary go with it, in the same layout;
    /// those the bundle lacks are skipped.
    #[test]
    fn install_takes_the_companions_present() {
        let home = tempfile::tempdir().unwrap();
        let bundle = home.path().join("bundle");
        fs::create_dir_all(bundle.join("x64")).unwrap();
        let src = bundle.join("vorn-sessiond");
        fs::write(&src, b"exe").unwrap();
        fs::write(bundle.join("conpty.dll"), b"dll").unwrap();
        fs::write(bundle.join("x64").join("OpenConsole.exe"), b"host").unwrap();
        // Not a companion: stays behind.
        fs::write(bundle.join("other.txt"), b"x").unwrap();
        let exe = install(&src, home.path(), "1.0.0").unwrap();
        let dir = exe.parent().unwrap();
        assert_eq!(fs::read(dir.join("conpty.dll")).unwrap(), b"dll");
        assert_eq!(
            fs::read(dir.join("x64").join("OpenConsole.exe")).unwrap(),
            b"host"
        );
        assert!(!dir.join("arm64").exists(), "missing ones are skipped");
        assert!(!dir.join("other.txt").exists());

        // A binary with nothing beside it installs alone.
        let lone = home.path().join("lone");
        fs::create_dir_all(&lone).unwrap();
        fs::write(lone.join("vorn-sessiond"), b"exe").unwrap();
        let exe = install(&lone.join("vorn-sessiond"), home.path(), "1.0.1").unwrap();
        let names: Vec<_> = fs::read_dir(exe.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1, "{names:?}");
    }

    /// Two launchers installing one version at once both end up with the
    /// same complete install.
    #[test]
    fn concurrent_installs_agree() {
        let home = tempfile::tempdir().unwrap();
        let src = home.path().join("bundled");
        fs::write(&src, vec![7u8; 1 << 20]).unwrap();
        fs::write(home.path().join("conpty.dll"), b"dll").unwrap();
        let got: Vec<PathBuf> = std::thread::scope(|s| {
            let hs: Vec<_> = (0..8)
                .map(|_| s.spawn(|| install(&src, home.path(), "2.0.0").unwrap()))
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(got.iter().all(|p| *p == got[0]));
        assert_eq!(fs::read(&got[0]).unwrap().len(), 1 << 20);
        assert!(got[0].parent().unwrap().join("conpty.dll").exists());
        let entries = fs::read_dir(home.path().join("bin")).unwrap().count();
        assert_eq!(entries, 1, "temporary directories are cleaned up");
    }
}
