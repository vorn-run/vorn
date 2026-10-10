//! This process's CPU time and memory, and killing a stray child by pid.
//! Each prototype measures itself, so the numbers never include vornd, the
//! session holder or the load producers.

/// What the OS charges this process for so far.
#[derive(Debug, Clone, Copy, Default)]
pub struct Usage {
    /// User plus system CPU seconds.
    pub cpu_s: f64,
    /// Peak resident set (working set on Windows), in bytes.
    pub peak_rss: u64,
    /// What the OS reports as the process's memory now: the physical
    /// footprint on macOS (Activity Monitor's figure), private bytes on
    /// Windows, resident set elsewhere.
    pub footprint: u64,
}

#[cfg(target_os = "macos")]
pub fn usage() -> Usage {
    #[repr(C)]
    struct RusageV2 {
        uuid: [u8; 16],
        user_time: u64,
        system_time: u64,
        pkg_idle_wkups: u64,
        interrupt_wkups: u64,
        pageins: u64,
        wired_size: u64,
        resident_size: u64,
        phys_footprint: u64,
        rest: [u64; 24],
    }
    extern "C" {
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut RusageV2) -> i32;
    }
    let (cpu_s, peak_rss) = getrusage();
    // SAFETY: the struct is larger than rusage_info_v2 and outlives the call.
    let mut r: RusageV2 = unsafe { std::mem::zeroed() };
    let ok = unsafe { proc_pid_rusage(std::process::id() as i32, 2, &mut r) } == 0;
    Usage {
        cpu_s,
        peak_rss,
        footprint: if ok { r.phys_footprint } else { 0 },
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn usage() -> Usage {
    let (cpu_s, peak_rss) = getrusage();
    let rss = std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
        .map_or(0, |pages| pages * 4096);
    Usage {
        cpu_s,
        peak_rss,
        footprint: rss,
    }
}

#[cfg(unix)]
fn getrusage() -> (f64, u64) {
    // SAFETY: getrusage writes the struct it is given.
    let mut r: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut r) };
    let s = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    // ru_maxrss is bytes on macOS and kilobytes on Linux.
    let scale = if cfg!(target_os = "macos") { 1 } else { 1024 };
    (s(r.ru_utime) + s(r.ru_stime), r.ru_maxrss as u64 * scale)
}

#[cfg(windows)]
pub fn usage() -> Usage {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) as f64 / 1e7;
    // SAFETY: every out-pointer is a live local of the right type and size.
    unsafe {
        let p = GetCurrentProcess();
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut c, mut e, mut k, mut u) = (zero, zero, zero, zero);
        GetProcessTimes(p, &mut c, &mut e, &mut k, &mut u);
        let mut m: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        m.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        GetProcessMemoryInfo(
            p,
            (&mut m as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
            m.cb,
        );
        Usage {
            cpu_s: ft(k) + ft(u),
            peak_rss: m.PeakWorkingSetSize as u64,
            footprint: m.PrivateUsage as u64,
        }
    }
}

#[cfg(unix)]
pub fn kill(pid: u32) {
    // SAFETY: kill(2) takes any pid; a stale one only fails.
    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
}

#[cfg(windows)]
pub fn kill(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    // SAFETY: the handle is checked and closed; a stale pid only fails to open.
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !h.is_null() {
            TerminateProcess(h, 1);
            CloseHandle(h);
        }
    }
}
