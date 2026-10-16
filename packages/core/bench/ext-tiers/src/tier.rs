//! One extension under one tier: compiled in (the built-in's way, as the
//! baseline), a program, or a Wasm component; the last two through the
//! router, the lifecycle and the gate as vornd runs them.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use ext_tiers_logic::{Error, Extension, Host, Op};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use vorn_extensions::api::types::{CallCtx, RowOp};
use vorn_extensions::api::ExtError;
use vorn_extensions::events::{Bus, Owner, Queue, Tally, QUEUE};
use vorn_extensions::gate::{self, Caller, World};
use vorn_extensions::install::Installs;
use vorn_extensions::lifecycle::{Lifecycle, Policy};
use vorn_extensions::manifest::{file, Manifest};
use vorn_extensions::pack::PackStore;
use vorn_extensions::router::{Router, RouterSettings};

use crate::world::Fake;

/// Where the built-ins' manifests and templates are: every tier draws the same files.
pub const BUILTIN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../crates/vornd/src/native/extensions/builtin"
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Builtin,
    Program,
    Wasm,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "builtin" => Some(Kind::Builtin),
            "program" => Some(Kind::Program),
            "wasm" => Some(Kind::Wasm),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Builtin => "builtin",
            Kind::Program => "program",
            Kind::Wasm => "wasm",
        }
    }
}

/// Worktree cleanup or the Widget: its id, files and events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ext {
    pub id: &'static str,
    pub toml: &'static str,
    pub views: &'static [&'static str],
    pub events: &'static [&'static str],
    pub collections: &'static [&'static str],
}

pub const WORKTREES: Ext = Ext {
    id: "vorn.worktrees",
    toml: "worktrees.toml",
    views: &["idle.kdl", "confirm.kdl", "settings.kdl"],
    events: &["worktree.idle"],
    collections: &["idle", "asked", "worktrees"],
};

pub const WIDGET: Ext = Ext {
    id: "vorn.widget",
    toml: "widget.toml",
    views: &["widget.kdl", "compact.kdl"],
    events: &[
        "session.created",
        "session.status",
        "session.exited",
        "agent.permission",
        "agent.permissionClosed",
    ],
    collections: &["agents", "permissions", "suggestions", "options"],
};

impl Ext {
    pub fn parse(s: &str) -> Option<Ext> {
        match s {
            "worktrees" => Some(WORKTREES),
            "widget" => Some(WIDGET),
            _ => None,
        }
    }

    pub fn short(self) -> &'static str {
        self.id.trim_start_matches("vorn.")
    }

    fn manifest_text(self) -> String {
        std::fs::read_to_string(Path::new(BUILTIN).join(self.toml))
            .expect("the built-in's manifest")
    }

    /// Its package folder: the built-in's manifest with one `[service]`, and its templates.
    fn folder(self, at: &Path, service: &str) -> PathBuf {
        let dir = at.join(self.short());
        std::fs::create_dir_all(dir.join("views")).expect("pack folder");
        let events: Vec<String> = self.events.iter().map(|e| format!("\"{e}\"")).collect();
        let toml = format!(
            "{}\n[service]\n{service}\nevents = [{}]\n",
            self.manifest_text(),
            events.join(", ")
        );
        std::fs::write(dir.join("vorn-extension.toml"), toml).expect("manifest");
        for v in self.views {
            std::fs::copy(
                Path::new(BUILTIN).join("views").join(v),
                dir.join("views").join(v),
            )
            .expect("view");
        }
        dir
    }
}

/// What a tier runs from.
pub struct Paths {
    pub program: PathBuf,
    pub component: PathBuf,
}

/// The bench's compiled form of `component`, compiled once per build of it.
pub fn precompiled(component: &Path) -> PathBuf {
    let cache = component.with_file_name("ext-tiers-compiled");
    let bytes = std::fs::read(component).expect("the component");
    let compiled = vorn_extensions::wasm::compiled(&cache, &bytes);
    if !compiled.is_file() {
        vorn_extensions::wasm::precompile(component, &cache).expect("compiles");
    }
    compiled
}

fn env(home: &Path) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| ["PATH", "TMPDIR"].contains(&k.as_str()))
        .collect();
    env.push(("HOME".into(), home.to_string_lossy().into_owned()));
    env
}

fn ctx() -> CallCtx {
    CallCtx {
        caller: "bench".into(),
        project: None,
    }
}

/// The compiled-in extension: its logic on vornd's runtime, its calls through the gate.
struct InProc {
    ext: Mutex<Option<Box<dyn Extension>>>,
    manifest: Arc<Manifest>,
    caller: Caller,
    router: Arc<Router>,
    tally: Arc<Tally>,
}

struct Direct<'a> {
    me: &'a InProc,
}

fn sync<F: std::future::Future>(f: F) -> F::Output {
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f))
}

impl Host for Direct<'_> {
    fn call(&mut self, method: &str, args: Value) -> Result<Value, Error> {
        let me = self.me;
        let answer = gate::admit_call(&me.manifest, method)
            .and_then(|_| sync(me.router.world().call(&me.caller, method, args)));
        answer.map_err(|e| Error {
            code: e.code,
            message: e.message,
        })
    }

    fn publish(&mut self, collection: &str, ops: Vec<Op>) {
        let ops = ops
            .into_iter()
            .map(|op| match op {
                Op::Upsert(row) => RowOp::Upsert(row),
                Op::Delete(id) => RowOp::Delete(id),
                Op::Clear => RowOp::Clear,
            })
            .collect();
        self.me.router.publish(&self.me.caller, collection, ops);
    }
}

impl InProc {
    fn with<T>(&self, f: impl FnOnce(&mut dyn Extension, &mut Direct<'_>) -> T) -> Option<T> {
        let mut guard = self.ext.lock().unwrap_or_else(PoisonError::into_inner);
        let ext = guard.as_mut()?;
        Some(tokio::task::block_in_place(|| {
            f(ext.as_mut(), &mut Direct { me: self })
        }))
    }

    /// Starts it as vornd starts a switched-on built-in.
    fn start(self: &Arc<Self>, ext: Ext) {
        let mut fresh = ext_tiers_logic::by_id(ext.id).expect("a shipped extension");
        let me = Arc::clone(self);
        tokio::task::block_in_place(|| fresh.init(&mut Direct { me: &me }));
        *self.ext.lock().unwrap_or_else(PoisonError::into_inner) = Some(fresh);
        let (tx, mut rx) = mpsc::channel(QUEUE);
        let names: Vec<String> = ext.events.iter().map(|n| (*n).to_owned()).collect();
        self.router.bus().subscribe_with(
            Owner::Core(format!("builtin:{}", ext.id)),
            &names,
            Queue {
                tx,
                tally: Some(Arc::clone(&self.tally)),
            },
        );
        tokio::spawn(async move {
            while let Some(first) = rx.recv().await {
                let mut batch = vec![first];
                while let Ok(next) = rx.try_recv() {
                    batch.push(next);
                }
                let events: Vec<(String, Value)> = batch
                    .iter()
                    .map(|e| (e.name.clone(), e.data.clone()))
                    .collect();
                me.with(|ext, host| ext.on_events(host, &events));
                me.tally.handled(batch.len());
            }
        });
    }

    fn stop(&self, ext: Ext) {
        self.router
            .bus()
            .unsubscribe(&Owner::Core(format!("builtin:{}", ext.id)));
        self.ext
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }
}

/// One extension running under one tier.
pub struct Tier {
    pub ext: Ext,
    pub router: Arc<Router>,
    pub world: Arc<Fake>,
    inproc: Option<Arc<InProc>>,
    _home: tempfile::TempDir,
}

impl Tier {
    /// Sets it up, started for none: nothing runs until something uses it.
    pub fn new(kind: Kind, ext: Ext, paths: &Paths, call_timeout: Duration) -> Tier {
        let home = tempfile::tempdir().expect("temp home");
        let store = PackStore::new(home.path().join("extensions"));
        let world = Arc::new(Fake::new());
        let mut settings = RouterSettings::new(env(home.path()), "bench");
        settings.call_timeout = call_timeout;
        let router = Router::new(
            store.clone(),
            Lifecycle::new(Policy::default()),
            Arc::clone(&world) as Arc<dyn World>,
            settings,
            Bus::new(),
        );
        let inproc = match kind {
            Kind::Builtin => {
                let manifest = file::parse_builtin(&ext.manifest_text()).expect("manifest");
                Some(Arc::new(InProc {
                    ext: Mutex::new(None),
                    manifest: Arc::new(manifest),
                    caller: Caller {
                        extension: ext.id.to_owned(),
                        project: None,
                    },
                    router: Arc::clone(&router),
                    tally: Arc::new(Tally::default()),
                }))
            }
            Kind::Program | Kind::Wasm => {
                let packs = home.path().join("packs");
                let service = if kind == Kind::Program {
                    format!("command = [{:?}]", paths.program.display().to_string())
                } else {
                    "component = \"ext.wasm\"".to_owned()
                };
                let folder = ext.folder(&packs, &service);
                if kind == Kind::Wasm {
                    std::fs::copy(&paths.component, folder.join("ext.wasm")).expect("component");
                    // In Vorn's cache already, as `vorn ext install` would leave it,
                    // so the tier runs on the runtime vornd ships.
                    let compiled = precompiled(&paths.component);
                    let cache = store.root().join(vorn_extensions::wasm::COMPILED);
                    std::fs::create_dir_all(&cache).expect("cache");
                    let name = compiled.file_name().expect("named");
                    std::fs::copy(&compiled, cache.join(name)).expect("compiled");
                }
                Installs::new(store, "git", env(home.path()))
                    .link(&folder)
                    .expect("linked");
                router.reindex();
                None
            }
        };
        Tier {
            ext,
            router,
            world,
            inproc,
            _home: home,
        }
    }

    /// Starts it, as its first use would.
    pub async fn start(&self) -> Result<(), ExtError> {
        match &self.inproc {
            Some(inproc) => {
                inproc.start(self.ext);
                Ok(())
            }
            None => self.call("bench.echo", json!({})).await.map(drop),
        }
    }

    /// Stops it; the next use starts it fresh.
    pub async fn stop(&self) {
        match &self.inproc {
            Some(inproc) => inproc.stop(self.ext),
            None => self.router.lifecycle().stop_all().await,
        }
    }

    pub async fn call(&self, action: &str, args: Value) -> Result<Value, ExtError> {
        match &self.inproc {
            Some(inproc) => {
                if let Some(probed) = ext_tiers_logic::probe(action, &args) {
                    return probed.map_err(|e| ExtError::new(&e.code, e.message));
                }
                inproc
                    .with(|ext, host| ext.handle(host, action, &args))
                    .unwrap_or_else(|| Err(Error::new("start", "not started")))
                    .map_err(|e| ExtError::new(&e.code, e.message))
            }
            None => self.router.call(self.ext.id, action, args, ctx()).await,
        }
    }

    /// Queues an event, waiting while its queue is full; answers how many tries it took.
    pub async fn emit(&self, name: &str, data: &Value) -> u32 {
        let mut tries = 1;
        while self.router.emit(name, data.clone()) == 0 {
            tries += 1;
            tokio::time::sleep(Duration::from_micros(50)).await;
        }
        tries
    }

    /// Resolves once every event queued so far was handled.
    pub async fn settled(&self) {
        match &self.inproc {
            Some(inproc) => inproc.tally.settled().await,
            None => {
                // The router counts an event once it is handed over; the guest
                // serves a call only after what it was handed before it.
                self.router.settled(self.ext.id).await;
                let _ = self.call("bench.echo", json!({})).await;
            }
        }
    }

    /// Settled, and no row has moved for a while: a program's publishes are
    /// answered on tasks of their own, so they can land after its last answer.
    pub async fn quiet(&self) {
        self.settled().await;
        loop {
            let seen = self.world.published.load(Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            if self.world.published.load(Ordering::SeqCst) == seen {
                return;
            }
        }
    }

    pub fn rows(&self, collection: &str) -> Vec<Value> {
        self.router.rows(self.ext.id, collection)
    }

    /// Every collection's rows.
    pub fn snapshot(&self) -> Value {
        let mut all = serde_json::Map::new();
        for c in self.ext.collections {
            all.insert((*c).to_owned(), Value::Array(self.rows(c)));
        }
        Value::Object(all)
    }

    /// Waits until `collection` holds a row `id`.
    pub async fn row_appears(&self, collection: &str, id: &str) {
        loop {
            let moved = self.world.moved.notified();
            if self.rows(collection).iter().any(|r| r["id"] == id) {
                return;
            }
            moved.await;
        }
    }

    /// Waits until something was published after the `before`th publish.
    pub async fn published_after(&self, before: u64) {
        loop {
            let moved = self.world.moved.notified();
            if self.world.published.load(Ordering::SeqCst) > before {
                return;
            }
            moved.await;
        }
    }

    /// The program's process, when the tier runs one.
    pub fn pid(&self) -> Option<u32> {
        self.router.running(self.ext.id).and_then(|l| l.pid())
    }

    /// The component's linear memory, in bytes.
    pub fn linear_memory(&self) -> Option<usize> {
        self.router
            .running(self.ext.id)
            .and_then(|l| l.component_memory())
    }
}
