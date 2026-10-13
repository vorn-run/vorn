//! Ending with another process.
//!
//! [`wait_for_exit`] blocks until a process ends, any process, not only a
//! child: a pidfd on Linux, a kqueue on macOS, the process handle on
//! Windows. vornd and vorn-sessiond end with the process [`EXIT_WITH`]
//! names, so a test run that sets it to its own pid leaves neither behind,
//! even when it is killed or panics past its cleanup. sessiond is detached
//! from whoever starts it, so a parent-death signal or a job object would
//! not reach it; this does.

use std::io;

/// The pid of a process to end with, for vornd and vorn-sessiond.
pub const EXIT_WITH: &str = "VORN_EXIT_WITH";

/// The process [`EXIT_WITH`] names, if it names one.
pub fn exit_with() -> Option<u32> {
    std::env::var(EXIT_WITH)
        .ok()?
        .trim()
        .parse()
        .ok()
        .filter(|&pid| pid != 0)
}

/// Runs `then` on a thread of its own once process `pid` has ended. Nothing
/// runs if it cannot be watched.
pub fn on_exit(pid: u32, then: impl FnOnce() + Send + 'static) {
    let _ = std::thread::Builder::new()
        .name("exit-with".into())
        .spawn(move || {
            if wait_for_exit(pid).is_ok() {
                then();
            }
        });
}

/// This process's parent, while it has one other than init.
pub fn parent() -> Option<u32> {
    #[cfg(unix)]
    {
        // SAFETY: getppid has no preconditions and cannot fail.
        let pid = unsafe { libc::getppid() };
        u32::try_from(pid).ok().filter(|&p| p > 1)
    }
    #[cfg(windows)]
    {
        windows::parent()
    }
}

/// Blocks until process `pid` has ended. Fails only when it cannot be
/// watched; a pid that names no process has ended.
pub fn wait_for_exit(pid: u32) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    return linux::wait(pid);
    #[cfg(target_os = "macos")]
    return macos::wait(pid);
    #[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
    return poll(pid);
    #[cfg(windows)]
    return windows::wait(pid);
}

/// Whether `pid` names a process, for the platforms that cannot wait on one.
#[cfg(all(unix, not(target_os = "macos")))]
fn alive(pid: u32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
    r == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn poll(pid: u32) -> io::Result<()> {
    while alive(pid) {
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
mod linux {
    use std::io;

    pub fn wait(pid: u32) -> io::Result<()> {
        // SAFETY: pidfd_open takes a pid and flags, and returns a new descriptor or -1.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) };
        if fd < 0 {
            let e = io::Error::last_os_error();
            return match e.raw_os_error() {
                Some(libc::ESRCH) => Ok(()),
                // A kernel before 5.3.
                Some(libc::ENOSYS) => super::poll(pid),
                _ => Err(e),
            };
        }
        let fd = fd as libc::c_int;
        let mut p = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let res = loop {
            // SAFETY: one valid pollfd, for a descriptor this function owns.
            if unsafe { libc::poll(&mut p, 1, -1) } >= 0 {
                break Ok(());
            }
            let e = io::Error::last_os_error();
            if e.kind() != io::ErrorKind::Interrupted {
                break Err(e);
            }
        };
        // SAFETY: closes the descriptor opened above, once.
        unsafe { libc::close(fd) };
        res
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::io;

    pub fn wait(pid: u32) -> io::Result<()> {
        // SAFETY: kqueue has no preconditions; it returns a new descriptor or -1.
        let kq = unsafe { libc::kqueue() };
        if kq < 0 {
            return Err(io::Error::last_os_error());
        }
        let change = libc::kevent {
            ident: pid as libc::uintptr_t,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD | libc::EV_ONESHOT | libc::EV_RECEIPT,
            fflags: libc::NOTE_EXIT,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        // SAFETY: a zeroed kevent is a valid one to be written over.
        let mut got: libc::kevent = unsafe { std::mem::zeroed() };
        // SAFETY: one change and room for one event, both valid for the call.
        let added = unsafe { libc::kevent(kq, &change, 1, &mut got, 1, std::ptr::null()) };
        let res = if added < 0 {
            Err(io::Error::last_os_error())
        } else if got.flags & libc::EV_ERROR != 0 && got.data != 0 {
            // EV_RECEIPT reports the registration: ESRCH is a process already gone.
            if got.data == libc::ESRCH as isize {
                Ok(())
            } else {
                Err(io::Error::from_raw_os_error(got.data as i32))
            }
        } else {
            loop {
                // SAFETY: no changes, and room for one event.
                let n =
                    unsafe { libc::kevent(kq, std::ptr::null(), 0, &mut got, 1, std::ptr::null()) };
                if n > 0 {
                    break Ok(());
                }
                let e = io::Error::last_os_error();
                if n < 0 && e.kind() != io::ErrorKind::Interrupted {
                    break Err(e);
                }
            }
        };
        // SAFETY: closes the descriptor opened above, once.
        unsafe { libc::close(kq) };
        res
    }
}

#[cfg(windows)]
mod windows {
    use std::io;

    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_INVALID_PARAMETER, INVALID_HANDLE_VALUE, WAIT_FAILED,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, WaitForSingleObject, INFINITE, PROCESS_SYNCHRONIZE,
    };

    pub fn wait(pid: u32) -> io::Result<()> {
        // SAFETY: the handle is checked, waited on and closed here.
        unsafe {
            let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if h.is_null() {
                // No process has that id.
                if GetLastError() == ERROR_INVALID_PARAMETER {
                    return Ok(());
                }
                return Err(io::Error::last_os_error());
            }
            let r = WaitForSingleObject(h, INFINITE);
            let res = if r == WAIT_FAILED {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            };
            CloseHandle(h);
            res
        }
    }

    pub fn parent() -> Option<u32> {
        // SAFETY: the snapshot is checked and closed, and the entry is
        // sized as Process32FirstW requires.
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE {
                return None;
            }
            let me = GetCurrentProcessId();
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut found = None;
            let mut more = Process32FirstW(snap, &mut entry) != 0;
            while more {
                if entry.th32ProcessID == me {
                    found = Some(entry.th32ParentProcessID).filter(|&p| p != 0);
                    break;
                }
                more = Process32NextW(snap, &mut entry) != 0;
            }
            CloseHandle(snap);
            found
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    fn sleeper() -> std::process::Child {
        let mut cmd = if cfg!(windows) {
            crate::command("ping")
        } else {
            crate::command("sleep")
        };
        if cfg!(windows) {
            cmd.args(["-n", "30", "127.0.0.1"]);
        } else {
            cmd.arg("30");
        }
        cmd.stdout(Stdio::null()).spawn().expect("a sleeper starts")
    }

    #[test]
    fn waits_until_the_process_ends() {
        let mut child = sleeper();
        let pid = child.id();
        let (tx, rx) = std::sync::mpsc::channel();
        on_exit(pid, move || {
            let _ = tx.send(Instant::now());
        });
        assert!(
            rx.recv_timeout(Duration::from_millis(300)).is_err(),
            "still running"
        );
        let killed = Instant::now();
        child.kill().unwrap();
        child.wait().unwrap();
        let ended = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("noticed the exit");
        assert!(ended >= killed);
    }

    #[test]
    fn a_process_already_gone_has_ended() {
        let mut child = sleeper();
        let pid = child.id();
        child.kill().unwrap();
        child.wait().unwrap();
        wait_for_exit(pid).expect("ended");
    }

    #[test]
    fn knows_its_parent() {
        assert_eq!(parent().map(|p| p != std::process::id()), Some(true));
    }
}
