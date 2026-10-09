//! `vorn-scale-bench`: how the session holder and vornd hold up with many
//! terminals. `run` measures one tier and writes its JSON; `report` turns
//! tier JSON into the markdown `RESULTS.md` keeps; `compare` and
//! `report-compare` do the same for the comparison method, one build at a
//! time; `buildlog` is the program busy sessions run.
//!
//! Tiers above [`LAPTOP_TIER`] start thousands of processes and threads,
//! so they run only where `VORN_BENCH_HOST=1` says the machine is meant
//! for it.

mod buildlog;
mod compare;
mod procfs;
mod report;
mod stats;

#[cfg(unix)]
mod app_client;
#[cfg(unix)]
mod compare_phase;
#[cfg(unix)]
mod daemon;
#[cfg(unix)]
mod error;
#[cfg(unix)]
mod grid_client;
#[cfg(unix)]
mod holder_client;
#[cfg(unix)]
mod holder_phase;
#[cfg(unix)]
mod spawn;
#[cfg(unix)]
mod stack_phase;

#[cfg(unix)]
use std::path::PathBuf;
use std::process::ExitCode;

use crate::compare::Comparison;
use crate::report::{Meta, TierResult};

/// The largest tier a machine not marked as a bench host may run.
#[cfg(unix)]
const LAPTOP_TIER: usize = 100;

const USAGE: &str = "usage:
  vorn-scale-bench run --tier N --sessiond PATH --vornd PATH --work DIR --out FILE
                       [--phases holder,stack] [--busy-rate BYTES_PER_SEC] [--probes N]
                       [--flood-sessions N] [--flood-bytes N]
  vorn-scale-bench report --date D --commit C --machine M FILE...
  vorn-scale-bench compare --sessiond PATH --vornd PATH --work DIR --out FILE
                           [--sizes 1000,3000,10000] [--bash N] [--response N]
                           [--busy 0,10,100] [--probes N]
  vorn-scale-bench report-compare --date D --commit C --machine M [BEFORE] AFTER
  vorn-scale-bench buildlog [--rate N] [--total N] [--marker S] [--start-at MS] [--seed N]

Tiers above 100 need VORN_BENCH_HOST=1.";

/// One tier's settings.
#[cfg(unix)]
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// Sessions in all: one probe, a tenth busy, the rest idle.
    pub tier: usize,
    pub sessiond: PathBuf,
    pub vornd: PathBuf,
    pub work: PathBuf,
    pub out: PathBuf,
    /// This program, which busy sessions run as `buildlog`.
    pub me: PathBuf,
    pub holder: bool,
    pub stack: bool,
    pub busy_rate: u64,
    pub probes: usize,
    pub flood_sessions: usize,
    pub flood_bytes: u64,
}

#[cfg(unix)]
impl Plan {
    /// Idle and busy sessions; the one left over is the probe.
    pub fn split(&self) -> (usize, usize) {
        let busy = self.tier / 10;
        (self.tier.saturating_sub(busy + 1), busy)
    }

    fn parse(mut args: impl Iterator<Item = String>, me: PathBuf) -> Result<Plan, String> {
        let mut p = Plan {
            tier: 0,
            sessiond: PathBuf::new(),
            vornd: PathBuf::new(),
            work: PathBuf::new(),
            out: PathBuf::new(),
            me,
            holder: true,
            stack: true,
            busy_rate: 16 << 10,
            probes: 200,
            flood_sessions: 16,
            flood_bytes: 16 << 20,
        };
        while let Some(flag) = args.next() {
            let mut value = || args.next().ok_or_else(|| format!("{flag} needs a value"));
            let number = |v: String| v.parse::<u64>().map_err(|e| format!("{flag}: {e}"));
            match flag.as_str() {
                "--tier" => p.tier = number(value()?)? as usize,
                "--sessiond" => p.sessiond = value()?.into(),
                "--vornd" => p.vornd = value()?.into(),
                "--work" => p.work = value()?.into(),
                "--out" => p.out = value()?.into(),
                "--busy-rate" => p.busy_rate = number(value()?)?,
                "--probes" => p.probes = number(value()?)? as usize,
                "--flood-sessions" => p.flood_sessions = number(value()?)? as usize,
                "--flood-bytes" => p.flood_bytes = number(value()?)?,
                "--phases" => {
                    let v = value()?;
                    p.holder = v.split(',').any(|s| s == "holder");
                    p.stack = v.split(',').any(|s| s == "stack");
                }
                other => return Err(format!("unknown argument {other}")),
            }
        }
        let missing = [
            ("--tier", p.tier == 0),
            ("--sessiond", p.sessiond.as_os_str().is_empty()),
            ("--vornd", p.stack && p.vornd.as_os_str().is_empty()),
            ("--work", p.work.as_os_str().is_empty()),
            ("--out", p.out.as_os_str().is_empty()),
        ];
        if let Some((flag, _)) = missing.iter().find(|(_, m)| *m) {
            return Err(format!("{flag} is required"));
        }
        if p.probes == 0 {
            return Err("--probes must be at least 1".into());
        }
        Ok(p)
    }
}

/// Refuses tiers a laptop should not run unless the host says it is a
/// bench host.
#[cfg(unix)]
fn guard(tier: usize, bench_host: Option<&str>) -> Result<(), String> {
    if tier <= LAPTOP_TIER || bench_host == Some("1") {
        return Ok(());
    }
    Err(format!(
        "tier {tier} starts thousands of processes; it runs only on a bench host \
         (VORN_BENCH_HOST=1), tiers up to {LAPTOP_TIER} run anywhere"
    ))
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let res = match args.next().as_deref() {
        Some("buildlog") => buildlog::Spec::parse(args).map(|spec| {
            // A failed write means the terminal went away; that is how a log ends.
            let _ = buildlog::run(&spec, &mut std::io::stdout().lock());
        }),
        Some("report") => report_cmd(args),
        Some("run") => run_cmd(args),
        Some("compare") => compare_cmd(args),
        Some("report-compare") => report_compare_cmd(args),
        _ => Err(USAGE.to_owned()),
    };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("vorn-scale-bench: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The heading's flags, and the files after them.
fn meta_and_files(mut args: impl Iterator<Item = String>) -> Result<(Meta, Vec<String>), String> {
    let mut meta = Meta::default();
    let mut files = Vec::new();
    while let Some(a) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--date" => meta.date = value()?,
            "--commit" => meta.commit = value()?,
            "--machine" => meta.machine = value()?,
            _ => files.push(a),
        }
    }
    Ok((meta, files))
}

fn read_json<T: serde::de::DeserializeOwned>(f: &str) -> Result<T, String> {
    let text = std::fs::read_to_string(f).map_err(|e| format!("{f}: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("{f}: {e}"))
}

fn report_cmd(args: impl Iterator<Item = String>) -> Result<(), String> {
    let (meta, files) = meta_and_files(args)?;
    for f in files {
        let r: TierResult = read_json(&f)?;
        println!("{}", report::markdown(&r, &meta));
    }
    Ok(())
}

fn report_compare_cmd(args: impl Iterator<Item = String>) -> Result<(), String> {
    let (meta, files) = meta_and_files(args)?;
    // With one file there is no before: its column stays empty.
    let (before, after) = match files.as_slice() {
        [after] => (Comparison::default(), read_json(after)?),
        [before, after] => (read_json(before)?, read_json(after)?),
        _ => return Err("report-compare needs [BEFORE] AFTER".into()),
    };
    println!("{}", compare::markdown(&before, &after, &meta));
    Ok(())
}

#[cfg(not(unix))]
fn compare_cmd(_: impl Iterator<Item = String>) -> Result<(), String> {
    Err("compare needs a Unix host".into())
}

#[cfg(unix)]
fn compare_cmd(args: impl Iterator<Item = String>) -> Result<(), String> {
    let spec = parse_compare(args)?;
    guard(
        spec.largest(),
        std::env::var("VORN_BENCH_HOST").ok().as_deref(),
    )?;
    std::fs::create_dir_all(&spec.work).map_err(|e| format!("{}: {e}", spec.work.display()))?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .map_err(|e| format!("runtime: {e}"))?;
    let mut out = Comparison {
        host: host(),
        ..Comparison::default()
    };
    let save = |c: &Comparison| {
        if let Err(e) = write_json(&spec.out, c) {
            eprintln!("vorn-scale-bench: {e}");
        }
    };
    rt.block_on(compare_phase::run(&spec, &mut out, &save));
    rt.shutdown_timeout(std::time::Duration::from_secs(5));
    write_json(&spec.out, &out)
}

#[cfg(unix)]
fn parse_compare(mut args: impl Iterator<Item = String>) -> Result<compare_phase::Spec, String> {
    let mut s = compare_phase::Spec {
        sessiond: PathBuf::new(),
        vornd: PathBuf::new(),
        work: PathBuf::new(),
        out: PathBuf::new(),
        sizes: vec![1000, 3000, 10_000],
        bash: 1000,
        response: 1000,
        busy: vec![0, 10, 100],
        probes: 200,
    };
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{flag} needs a value"));
        let number = |v: &str| v.parse::<usize>().map_err(|e| format!("{flag}: {e}"));
        let list = |v: String| -> Result<Vec<usize>, String> {
            v.split(',').filter(|x| !x.is_empty()).map(number).collect()
        };
        match flag.as_str() {
            "--sessiond" => s.sessiond = value()?.into(),
            "--vornd" => s.vornd = value()?.into(),
            "--work" => s.work = value()?.into(),
            "--out" => s.out = value()?.into(),
            "--sizes" => s.sizes = list(value()?)?,
            "--bash" => s.bash = number(&value()?)?,
            "--response" => s.response = number(&value()?)?,
            "--busy" => s.busy = list(value()?)?,
            "--probes" => s.probes = number(&value()?)?,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    for (flag, missing) in [
        ("--sessiond", s.sessiond.as_os_str().is_empty()),
        ("--vornd", s.vornd.as_os_str().is_empty()),
        ("--work", s.work.as_os_str().is_empty()),
        ("--out", s.out.as_os_str().is_empty()),
    ] {
        if missing {
            return Err(format!("{flag} is required"));
        }
    }
    if s.sizes.contains(&0) || s.response < 2 || s.probes == 0 {
        return Err(
            "terminal counts and --probes must be at least 1, --response at least 2".into(),
        );
    }
    Ok(s)
}

#[cfg(unix)]
fn write_json<T: serde::Serialize>(path: &std::path::Path, v: &T) -> Result<(), String> {
    let text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    std::fs::write(path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(unix)]
fn host() -> report::Host {
    report::Host {
        cpus: std::thread::available_parallelism().map_or(0, |n| n.get()),
        mem_total: procfs::mem_total().unwrap_or(0),
        kernel: std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .map(|s| s.trim().to_owned())
            .unwrap_or_default(),
        limits: procfs::Limits::read(),
    }
}

#[cfg(not(unix))]
fn run_cmd(_: impl Iterator<Item = String>) -> Result<(), String> {
    Err("run needs a Unix host".into())
}

#[cfg(unix)]
fn run_cmd(args: impl Iterator<Item = String>) -> Result<(), String> {
    let me = std::env::current_exe().map_err(|e| format!("own path: {e}"))?;
    let plan = Plan::parse(args, me)?;
    guard(plan.tier, std::env::var("VORN_BENCH_HOST").ok().as_deref())?;
    std::fs::create_dir_all(&plan.work).map_err(|e| format!("{}: {e}", plan.work.display()))?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .map_err(|e| format!("runtime: {e}"))?;
    let result = rt.block_on(run_tier(&plan));
    rt.shutdown_timeout(std::time::Duration::from_secs(5));
    save(&plan, &result)
}

#[cfg(unix)]
async fn run_tier(plan: &Plan) -> TierResult {
    use crate::report::{Limit, Workload};

    let mut r = TierResult {
        tier: plan.tier,
        host: host(),
        workload: Workload {
            busy_rate: plan.busy_rate,
            probes: plan.probes,
            flood_sessions: plan.flood_sessions,
            flood_bytes: plan.flood_bytes,
        },
        ..TierResult::default()
    };
    if plan.holder {
        let mut phase = report::HolderPhase::default();
        if let Err(e) = holder_phase::run(plan, &mut phase).await {
            eprintln!("vorn-scale-bench: holder phase: {e}");
            r.limits.push(Limit {
                phase: "holder".into(),
                sessions: 1 + phase.idle + phase.busy,
                what: e.to_string(),
            });
        }
        r.holder = Some(phase);
        let _ = save(plan, &r);
    }
    if plan.stack {
        let mut phase = report::StackPhase::default();
        if let Err(e) = stack_phase::run(plan, &mut phase).await {
            eprintln!("vorn-scale-bench: stack phase: {e}");
            r.limits.push(Limit {
                phase: "stack".into(),
                sessions: 1 + phase.idle + phase.busy,
                what: e.to_string(),
            });
        }
        r.stack = Some(phase);
    }
    r
}

#[cfg(unix)]
fn save(plan: &Plan, r: &TierResult) -> Result<(), String> {
    write_json(&plan.out, r)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn args(s: &str) -> impl Iterator<Item = String> + '_ {
        s.split_whitespace().map(String::from)
    }

    #[cfg(unix)]
    #[test]
    fn the_comparison_reads_its_lists() {
        let s = parse_compare(args(
            "--sessiond s --vornd v --work w --out o --sizes 20,30 --bash 0 --busy 0,5",
        ))
        .unwrap();
        assert_eq!((s.sizes, s.bash, s.busy), (vec![20, 30], 0, vec![0, 5]));
        assert_eq!(s.probes, 200);
        let s = parse_compare(args("--sessiond s --vornd v --work w --out o")).unwrap();
        assert_eq!(s.largest(), 10_000);
        assert!(parse_compare(args("--sessiond s --vornd v --work w")).is_err());
        assert!(parse_compare(args("--sessiond s --vornd v --work w --out o --sizes 0")).is_err());
    }

    #[test]
    fn large_tiers_need_a_bench_host() {
        assert!(guard(100, None).is_ok());
        assert!(guard(101, None).is_err());
        assert!(guard(10_000, Some("0")).is_err());
        assert!(guard(10_000, Some("1")).is_ok());
    }

    #[test]
    fn a_tier_is_one_probe_a_tenth_busy_and_the_rest_idle() {
        let p = Plan::parse(
            args("--tier 1000 --sessiond s --vornd v --work w --out o"),
            "me".into(),
        )
        .unwrap();
        assert_eq!(p.split(), (899, 100));
        let p = Plan { tier: 5, ..p };
        assert_eq!(p.split(), (4, 0));
    }

    #[test]
    fn the_plan_reads_its_flags_and_wants_the_paths() {
        let p = Plan::parse(
            args("--tier 10 --sessiond s --work w --out o --phases holder --probes 5"),
            "me".into(),
        )
        .unwrap();
        assert!(p.holder && !p.stack);
        assert_eq!((p.probes, p.busy_rate), (5, 16 << 10));
        let err = Plan::parse(args("--tier 10 --sessiond s --work w --out o"), "me".into());
        assert_eq!(err.unwrap_err(), "--vornd is required");
        assert!(Plan::parse(args("--tier 10 --bogus"), "me".into()).is_err());
    }
}
