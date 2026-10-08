//! CPU and memory of a process group, sampled with `proc_pid_rusage`.
//!
//! The client's group is the client and every process macOS holds it
//! responsible for, so a webview's out-of-process helpers count against
//! the client that started them. A client the harness starts is not its own
//! responsible process (the terminal is), so the system webview's helpers
//! (WebKit's XPC services) are also matched by name and start time.

use std::collections::BTreeMap;
use std::time::Instant;

use serde_json::{json, Value};

#[repr(C)]
#[derive(Default)]
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

#[repr(C)]
struct Timebase {
    numer: u32,
    denom: u32,
}

extern "C" {
    fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut RusageV2) -> i32;
    fn proc_listallpids(buffer: *mut i32, size: i32) -> i32;
    fn responsibility_get_pid_responsible_for_pid(pid: i32) -> i32;
    fn mach_timebase_info(info: *mut Timebase) -> i32;
    fn proc_pidpath(pid: i32, buffer: *mut u8, size: u32) -> i32;
}

/// When a process started (mach absolute time), from rusage v2.
fn start_abs(pid: i32) -> Option<u64> {
    let mut r = RusageV2::default();
    // SAFETY: as in `usage`.
    let ok = unsafe { proc_pid_rusage(pid, 2, &mut r) } == 0;
    ok.then_some(r.rest[0])
}

fn exe_path(pid: i32) -> String {
    let mut buf = vec![0u8; 4096];
    // SAFETY: the buffer is as long as the size given.
    let n = unsafe { proc_pidpath(pid, buf.as_mut_ptr(), buf.len() as u32) };
    buf.truncate(n.max(0) as usize);
    String::from_utf8_lossy(&buf).into_owned()
}

#[derive(Clone, Copy, Default)]
struct Usage {
    cpu_ns: u64,
    rss: u64,
    footprint: u64,
}

fn usage(pid: i32, tb: (u64, u64)) -> Option<Usage> {
    let mut r = RusageV2::default();
    // SAFETY: `r` is larger than rusage_info_v2 and lives across the call.
    let ok = unsafe { proc_pid_rusage(pid, 2, &mut r) } == 0;
    ok.then(|| Usage {
        cpu_ns: (r.user_time + r.system_time) * tb.0 / tb.1,
        rss: r.resident_size,
        footprint: r.phys_footprint,
    })
}

fn all_pids() -> Vec<i32> {
    let mut v = vec![0i32; 8192];
    // SAFETY: the buffer is as long as the size given, in bytes.
    let n = unsafe { proc_listallpids(v.as_mut_ptr(), (v.len() * 4) as i32) };
    v.truncate(n.max(0) as usize);
    v
}

/// CPU time of a process `proc_pid_rusage` may not read (WindowServer):
/// `ps`'s `[[hh:]mm:]ss.cc`, in nanoseconds.
fn ps_cpu_ns(pid: i32) -> Option<u64> {
    let out = std::process::Command::new("ps")
        .args(["-o", "time=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let t = String::from_utf8_lossy(&out.stdout);
    let mut secs = 0.0;
    for part in t.trim().split(':') {
        secs = secs * 60.0 + part.parse::<f64>().ok()?;
    }
    Some((secs * 1e9) as u64)
}

fn window_server() -> Option<i32> {
    let out = std::process::Command::new("pgrep")
        .args(["-x", "WindowServer"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).split_whitespace().next()?.parse().ok()
}

pub struct Sampler {
    window_server: Option<i32>,
    client: i32,
    client_start: u64,
    client_resp: i32,
    vornd: i32,
    sessiond: Option<i32>,
    tb: (u64, u64),
    start: Instant,
    /// Helpers seen so far and their last usage (they may exit).
    helpers: BTreeMap<i32, Usage>,
    rows: Vec<Value>,
    marks: Vec<(String, f64)>,
}

impl Sampler {
    pub fn new(client: i32, vornd: i32, sessiond: Option<i32>) -> Sampler {
        let mut t = Timebase { numer: 1, denom: 1 };
        // SAFETY: plain out-parameter.
        unsafe { mach_timebase_info(&mut t) };
        Sampler {
            window_server: window_server(),
            client,
            client_start: start_abs(client).unwrap_or(0),
            // SAFETY: takes and returns a pid.
            client_resp: unsafe { responsibility_get_pid_responsible_for_pid(client) },
            vornd,
            sessiond,
            tb: (u64::from(t.numer), u64::from(t.denom)),
            start: Instant::now(),
            helpers: BTreeMap::new(),
            rows: Vec::new(),
            marks: Vec::new(),
        }
    }

    pub fn mark(&mut self, what: &str) {
        self.marks
            .push((what.to_owned(), self.start.elapsed().as_secs_f64()));
    }

    pub fn sample(&mut self) {
        let t = self.start.elapsed().as_secs_f64();
        let Some(main) = usage(self.client, self.tb) else {
            return;
        };
        for pid in all_pids() {
            if pid == self.client || pid <= 1 {
                continue;
            }
            // SAFETY: takes and returns a pid.
            let resp = unsafe { responsibility_get_pid_responsible_for_pid(pid) };
            let webkit = || {
                resp == self.client_resp
                    && start_abs(pid).is_some_and(|t| t >= self.client_start)
                    && exe_path(pid).contains("com.apple.WebKit.")
            };
            if resp == self.client || self.helpers.contains_key(&pid) || webkit() {
                if let Some(u) = usage(pid, self.tb) {
                    self.helpers.insert(pid, u);
                }
            }
        }
        let helpers: Vec<Usage> = self.helpers.values().copied().collect();
        let h_cpu: u64 = helpers.iter().map(|u| u.cpu_ns).sum();
        let h_rss: u64 = helpers.iter().map(|u| u.rss).sum();
        let h_fp: u64 = helpers.iter().map(|u| u.footprint).sum();
        let v = usage(self.vornd, self.tb).unwrap_or_default();
        let s = self
            .sessiond
            .and_then(|p| usage(p, self.tb))
            .unwrap_or_default();
        self.rows.push(json!([
            t,
            main.cpu_ns + h_cpu,
            main.rss + h_rss,
            main.footprint + h_fp,
            v.cpu_ns,
            v.rss,
            s.cpu_ns,
            helpers.len(),
            main.footprint,
            self.window_server.and_then(ps_cpu_ns).unwrap_or(0),
        ]));
    }

    pub fn to_json(&self) -> Value {
        json!({
            "columns": ["t_s", "client_cpu_ns", "client_rss", "client_footprint",
                        "vornd_cpu_ns", "vornd_rss", "sessiond_cpu_ns", "helpers",
                        "client_main_footprint", "windowserver_cpu_ns"],
            "rows": self.rows,
            "marks": self.marks.iter().map(|(k, t)| json!([k, t])).collect::<Vec<_>>(),
        })
    }
}
