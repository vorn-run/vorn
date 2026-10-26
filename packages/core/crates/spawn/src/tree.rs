//! What a process and everything it started are running.
//!
//! [`runs_any`] walks the tree under a process (its children, theirs, and
//! so on) and says whether any of them runs a program by one of the given
//! names. It reads the process table once per call, so call it when the
//! answer is needed, not per byte.

/// Whether process `root`, or any process under it, runs a program named
/// one of `names` (the file name, without `.exe` on Windows, in any case,
/// or as it renames itself: the name then `:`, as `tmux: client`).
/// `None` when the process table cannot be read or `root` is not running.
pub fn runs_any(root: u32, names: &[&str]) -> Option<bool> {
    let named = |name: &str| is_named(name, names);
    let table = table()?;
    if !table.iter().any(|p| p.pid == root) {
        return None;
    }
    let mut seen = vec![root];
    let mut next = 0;
    while let Some(&pid) = seen.get(next) {
        next += 1;
        for p in &table {
            if p.pid == pid && named(&p.name) {
                return Some(true);
            }
            if p.ppid == pid && p.pid != pid && !seen.contains(&p.pid) {
                seen.push(p.pid);
            }
        }
    }
    Some(false)
}

/// Whether a process named `name` runs one of `names`.
fn is_named(name: &str, names: &[&str]) -> bool {
    let name = name.strip_suffix(".exe").unwrap_or(name);
    names.iter().any(|n| {
        name.get(..n.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(n))
            && matches!(name.as_bytes().get(n.len()), None | Some(b':'))
    })
}

/// One row of the process table.
#[derive(Debug)]
struct Proc {
    pid: u32,
    ppid: u32,
    name: String,
}

#[cfg(target_os = "linux")]
fn table() -> Option<Vec<Proc>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse().ok()) else {
            continue;
        };
        // Gone since the directory was read.
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        // `pid (comm) state ppid ...`; comm may hold spaces and parentheses.
        let (Some(open), Some(close)) = (stat.find('('), stat.rfind(')')) else {
            continue;
        };
        let ppid = stat[close + 1..]
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok());
        if let Some(ppid) = ppid {
            out.push(Proc {
                pid,
                ppid,
                name: stat[open + 1..close].to_owned(),
            });
        }
    }
    Some(out)
}

#[cfg(target_os = "macos")]
fn table() -> Option<Vec<Proc>> {
    use std::ffi::CStr;
    let mut pids = vec![0 as libc::pid_t; 8192];
    let size = libc::c_int::try_from(std::mem::size_of_val(pids.as_slice())).ok()?;
    // SAFETY: the buffer holds `size` bytes of pids for the call to fill.
    let n = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), size) };
    let n = usize::try_from(n).ok()?;
    if n >= pids.len() {
        // The table may not fit: unknown rather than half read.
        return None;
    }
    let mut out = Vec::with_capacity(n);
    for &pid in &pids[..n] {
        // SAFETY: a zeroed proc_bsdinfo is a valid one to be written over.
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let want = libc::c_int::try_from(std::mem::size_of::<libc::proc_bsdinfo>()).ok()?;
        // SAFETY: the buffer is one proc_bsdinfo, of the size given.
        let got = unsafe {
            libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), want)
        };
        if got != want {
            // Gone, or not ours to read: it cannot be under a session's shell.
            continue;
        }
        // SAFETY: pbi_comm is NUL-terminated within its array by the kernel.
        let name = unsafe { CStr::from_ptr(info.pbi_comm.as_ptr()) };
        out.push(Proc {
            pid: info.pbi_pid,
            ppid: info.pbi_ppid,
            name: name.to_string_lossy().into_owned(),
        });
    }
    Some(out)
}

#[cfg(windows)]
fn table() -> Option<Vec<Proc>> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    // SAFETY: the snapshot is checked and closed, and the entry is sized as
    // Process32FirstW requires.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut out = Vec::new();
        let mut more = Process32FirstW(snap, &mut entry) != 0;
        while more {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            out.push(Proc {
                pid: entry.th32ProcessID,
                ppid: entry.th32ParentProcessID,
                name: String::from_utf16_lossy(&entry.szExeFile[..len]),
            });
            more = Process32NextW(snap, &mut entry) != 0;
        }
        CloseHandle(snap);
        Some(out)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn table() -> Option<Vec<Proc>> {
    None
}

#[cfg(test)]
mod names {
    use super::is_named;

    #[test]
    fn matches_a_name_or_the_name_a_program_gives_itself() {
        let names = ["tmux", "ssh"];
        for name in [
            "tmux",
            "TMUX",
            "tmux: client",
            "tmux: server",
            "ssh.exe",
            "ssh:",
        ] {
            assert!(is_named(name, &names), "{name}");
        }
        for name in ["tmuxinator", "sshd", "tmu", "", "xtmux", "tmux client", "é"] {
            assert!(!is_named(name, &names), "{name}");
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn finds_a_program_anywhere_under_the_process() {
        let me = std::process::id();
        // This test binary is under itself, and nothing named so is.
        assert_eq!(runs_any(me, &["no-such-program-here"]), Some(false));
        // A name no other test runs, as tests run side by side.
        let dir = std::env::temp_dir().join(format!("vorn-tree-{me}"));
        std::fs::create_dir_all(&dir).unwrap();
        // This test binary, renamed, running the test that only waits.
        let waits = dir.join("treewaiter");
        std::fs::copy(std::env::current_exe().unwrap(), &waits).unwrap();
        let mut child = crate::command(&waits)
            .args(["--ignored", "--exact", "tree::tests::waits_a_while"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("it starts");
        // Until it execs, the child still has this binary's name.
        let named = (0..100).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(20));
            runs_any(me, &["TREEWAITER"]) == Some(true)
        });
        assert!(named);
        assert_eq!(runs_any(child.id(), &["treewaiter"]), Some(true));
        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(runs_any(me, &["treewaiter"]), Some(false));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// What the renamed copy of this binary runs, above.
    #[test]
    #[ignore = "run only as a stand-in program"]
    fn waits_a_while() {
        std::thread::sleep(std::time::Duration::from_secs(30));
    }

    #[test]
    fn knows_nothing_of_a_process_not_running() {
        let mut child = crate::command("true").spawn().expect("true starts");
        let pid = child.id();
        child.wait().unwrap();
        assert_eq!(runs_any(pid, &["true"]), None);
    }
}
