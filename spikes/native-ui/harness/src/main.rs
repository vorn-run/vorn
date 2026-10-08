//! The spike's harness: one way to measure every prototype.
//!
//! ```text
//! spike-harness run --client swift|gpui|tauri --panes N --mode latency|idle|load|start
//!               [--producer yes|buildlog|rec:vim|...] [--duration S] [--probes K]
//!               [--hz 120] [--name TAG]
//! spike-harness serve --panes N [--producer P]    an isolated vornd to try a client by hand
//! spike-harness produce yes|buildlog|rec:NAME     what a busy session runs
//! spike-harness report                             summary table of results/raw/*.json
//! ```
//!
//! `run` starts its own vorn-sessiond and vornd with `--home /tmp/vorn-spike-<pid>`
//! (TMPDIR=/tmp), spawns one session per pane through vornd's app channel,
//! launches the client with the grid endpoint and the session ids in its
//! environment, samples the client's and vornd's CPU and memory with
//! `proc_pid_rusage`, and merges what the client wrote with its own numbers
//! into `results/raw/<client>-<mode>-<N>[-producer].json`.

mod daemon;
mod produce;
mod report;
mod sample;

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

pub fn spike_root() -> PathBuf {
    if let Ok(r) = std::env::var("SPIKE_ROOT") {
        return PathBuf::from(r);
    }
    // target/release/spike-harness
    let exe = std::env::current_exe().expect("current exe");
    exe.ancestors().nth(3).expect("spike root").to_path_buf()
}

pub fn epoch_ns() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos())
}

struct Args(Vec<String>);

impl Args {
    fn get(&self, k: &str) -> Option<String> {
        let i = self.0.iter().position(|a| a == k)?;
        self.0.get(i + 1).cloned()
    }
    fn num(&self, k: &str, d: u64) -> u64 {
        self.get(k).and_then(|v| v.parse().ok()).unwrap_or(d)
    }
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = argv.first().cloned() else {
        eprintln!("usage: spike-harness run|serve|produce|report ...");
        std::process::exit(2);
    };
    let args = Args(argv[1..].to_vec());
    match cmd.as_str() {
        "produce" => produce::main(argv.get(1).map_or("yes", String::as_str)),
        "report" => report::main(),
        "serve" => serve(&args),
        "run" => {
            if let Err(e) = run(&args) {
                eprintln!("spike-harness: {e}");
                std::process::exit(1);
            }
        }
        other => {
            eprintln!("unknown command {other}");
            std::process::exit(2);
        }
    }
}

/// What each pane runs.
fn programs(mode: &str, panes: usize, producer: Option<&str>) -> Vec<Vec<String>> {
    let me = std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let busy = |p: &str| vec![me.clone(), "produce".into(), p.into()];
    let shell = || vec!["/bin/sh".to_string(), "-i".to_string()];
    (0..panes)
        .map(|i| match (mode, producer) {
            ("latency", _) if i == 0 => vec!["/bin/cat".to_string()],
            ("look", _) => look_program(),
            ("load", Some(p)) | ("latency", Some(p)) => busy(p),
            _ => shell(),
        })
        .collect()
}

/// Commits left out of the look test's log: their subjects are about
/// dependencies and tooling rather than the app.
const LOOK_SKIP: [&str; 7] =
    ["fb3ec6c9", "3fa1c988", "69acc4ba", "57fa5c96", "f1db8488", "9d5de282", "65c9e3d1"];

/// The look test's terminal: this repository's history as a coloured graph
/// (after the client has sized the pane), then a shell prompt.
fn look_program() -> Vec<String> {
    let repo = spike_root().join("../..");
    let script = format!(
        "sleep 1.5; git --no-pager -C '{}' log --graph --color=always -n 60 \
         --format='%C(yellow)%h%C(reset) %C(blue)%<(13)%cr%C(reset)  %s' \
         | grep -vE '{}'; \
         PS1='vorn $ ' exec /bin/sh -i",
        repo.display(),
        LOOK_SKIP.join("|")
    );
    vec!["/bin/sh".into(), "-c".into(), script]
}

fn client_command(client: &str) -> Result<PathBuf, String> {
    let root = spike_root();
    let p = match client {
        "swift" => root.join("swift/build/VornSpikeSwift.app/Contents/MacOS/VornSpikeSwift"),
        "gpui" => root.join("target/release/vorn-spike-gpui"),
        "tauri" => root.join("target/release/vorn-spike-tauri"),
        "slint" => root.join("target/release/vorn-spike-slint"),
        other => PathBuf::from(other),
    };
    if p.exists() {
        Ok(p)
    } else {
        Err(format!("no client at {}", p.display()))
    }
}

fn serve(args: &Args) {
    let panes = args.num("--panes", 4) as usize;
    let producer = args.get("--producer");
    let d = daemon::Daemon::start().unwrap_or_else(|e| panic!("vornd: {e}"));
    let mode = if producer.is_some() { "load" } else { "idle" };
    let ids = d
        .spawn_programs(&programs(mode, panes, producer.as_deref()))
        .unwrap_or_else(|e| panic!("spawn: {e}"));
    println!("export VORN_SPIKE_GRID={}", d.grid);
    println!("export VORN_SPIKE_SESSIONS={}", ids.join(","));
    println!("(enter to stop)");
    let mut s = String::new();
    let _ = std::io::stdin().read_line(&mut s);
    d.stop();
}

fn run(args: &Args) -> Result<(), String> {
    let client = args.get("--client").ok_or("--client is required")?;
    let panes = args.num("--panes", 1) as usize;
    let mode = args.get("--mode").unwrap_or_else(|| "idle".into());
    let producer = args.get("--producer");
    let duration = args.num("--duration", 10);
    let settle_ms = args.num("--settle-ms", if args.get("--mode").as_deref() == Some("look") { 4000 } else { 2000 });
    let probes = args.num("--probes", 150);
    let hz = args.num("--hz", 120) as f64;
    let cmd = client_command(&client)?;

    let name = args.get("--name").unwrap_or_else(|| {
        let mut n = format!("{client}-{mode}-{panes}");
        if let Some(p) = &producer {
            n.push('-');
            n.push_str(&p.replace(':', "_"));
        }
        n
    });
    let raw = spike_root().join("results/raw");
    std::fs::create_dir_all(&raw).map_err(|e| e.to_string())?;
    let out = raw.join(format!("{name}.json"));
    let client_out = std::env::temp_dir().join(format!("vorn-spike-client-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&client_out);

    let d = daemon::Daemon::start()?;
    let progs = programs(&mode, panes, producer.as_deref());
    let ids = match d.spawn_programs(&progs) {
        Ok(ids) => ids,
        Err(e) => {
            d.stop();
            return Err(e);
        }
    };
    // Let the shells print their prompts and the producers get going.
    std::thread::sleep(Duration::from_millis(500));

    let look = mode == "look";
    let polish = args.0.iter().any(|a| a == "--polish");
    let shot = spike_root().join("results/look").join(format!(
        "{}{}.png",
        client.rsplit('/').next().unwrap_or(&client),
        if polish { "-polish" } else { "" }
    ));
    if look {
        std::fs::create_dir_all(shot.parent().unwrap()).map_err(|e| e.to_string())?;
    }
    let client_mode = match mode.as_str() {
        "latency" => "latency",
        "start" | "look" => "start",
        _ => "frames",
    };
    let launched_ns = epoch_ns();
    let launched = Instant::now();
    let mut child = Command::new(&cmd)
        .env("VORN_SPIKE_GRID", &d.grid)
        .env("VORN_SPIKE_SESSIONS", ids.join(","))
        .env("VORN_SPIKE_MODE", client_mode)
        .env("VORN_SPIKE_DURATION_MS", (duration * 1000).to_string())
        .env("VORN_SPIKE_SETTLE_MS", settle_ms.to_string())
        .env("VORN_SPIKE_PROBES", probes.to_string())
        .env("VORN_SPIKE_OUT", &client_out)
        .env("VORN_SPIKE_LOOK", if look { "1" } else { "0" })
        .env("VORN_SPIKE_POLISH", if polish { "1" } else { "0" })
        .env("VORN_SPIKE_FONT_SIZE", if look { "13" } else { "12" })
        .envs(look.then(|| ("VORN_SPIKE_SHOT", shot.clone())))
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("{}: {e}", cmd.display()))?;
    let pid = child.id() as i32;

    // FIRST_FRAME from the client's stdout.
    let (tx, rx) = mpsc::channel::<u128>();
    let stdout = child.stdout.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(ns) = line.strip_prefix("FIRST_FRAME ") {
                if let Ok(ns) = ns.trim().parse() {
                    let _ = tx.send(ns);
                }
            } else {
                eprintln!("[client] {line}");
            }
        }
    });

    let deadline = launched + Duration::from_secs(duration + 60 + probes / 3);
    let mut first_frame_ns = None;
    let mut sampler = sample::Sampler::new(pid, d.vornd_pid, d.sessiond_pid());
    let mut window_open = None;
    let mut status = None;
    while Instant::now() < deadline {
        if first_frame_ns.is_none() {
            if let Ok(ns) = rx.try_recv() {
                first_frame_ns = Some(ns);
                sampler.mark("first_frame");
            }
        }
        if let Some(ff) = first_frame_ns {
            if window_open.is_none()
                && epoch_ns() > ff + u128::from(settle_ms) * 1_000_000
            {
                window_open = Some(Instant::now());
                sampler.mark("window");
            }
        }
        sampler.sample();
        if first_frame_ns.is_none() && launched.elapsed() > Duration::from_secs(30) {
            eprintln!("spike-harness: no first frame in 30 s");
            break;
        }
        if let Ok(Some(s)) = child.try_wait() {
            status = Some(s);
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    d.stop();

    let client_json: Value = std::fs::read_to_string(&client_out)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    let _ = std::fs::remove_file(&client_out);
    let cold_start_ms = first_frame_ns.map(|ns| (ns.saturating_sub(launched_ns)) as f64 / 1e6);
    let mut result = json!({
        "name": name,
        "client": client,
        "mode": mode,
        "panes": panes,
        "producer": producer,
        "duration_s": duration,
        "hz": hz,
        "cold_start_ms": cold_start_ms,
        "exit": status.map(|s| s.to_string()),
        "binary": cmd.to_string_lossy(),
        "samples": sampler.to_json(),
        "client_report": client_json,
    });
    result["summary"] = report::summarize(&result);
    std::fs::write(&out, serde_json::to_string_pretty(&result).unwrap())
        .map_err(|e| e.to_string())?;
    println!("{}", serde_json::to_string(&result["summary"]).unwrap());
    println!("wrote {}", out.display());
    Ok(())
}

pub fn repo_root() -> PathBuf {
    spike_root()
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .expect("repo root")
}
