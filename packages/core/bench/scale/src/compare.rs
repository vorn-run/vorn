//! The comparison method: memory with every terminal's program counted,
//! live and after a minute idle, and how long a shell command takes to
//! answer beside busy terminals. One run is one build; two runs, before
//! and after a change, make one table.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::report::{bytes, thousands, Host, Limit, Meta};
use crate::stats::Latency;

/// What one build measured.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Comparison {
    pub host: Host,
    pub memory: Vec<Memory>,
    pub response: Vec<Response>,
    pub limits: Vec<Limit>,
}

/// Anonymous RSS summed over vornd, the holder and everything either
/// started, with `terminals` sessions each running `shell`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Memory {
    pub shell: String,
    pub terminals: usize,
    /// Once every terminal has printed its lines and gone quiet.
    pub live: Option<u64>,
    /// After the terminals sat untouched for a minute and more.
    pub idle: Option<u64>,
    /// Seconds from quiet to the idle reading.
    pub idle_after_secs: f64,
    /// vornd and the holder went quiet before the live reading.
    pub quiet: bool,
    /// Processes summed in the live reading.
    pub processes: usize,
    pub live_parts: Option<Parts>,
    pub idle_parts: Option<Parts>,
    /// Sessions vornd reported asleep at the idle reading.
    pub asleep: Option<usize>,
}

/// One memory reading split by who holds it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Parts {
    pub vornd: u64,
    pub holder: u64,
    /// Every other process: the holder's helpers and the programs.
    pub programs: u64,
}

impl Parts {
    #[cfg(unix)]
    pub fn total(&self) -> u64 {
        self.vornd + self.holder + self.programs
    }
}

/// From typing a command into one terminal to its output on screen, while
/// `busy` others print the date ten times a second.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Response {
    pub terminals: usize,
    pub busy: usize,
    pub latency: Option<Latency>,
    /// The whole machine's CPU use while the commands ran, in percent.
    pub cpu: Option<f64>,
}

/// The section `RESULTS.md` gets: memory and response time, before and
/// after, side by side.
pub fn markdown(before: &Comparison, after: &Comparison, meta: &Meta) -> String {
    let mut out = String::new();
    let h = &after.host;
    let _ = writeln!(
        out,
        "### Comparison method · {} · `{}` · {} ({} vCPU, {}, Linux {})\n",
        meta.date,
        meta.commit,
        meta.machine,
        h.cpus,
        bytes(h.mem_total as i64),
        h.kernel,
    );
    out.push_str(
        "Anonymous RSS of vornd, the holder and every terminal's program, after each terminal printed 1,000 numbered lines.\n\n\
         | terminals | shell | live before | live after | idle 60 s before | idle 60 s after |\n\
         |---|---|---|---|---|---|\n",
    );
    for a in &after.memory {
        let b = before
            .memory
            .iter()
            .find(|b| b.shell == a.shell && b.terminals == a.terminals);
        let cell = |m: Option<&Memory>, f: fn(&Memory) -> Option<u64>| {
            m.and_then(|m| f(m).map(|v| per_terminal(v, m.terminals)))
                .unwrap_or_else(dash)
        };
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} |",
            thousands(a.terminals as u64),
            a.shell,
            cell(b, |m| m.live),
            cell(Some(a), |m| m.live),
            cell(b, |m| m.idle),
            cell(Some(a), |m| m.idle),
        );
    }
    out.push_str(
        "\nFrom sending a shell command to one terminal to its output, while others run `date` every 0.1 s.\n\n\
         | terminals | busy | p50 / p99 before | p50 / p99 after | machine CPU before | machine CPU after |\n\
         |---|---|---|---|---|---|\n",
    );
    for a in &after.response {
        let b = before.response.iter().find(|b| b.busy == a.busy);
        let lat = |r: Option<&Response>| {
            r.and_then(|r| r.latency)
                .map_or_else(dash, |l| format!("{:.2} / {:.2} ms", l.p50_ms, l.p99_ms))
        };
        let cpu = |r: Option<&Response>| {
            r.and_then(|r| r.cpu)
                .map_or_else(dash, |c| format!("{c:.1}%"))
        };
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} |",
            thousands(a.terminals as u64),
            a.busy,
            lat(b),
            lat(Some(a)),
            cpu(b),
            cpu(Some(a)),
        );
    }
    for (which, c) in [("before", before), ("after", after)] {
        for l in &c.limits {
            let _ = writeln!(
                out,
                "\n- {which}, {} at {} terminals: {}",
                l.phase,
                thousands(l.sessions as u64),
                l.what.replace('\n', " ")
            );
        }
    }
    out
}

fn per_terminal(total: u64, terminals: usize) -> String {
    let each = total / terminals.max(1) as u64;
    format!("{} ({} each)", bytes(total as i64), bytes(each as i64))
}

fn dash() -> String {
    "—".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory(shell: &str, terminals: usize, live: u64, idle: u64) -> Memory {
        Memory {
            shell: shell.into(),
            terminals,
            live: Some(live << 20),
            idle: Some(idle << 20),
            ..Memory::default()
        }
    }

    fn response(busy: usize, p50: f64, cpu: f64) -> Response {
        Response {
            terminals: 1000,
            busy,
            latency: Some(Latency {
                n: 200,
                p50_ms: p50,
                p99_ms: p50 * 2.0,
                max_ms: p50 * 3.0,
            }),
            cpu: Some(cpu),
        }
    }

    #[test]
    fn rows_pair_before_and_after_by_what_they_ran() {
        let before = Comparison {
            memory: vec![memory("dash", 1000, 2000, 2000)],
            response: vec![response(10, 1.0, 5.0)],
            ..Comparison::default()
        };
        let after = Comparison {
            memory: vec![
                memory("bash", 1000, 3000, 1000),
                memory("dash", 1000, 1000, 500),
            ],
            response: vec![response(0, 0.5, 1.0), response(10, 0.75, 4.0)],
            limits: vec![Limit {
                phase: "memory".into(),
                sessions: 10_000,
                what: "out\nof memory".into(),
            }],
            ..Comparison::default()
        };
        let md = markdown(&before, &after, &Meta::default());
        assert!(
            md.contains(
                "| 1,000 | bash | — | 2.9 GiB (3.0 MiB each) | — | 1000.0 MiB (1.0 MiB each) |"
            ),
            "{md}"
        );
        assert!(
            md.contains("| 1,000 | dash | 2.0 GiB (2.0 MiB each) | 1000.0 MiB (1.0 MiB each) | 2.0 GiB (2.0 MiB each) | 500.0 MiB (512.0 KiB each) |"),
            "{md}"
        );
        assert!(
            md.contains("| 1,000 | 0 | — | 0.50 / 1.00 ms | — | 1.0% |"),
            "{md}"
        );
        assert!(
            md.contains("| 1,000 | 10 | 1.00 / 2.00 ms | 0.75 / 1.50 ms | 5.0% | 4.0% |"),
            "{md}"
        );
        assert!(md.contains("- after, memory at 10,000 terminals: out of memory"));
    }

    #[test]
    fn a_comparison_round_trips_through_json() {
        let c = Comparison {
            memory: vec![memory("dash", 3000, 10, 5)],
            response: vec![response(100, 2.0, 30.0)],
            ..Comparison::default()
        };
        let back: Comparison = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back.memory[0].idle, Some(5 << 20));
        assert_eq!(back.response[0].busy, 100);
    }
}
