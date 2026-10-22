//! One extension in one tier behind a router of its own: the manifest the
//! extension ships, linked from a folder that holds either its component or
//! a `command` naming its program. Both run through the same router, gate
//! and lifecycle vornd runs; only `[service]` differs.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use vorn_extensions::api::types::CallCtx;
use vorn_extensions::api::ExtError;
use vorn_extensions::component::{Compiler, COMPILED};
use vorn_extensions::gate::World;
use vorn_extensions::install::Installs;
use vorn_extensions::lifecycle::{Lifecycle, Policy};
use vorn_extensions::pack::PackStore;
use vorn_extensions::router::{Router, RouterSettings};

use crate::program::FLUSHED;
use crate::world::Bench;

/// An extension the bench measures: its manifest, its bench id, and its component's file name.
#[derive(Debug, Clone, Copy)]
pub struct Ext {
    pub name: &'static str,
    pub id: &'static str,
    /// The cargo package both tiers are built from.
    pub package: &'static str,
    /// The program-tier binary of this crate.
    pub bin: &'static str,
    pub manifest: &'static str,
}

pub const WIDGET: Ext = Ext {
    name: "Widget",
    id: "bench.widget",
    package: "vorn-bench-widget",
    bin: "tier-widget",
    manifest: include_str!("../widget/vorn-extension.toml"),
};

pub const WORKTREES: Ext = Ext {
    name: "Worktree cleanup",
    id: "bench.worktrees",
    package: "vorn-bench-worktrees",
    bin: "tier-worktrees",
    manifest: include_str!("../worktrees/vorn-extension.toml"),
};

pub const RSS: Ext = Ext {
    name: "RSS",
    id: "bench.rss",
    package: "vorn-bench-rss",
    bin: "tier-rss",
    manifest: include_str!("../rss/vorn-extension.toml"),
};

pub const CRASH: Ext = Ext {
    name: "Crash",
    id: "bench.crash",
    package: "vorn-bench-crash",
    bin: "tier-crash",
    manifest: include_str!("../crash/vorn-extension.toml"),
};

pub const ALL: [Ext; 4] = [WIDGET, WORKTREES, RSS, CRASH];

/// How Vorn runs the extension.
#[derive(Debug, Clone)]
pub enum Tier {
    /// The component at this path, sandboxed in vornd.
    Wasm(PathBuf),
    /// The program at this path, a process of its own.
    Program(PathBuf),
}

impl Tier {
    pub fn label(&self) -> &'static str {
        match self {
            Tier::Wasm(_) => "wasm",
            Tier::Program(_) => "program",
        }
    }
}

impl Ext {
    /// The component file the manifest names.
    fn component(&self) -> &'static str {
        self.manifest
            .lines()
            .find_map(|l| l.strip_prefix("component = \""))
            .and_then(|l| l.strip_suffix('"'))
            .unwrap_or("extension.wasm")
    }

    /// The manifest for `tier`, under its bench id: the same file but for how `[service]` runs it.
    pub fn manifest_for(&self, tier: &Tier) -> String {
        let component = format!("component = \"{}\"", self.component());
        let service = match tier {
            Tier::Wasm(_) => component.clone(),
            Tier::Program(bin) => format!("command = [{:?}]", bin.display().to_string()),
        };
        let mut named = false;
        self.manifest
            .lines()
            .map(|l| {
                // The first `id` is the extension's; later ones name its contributions.
                if !named && l.starts_with("id = ") {
                    named = true;
                    format!("id = \"{}\"", self.id)
                } else if l == component {
                    service.clone()
                } else {
                    l.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// An action no extension has, called to know it is idle.
pub const BARRIER: &str = "bench.barrier";

pub fn ctx() -> CallCtx {
    CallCtx {
        caller: "client".into(),
        project: None,
    }
}

/// One extension in one tier, linked into a store of its own.
pub struct Rig {
    pub router: Arc<Router>,
    pub world: Arc<Bench>,
    pub ext: Ext,
    pub tier: Tier,
    /// Publishes the program has said it sent, summed over every [`FLUSHED`].
    flushed: AtomicU64,
    _root: tempfile::TempDir,
}

impl Rig {
    /// A router with `ext` linked in `tier`; `cache` holds the component
    /// already compiled, as vornd's does after its first start, or else it
    /// compiles in this process.
    pub fn new(ext: Ext, tier: Tier, world: Bench, cache: Option<&Path>) -> Result<Rig, String> {
        let root = tempfile::tempdir().map_err(|e| e.to_string())?;
        let store = root.path().join("store");
        let folder = root.path().join("pack");
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        std::fs::write(folder.join("vorn-extension.toml"), ext.manifest_for(&tier))
            .map_err(|e| e.to_string())?;
        if let Tier::Wasm(wasm) = &tier {
            std::fs::copy(wasm, folder.join(ext.component())).map_err(|e| e.to_string())?;
        }
        let mut settings = RouterSettings::new(Vec::new(), "bench");
        settings.call_timeout = Duration::from_secs(5);
        settings.compiler = match cache {
            Some(cache) => {
                let compiled = store.join(COMPILED);
                std::fs::create_dir_all(&compiled).map_err(|e| e.to_string())?;
                for entry in std::fs::read_dir(cache)
                    .map_err(|e| e.to_string())?
                    .flatten()
                {
                    std::fs::copy(entry.path(), compiled.join(entry.file_name()))
                        .map_err(|e| e.to_string())?;
                }
                // Nothing compiles here: a run loads what the cache holds, as vornd does.
                Compiler::None
            }
            None => Compiler::InProcess,
        };
        let pack_store = PackStore::new(&store);
        Installs::new(pack_store.clone(), "git", Vec::new()).link(&folder)?;
        let world = Arc::new(world);
        let router = Router::new(
            pack_store,
            Lifecycle::new(Policy::default()),
            Arc::clone(&world) as Arc<dyn World>,
            settings,
            vorn_extensions::events::Bus::new(),
        );
        router.reindex();
        Ok(Rig {
            router,
            world,
            ext,
            tier,
            flushed: AtomicU64::new(0),
            _root: root,
        })
    }

    pub async fn call(&self, action: &str, args: Value) -> Result<Value, ExtError> {
        self.router.call(self.ext.id, action, args, ctx()).await
    }

    /// Queues `name` for the extension; it is handed over on the router's own task.
    pub fn emit(&self, name: &str, data: Value) -> usize {
        self.router.emit(name, data)
    }

    /// Waits until every event queued so far has been handed over.
    pub async fn settled(&self) {
        self.router.settled(self.ext.id).await;
    }

    /// Waits until the extension has handled every event queued so far.
    /// A program is handed events as notifications, so a call it answers
    /// after them is the only sign it is done; the call asks Vorn nothing.
    pub async fn handled(&self) {
        self.settled().await;
        let _ = self.call(BARRIER, Value::Null).await;
    }

    /// Waits until Vorn has applied every publish the extension has sent. A
    /// component's land inside the call that made them; a program's are each
    /// applied on a task of their own, so it is asked how many it has sent
    /// that change rows ([`FLUSHED`]) and the world is waited on to have seen
    /// as many.
    pub async fn quiet(&self) {
        if !matches!(self.tier, Tier::Program(_)) {
            return;
        }
        let sent = match self.call(FLUSHED, Value::Null).await {
            Ok(n) => n.as_u64().unwrap_or(0),
            Err(e) => panic!(
                "{} did not say what it published: {}",
                self.ext.name, e.message
            ),
        };
        let want = self.flushed.fetch_add(sent, Ordering::SeqCst) + sent;
        let t = Instant::now();
        while self.world.published() < want {
            assert!(
                t.elapsed() < Duration::from_secs(10),
                "{} sent {want} publishes; Vorn applied {}",
                self.ext.name,
                self.world.published()
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }

    pub fn rows(&self, collection: &str) -> Vec<Value> {
        self.router.rows(self.ext.id, collection)
    }

    pub async fn stop(&self) {
        self.router.stop(self.ext.id).await;
    }

    /// The program's process, while it runs.
    pub fn pid(&self) -> Option<u32> {
        self.router.running(self.ext.id)?.pid()
    }

    /// The most linear memory the component's instance has held, while it runs.
    pub fn linear_bytes(&self) -> Option<usize> {
        self.router.running(self.ext.id)?.component_memory()
    }
}
