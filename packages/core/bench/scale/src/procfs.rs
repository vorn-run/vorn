//! What a process costs, read from Linux's `/proc`: resident memory,
//! threads and open descriptors, plus the machine's free memory and the
//! kernel limits a tier can run into. On other Unix systems every read
//! fails, which the bench reports rather than guesses around; off Unix only
//! the types a report reads are built, since `run` is Unix-only.

#[cfg(unix)]
use std::collections::HashMap;
#[cfg(unix)]
use std::io;
#[cfg(unix)]
use std::path::Path;

use serde::{Deserialize, Serialize};

/// One process at one moment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Resident set, in bytes.
    pub rss: u64,
    pub threads: u64,
    pub fds: u64,
}

#[cfg(unix)]
pub fn usage(pid: u32) -> io::Result<Usage> {
    let proc = Path::new("/proc").join(pid.to_string());
    let status = std::fs::read_to_string(proc.join("status"))?;
    let (rss, threads) = parse_status(&status)
        .ok_or_else(|| io::Error::other(format!("no VmRSS or Threads for pid {pid}")))?;
    let fds = std::fs::read_dir(proc.join("fd"))?.count() as u64;
    Ok(Usage { rss, threads, fds })
}

/// `VmRSS` in bytes and `Threads` from `/proc/<pid>/status`.
#[cfg(unix)]
pub fn parse_status(text: &str) -> Option<(u64, u64)> {
    Some((field(text, "VmRSS:")? * 1024, field(text, "Threads:")?))
}

/// The first number after `key` at the start of a line.
#[cfg(unix)]
fn field(text: &str, key: &str) -> Option<u64> {
    text.lines()
        .find_map(|l| l.strip_prefix(key))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse().ok())
}

/// `RssAnon` in bytes: the private memory a process dirtied, without the
/// file pages (the binary, shared libraries) every copy of it shares.
#[cfg(unix)]
pub fn rss_anon(pid: u32) -> io::Result<u64> {
    let status = std::fs::read_to_string(Path::new("/proc").join(pid.to_string()).join("status"))?;
    field(&status, "RssAnon:")
        .map(|kb| kb * 1024)
        .ok_or_else(|| io::Error::other(format!("no RssAnon for pid {pid}")))
}

/// CPU time a process has used, user and system, in clock ticks.
#[cfg(unix)]
pub fn cpu_ticks(pid: u32) -> io::Result<u64> {
    let stat = std::fs::read_to_string(Path::new("/proc").join(pid.to_string()).join("stat"))?;
    parse_cpu_ticks(&stat).ok_or_else(|| io::Error::other(format!("no CPU times for pid {pid}")))
}

/// utime + stime, the 14th and 15th fields of `/proc/<pid>/stat`.
#[cfg(unix)]
pub fn parse_cpu_ticks(stat: &str) -> Option<u64> {
    let mut after = stat[stat.rfind(')')? + 1..].split_whitespace().skip(11);
    Some(after.next()?.parse::<u64>().ok()? + after.next()?.parse::<u64>().ok()?)
}

/// The machine's CPU time so far, busy and in all, in clock ticks, from
/// the first line of `/proc/stat`.
#[cfg(unix)]
pub fn machine_ticks() -> io::Result<(u64, u64)> {
    let text = std::fs::read_to_string("/proc/stat")?;
    parse_machine_ticks(&text).ok_or_else(|| io::Error::other("no cpu line in /proc/stat"))
}

/// Busy is everything but idle and iowait.
#[cfg(unix)]
pub fn parse_machine_ticks(text: &str) -> Option<(u64, u64)> {
    let line = text.lines().find(|l| l.starts_with("cpu "))?;
    let v: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let total: u64 = v.iter().sum();
    let idle = v.get(3)? + v.get(4).copied().unwrap_or(0);
    Some((total - idle, total))
}

/// `MemAvailable`, in bytes.
#[cfg(unix)]
pub fn mem_available() -> io::Result<u64> {
    meminfo("MemAvailable:")
}

#[cfg(unix)]
pub fn mem_total() -> io::Result<u64> {
    meminfo("MemTotal:")
}

#[cfg(unix)]
fn meminfo(key: &str) -> io::Result<u64> {
    let text = std::fs::read_to_string("/proc/meminfo")?;
    field(&text, key)
        .map(|kb| kb * 1024)
        .ok_or_else(|| io::Error::other(format!("no {key} in /proc/meminfo")))
}

/// The parent pid in `/proc/<pid>/stat`. The command name before it is in
/// parentheses and may hold spaces and parentheses itself.
#[cfg(unix)]
pub fn parse_ppid(stat: &str) -> Option<u32> {
    let after = &stat[stat.rfind(')')? + 1..];
    after.split_whitespace().nth(1)?.parse().ok()
}

/// Every live descendant of `roots`, children before grandchildren.
#[cfg(unix)]
pub fn descendants(roots: &[u32]) -> Vec<u32> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    for e in dir.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        if let Some(ppid) = std::fs::read_to_string(e.path().join("stat"))
            .ok()
            .as_deref()
            .and_then(parse_ppid)
        {
            children.entry(ppid).or_default().push(pid);
        }
    }
    tree(&children, roots)
}

#[cfg(unix)]
fn tree(children: &HashMap<u32, Vec<u32>>, roots: &[u32]) -> Vec<u32> {
    let mut out = Vec::new();
    let mut next: Vec<u32> = roots.to_vec();
    while !next.is_empty() {
        let level: Vec<u32> = next
            .iter()
            .filter_map(|p| children.get(p))
            .flatten()
            .copied()
            .collect();
        out.extend_from_slice(&level);
        next = level;
    }
    out
}

/// The kernel and process limits that bound a tier, as this host has them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub pty_max: Option<u64>,
    pub pid_max: Option<u64>,
    pub threads_max: Option<u64>,
    pub max_map_count: Option<u64>,
    pub nofile: Option<u64>,
    pub nproc: Option<u64>,
}

#[cfg(unix)]
impl Limits {
    pub fn read() -> Limits {
        let sysctl = |name: &str| {
            std::fs::read_to_string(Path::new("/proc/sys").join(name))
                .ok()
                .and_then(|t| t.trim().parse().ok())
        };
        Limits {
            pty_max: sysctl("kernel/pty/max"),
            pid_max: sysctl("kernel/pid_max"),
            threads_max: sysctl("kernel/threads-max"),
            max_map_count: sysctl("vm/max_map_count"),
            nofile: rlimit(Resource::Files),
            nproc: rlimit(Resource::Processes),
        }
    }
}

#[cfg(unix)]
enum Resource {
    Files,
    Processes,
}

/// The soft limit, `None` when unlimited or unknown.
#[cfg(unix)]
fn rlimit(r: Resource) -> Option<u64> {
    let resource = match r {
        Resource::Files => libc::RLIMIT_NOFILE,
        Resource::Processes => libc::RLIMIT_NPROC,
    };
    let mut lim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit only writes the struct it is given.
    let ok = unsafe { libc::getrlimit(resource, &mut lim) } == 0;
    (ok && lim.rlim_cur != libc::RLIM_INFINITY).then_some(lim.rlim_cur)
}

/// Bytes each of `n` sessions added between two readings; negative when
/// memory went back to the system in between.
pub fn per_session(before: u64, after: u64, n: usize) -> Option<i64> {
    let n = i64::try_from(n).ok().filter(|&n| n > 0)?;
    let delta = i64::try_from(after).ok()? - i64::try_from(before).ok()?;
    Some(delta / n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn reads_rss_and_threads_from_status() {
        let status =
            "Name:\tvorn-sessiond\nVmPeak:\t  9000 kB\nVmRSS:\t   1536 kB\nThreads:\t30003\n";
        assert_eq!(parse_status(status), Some((1536 * 1024, 30003)));
        assert_eq!(parse_status("Name:\tzombie\nThreads:\t1\n"), None);
    }

    #[cfg(unix)]
    #[test]
    fn cpu_times_come_after_the_name() {
        let stat = "42 (a b) c) S 7 42 42 0 -1 4194560 100 0 0 0 250 50 0 0 20 0 1 0";
        assert_eq!(parse_cpu_ticks(stat), Some(300));
        assert_eq!(parse_cpu_ticks("42 (x) S 7"), None);
        let text = "cpu  100 5 50 800 20 0 25 0 0 0\ncpu0 1 2 3 4\n";
        assert_eq!(parse_machine_ticks(text), Some((180, 1000)));
        assert_eq!(parse_machine_ticks("intr 1\n"), None);
    }

    #[test]
    fn the_parent_follows_the_last_parenthesis() {
        assert_eq!(parse_ppid("42 (bash) S 7 42 42 0"), Some(7));
        assert_eq!(parse_ppid("43 (a) b) (c) R 9 1 1"), Some(9));
        assert_eq!(parse_ppid("garbage"), None);
    }

    #[cfg(unix)]
    #[test]
    fn descendants_come_level_by_level() {
        let children = HashMap::from([(1, vec![2, 3]), (2, vec![4]), (4, vec![5]), (9, vec![10])]);
        assert_eq!(tree(&children, &[1]), [2, 3, 4, 5]);
        assert_eq!(tree(&children, &[6]), Vec::<u32>::new());
    }

    #[test]
    fn per_session_cost_is_signed_and_needs_sessions() {
        assert_eq!(per_session(1000, 3000, 4), Some(500));
        assert_eq!(per_session(3000, 1000, 4), Some(-500));
        assert_eq!(per_session(1000, 3000, 0), None);
    }
}
