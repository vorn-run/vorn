//! Clearing `run/` of what dead owners left there: the sockets of holders and
//! vornds that were killed before they could remove them, and the
//! announcements of holders that crashed.
//!
//! Only `run/` under the given home is read, and an announcement naming an
//! endpoint anywhere else never gets that endpoint removed, so another
//! `VORN_HOME` is never touched. A socket goes only when its owner is known
//! to be dead or nothing listens on it; one a live holder announced is never
//! even connected to. A holder counts as live while its pid runs the binary
//! its announcement names (`exe=`); a pid since reused by another program
//! leaves the announcement and its socket to go once nothing listens there.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::launch::{alive, Instance};

/// Prefixes of the sockets named for the pid of the process serving them,
/// `<prefix><pid>.sock`: vornd's app and grid endpoints.
pub const PID_NAMED: &[&str] = &["vornd-app-", "vornd-grid-"];

/// How old a socket no live owner claims must be before a refused connect
/// counts as its owner gone: a process between bind and listen refuses too.
pub const GRACE: Duration = Duration::from_secs(5);

/// Removes what dead owners left in `home`'s `run/`, and answers how many
/// files went. Blocks on the file system and on connects to local sockets,
/// so async callers run it on a blocking thread.
pub fn sweep(home: &Path) -> usize {
    sweep_older_than(home, GRACE)
}

/// [`sweep`], with the age a socket no live owner claims must reach before
/// a refused connect removes it.
pub fn sweep_older_than(home: &Path, grace: Duration) -> usize {
    let run = home.join("run");
    let Ok(dir) = fs::read_dir(&run) else {
        return 0;
    };
    let (infos, others): (Vec<PathBuf>, Vec<PathBuf>) = dir
        .flatten()
        .map(|e| e.path())
        .partition(|p| p.extension().is_some_and(|x| x == "info"));
    let mut removed = 0;
    let mut live = HashSet::new();
    for path in infos {
        let Some(i) = fs::read_to_string(&path)
            .ok()
            .as_deref()
            .and_then(crate::launch::parse)
        else {
            continue;
        };
        let gone = !alive(i.pid) || (reused(&i) && !listens(Path::new(&i.endpoint)));
        if gone {
            removed += forget(&run, &path, &i);
        } else {
            live.insert(PathBuf::from(i.endpoint));
        }
    }
    for path in others {
        if is_socket(&path) && !live.contains(&path) && owner_gone(&path, grace) {
            removed += usize::from(fs::remove_file(&path).is_ok());
        }
    }
    removed
}

/// Removes the announcement at `info` of the dead holder `i`, and its socket
/// when that is in `run`; answers how many files went.
pub(crate) fn forget(run: &Path, info: &Path, i: &Instance) -> usize {
    let endpoint = Path::new(&i.endpoint);
    let socket =
        endpoint.parent() == Some(run) && is_socket(endpoint) && fs::remove_file(endpoint).is_ok();
    usize::from(fs::remove_file(info).is_ok()) + usize::from(socket)
}

/// Whether `i`'s pid now runs a binary other than the one it announced.
/// An announcement without `exe=`, or a pid whose binary cannot be read, is
/// taken at its word.
fn reused(i: &Instance) -> bool {
    i.exe.is_some() && exe_of(i.pid).is_some_and(|now| !i.runs(&now))
}

/// Whether something accepts connections on the socket at `path`.
fn listens(path: &Path) -> bool {
    is_socket(path) && !refused(path)
}

/// The binary process `pid` runs.
#[cfg(target_os = "linux")]
fn exe_of(pid: u32) -> Option<PathBuf> {
    fs::read_link(format!("/proc/{pid}/exe")).ok()
}

/// The binary process `pid` runs.
#[cfg(target_os = "macos")]
fn exe_of(pid: u32) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let pid = libc::c_int::try_from(pid).ok()?;
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let len = u32::try_from(buf.len()).ok()?;
    // SAFETY: `buf` is writable for the `len` bytes passed with it.
    let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), len) };
    let n = usize::try_from(n).ok().filter(|&n| n > 0)?;
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(buf.get(..n)?)))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn exe_of(_: u32) -> Option<PathBuf> {
    None
}

/// Whether the socket at `path`, which no live holder announced, has lost
/// its owner: the pid it is named for is dead, or it is older than `grace`
/// and refuses a connect.
fn owner_gone(path: &Path, grace: Duration) -> bool {
    if pid_named(path).is_some_and(|pid| !alive(pid)) {
        return true;
    }
    let age = fs::symlink_metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .unwrap_or_default();
    age >= grace && refused(path)
}

/// The pid a [`PID_NAMED`] socket is named for.
fn pid_named(path: &Path) -> Option<u32> {
    let name = path.file_name()?.to_str()?.strip_suffix(".sock")?;
    PID_NAMED
        .iter()
        .find_map(|p| name.strip_prefix(p))
        .and_then(|pid| pid.parse().ok())
}

#[cfg(unix)]
fn is_socket(path: &Path) -> bool {
    use std::os::unix::fs::FileTypeExt;
    fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_socket())
}

// Windows endpoints are named pipes, which leave no file behind.
#[cfg(windows)]
fn is_socket(_: &Path) -> bool {
    false
}

#[cfg(unix)]
fn refused(path: &Path) -> bool {
    matches!(
        std::os::unix::net::UnixStream::connect(path),
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused
    )
}

#[cfg(windows)]
fn refused(_: &Path) -> bool {
    false
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::launch::announce;
    use std::os::unix::net::UnixListener;

    /// A pid no process has: a child that has exited and been reaped.
    fn dead_pid() -> u32 {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(!alive(pid));
        pid
    }

    fn run(home: &Path) -> PathBuf {
        let run = home.join("run");
        fs::create_dir_all(&run).unwrap();
        run
    }

    /// A socket file nothing listens on, as a killed owner leaves it.
    fn stale(path: &Path) {
        drop(UnixListener::bind(path).unwrap());
        assert!(is_socket(path));
    }

    fn info(home: &Path, endpoint: &Path, pid: u32, instance: u128) -> PathBuf {
        info_running(home, endpoint, pid, instance, std::env::current_exe().ok())
    }

    fn info_running(
        home: &Path,
        endpoint: &Path,
        pid: u32,
        instance: u128,
        exe: Option<PathBuf>,
    ) -> PathBuf {
        announce(
            home,
            &Instance {
                endpoint: endpoint.to_string_lossy().into_owned(),
                pid,
                proto: 1,
                build: "test".into(),
                instance,
                exe,
            },
        )
        .unwrap();
        home.join("run").join(format!("sessiond-{instance:x}.info"))
    }

    #[test]
    fn a_live_holders_socket_and_announcement_are_kept() {
        let home = tempfile::tempdir().unwrap();
        let sock = run(home.path()).join("sessiond-1-a.sock");
        let _listening = UnixListener::bind(&sock).unwrap();
        let announced = info(home.path(), &sock, std::process::id(), 0xa);
        let unannounced = home.path().join("run").join("sessiond-1-b.sock");
        let _also = UnixListener::bind(&unannounced).unwrap();
        let mine = home
            .path()
            .join("run")
            .join(format!("vornd-app-{}.sock", std::process::id()));
        let _mine = UnixListener::bind(&mine).unwrap();

        assert_eq!(sweep_older_than(home.path(), Duration::ZERO), 0);
        assert!(sock.exists() && announced.exists());
        assert!(unannounced.exists() && mine.exists());
    }

    #[test]
    fn a_live_holders_socket_is_kept_even_while_it_refuses() {
        let home = tempfile::tempdir().unwrap();
        let sock = run(home.path()).join("sessiond-1-a.sock");
        stale(&sock);
        info(home.path(), &sock, std::process::id(), 0xa);
        assert_eq!(sweep_older_than(home.path(), Duration::ZERO), 0);
        assert!(sock.exists());
    }

    #[test]
    fn a_dead_owners_sockets_and_announcement_are_removed() {
        let home = tempfile::tempdir().unwrap();
        let run = run(home.path());
        let dead = dead_pid();
        let announced = run.join("sessiond-1-a.sock");
        stale(&announced);
        let announcement = info(home.path(), &announced, dead, 0xa);
        let orphan = run.join("sessiond-1-b.sock");
        stale(&orphan);
        let app = run.join(format!("vornd-app-{dead}.sock"));
        let grid = run.join(format!("vornd-grid-{dead}.sock"));
        stale(&app);
        stale(&grid);
        // A live pid that is not the socket's owner any more.
        let reused = run.join(format!("vornd-grid-{}.sock", std::process::id()));
        stale(&reused);

        assert_eq!(sweep_older_than(home.path(), Duration::ZERO), 6);
        for p in [&announced, &announcement, &orphan, &app, &grid, &reused] {
            assert!(!p.exists(), "{} is left", p.display());
        }
    }

    /// A live pid running another binary than the one announced is a reused
    /// pid: its socket goes once nothing listens on it, never while it does.
    #[test]
    fn a_reused_pid_holds_its_socket_only_while_it_listens() {
        let home = tempfile::tempdir().unwrap();
        let run = run(home.path());
        let me = std::process::id();
        let other = Some(PathBuf::from("/nonexistent/vorn-sessiond"));
        let refusing = run.join("sessiond-1-a.sock");
        stale(&refusing);
        let dropped = info_running(home.path(), &refusing, me, 0xa, other.clone());
        let listening = run.join("sessiond-1-b.sock");
        let _listener = UnixListener::bind(&listening).unwrap();
        let kept = info_running(home.path(), &listening, me, 0xb, other);
        let unnamed = run.join("sessiond-1-c.sock");
        stale(&unnamed);
        let older = info_running(home.path(), &unnamed, me, 0xc, None);

        assert_eq!(sweep_older_than(home.path(), Duration::ZERO), 2);
        assert!(!refusing.exists() && !dropped.exists());
        assert!(listening.exists() && kept.exists());
        assert!(unnamed.exists() && older.exists());
    }

    #[test]
    fn a_young_refusing_socket_waits_for_its_grace() {
        let home = tempfile::tempdir().unwrap();
        let sock = run(home.path()).join("sessiond-1-a.sock");
        stale(&sock);
        assert_eq!(sweep(home.path()), 0);
        assert!(sock.exists());
    }

    #[test]
    fn another_homes_sockets_are_untouched() {
        let mine = tempfile::tempdir().unwrap();
        let theirs = tempfile::tempdir().unwrap();
        run(mine.path());
        let dead = dead_pid();
        let named = run(theirs.path()).join("sessiond-1-a.sock");
        stale(&named);
        let announcement = info(mine.path(), &named, dead, 0xa);
        let unnamed = theirs
            .path()
            .join("run")
            .join(format!("vornd-app-{dead}.sock"));
        stale(&unnamed);

        assert_eq!(sweep_older_than(mine.path(), Duration::ZERO), 1);
        assert!(!announcement.exists());
        assert!(named.exists() && unnamed.exists());
    }

    #[test]
    fn only_sockets_are_swept() {
        let home = tempfile::tempdir().unwrap();
        let run = run(home.path());
        let dead = dead_pid();
        let file = run.join(format!("vornd-app-{dead}.sock"));
        fs::write(&file, "").unwrap();
        let announcement = run.join("vornd-app");
        fs::write(&announcement, "{}").unwrap();
        assert_eq!(sweep_older_than(home.path(), Duration::ZERO), 0);
        assert!(file.exists() && announcement.exists());
    }

    #[test]
    fn a_missing_run_dir_is_nothing_to_sweep() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(sweep(home.path()), 0);
    }

    #[test]
    fn pid_named_sockets_are_read_by_their_prefix() {
        assert_eq!(pid_named(Path::new("/r/vornd-app-42.sock")), Some(42));
        assert_eq!(pid_named(Path::new("/r/vornd-grid-7.sock")), Some(7));
        assert_eq!(pid_named(Path::new("/r/sessiond-1-42.sock")), None);
        assert_eq!(pid_named(Path::new("/r/vornd-app-x.sock")), None);
        assert_eq!(pid_named(Path::new("/r/vornd-app")), None);
    }
}
