//! One binary, one engine per run, so engines never share a process while
//! one of them is being measured.
//!
//! ```text
//! vt-harness throughput --engine E --corpus C [--mb N] [--runs N]
//! vt-harness memory     --engine E --corpus C [--mb N] [--sessions N]
//! vt-harness resize     --engine E [--runs N]
//! vt-harness compare    --engine E
//! vt-harness features   --engine E
//! ```
//!
//! Every command prints JSON lines on stdout; `scripts/run-all.sh` collects
//! them into `results/`.

mod compare;
mod corpus;
mod features;
mod json;

use std::hint::black_box;
use std::process::ExitCode;
use std::time::Instant;

use corpus::{Corpus, Op};
use json::Obj;
use vt_alacritty::Alacritty;
use vt_api::Engine;
use vt_ghostty::{Ghostty, GhosttyRaw};
use vt_vt100::Vt100;
use vt_wezterm::Wezterm;

/// History every session keeps, as in the spec: 10k lines.
pub const SCROLLBACK: usize = 10_000;

struct Args {
    cmd: String,
    engine: String,
    corpus: String,
    mb: usize,
    runs: usize,
    sessions: usize,
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let cmd = it.next().ok_or("missing command")?;
    let mut a = Args {
        cmd,
        engine: String::new(),
        corpus: "build-log".into(),
        mb: 8,
        runs: 15,
        sessions: 32,
    };
    while let Some(flag) = it.next() {
        let v = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
        let num = || v.parse::<usize>().map_err(|e| format!("{flag}: {e}"));
        match flag.as_str() {
            "--engine" => a.engine = v.clone(),
            "--corpus" => a.corpus = v.clone(),
            "--mb" => a.mb = num()?,
            "--runs" => a.runs = num()?.max(1),
            "--sessions" => a.sessions = num()?.max(1),
            _ => return Err(format!("unknown flag {flag}")),
        }
    }
    Ok(a)
}

/// Calls `$f::<Engine>($args)` for the engine named `$name`.
macro_rules! dispatch {
    ($name:expr, $f:ident($($a:expr),*)) => {
        match $name {
            "ghostty" => Ok($f::<Ghostty>($($a),*)),
            "ghostty-raw" => Ok($f::<GhosttyRaw>($($a),*)),
            "alacritty" => Ok($f::<Alacritty>($($a),*)),
            "wezterm" => Ok($f::<Wezterm>($($a),*)),
            "vt100" => Ok($f::<Vt100>($($a),*)),
            other => Err(format!("unknown engine {other}")),
        }
    };
}

fn main() -> ExitCode {
    let run = || -> Result<(), String> {
        let a = parse_args()?;
        if !corpus::NAMES.contains(&a.corpus.as_str()) {
            return Err(format!("unknown corpus {}", a.corpus));
        }
        match a.cmd.as_str() {
            "throughput" => dispatch!(a.engine.as_str(), throughput(&a)),
            "memory" => dispatch!(a.engine.as_str(), memory(&a)),
            "resize" => dispatch!(a.engine.as_str(), resize(&a)),
            "compare" => dispatch!(a.engine.as_str(), compare_all()),
            "features" => dispatch!(a.engine.as_str(), features_all()),
            other => Err(format!("unknown command {other}")),
        }
    };
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("vt-harness: {e}");
            ExitCode::FAILURE
        }
    }
}

pub fn play<E: Engine>(e: &mut E, ops: &[Op]) {
    for op in ops {
        match op {
            Op::Data(b) => e.feed(b),
            &Op::Resize(c, r) => e.resize(c, r),
        }
    }
}

/// Minimum, quartiles and maximum of a sample, sorted in place.
fn stats(xs: &mut [f64]) -> [f64; 5] {
    xs.sort_by(f64::total_cmp);
    let at = |q: f64| {
        let i = q * (xs.len() - 1) as f64;
        let (lo, hi) = (i.floor() as usize, i.ceil() as usize);
        xs[lo] + (xs[hi] - xs[lo]) * (i - lo as f64)
    };
    [at(0.0), at(0.25), at(0.5), at(0.75), at(1.0)]
}

fn throughput<E: Engine>(a: &Args) {
    let c: Corpus = corpus::load(&a.corpus, a.mb << 20);
    let mib = c.data_bytes() as f64 / f64::from(1 << 20);
    let once = |c: &Corpus| {
        let mut e = E::new(c.cols, c.rows, SCROLLBACK);
        let t = Instant::now();
        play(&mut e, &c.ops);
        black_box(e.cursor());
        let s = t.elapsed().as_secs_f64();
        drop(e);
        s
    };
    once(&c);
    let mut rates: Vec<f64> = (0..a.runs).map(|_| mib / once(&c)).collect();
    let listed: Vec<String> = rates.iter().map(|r| format!("{r:.2}")).collect();
    let [min, q1, med, q3, max] = stats(&mut rates);
    println!(
        "{}",
        Obj::new()
            .str("engine", E::NAME)
            .str("corpus", &a.corpus)
            .num("mib", mib)
            .num("runs", a.runs as f64)
            .num("median_mib_s", med)
            .num("min_mib_s", min)
            .num("q1_mib_s", q1)
            .num("q3_mib_s", q3)
            .num("max_mib_s", max)
            .raw("rates", &format!("[{}]", listed.join(",")))
    );
}

/// Resident set size of this process in KiB, from `ps` so the harness needs
/// no platform crate.
fn rss_kib() -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .expect("ps prints a number")
}

fn memory<E: Engine>(a: &Args) {
    let c = corpus::load_once(&a.corpus, a.mb << 20);
    // Calibration and lazy statics belong to the baseline, not to a session.
    drop(E::new(c.cols, c.rows, SCROLLBACK));
    let before = rss_kib();
    let mut sessions: Vec<E> = (0..a.sessions)
        .map(|_| E::new(c.cols, c.rows, SCROLLBACK))
        .collect();
    let empty = rss_kib();
    for e in &mut sessions {
        play(e, &c.ops);
    }
    let after = rss_kib();
    let retained = sessions[0].scrollback_lines();
    let n = a.sessions as f64;
    println!(
        "{}",
        Obj::new()
            .str("engine", E::NAME)
            .str("corpus", &a.corpus)
            .num("mib_fed", c.data_bytes() as f64 / f64::from(1 << 20))
            .num("sessions", n)
            .num("rss_before_kib", before as f64)
            .num("rss_empty_kib", empty as f64)
            .num("rss_after_kib", after as f64)
            .num(
                "empty_kib_per_session",
                empty.saturating_sub(before) as f64 / n
            )
            .num("kib_per_session", after.saturating_sub(before) as f64 / n)
            .num("history_lines_retained", retained as f64)
    );
    black_box(sessions);
}

/// Time of one resize with a full history: the build log fills it at
/// 120x40, then the screen alternates between 100x30 and 120x40.
fn resize<E: Engine>(a: &Args) {
    let c = corpus::build_log(7, 4 << 20);
    let mut e = E::new(c.cols, c.rows, SCROLLBACK);
    play(&mut e, &c.ops);
    let history = e.scrollback_lines();
    let mut ms: Vec<f64> = (0..a.runs)
        .map(|i| {
            let (w, h) = if i % 2 == 0 {
                (100, 30)
            } else {
                (c.cols, c.rows)
            };
            let t = Instant::now();
            e.resize(w, h);
            black_box(e.cursor());
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    let [min, _, med, _, max] = stats(&mut ms);
    println!(
        "{}",
        Obj::new()
            .str("engine", E::NAME)
            .num("history_lines", history as f64)
            .num("runs", a.runs as f64)
            .num("median_ms", med)
            .num("min_ms", min)
            .num("max_ms", max)
    );
}

fn compare_all<E: Engine>() {
    for name in corpus::NAMES.into_iter().chain(["seeded-fixed"]) {
        for line in compare::run::<E>(name) {
            println!("{line}");
        }
    }
}

fn features_all<E: Engine>() {
    println!("{}", features::probe::<E>());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_interpolates_quartiles() {
        let mut xs = [5.0, 1.0, 3.0, 2.0, 4.0];
        assert_eq!(stats(&mut xs), [1.0, 2.0, 3.0, 4.0, 5.0]);
        let mut ys = [1.0, 2.0];
        assert_eq!(stats(&mut ys)[2], 1.5);
    }
}
