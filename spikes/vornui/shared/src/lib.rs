//! What both UI prototypes share: the grid client ([`grid`]), what a pane
//! looks like to a UI ([`view`]), a test vornd ([`daemon`]), the load each
//! pane runs ([`produce`]), the bench driver ([`bench`]), process numbers
//! ([`metrics`]) and the main screen's values ([`look`]). A prototype brings
//! only its drawing and its input path.

pub mod bench;
pub mod daemon;
pub mod grid;
pub mod look;
pub mod metrics;
pub mod produce;
pub mod view;

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

pub use grid::{layout, Grid};
pub use view::{PaneView, RunC};

/// `<prototype> MODE [--key value]...`.
pub struct Args {
    pub mode: String,
    opts: HashMap<String, String>,
}

impl Args {
    pub fn parse() -> Args {
        let mut it = std::env::args().skip(1);
        let mode = it.next().unwrap_or_default();
        let mut opts = HashMap::new();
        let rest: Vec<String> = it.collect();
        let mut i = 0;
        while i < rest.len() {
            if let Some(k) = rest[i].strip_prefix("--") {
                let v = rest.get(i + 1).cloned().unwrap_or_default();
                opts.insert(k.to_owned(), v);
                i += 2;
            } else {
                // A bare word is the second positional: the producer's kind.
                opts.insert("kind".into(), rest[i].clone());
                i += 1;
            }
        }
        Args { mode, opts }
    }

    pub fn get(&self, k: &str) -> Option<&str> {
        self.opts.get(k).map(String::as_str)
    }

    pub fn num(&self, k: &str, default: u64) -> u64 {
        self.get(k).and_then(|v| v.parse().ok()).unwrap_or(default)
    }

    pub fn out(&self, default: &str) -> String {
        self.get("out").unwrap_or(default).to_owned()
    }
}

/// What the panes run.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Load {
    /// Pane 0 echoes what is typed; the others stream at 1 MB/s.
    Busy,
    /// Every pane shows a finished build log and idles.
    Still,
}

/// A test vornd with one session per pane.
pub struct Rig {
    pub vornd: daemon::TestVornd,
    pub sessions: Vec<String>,
}

impl Rig {
    pub fn start(panes: usize, load: Load, cols: u16, rows: u16) -> Result<Rig, String> {
        let vornd = daemon::TestVornd::start()?;
        let me = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .into_owned();
        let argvs: Vec<Vec<String>> = (0..panes)
            .map(|i| {
                let kind = match (load, i) {
                    (Load::Busy, 0) => "echo",
                    (Load::Busy, _) => "yes",
                    (Load::Still, _) => "buildlog",
                };
                vec![me.clone(), "produce".into(), kind.into()]
            })
            .collect();
        let sessions = vornd.spawn(&argvs, cols, rows)?;
        Ok(Rig { vornd, sessions })
    }

    pub fn connect(&self, cols: u16, rows: u16) -> Result<Arc<Grid>, String> {
        Grid::connect(&self.vornd.grid, self.sessions.clone(), cols, rows)
            .map_err(|e| format!("grid {}: {e}", self.vornd.grid))
    }
}

/// A pane's size in cells when `n` panes share the window, `cell` being the
/// terminal cell in logical pixels.
pub fn pane_cells(n: usize, cell: (f32, f32)) -> (u16, u16) {
    let (w, h) = pane_px(n);
    ((w / cell.0).floor() as u16, (h / cell.1).floor() as u16)
}

/// A pane's size in logical pixels when `n` panes share the window under
/// the top bar.
pub fn pane_px(n: usize) -> (f32, f32) {
    let (cols, rows) = layout(n);
    let (w, h) = (look::WINDOW.0, look::WINDOW.1 - look::TOP_BAR_H);
    (
        (w - look::GAP * (cols as f32 - 1.0)) / cols as f32,
        (h - look::GAP * (rows as f32 - 1.0)) / rows as f32,
    )
}

/// Bench length from `--secs` (measured) after a 2 s settle.
pub fn bench_config(args: &Args) -> bench::Config {
    bench::Config {
        settle: Duration::from_secs(2),
        measure: Duration::from_secs(args.num("secs", 20)),
    }
}

pub fn write_json(path: &str, v: &Value) {
    if let Some(dir) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_string_pretty(v) {
        Ok(s) => {
            if let Err(e) = std::fs::write(path, s) {
                eprintln!("write {path}: {e}");
            }
        }
        Err(e) => eprintln!("json: {e}"),
    }
}

/// Environment a cold-start child reads its grid from.
pub const ENV_GRID: &str = "VORN_SPIKE_GRID";
pub const ENV_SESSIONS: &str = "VORN_SPIKE_SESSIONS";

/// The cold-start parent: one test vornd with `--panes` busy sessions, then
/// `--runs` fresh children (`<prototype> coldchild`), each timed from spawn
/// to the line it prints once its first complete frame is submitted.
pub fn cold_start(args: &Args, cell: (f32, f32)) -> Result<Value, String> {
    let panes = args.num("panes", 8) as usize;
    let (cols, rows) = pane_cells(panes, cell);
    let rig = Rig::start(panes, Load::Busy, cols, rows)?;
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut runs = Vec::new();
    for _ in 0..args.num("runs", 5) {
        let t = Instant::now();
        let mut child = Command::new(&me)
            .arg("coldchild")
            .env(ENV_GRID, &rig.vornd.grid)
            .env(ENV_SESSIONS, rig.sessions.join(","))
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let out = child.stdout.take().ok_or("no stdout")?;
        let line = BufReader::new(out)
            .lines()
            .map_while(Result::ok)
            .find(|l| l.starts_with("FIRST_FRAME"));
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let _ = child.wait();
        if line.is_some() {
            runs.push(ms);
        }
    }
    let mut sorted = runs.clone();
    Ok(json!({ "panes": panes, "runs_ms": runs, "ms": bench::summary(&mut sorted) }))
}

/// The cold-start child's grid, from the parent's environment.
pub fn cold_child_grid(cell: (f32, f32)) -> Result<Arc<Grid>, String> {
    let endpoint = std::env::var(ENV_GRID).map_err(|_| "no grid endpoint")?;
    let sessions: Vec<String> = std::env::var(ENV_SESSIONS)
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    let (cols, rows) = pane_cells(sessions.len(), cell);
    Grid::connect(&endpoint, sessions, cols, rows).map_err(|e| e.to_string())
}

/// Tells the cold-start parent the first frame is submitted.
pub fn print_first_frame() {
    use std::io::Write;
    let mut o = std::io::stdout().lock();
    let _ = writeln!(o, "FIRST_FRAME");
    let _ = o.flush();
}

/// Waits until `text` shows on `pane`'s screen, for the IME check.
pub fn wait_for_text(grid: &Grid, pane: usize, text: &str, timeout: Duration) -> bool {
    let t = Instant::now();
    while t.elapsed() < timeout {
        if grid.screen_text(pane).contains(text) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// Waits until every pane has its first screen.
pub fn wait_snapshots(grid: &Grid, timeout: Duration) -> bool {
    let t = Instant::now();
    while t.elapsed() < timeout {
        if grid.all_snapshotted() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}
