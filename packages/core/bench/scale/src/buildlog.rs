//! The busy workload: a program that prints a build log, the output a
//! terminal running a compiler shows, at a set rate or as fast as the
//! terminal takes it. Sessions run it as `vorn-scale-bench buildlog`, so a
//! bench host needs nothing installed beyond the bench itself.

use std::io::{self, Write};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How often a rate-limited log writes.
const TICK: Duration = Duration::from_millis(50);
/// What an unlimited log writes per call.
const CHUNK: usize = 64 << 10;

/// What one log prints.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Spec {
    /// Bytes per second; 0 for as fast as the terminal takes them.
    pub rate: u64,
    /// Stop after this many bytes, print the marker and wait to be killed.
    pub total: Option<u64>,
    pub marker: Option<String>,
    /// Wall-clock milliseconds since the epoch to start at, so many logs
    /// begin together and the bench knows when.
    pub start_at_ms: Option<u64>,
    pub seed: u64,
}

impl Spec {
    pub fn parse(mut args: impl Iterator<Item = String>) -> Result<Spec, String> {
        let mut spec = Spec::default();
        while let Some(flag) = args.next() {
            let mut value = || args.next().ok_or_else(|| format!("{flag} needs a value"));
            let number = |v: String| v.parse::<u64>().map_err(|e| format!("{flag}: {e}"));
            match flag.as_str() {
                "--rate" => spec.rate = number(value()?)?,
                "--total" => spec.total = Some(number(value()?)?),
                "--marker" => spec.marker = Some(value()?),
                "--start-at" => spec.start_at_ms = Some(number(value()?)?),
                "--seed" => spec.seed = number(value()?)?,
                other => return Err(format!("buildlog: unknown argument {other}")),
            }
        }
        Ok(spec)
    }
}

const CRATES: [&str; 8] = [
    "serde",
    "tokio",
    "hyper",
    "regex",
    "syn",
    "vorn-engine",
    "libghostty-vt",
    "postcard",
];
const FILES: [&str; 6] = [
    "src/lib.rs",
    "src/session.rs",
    "src/grid/frame.cpp",
    "src/parser/osc.c",
    "include/vt.h",
    "tests/live.rs",
];

/// Lines of a build log, the same ones for the same seed.
#[derive(Debug)]
pub struct Lines {
    state: u64,
    n: u64,
}

impl Lines {
    pub fn new(seed: u64) -> Lines {
        Lines {
            state: seed | 1,
            n: 0,
        }
    }

    fn next_u64(&mut self) -> u64 {
        // xorshift64: no dependency, and plenty for picking log lines.
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        self.state
    }

    /// Appends one line, colours and all, to `out`.
    pub fn next_into(&mut self, out: &mut Vec<u8>) {
        self.n += 1;
        let r = self.next_u64();
        let krate = CRATES[(r % CRATES.len() as u64) as usize];
        let file = FILES[((r >> 8) % FILES.len() as u64) as usize];
        let line = (r >> 16) % 900 + 1;
        let pct = (self.n % 100) as u8;
        let line = match (r >> 32) % 8 {
            0 => format!(
                "\x1b[1;33mwarning\x1b[0m: unused variable `x{}`\n  \x1b[1;34m-->\x1b[0m {file}:{line}:9\n",
                r % 97
            ),
            1..=3 => format!(
                "\x1b[1;32m   Compiling\x1b[0m {krate} v0.{}.{}\n",
                r % 9,
                (r >> 4) % 30
            ),
            _ => format!("[{pct:3}%] Building CXX object {file}.o (step {})\n", self.n),
        };
        out.extend_from_slice(line.as_bytes());
    }
}

/// Prints the log to `out` until it is told to stop by a failed write: the
/// terminal went away. Returns once the total is printed only when there
/// is no marker to wait behind.
pub fn run(spec: &Spec, out: &mut impl Write) -> io::Result<()> {
    if let Some(at) = spec.start_at_ms {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        std::thread::sleep(Duration::from_millis(at.saturating_sub(now)));
    }
    let mut lines = Lines::new(spec.seed);
    let mut buf = Vec::with_capacity(CHUNK + 256);
    let mut sent = 0u64;
    let start = Instant::now();
    let mut ticks = 0u32;
    loop {
        let budget = match spec.rate {
            0 => CHUNK as u64,
            rate => {
                ticks += 1;
                let due = start + TICK * ticks;
                std::thread::sleep(due.saturating_duration_since(Instant::now()));
                // What the rate allows by now, so a late tick catches up.
                (rate * (TICK * ticks).as_millis() as u64 / 1000).saturating_sub(sent)
            }
        };
        let budget = spec.total.map_or(budget, |t| budget.min(t - sent));
        buf.clear();
        while (buf.len() as u64) < budget {
            lines.next_into(&mut buf);
        }
        out.write_all(&buf)?;
        out.flush()?;
        sent += buf.len() as u64;
        if spec.total.is_some_and(|t| sent >= t) {
            break;
        }
    }
    if let Some(marker) = &spec.marker {
        writeln!(out, "{marker}")?;
        out.flush()?;
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_its_flags() {
        let args = [
            "--rate",
            "16384",
            "--total",
            "100",
            "--marker",
            "DONE",
            "--start-at",
            "5",
        ];
        let spec = Spec::parse(args.iter().map(|s| s.to_string())).unwrap();
        assert_eq!(
            spec,
            Spec {
                rate: 16384,
                total: Some(100),
                marker: Some("DONE".into()),
                start_at_ms: Some(5),
                seed: 0,
            }
        );
        assert!(Spec::parse(["--rate".to_string()].into_iter()).is_err());
        assert!(Spec::parse(["--rate", "x"].iter().map(|s| s.to_string())).is_err());
        assert!(Spec::parse(["--what".to_string()].into_iter()).is_err());
    }

    #[test]
    fn the_same_seed_prints_the_same_log() {
        let print = |seed| {
            let mut l = Lines::new(seed);
            let mut out = Vec::new();
            for _ in 0..50 {
                l.next_into(&mut out);
            }
            out
        };
        assert_eq!(print(3), print(3));
        assert_ne!(print(3), print(4));
        let text = String::from_utf8(print(3)).unwrap();
        assert!(text.lines().any(|l| l.contains("Compiling")));
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn an_unlimited_log_prints_at_least_its_total_then_returns() {
        let spec = Spec {
            total: Some(200_000),
            ..Spec::default()
        };
        let mut out = Vec::new();
        run(&spec, &mut out).unwrap();
        // Whole lines only, so it can run a line past the total.
        assert!(
            out.len() >= 200_000 && out.len() < 200_000 + 256,
            "{}",
            out.len()
        );
    }

    #[test]
    fn a_rated_log_keeps_to_its_rate() {
        let spec = Spec {
            rate: 40_000,
            total: Some(8_000),
            ..Spec::default()
        };
        let t = Instant::now();
        let mut out = Vec::new();
        run(&spec, &mut out).unwrap();
        // 8 kB at 40 kB/s is 200 ms, written in 50 ms ticks.
        assert!(
            t.elapsed() >= Duration::from_millis(150),
            "{:?}",
            t.elapsed()
        );
    }
}
