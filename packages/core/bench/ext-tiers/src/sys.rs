//! A process's memory and CPU time.

/// A process's physical footprint, in MB: what Activity Monitor shows.
#[cfg(target_os = "macos")]
pub fn memory(pid: u32) -> Option<f64> {
    let pid = i32::try_from(pid).ok()?;
    // SAFETY: rusage_info_v4 is plain data that proc_pid_rusage fills for the flavour named.
    let info = unsafe {
        let mut info: libc::rusage_info_v4 = std::mem::zeroed();
        let ptr = std::ptr::addr_of_mut!(info).cast::<libc::rusage_info_t>();
        (libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V4, ptr) == 0).then_some(info)
    }?;
    #[allow(clippy::cast_precision_loss)]
    Some(info.ri_phys_footprint as f64 / (1024.0 * 1024.0))
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
        let ptr = std::ptr::addr_of_mut!(info).cast::<libc::rusage_info_t>();
        let mut base = Timebase { numer: 1, denom: 1 };
        mach_timebase_info(&mut base);
        (
            (libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V4, ptr) == 0).then_some(info)?,
            base,
        )
    };
    // The times are in Mach ticks, which the timebase turns into nanoseconds.
    #[allow(clippy::cast_precision_loss)]
    let ns = (info.ri_user_time + info.ri_system_time) as f64 * f64::from(base.numer)
        / f64::from(base.denom.max(1));
    Some(ns / 1e9)
}

/// A process's resident size in MB, from ps: there is no footprint off macOS.
#[cfg(not(target_os = "macos"))]
pub fn memory(pid: u32) -> Option<f64> {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let kb: f64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    Some(kb / 1024.0)
}

/// A process's CPU time, in seconds, from ps.
#[cfg(not(target_os = "macos"))]
pub fn cpu(pid: u32) -> Option<f64> {
    let out = std::process::Command::new("ps")
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
