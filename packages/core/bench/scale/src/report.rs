//! What a tier measured, as the JSON the bench writes, and the markdown
//! table `RESULTS.md` gets for it.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::procfs::{per_session, Limits, Usage};
use crate::stats::{Latency, Rate};

/// Everything one tier measured. Phases that stopped early keep what they
/// got; why they stopped is in `limits`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TierResult {
    pub tier: usize,
    pub host: Host,
    pub workload: Workload,
    pub holder: Option<HolderPhase>,
    pub stack: Option<StackPhase>,
    pub limits: Vec<Limit>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Host {
    pub cpus: usize,
    pub mem_total: u64,
    pub kernel: String,
    pub limits: Limits,
}

/// What the sessions ran.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Workload {
    /// Bytes per second each busy session prints.
    pub busy_rate: u64,
    pub probes: usize,
    pub flood_sessions: usize,
    pub flood_bytes: u64,
}

/// Where a phase stopped short: which phase, how many sessions it had, and
/// what failed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Limit {
    pub phase: String,
    pub sessions: usize,
    pub what: String,
}

/// A process read before the sessions, with the idle ones, and with the
/// busy ones too.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Readings<T> {
    pub base: Option<T>,
    pub idle: Option<T>,
    pub busy: Option<T>,
}

/// The session holder on its own, driven over its socket as vornd would.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HolderPhase {
    pub idle: usize,
    pub busy: usize,
    pub spawn: Option<Rate>,
    pub holder: Readings<Usage>,
    pub probe_idle: Option<Latency>,
    pub probe_busy: Option<Latency>,
    /// Output bytes the bench took from the holder while the busy ones ran.
    pub streamed_bytes: u64,
    pub handoff: Option<Handoff>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Handoff {
    pub sessions: usize,
    pub ms: f64,
    /// The newer holder once it has them all.
    pub adopter: Option<Usage>,
}

/// vornd with its session holder, driven as the app is: sessions started
/// on the app's channel, typed into and watched over the grid endpoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StackPhase {
    pub idle: usize,
    pub busy: usize,
    pub spawn: Option<Rate>,
    pub vornd: Readings<Usage>,
    pub holder: Readings<Usage>,
    pub mem_available: Readings<u64>,
    /// From Attach to the first snapshot, for idle sessions.
    pub attach: Option<Latency>,
    pub probe_idle: Option<Latency>,
    pub probe_busy: Option<Latency>,
    pub throughput: Option<Throughput>,
}

/// Sessions printing as fast as they can, all followed by one grid client.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Throughput {
    pub sessions: usize,
    /// Bytes the sessions printed.
    pub bytes: u64,
    /// From the moment they started until the client showed every last line.
    pub secs: f64,
    pub frames: u64,
    pub frame_bytes: u64,
}

/// Where a run happened, for the heading.
#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub date: String,
    pub commit: String,
    pub machine: String,
}

/// The tier's section for `RESULTS.md`: a heading, one table, and the
/// limits it hit.
pub fn markdown(r: &TierResult, meta: &Meta) -> String {
    let mut out = String::new();
    let h = &r.host;
    let _ = writeln!(
        out,
        "### {} sessions · {} · `{}` · {} ({} vCPU, {}, Linux {})\n",
        thousands(r.tier as u64),
        meta.date,
        meta.commit,
        meta.machine,
        h.cpus,
        bytes(h.mem_total as i64),
        h.kernel,
    );
    let hp = r.holder.as_ref();
    let sp = r.stack.as_ref();
    let mut rows: Vec<(String, String, String)> = Vec::new();
    let sessions = |idle: usize, busy: usize| {
        format!("{} + {}", thousands(idle as u64), thousands(busy as u64))
    };
    rows.push((
        "sessions (idle + busy)".to_owned(),
        hp.map_or_else(dash, |p| sessions(p.idle, p.busy)),
        sp.map_or_else(dash, |p| sessions(p.idle, p.busy)),
    ));
    rows.push((
        "spawn rate (each p50 / p99)".to_owned(),
        hp.and_then(|p| p.spawn).map_or_else(dash, rate),
        sp.and_then(|p| p.spawn).map_or_else(dash, rate),
    ));
    let cells = |p: Option<&Readings<Usage>>, idle: usize, busy: usize| -> [String; 4] {
        p.map_or_else(
            || [dash(), dash(), dash(), dash()],
            |u| usage_cells(u, idle, busy),
        )
    };
    let hh = cells(
        hp.map(|p| &p.holder),
        hp.map_or(0, |p| p.idle),
        hp.map_or(0, |p| p.busy),
    );
    let sh = cells(
        sp.map(|p| &p.holder),
        sp.map_or(0, |p| p.idle),
        sp.map_or(0, |p| p.busy),
    );
    let sv = cells(
        sp.map(|p| &p.vornd),
        sp.map_or(0, |p| p.idle),
        sp.map_or(0, |p| p.busy),
    );
    let labels = [
        "RSS idle / busy",
        "per idle / per busy terminal",
        "threads idle / busy",
        "fds idle / busy",
    ];
    for (i, label) in labels.iter().enumerate() {
        rows.push((format!("holder {label}"), hh[i].clone(), sh[i].clone()));
    }
    for (i, label) in labels.iter().enumerate() {
        rows.push((format!("vornd {label}"), dash(), sv[i].clone()));
    }
    rows.push((
        "machine memory per idle terminal, program included".to_owned(),
        dash(),
        sp.and_then(|p| per_session(p.mem_available.idle?, p.mem_available.base?, p.idle))
            .map_or_else(dash, bytes),
    ));
    rows.push((
        "probe echo p50 / p99, others idle".to_owned(),
        hp.and_then(|p| p.probe_idle).map_or_else(dash, latency),
        sp.and_then(|p| p.probe_idle).map_or_else(dash, latency),
    ));
    rows.push((
        "probe echo p50 / p99, 10 % streaming".to_owned(),
        hp.and_then(|p| p.probe_busy).map_or_else(dash, latency),
        sp.and_then(|p| p.probe_busy).map_or_else(dash, latency),
    ));
    rows.push((
        "attach to snapshot p50 / p99".to_owned(),
        dash(),
        sp.and_then(|p| p.attach).map_or_else(dash, latency),
    ));
    rows.push((
        "throughput to one grid client".to_owned(),
        dash(),
        sp.and_then(|p| p.throughput.as_ref())
            .map_or_else(dash, throughput),
    ));
    rows.push((
        "handoff of every live session".to_owned(),
        hp.and_then(|p| p.handoff.as_ref()).map_or_else(dash, |h| {
            format!(
                "{} sessions in {:.0} ms",
                thousands(h.sessions as u64),
                h.ms
            )
        }),
        dash(),
    ));
    out.push_str("| | holder alone | vornd + holder |\n|---|---|---|\n");
    for (label, a, b) in rows {
        let _ = writeln!(out, "| {label} | {a} | {b} |");
    }
    let w = &r.workload;
    let _ = writeln!(
        out,
        "\nIdle sessions run `cat` (holder alone) or `bash` (vornd + holder); busy ones print a build log at {}/s each. \
         Throughput: {} sessions printing {} each as fast as they can.",
        bytes(w.busy_rate as i64),
        w.flood_sessions,
        bytes(w.flood_bytes as i64),
    );
    if !r.limits.is_empty() {
        out.push_str("\nLimits hit:\n\n");
        for l in &r.limits {
            let _ = writeln!(
                out,
                "- {} at {} sessions: {}",
                l.phase,
                thousands(l.sessions as u64),
                l.what.replace('\n', " ")
            );
        }
    }
    out
}

fn usage_cells(u: &Readings<Usage>, idle: usize, busy: usize) -> [String; 4] {
    let pair = |f: &dyn Fn(&Usage) -> String| {
        format!(
            "{} / {}",
            u.idle.as_ref().map_or_else(dash, f),
            u.busy.as_ref().map_or_else(dash, f)
        )
    };
    let per_idle = u
        .base
        .zip(u.idle)
        .and_then(|(b, i)| per_session(b.rss, i.rss, idle));
    let per_busy = u
        .idle
        .zip(u.busy)
        .and_then(|(i, b)| per_session(i.rss, b.rss, busy));
    [
        pair(&|x| bytes(x.rss as i64)),
        format!(
            "{} / {}",
            per_idle.map_or_else(dash, bytes),
            per_busy.map_or_else(dash, bytes)
        ),
        pair(&|x| thousands(x.threads)),
        pair(&|x| thousands(x.fds)),
    ]
}

fn dash() -> String {
    "—".to_owned()
}

fn rate(r: Rate) -> String {
    format!(
        "{:.0}/s ({:.1} / {:.1} ms)",
        r.per_sec, r.each.p50_ms, r.each.p99_ms
    )
}

fn latency(l: Latency) -> String {
    format!("{:.2} / {:.2} ms", l.p50_ms, l.p99_ms)
}

fn throughput(t: &Throughput) -> String {
    let per_sec = |n: f64| if t.secs > 0.0 { n / t.secs } else { 0.0 };
    format!(
        "{:.1} MB/s over {} sessions, {:.0} frames/s ({}/s of frames)",
        per_sec(t.bytes as f64) / 1e6,
        t.sessions,
        per_sec(t.frames as f64),
        bytes(per_sec(t.frame_bytes as f64) as i64),
    )
}

/// Binary units, one decimal from KiB up.
pub fn bytes(n: i64) -> String {
    let units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n.unsigned_abs() as f64;
    let mut u = 0;
    while v >= 1024.0 && u < units.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    let sign = if n < 0 { "-" } else { "" };
    if u == 0 {
        format!("{sign}{v:.0} B")
    } else {
        format!("{sign}{v:.1} {}", units[u])
    }
}

pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_read_as_people_write_them() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(1536), "1.5 KiB");
        assert_eq!(bytes(-3 << 20), "-3.0 MiB");
        assert_eq!(bytes(5 << 30), "5.0 GiB");
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(10_000), "10,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    fn usage(rss_mib: u64, threads: u64, fds: u64) -> Usage {
        Usage {
            rss: rss_mib << 20,
            threads,
            fds,
        }
    }

    #[test]
    fn a_tier_reads_as_one_table_with_its_limits() {
        let r = TierResult {
            tier: 1000,
            host: Host {
                cpus: 16,
                mem_total: 64 << 30,
                kernel: "6.8.0".into(),
                limits: Limits::default(),
            },
            workload: Workload {
                busy_rate: 16 << 10,
                probes: 200,
                flood_sessions: 16,
                flood_bytes: 16 << 20,
            },
            holder: Some(HolderPhase {
                idle: 900,
                busy: 100,
                holder: Readings {
                    base: Some(usage(4, 1, 8)),
                    idle: Some(usage(94, 2701, 2708)),
                    busy: Some(usage(104, 3001, 3008)),
                },
                probe_idle: Some(Latency {
                    n: 200,
                    p50_ms: 0.21,
                    p99_ms: 0.9,
                    max_ms: 2.0,
                }),
                handoff: Some(Handoff {
                    sessions: 1001,
                    ms: 812.4,
                    adopter: None,
                }),
                ..HolderPhase::default()
            }),
            stack: None,
            limits: vec![Limit {
                phase: "vornd + holder".into(),
                sessions: 640,
                what: "spawn failed:\nout of ptys".into(),
            }],
        };
        let md = markdown(
            &r,
            &Meta {
                date: "2026-10-08".into(),
                commit: "abc1234".into(),
                machine: "n2-standard-16".into(),
            },
        );
        assert!(md.starts_with(
            "### 1,000 sessions · 2026-10-08 · `abc1234` · n2-standard-16 (16 vCPU, 64.0 GiB, Linux 6.8.0)"
        ));
        assert!(
            md.contains("| sessions (idle + busy) | 900 + 100 | — |"),
            "{md}"
        );
        // 90 MiB over 900 idle sessions, then 10 MiB over 100 busy ones.
        assert!(
            md.contains("| holder per idle / per busy terminal | 102.4 KiB / 102.4 KiB | — |"),
            "{md}"
        );
        assert!(
            md.contains("| holder threads idle / busy | 2,701 / 3,001 |"),
            "{md}"
        );
        assert!(
            md.contains("| probe echo p50 / p99, others idle | 0.21 / 0.90 ms | — |"),
            "{md}"
        );
        assert!(md.contains("1,001 sessions in 812 ms"), "{md}");
        assert!(
            md.contains("- vornd + holder at 640 sessions: spawn failed: out of ptys"),
            "{md}"
        );
    }

    #[test]
    fn results_round_trip_through_json() {
        let r = TierResult {
            tier: 100,
            stack: Some(StackPhase {
                idle: 90,
                throughput: Some(Throughput {
                    sessions: 4,
                    bytes: 1 << 20,
                    secs: 0.5,
                    frames: 30,
                    frame_bytes: 9000,
                }),
                ..StackPhase::default()
            }),
            ..TierResult::default()
        };
        let back: TierResult = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back.stack.unwrap().throughput.unwrap().frames, 30);
    }
}
