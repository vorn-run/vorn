//! When this machine last started, which tells a session that ended before
//! it (one a reboot took) from one that ended while the machine ran.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The machine's boot, in milliseconds since the epoch; 0 when it cannot be
/// read, which reads every ended session as one a reboot did not take.
pub fn time_ms() -> i64 {
    let Some(at) = booted() else {
        return 0;
    };
    at.duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(target_os = "linux")]
fn booted() -> Option<SystemTime> {
    let stat = std::fs::read_to_string("/proc/stat").ok()?;
    btime(&stat).map(|secs| UNIX_EPOCH + Duration::from_secs(secs))
}

#[cfg(target_os = "macos")]
fn booted() -> Option<SystemTime> {
    let out = std::process::Command::new("/usr/sbin/sysctl")
        .args(["-n", "kern.boottime"])
        .output()
        .ok()?;
    let secs = boottime(&String::from_utf8_lossy(&out.stdout))?;
    Some(UNIX_EPOCH + Duration::from_secs(secs))
}

#[cfg(windows)]
fn booted() -> Option<SystemTime> {
    // SAFETY: GetTickCount64 takes nothing and only reads the system's tick count.
    let up = unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() };
    SystemTime::now().checked_sub(Duration::from_millis(up))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn booted() -> Option<SystemTime> {
    None
}

/// The `btime` line of `/proc/stat`: seconds since the epoch.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn btime(stat: &str) -> Option<u64> {
    stat.lines()
        .find_map(|l| l.strip_prefix("btime "))
        .and_then(|s| s.trim().parse().ok())
}

/// `sysctl -n kern.boottime`: `{ sec = 1791402086, usec = 970214 } Wed Oct …`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn boottime(out: &str) -> Option<u64> {
    let rest = out.split_once("sec = ")?.1;
    rest.split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_boot_from_each_systems_words() {
        assert_eq!(
            btime("cpu  1 2 3\nbtime 1791402086\nprocesses 9\n"),
            Some(1_791_402_086)
        );
        assert_eq!(btime("cpu 1 2 3\n"), None);
        assert_eq!(
            boottime("{ sec = 1791402086, usec = 970214 } Wed Oct  7 13:41:26 2026\n"),
            Some(1_791_402_086)
        );
        assert_eq!(boottime("unknown oid"), None);
    }

    #[test]
    fn the_machine_started_before_now() {
        let at = time_ms();
        if cfg!(any(target_os = "linux", target_os = "macos", windows)) {
            assert!(at > 0 && at < crate::registry::now_ms(), "{at}");
        }
    }
}
