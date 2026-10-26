//! The extensions a load run installs, the calls that keep them busy and
//! what they cost: Node, Python and Bash programs, a Rust program and Rust
//! Wasm components, services and one-shots, written into the run's
//! temporary HOME from the templates in `bench/ext-load/templates` and
//! linked into its vornd. Their ids start `bench.`, never `vorn.`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use vorn_app::client::{Endpoint, Rpc};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Lang {
    Node,
    Python,
    Bash,
    /// A Rust program on stdio (`tier-load`).
    Rust,
    /// The same Rust source as a Wasm component, run inside vornd.
    Wasm,
}

impl Lang {
    pub const ALL: [Lang; 5] = [Lang::Node, Lang::Python, Lang::Bash, Lang::Rust, Lang::Wasm];

    pub fn name(self) -> &'static str {
        match self {
            Lang::Node => "node",
            Lang::Python => "python",
            Lang::Bash => "bash",
            Lang::Rust => "rust",
            Lang::Wasm => "wasm",
        }
    }
}

/// The twenty: 15 services and 5 one-shots. The first four are a service in
/// Node, Python, Bash and Wasm, so a short run has the common kinds without
/// building the Rust program; the first nine have every kind.
const MIX: [(Lang, bool); 20] = [
    (Lang::Node, true),
    (Lang::Python, true),
    (Lang::Bash, true),
    (Lang::Wasm, true),
    (Lang::Rust, true),
    (Lang::Node, false),
    (Lang::Python, false),
    (Lang::Bash, false),
    (Lang::Rust, false),
    (Lang::Node, true),
    (Lang::Python, true),
    (Lang::Wasm, true),
    (Lang::Bash, true),
    (Lang::Node, true),
    (Lang::Wasm, true),
    (Lang::Python, true),
    (Lang::Rust, true),
    (Lang::Node, true),
    (Lang::Wasm, true),
    (Lang::Bash, false),
];

pub const MAX: usize = MIX.len();

/// One installed bench extension.
pub struct Ext {
    pub id: String,
    pub lang: Lang,
    /// A service stays up between calls; a one-shot is a process per call.
    pub service: bool,
    pub dir: PathBuf,
}

/// The action every bench extension answers; a service's asks Vorn for its sessions.
pub const ACTION: &str = "work";

/// Writes the first `n` of the mix into `dir`.
pub fn generate(dir: &Path, n: usize) -> Result<Vec<Ext>, String> {
    let mut seen: HashMap<(Lang, bool), usize> = HashMap::new();
    MIX.iter()
        .take(n)
        .map(|&(lang, service)| {
            let k = seen.entry((lang, service)).or_default();
            *k += 1;
            let kind = if service { "svc" } else { "once" };
            let id = format!("bench.{}-{kind}-{k}", lang.name());
            let ext = Ext {
                dir: dir.join(&id),
                id,
                lang,
                service,
            };
            write(&ext)?;
            Ok(ext)
        })
        .collect()
}

macro_rules! template {
    ($path:literal) => {
        include_str!(concat!("../../../../bench/ext-load/templates/", $path))
    };
}

const SERVICE_MANIFEST: &str = template!("manifest/service.toml");
const ONCE_MANIFEST: &str = template!("manifest/once.toml");
const COMPONENT_MANIFEST: &str = template!("manifest/component.toml");
const EVENTS: &str = r#"["session.created", "session.status", "session.exited"]"#;

fn write(ext: &Ext) -> Result<(), String> {
    let put = |name: &str, text: &[u8]| {
        std::fs::write(ext.dir.join(name), text).map_err(|e| format!("{name}: {e}"))
    };
    let copy = |from: &Path, name: &str| {
        std::fs::copy(from, ext.dir.join(name))
            .map(drop)
            .map_err(|e| format!("{}: {e}", from.display()))
    };
    std::fs::create_dir_all(&ext.dir).map_err(|e| e.to_string())?;
    let id = ext.id.as_str();
    // The id rides along as an argument, so `ps` tells whose process it is.
    let (runner, file): (Option<&str>, &str) = match (ext.lang, ext.service) {
        (Lang::Node, true) => (Some("node"), "service.mjs"),
        (Lang::Node, false) => (Some("node"), "once.mjs"),
        (Lang::Python, true) => (Some("python3"), "service.py"),
        (Lang::Python, false) => (Some("python3"), "once.py"),
        (Lang::Bash, true) => (Some("bash"), "service.sh"),
        (Lang::Bash, false) => (Some("bash"), "once.sh"),
        (Lang::Rust, _) => (None, "./load"),
        (Lang::Wasm, _) => {
            copy(&component()?, "load.wasm")?;
            let manifest = COMPONENT_MANIFEST
                .replace("{id}", id)
                .replace("{events}", EVENTS);
            return put("vorn-extension.toml", manifest.as_bytes());
        }
    };
    match ext.lang {
        Lang::Rust => copy(&program()?, "load")?,
        lang => put(file, script(lang, ext.service).as_bytes())?,
    }
    let mut argv: Vec<&str> = runner.into_iter().chain([file]).collect();
    if ext.lang == Lang::Rust && !ext.service {
        argv.push("--once");
    }
    argv.push(id);
    let command = serde_json::to_string(&argv).map_err(|e| e.to_string())?;
    let manifest = if ext.service {
        // The Bash service answers calls only; it is woken for no events.
        let events = if ext.lang == Lang::Bash { "[]" } else { EVENTS };
        SERVICE_MANIFEST.replace("{events}", events)
    } else {
        ONCE_MANIFEST.to_owned()
    };
    let manifest = manifest.replace("{id}", id).replace("{command}", &command);
    put("vorn-extension.toml", manifest.as_bytes())
}

/// The script template of a Node, Python or Bash extension.
fn script(lang: Lang, service: bool) -> &'static str {
    match (lang, service) {
        (Lang::Node, true) => template!("node/service.mjs"),
        (Lang::Node, false) => template!("node/once.mjs"),
        (Lang::Python, true) => template!("python/service.py"),
        (Lang::Python, false) => template!("python/once.py"),
        (Lang::Bash, true) => template!("bash/service.sh"),
        _ => template!("bash/once.sh"),
    }
}

/// Builds what the first `n` of the mix need from the workspace, before the
/// bench moves HOME (cargo and rustup find their caches under the real one).
pub fn prepare(n: usize) -> Result<(), String> {
    let langs: HashSet<Lang> = MIX.iter().take(n).map(|&(lang, _)| lang).collect();
    if langs.contains(&Lang::Wasm) {
        component()?;
    }
    if langs.contains(&Lang::Rust) {
        program()?;
    }
    Ok(())
}

/// The Rust template built as a component, once a run.
fn component() -> Result<PathBuf, String> {
    static BUILT: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    BUILT
        .get_or_init(|| vorn_guest_build::component("vorn-bench-load"))
        .clone()
}

/// The Rust template built as a program: `tier-load`, beside this example's
/// folder in the same profile's target directory.
fn program() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let name = format!("tier-load{}", std::env::consts::EXE_SUFFIX);
    exe.parent()
        .and_then(Path::parent)
        .map(|profile| profile.join(&name))
        .filter(|p| p.is_file())
        .ok_or_else(|| {
            format!("build {name} first: cargo build --profile <this one> -p vorn-bench-ext-tiers --bin tier-load")
        })
}

/// Links `exts` into the vornd `rpc` talks to.
pub fn link(rpc: &Rpc, exts: &[Ext]) -> Result<(), String> {
    for ext in exts {
        rpc.call("extension:link", json!({ "path": ext.dir }))
            .map_err(|e| format!("extension:link {}: {e:?}", ext.id))?;
    }
    Ok(())
}

/// One call to `ext`, in milliseconds.
pub fn call(rpc: &Rpc, ext: &Ext) -> Result<f64, String> {
    let t = Instant::now();
    rpc.call(
        "extension:call",
        json!({ "id": ext.id, "action": ACTION, "args": {} }),
    )
    .map_err(|e| format!("{}: {e:?}", ext.id))?;
    Ok(t.elapsed().as_secs_f64() * 1000.0)
}

/// The services running now: the bench's, and Vorn's own (the shipped set
/// it installs on first start and the built-ins compiled into vornd).
pub fn running(rpc: &Rpc) -> Result<(usize, usize), String> {
    let running = |method: &str| -> Result<Vec<bool>, String> {
        let list = rpc
            .call(method, Value::Null)
            .map_err(|e| format!("{method}: {e:?}"))?;
        Ok(list
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| p["running"] == true)
            .map(|p| p["id"].as_str().is_some_and(|id| id.starts_with("bench.")))
            .collect())
    };
    let installed = running("extension:installed")?;
    let bench = installed.iter().filter(|&&b| b).count();
    Ok((
        bench,
        installed.len() - bench + running("extension:builtins")?.len(),
    ))
}

/// What a driver's calls took, and the most services it saw running.
#[derive(Default)]
pub struct Calls {
    pub ms: Vec<f64>,
    pub errors: Vec<String>,
    pub peak_bench: usize,
    pub peak_vorn: usize,
    /// When each call was in flight, and to what kind of extension.
    pub spans: Vec<Span>,
}

/// One call in flight, so a slow key press can be laid against it.
pub struct Span {
    pub start: Instant,
    pub end: Instant,
    pub lang: Lang,
    pub service: bool,
}

impl Calls {
    /// The kind of extension a call was in flight to over `from..to`, if any.
    pub fn during(&self, from: Instant, to: Instant) -> Option<String> {
        self.spans
            .iter()
            .find(|s| s.start < to && from < s.end)
            .map(|s| kind(s.lang, s.service))
    }
}

/// A kind's name in the report, as `node service` or `rust one-shot`.
pub fn kind(lang: Lang, service: bool) -> String {
    let what = if service { "service" } else { "one-shot" };
    format!("{} {what}", lang.name())
}

/// Calls every extension in turn, each once per period, on a connection of
/// its own, until it is finished; once a period it counts what is running.
pub struct Driver {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<Calls>,
}

impl Driver {
    pub fn start(home: &Path, exts: Arc<Vec<Ext>>, period: Duration) -> Result<Driver, String> {
        let ep = Endpoint::find(home).map_err(|e| format!("{e:?}"))?;
        let (rpc, events) = Rpc::connect(&ep, Arc::new(|| {})).map_err(|e| format!("{e:?}"))?;
        // Its events are the app's to draw; this connection only calls.
        drop(events);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let gap = period / (exts.len() + 1) as u32;
        let thread = std::thread::spawn(move || {
            let mut calls = Calls::default();
            let mut next = Instant::now();
            for tick in 0.. {
                if stopped.load(Ordering::Relaxed) {
                    break;
                }
                match exts.get(tick % (exts.len() + 1)) {
                    Some(ext) => {
                        let start = Instant::now();
                        match call(&rpc, ext) {
                            Ok(ms) => calls.ms.push(ms),
                            Err(e) => calls.errors.push(e),
                        }
                        calls.spans.push(Span {
                            start,
                            end: Instant::now(),
                            lang: ext.lang,
                            service: ext.service,
                        });
                    }
                    None => {
                        if let Ok((bench, vorn)) = running(&rpc) {
                            calls.peak_bench = calls.peak_bench.max(bench);
                            calls.peak_vorn = calls.peak_vorn.max(vorn);
                        }
                    }
                }
                next += gap;
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                } else {
                    next = now;
                }
            }
            calls
        });
        Ok(Driver { stop, thread })
    }

    pub fn finish(self) -> Calls {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.join().unwrap_or_default()
    }
}

/// The resident memory of the processes vornd started for extensions, in
/// MB, by language: everything under vornd outside its session holders.
/// Wasm components run inside vornd and count there.
pub fn programs_rss(vornd: u32, holders: &[u32]) -> BTreeMap<&'static str, (usize, f64)> {
    let Ok(out) = vorn_spawn::command("ps")
        .args(["-A", "-o", "pid=,ppid=,rss=,command="])
        .output()
    else {
        return BTreeMap::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let procs: Vec<(u32, u32, f64, &str)> = text
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let pid = f.next()?.parse().ok()?;
            let ppid = f.next()?.parse().ok()?;
            let rss: f64 = f.next()?.parse().ok()?;
            let command = l.split_once("bench.").map_or("", |(_, rest)| rest);
            Some((pid, ppid, rss / 1024.0, command))
        })
        .collect();
    let skip: HashSet<u32> = holders.iter().copied().collect();
    let mut under = HashSet::from([vornd]);
    // Parents come before children in pid order only mostly; settle by rounds.
    loop {
        let before = under.len();
        for &(pid, ppid, _, _) in &procs {
            if under.contains(&ppid) && !skip.contains(&pid) {
                under.insert(pid);
            }
        }
        if under.len() == before {
            break;
        }
    }
    let mut by: BTreeMap<&'static str, (usize, f64)> = BTreeMap::new();
    for &(pid, _, mb, bench) in &procs {
        if pid == vornd || !under.contains(&pid) {
            continue;
        }
        let lang = Lang::ALL
            .into_iter()
            .find(|l| bench.starts_with(l.name()))
            .map_or("other", Lang::name);
        let e = by.entry(lang).or_default();
        e.0 += 1;
        e.1 += mb;
    }
    by
}

/// What vornd's log says the services did.
#[derive(Default, Debug, PartialEq)]
pub struct Lifecycle {
    /// Services started, Vorn's own included.
    pub starts: usize,
    /// Starts past the first for a service: after the cap, idle or a crash stopped it.
    pub restarts: usize,
    /// Services the cap stopped, the built-ins' included.
    pub evictions: usize,
    pub crashes: usize,
    pub idle_stops: usize,
    /// The services started more than once, in id order.
    pub restarted: Vec<String>,
}

pub fn read_log(log: &str) -> Lifecycle {
    let mut starts: HashMap<&str, usize> = HashMap::new();
    let mut seen = Lifecycle::default();
    for line in log.lines() {
        let Some((_, said)) = line.split_once("[extensions] ") else {
            continue;
        };
        if said.contains(" stops for the cap of ") {
            seen.evictions += 1;
        } else if said.ends_with(" ended; it starts again") {
            seen.crashes += 1;
        } else if said.ends_with(" stops after sitting idle") {
            seen.idle_stops += 1;
        } else if let Some(id) = said
            .strip_suffix(" started")
            .or_else(|| said.strip_suffix(" started as a component"))
        {
            *starts.entry(id).or_default() += 1;
        }
    }
    seen.starts = starts.values().sum();
    seen.restarts = starts.values().map(|n| n - 1).sum();
    seen.restarted = starts
        .into_iter()
        .filter(|&(_, n)| n > 1)
        .map(|(id, _)| id.to_owned())
        .collect();
    seen.restarted.sort_unstable();
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_press_is_put_down_to_the_call_it_overlaps() {
        let t = Instant::now();
        let at = |ms: u64| t + Duration::from_millis(ms);
        let span = |from, to, lang, service| Span {
            start: at(from),
            end: at(to),
            lang,
            service,
        };
        let calls = Calls {
            spans: vec![
                span(0, 10, Lang::Wasm, true),
                span(20, 150, Lang::Node, false),
            ],
            ..Calls::default()
        };
        assert_eq!(calls.during(at(5), at(25)).as_deref(), Some("wasm service"));
        assert_eq!(
            calls.during(at(140), at(170)).as_deref(),
            Some("node one-shot")
        );
        assert_eq!(calls.during(at(10), at(20)), None);
        assert_eq!(calls.during(at(150), at(200)), None);
    }

    #[test]
    fn the_log_counts_restarts_past_each_services_first_start() {
        let log = "\
INFO [extensions] bench.node-svc-1 started
INFO [extensions] bench.rust-svc-1 started as a component
INFO [extensions] vorn.tasks started as a component
INFO [extensions] bench.node-svc-1 stops for the cap of 8 running
INFO [extensions] bench.node-svc-1 started
WARN [extensions] bench.rust-svc-1 ended; it starts again
INFO [extensions] bench.rust-svc-1 stops after sitting idle
";
        assert_eq!(
            read_log(log),
            Lifecycle {
                starts: 4,
                restarts: 1,
                evictions: 1,
                crashes: 1,
                idle_stops: 1,
                restarted: vec!["bench.node-svc-1".to_owned()],
            }
        );
    }
}
