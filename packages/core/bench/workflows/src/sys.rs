//! The machine's side: process memory, CPU, the builds that would spoil a timing, and what this machine is.

use std::process::Command;
use std::time::Duration;

use serde_json::{json, Value};

/// A process's physical footprint and resident size, in MB.
#[cfg(target_os = "macos")]
pub fn memory(pid: u32) -> Option<(f64, f64)> {
    let pid = i32::try_from(pid).ok()?;
    // SAFETY: rusage_info_v4 is plain data that proc_pid_rusage fills for the flavour named.
    let info = unsafe {
        let mut info: libc::rusage_info_v4 = std::mem::zeroed();
        let ptr = (&mut info as *mut libc::rusage_info_v4).cast::<libc::rusage_info_t>();
        (libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V4, ptr) == 0).then_some(info)
    }?;
    let mb = |b: u64| b as f64 / (1024.0 * 1024.0);
    Some((mb(info.ri_phys_footprint), mb(info.ri_resident_size)))
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct Timebase {
    numer: u32,
    denom: u32,
}

#[cfg(target_os = "macos")]
extern "C" {
    fn mach_timebase_info(info: *mut Timebase) -> libc::c_int;
}

/// A process's CPU time (user and system), in seconds.
#[cfg(target_os = "macos")]
pub fn cpu(pid: u32) -> Option<f64> {
    let pid = i32::try_from(pid).ok()?;
    // SAFETY: both are plain data the calls fill for the flavour named.
    let (info, base) = unsafe {
        let mut info: libc::rusage_info_v4 = std::mem::zeroed();
        let ptr = (&mut info as *mut libc::rusage_info_v4).cast::<libc::rusage_info_t>();
        let mut base = Timebase { numer: 1, denom: 1 };
        mach_timebase_info(&mut base);
        (
            (libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V4, ptr) == 0).then_some(info)?,
            base,
        )
    };
    // The times are in Mach ticks, which the timebase turns into nanoseconds.
    let ns = (info.ri_user_time + info.ri_system_time) as f64 * f64::from(base.numer)
        / f64::from(base.denom.max(1));
    Some(ns / 1e9)
}

/// A process's CPU time, in seconds, from ps.
#[cfg(not(target_os = "macos"))]
pub fn cpu(pid: u32) -> Option<f64> {
    let out = Command::new("ps")
        .args(["-o", "time=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let t = String::from_utf8_lossy(&out.stdout);
    Some(
        t.trim()
            .split(':')
            .fold(0.0, |acc, p| acc * 60.0 + p.parse::<f64>().unwrap_or(0.0)),
    )
}

/// A process's resident size, in MB, twice: there is no footprint off macOS.
#[cfg(not(target_os = "macos"))]
pub fn memory(pid: u32) -> Option<(f64, f64)> {
    let out = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let kb: f64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    Some((kb / 1024.0, kb / 1024.0))
}

/// Compilers and builds running on this machine now, which a timed run waits out.
pub fn builds() -> Vec<String> {
    let Ok(out) = Command::new("ps").args(["-Ao", "comm="]).output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.rsplit('/').next())
        .filter(|c| {
            matches!(
                *c,
                "cargo" | "rustc" | "zig" | "clang" | "cc" | "ld" | "ld64" | "swift-frontend"
            )
        })
        .map(str::to_owned)
        .collect()
}

/// Looks for builds once a second while a case runs, so a timing a build overlapped can be thrown out and run again.
pub struct BuildWatch {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: std::thread::JoinHandle<Vec<String>>,
}

impl BuildWatch {
    pub fn start() -> BuildWatch {
        use std::sync::atomic::{AtomicBool, Ordering};
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let s = std::sync::Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            let mut seen: Vec<String> = Vec::new();
            while !s.load(Ordering::Relaxed) {
                for b in builds() {
                    if !seen.contains(&b) {
                        seen.push(b);
                    }
                }
                for _ in 0..10 {
                    if s.load(Ordering::Relaxed) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            seen
        });
        BuildWatch { stop, thread }
    }

    /// The build processes seen while the case ran, none when it was quiet.
    pub fn finish(self) -> Vec<String> {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        self.thread.join().unwrap_or_default()
    }
}

/// The load average over the last minute.
pub fn load() -> f64 {
    let mut avg = [0f64; 3];
    // SAFETY: getloadavg writes at most the three entries it is given.
    let n = unsafe { libc::getloadavg(avg.as_mut_ptr(), 3) };
    if n < 1 {
        0.0
    } else {
        avg[0]
    }
}

pub fn machine() -> Value {
    let sysctl = |k: &str| {
        Command::new("sysctl")
            .args(["-n", k])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .filter(|s| !s.is_empty())
    };
    json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "cpu": sysctl("machdep.cpu.brand_string"),
        "cores": std::thread::available_parallelism().map_or(0, |n| n.get()),
        "memory_gb": sysctl("hw.memsize").and_then(|m| m.parse::<f64>().ok()).map(|b| (b / 1e9).round()),
    })
}
