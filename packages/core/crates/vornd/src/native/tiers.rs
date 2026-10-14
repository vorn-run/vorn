//! Extensions in two tiers ([`vorn_ext`]), as a spike beside today's packs.
//!
//! `ext:link` points vornd at a folder with a `vorn-extension.toml`. One
//! worker thread owns every linked extension: it reads its manifest and KDL
//! templates, runs it in its tier (a script or a `cargo run` service as the
//! person, or a component in the sandbox), asks it for each section's rows
//! when they are due, and tells clients each section as `ext:section`: the
//! checked template as JSON, the rows, or the errors naming their place.
//!
//! A linked folder is watched: a saved file reloads the extension and asks
//! again at once, so a quick extension's edit shows without a build. The
//! worker owns its state; calls reach it over a channel, and `ext:list`
//! reads the last sections it told, so a slow extension never holds a call.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tracing::{info, warn};
use vorn_ext::bridge::{Gate, World};
use vorn_ext::live::{Live, Vault};
use vorn_ext::manifest::{parse_duration, Manifest, Section, Tier};
use vorn_ext::quick::{self, CallCtx, Program, Service};
use vorn_ext::template::TemplateError;
use vorn_ext::wasm::{self, Instance, Runtime};
use vorn_ext::{view_file, ExtError};

use super::{Answer, Native};

/// The calls answered here.
pub const METHODS: &[&str] = &[
    "ext:link",
    "ext:unlink",
    "ext:list",
    "ext:refresh",
    "ext:bench",
];

/// What clients are told for each section, whenever it changes.
pub const SECTION: &str = "ext:section";

/// How often linked folders are looked at for a saved file.
const WATCH: Duration = Duration::from_millis(50);
/// How long a quick call may take.
const QUICK_DEADLINE: Duration = Duration::from_secs(60);
/// Rows are asked for again this often when the manifest does not say.
const DEFAULT_EVERY: Duration = Duration::from_secs(60);

/// The tiers host: the worker's channel and the sections it last told.
pub struct Tiers {
    tx: Mutex<Sender<Command>>,
    sections: Arc<Mutex<BTreeMap<String, Value>>>,
}

impl std::fmt::Debug for Tiers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tiers").finish_non_exhaustive()
    }
}

enum Command {
    Link {
        dir: PathBuf,
        project: String,
        dev: bool,
        reply: Sender<Result<Value, String>>,
    },
    Unlink {
        dir: PathBuf,
        reply: Sender<Result<Value, String>>,
    },
    Refresh,
    Bench {
        dir: PathBuf,
        calls: u32,
        instances: u32,
        reply: Sender<Result<Value, String>>,
    },
}

/// vornd's vault, as an extension's credential store: one connection per extension.
struct NativeVault(Weak<Native>);

impl Vault for NativeVault {
    fn get(&self, ext: &str, field: &str) -> Option<String> {
        let native = self.0.upgrade()?;
        match native.secrets.lookup(&format!("ext:{ext}")) {
            super::secrets::Known::Fields(f) => f.get(field).map(|s| s.expose().to_owned()),
            _ => None,
        }
    }

    fn set(&self, ext: &str, field: &str, value: &str) {
        if let Some(native) = self.0.upgrade() {
            native.secrets.merge(&format!("ext:{ext}"), field, value);
        }
    }
}

impl Tiers {
    /// Starts the worker; links kept in `<data_dir>/ext-links.json` come back.
    /// Returns at once: the environment, the vault and the links are read on
    /// the worker's own thread, never on the runtime that serves sessions.
    pub fn start(native: &Arc<Native>, data_dir: &Path) -> Arc<Tiers> {
        let (tx, rx) = mpsc::channel();
        let sections = Arc::new(Mutex::new(BTreeMap::new()));
        let (weak, store, shared) = (
            Arc::downgrade(native),
            data_dir.join("ext-links.json"),
            Arc::clone(&sections),
        );
        let spawned = std::thread::Builder::new()
            .name("ext-tiers".into())
            .spawn(move || {
                // The login shell's PATH, for the programs quick extensions run; may wait on the shell.
                let path = weak
                    .upgrade()
                    .and_then(|n| n.env.get().into_iter().find(|(k, _)| k == "PATH"))
                    .map(|(_, v)| v);
                let world = Arc::new(Live::new(
                    Some(Box::new(NativeVault(weak.clone()))),
                    path.clone(),
                ));
                let mut worker = Worker {
                    native: weak,
                    links: Vec::new(),
                    store,
                    sections: shared,
                    world,
                    path,
                    runtime: OnceLock::new(),
                };
                let kept: Vec<Value> = std::fs::read_to_string(&worker.store)
                    .ok()
                    .and_then(|t| serde_json::from_str(&t).ok())
                    .unwrap_or_default();
                for k in kept {
                    if let (Some(dir), Some(project)) = (k["dir"].as_str(), k["project"].as_str()) {
                        worker.link(PathBuf::from(dir), project.to_owned(), k["dev"] == true);
                    }
                }
                worker.run(&rx);
            });
        if let Err(err) = spawned {
            warn!(%err, "the tiers host could not start");
        }
        Arc::new(Tiers {
            tx: Mutex::new(tx),
            sections,
        })
    }

    /// Answers one call; blocks until the worker does, so it runs off the runtime.
    pub fn answer(&self, method: &str, params: &Value) -> Answer {
        let dir = || params["dir"].as_str().map(PathBuf::from);
        let ask = |make: &dyn Fn(Sender<Result<Value, String>>) -> Command| {
            let (reply, answer) = mpsc::channel();
            let sent = self
                .tx
                .lock()
                .map(|tx| tx.send(make(reply)).is_ok())
                .unwrap_or(false);
            if !sent {
                return Answer::Error("the tiers host has stopped".into());
            }
            match answer.recv() {
                Ok(Ok(v)) => Answer::Result(v),
                Ok(Err(e)) => Answer::Error(e),
                Err(_) => Answer::Error("the tiers host has stopped".into()),
            }
        };
        match method {
            "ext:list" => {
                let all = self
                    .sections
                    .lock()
                    .map(|s| s.values().cloned().collect())
                    .unwrap_or_default();
                Answer::Result(Value::Array(all))
            }
            "ext:refresh" => {
                if let Ok(tx) = self.tx.lock() {
                    let _ = tx.send(Command::Refresh);
                }
                Answer::Void
            }
            "ext:link" => match dir() {
                Some(dir) => {
                    let project = params["project"].as_str().unwrap_or_default().to_owned();
                    let dev = params["dev"] == true;
                    ask(&|reply| Command::Link {
                        dir: dir.clone(),
                        project: project.clone(),
                        dev,
                        reply,
                    })
                }
                None => Answer::Error("ext:link needs a dir".into()),
            },
            "ext:unlink" => match dir() {
                Some(dir) => ask(&|reply| Command::Unlink {
                    dir: dir.clone(),
                    reply,
                }),
                None => Answer::Error("ext:unlink needs a dir".into()),
            },
            "ext:bench" => match dir() {
                Some(dir) => {
                    let calls = params["calls"]
                        .as_u64()
                        .and_then(|n| u32::try_from(n).ok())
                        .unwrap_or(10_000);
                    let instances = params["instances"]
                        .as_u64()
                        .and_then(|n| u32::try_from(n).ok())
                        .unwrap_or(100);
                    ask(&|reply| Command::Bench {
                        dir: dir.clone(),
                        calls,
                        instances,
                        reply,
                    })
                }
                None => Answer::Error("ext:bench needs a dir".into()),
            },
            _ => super::not_answered(method),
        }
    }
}

/// How an extension runs in its tier.
enum Runner {
    OneShot(Program),
    Service(Program, Option<Service>),
    Component(Box<Instance>),
}

struct Link {
    dir: PathBuf,
    project: String,
    dev: bool,
    /// The newest modification times seen in the folder: code, views.
    seen: (SystemTime, SystemTime),
    loaded: Result<Loaded, String>,
    /// When a saved file was noticed, for the edit-to-visible measurement.
    changed_at: Option<f64>,
}

struct Loaded {
    manifest: Arc<Manifest>,
    tier: Tier,
    runner: Runner,
    sections: Vec<SectionState>,
}

struct SectionState {
    section: Section,
    /// The last rows, drawn again when only the template changed.
    rows: Option<Result<Value, String>>,
    every: Duration,
    template: Result<Value, Vec<TemplateError>>,
    due: Instant,
}

struct Worker {
    native: Weak<Native>,
    links: Vec<Link>,
    store: PathBuf,
    sections: Arc<Mutex<BTreeMap<String, Value>>>,
    world: Arc<Live>,
    path: Option<String>,
    runtime: OnceLock<Result<Arc<Runtime>, String>>,
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1e3)
        .unwrap_or(0.0)
}

impl Worker {
    fn run(mut self, rx: &Receiver<Command>) {
        loop {
            match rx.recv_timeout(WATCH) {
                Ok(cmd) => self.command(cmd),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            self.tick();
        }
    }

    fn command(&mut self, cmd: Command) {
        match cmd {
            Command::Link {
                dir,
                project,
                dev,
                reply,
            } => {
                let dir = dir.canonicalize().unwrap_or(dir);
                self.links.retain(|l| l.dir != dir);
                let at = self.link(dir, project, dev);
                self.save();
                let link = &self.links[at];
                let _ = reply.send(match &link.loaded {
                    Ok(l) => Ok(json!({ "id": l.manifest.id, "tier": l.tier, "sections": l.sections.len() })),
                    Err(e) => Err(e.clone()),
                });
                self.ask(at, true);
            }
            Command::Unlink { dir, reply } => {
                let dir = dir.canonicalize().unwrap_or(dir);
                let before = self.links.len();
                self.links.retain(|l| l.dir != dir);
                let key = dir.display().to_string();
                if let Ok(mut s) = self.sections.lock() {
                    s.retain(|k, _| !k.starts_with(&key));
                }
                self.tell(json!({ "dir": key, "removed": true }), None);
                self.save();
                let _ = reply.send(Ok(json!({ "removed": before - self.links.len() })));
            }
            Command::Refresh => {
                for i in 0..self.links.len() {
                    self.ask(i, true);
                }
            }
            Command::Bench {
                dir,
                calls,
                instances,
                reply,
            } => {
                let _ = reply.send(self.bench(&dir, calls, instances));
            }
        }
    }

    /// Adds a link and loads it; its index.
    fn link(&mut self, dir: PathBuf, project: String, dev: bool) -> usize {
        let seen = newest(&dir);
        let loaded = self.load(&dir, dev);
        if let Err(e) = &loaded {
            warn!(dir = %dir.display(), error = %e, "an extension did not load");
        }
        self.links.push(Link {
            dir,
            project,
            dev,
            seen,
            loaded,
            changed_at: None,
        });
        self.links.len() - 1
    }

    fn save(&self) {
        let kept: Vec<Value> = self
            .links
            .iter()
            .map(|l| json!({ "dir": l.dir, "project": l.project, "dev": l.dev }))
            .collect();
        if let Err(err) = std::fs::write(&self.store, Value::Array(kept).to_string()) {
            warn!(%err, "could not keep the linked extensions");
        }
    }

    fn runtime(&self) -> Result<Arc<Runtime>, String> {
        self.runtime
            .get_or_init(|| Runtime::new().map_err(|e| e.message))
            .clone()
    }

    fn load(&self, dir: &Path, dev: bool) -> Result<Loaded, String> {
        let manifest = Arc::new(Manifest::load(dir).map_err(|e| e.message)?);
        let tier = manifest.tier(dir, dev);
        let runner = match tier {
            Tier::Safe => {
                let rt = self.runtime()?;
                let file = dir.join(manifest.runtime.component.as_deref().unwrap_or_default());
                let c = rt.load(&file).map_err(|e| e.message)?;
                let world: Arc<dyn World> = Arc::clone(&self.world) as Arc<dyn World>;
                Runner::Component(Box::new(
                    Instance::new(rt, c, Arc::clone(&manifest), world).map_err(|e| e.message)?,
                ))
            }
            Tier::Quick => {
                let program = Program {
                    argv: manifest.runtime.command.clone().unwrap_or_default(),
                    dir: dir.to_owned(),
                    env: self
                        .path
                        .iter()
                        .map(|p| ("PATH".to_owned(), p.clone()))
                        .collect(),
                };
                if manifest.runtime.service {
                    Runner::Service(program, None)
                } else {
                    Runner::OneShot(program)
                }
            }
        };
        let sections = manifest
            .contributes
            .sidebar
            .iter()
            .map(|s| SectionState {
                section: s.clone(),
                every: s
                    .every
                    .as_deref()
                    .and_then(parse_duration)
                    .unwrap_or(DEFAULT_EVERY),
                template: std::fs::read_to_string(dir.join(&s.view))
                    .map_err(|e| {
                        vec![TemplateError {
                            at: None,
                            path: s.view.clone(),
                            message: format!("cannot read: {e}"),
                        }]
                    })
                    .and_then(|t| view_file::to_json(&t)),
                rows: None,
                due: Instant::now(),
            })
            .collect();
        info!(id = %manifest.id, ?tier, "an extension is linked");
        Ok(Loaded {
            manifest,
            tier,
            runner,
            sections,
        })
    }

    fn tick(&mut self) {
        for i in 0..self.links.len() {
            let (code, views) = newest(&self.links[i].dir);
            let (seen_code, seen_views) = self.links[i].seen;
            if code > seen_code {
                let noticed = now_ms();
                let (dir, dev) = (self.links[i].dir.clone(), self.links[i].dev);
                self.links[i].seen = (code, views);
                self.links[i].loaded = self.load(&dir, dev);
                self.links[i].changed_at = Some(noticed);
                self.ask(i, true);
                continue;
            }
            if views > seen_views {
                // Only a template changed: draw it again with the rows held, without asking.
                self.links[i].seen.1 = views;
                self.links[i].changed_at = Some(now_ms());
                self.reload_views(i);
                self.ask_held(i);
                continue;
            }
            self.ask(i, false);
        }
    }

    /// Re-reads link `i`'s templates.
    fn reload_views(&mut self, i: usize) {
        let dir = self.links[i].dir.clone();
        if let Ok(l) = &mut self.links[i].loaded {
            for s in &mut l.sections {
                s.template = std::fs::read_to_string(dir.join(&s.section.view))
                    .map_err(|e| {
                        vec![TemplateError {
                            at: None,
                            path: s.section.view.clone(),
                            message: format!("cannot read: {e}"),
                        }]
                    })
                    .and_then(|t| view_file::to_json(&t));
            }
        }
    }

    fn ask(&mut self, i: usize, now: bool) {
        self.ask_with(i, now, false);
    }

    fn ask_held(&mut self, i: usize) {
        self.ask_with(i, true, true);
    }

    /// Asks each due section of link `i` for its rows (all of them when `now`;
    /// none when `held`, which draws the rows it has) and tells clients.
    fn ask_with(&mut self, i: usize, now: bool, held: bool) {
        let world: Arc<dyn World> = Arc::clone(&self.world) as Arc<dyn World>;
        let link = &mut self.links[i];
        let key = link.dir.display().to_string();
        let loaded = match &mut link.loaded {
            Ok(l) => l,
            Err(e) => {
                let note =
                    json!({ "key": key, "dir": key, "error": e, "changedAt": link.changed_at });
                self.tell(note, Some(key));
                return;
            }
        };
        let ctx = CallCtx {
            project: link.project.clone(),
            caller: "sidebar".into(),
        };
        let mut notes = Vec::new();
        for s in &mut loaded.sections {
            if !now && Instant::now() < s.due {
                continue;
            }
            s.due = Instant::now() + s.every;
            let t = Instant::now();
            let answer = match (&s.template, &s.rows) {
                // A broken template draws its errors; the extension is not asked.
                (Err(_), _) => Ok(Value::Null),
                (Ok(_), Some(rows)) if held => rows.clone().map_err(|e| ExtError::new("failed", e)),
                (Ok(_), _) => call(
                    &mut loaded.runner,
                    &loaded.manifest,
                    world.as_ref(),
                    &s.section.rows,
                    &ctx,
                ),
            };
            if s.template.is_ok() && !held {
                s.rows = Some(answer.clone().map_err(|e| e.message));
            }
            let took = t.elapsed().as_secs_f64() * 1e3;
            let mut note = json!({
                "key": format!("{key}#{}", s.section.id),
                "dir": key,
                "ext": loaded.manifest.id,
                "name": loaded.manifest.name,
                "tier": loaded.tier,
                "section": s.section.id,
                "title": s.section.title,
                "project": ctx.project,
                "changedAt": link.changed_at,
                "answeredAt": now_ms(),
                "tookMs": took,
            });
            match &s.template {
                Ok(t) => note["template"] = t.clone(),
                Err(errs) => note["templateErrors"] = errors_json(&s.section.view, errs),
            }
            match answer {
                Ok(rows) => note["rows"] = rows,
                Err(e) => note["error"] = json!(e.message),
            }
            if let Runner::Component(inst) = &loaded.runner {
                note["timings"] = json!(inst.timings);
            }
            notes.push(note);
        }
        link.changed_at = None;
        for n in notes {
            let k = n["key"].as_str().unwrap_or_default().to_owned();
            self.tell(n, Some(k));
        }
    }

    fn tell(&self, note: Value, key: Option<String>) {
        if let (Some(k), Ok(mut s)) = (key, self.sections.lock()) {
            s.insert(k, note.clone());
        }
        if let Some(native) = self.native.upgrade() {
            native.broadcast(SECTION, note);
        }
    }

    /// The safe tier's numbers inside vornd: start, call and memory per instance.
    fn bench(&self, dir: &Path, calls: u32, instances: u32) -> Result<Value, String> {
        let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_owned());
        let link = self
            .links
            .iter()
            .find(|l| l.dir == dir)
            .ok_or("that folder is not linked")?;
        let loaded = link.loaded.as_ref().map_err(Clone::clone)?;
        if loaded.tier != Tier::Safe {
            return Err("bench measures the safe tier; link the component".into());
        }
        let rt = self.runtime()?;
        let file = dir.join(
            loaded
                .manifest
                .runtime
                .component
                .as_deref()
                .unwrap_or_default(),
        );
        let t = Instant::now();
        let component = rt.load(&file).map_err(|e| e.message)?;
        let load_ms = t.elapsed().as_secs_f64() * 1e3;
        let world: Arc<dyn World> = Arc::new(vorn_ext::checks::Replay(Default::default()));
        let ctx = CallCtx {
            project: link.project.clone(),
            caller: "bench".into(),
        };
        let rss_before = rss_kb();
        let mut fleet = Vec::new();
        let mut inst_us = Vec::new();
        let t = Instant::now();
        for _ in 0..instances.max(1) {
            let mut i = Instance::new(
                Arc::clone(&rt),
                component.clone(),
                Arc::clone(&loaded.manifest),
                Arc::clone(&world),
            )
            .map_err(|e| e.message)?;
            // `ping` answers without the bridge; the first call starts the instance.
            let _ = i.call("ping", &Value::Null, &ctx, wasm::CALL_DEADLINE);
            inst_us.push(i.timings.instantiate_us);
            fleet.push(i);
        }
        let fleet_ms = t.elapsed().as_secs_f64() * 1e3;
        let rss_after = rss_kb();
        let one = fleet.first_mut().ok_or("no instance")?;
        let t = Instant::now();
        for _ in 0..calls {
            let _ = one.call("ping", &Value::Null, &ctx, wasm::CALL_DEADLINE);
        }
        let call_us = t.elapsed().as_secs_f64() * 1e6 / f64::from(calls.max(1));
        inst_us.sort_by(f64::total_cmp);
        Ok(json!({
            "loadMs": load_ms,
            "instantiateUsMedian": inst_us[inst_us.len() / 2],
            "instances": instances,
            "fleetMs": fleet_ms,
            "callUs": call_us,
            "linearMemoryPerInstance": one.timings.memory,
            "rssPerInstanceKb": (rss_after.saturating_sub(rss_before)) as f64 / f64::from(instances.max(1)),
        }))
    }
}

/// One call in the extension's tier.
fn call(
    runner: &mut Runner,
    m: &Manifest,
    world: &dyn World,
    method: &str,
    ctx: &CallCtx,
) -> Result<Value, ExtError> {
    match runner {
        Runner::OneShot(p) => quick::one_shot(p, method, &json!({}), ctx, QUICK_DEADLINE),
        Runner::Service(p, live) => {
            if !live.as_mut().is_some_and(Service::alive) {
                *live = Some(Service::start(p, ctx)?);
            }
            let gate = Gate {
                manifest: m,
                world,
                project: &ctx.project,
            };
            let s = live.as_mut().expect("started above");
            s.call(method, &json!({}), ctx, &gate, QUICK_DEADLINE)
        }
        Runner::Component(i) => i.call(method, &json!({}), ctx, wasm::CALL_DEADLINE),
    }
}

fn errors_json(file: &str, errs: &[TemplateError]) -> Value {
    Value::Array(
        errs.iter()
            .map(|e| {
                let at = e.at.map_or(String::new(), |(l, c)| format!(":{l}:{c}"));
                json!({ "where": format!("{file}{at}"), "path": e.path, "message": e.message })
            })
            .collect(),
    )
}

/// The newest modification times in an extension's folder, `target/` and
/// dot folders aside: of its code and manifest, and of its `views/`.
fn newest(dir: &Path) -> (SystemTime, SystemTime) {
    fn walk(dir: &Path, depth: u8, best: &mut SystemTime) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.')
                || name == "target"
                || name == "checks"
                || (depth == 2 && name == "views")
            {
                continue;
            }
            let Ok(meta) = e.metadata() else { continue };
            if let Ok(m) = meta.modified() {
                *best = (*best).max(m);
            }
            if meta.is_dir() && depth > 0 {
                walk(&e.path(), depth - 1, best);
            }
        }
    }
    let (mut code, mut views) = (UNIX_EPOCH, UNIX_EPOCH);
    walk(dir, 2, &mut code);
    walk(&dir.join("views"), 1, &mut views);
    (code, views)
}

/// This process's resident memory, in KB.
fn rss_kb() -> u64 {
    let out = vorn_spawn::command("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output();
    out.ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(0)
}
