//! vornd as the Vorn server: what clients connect to, with no other process
//! behind it.
//!
//! Started with a data directory and no upstream, vornd holds that directory
//! ([`files`]), opens its database (creating and migrating it), binds the
//! port this install keeps ([`port`]) and publishes it, with the credential
//! clients on this machine read. It speaks the whole client protocol itself
//! ([`socket`]): the greeting, authentication, topics and every
//! notification ([`clients`]), and serves the web client and the plain
//! routes ([`http`]). It rebinds when Network Access changes, tells clients
//! when the configuration changes, however it changed, and stops on its own
//! once nothing has used it for a while ([`idle`]).

pub mod clients;
pub mod files;
pub mod http;
pub mod idle;
pub mod port;
pub mod socket;

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::{watch, Notify};
use tracing::{error, info, warn};

use crate::native::Native;
use clients::Clients;

/// How often the configuration's signal file is looked at.
const SIGNAL_EVERY: Duration = Duration::from_millis(500);

/// How vornd as the server was asked to run.
#[derive(Debug, Clone)]
pub struct ServeConfig {
    pub data_dir: PathBuf,
    /// `--port`, which wins over the remembered one and is never remembered.
    pub port: Option<u16>,
    /// `--host`, which wins over Network Access.
    pub host: Option<IpAddr>,
    /// The web client's build, served under `/app`.
    pub web: Option<PathBuf>,
    /// Stop after this long with nothing to do; `None` to run until told.
    pub idle: Option<Duration>,
    /// Which build: `dev` or `packaged`, said in the greeting.
    pub build_channel: String,
    pub app_version: String,
}

/// Why vornd cannot serve.
#[derive(Debug)]
pub enum StartError {
    Database(String),
    Bind(std::io::Error),
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartError::Database(why) => write!(f, "the database cannot open: {why}"),
            StartError::Bind(err) => write!(f, "could not listen: {err}"),
        }
    }
}

/// What vornd keeps as the server.
#[derive(Debug)]
pub struct Serving {
    held: files::Held,
    credential: Vec<u8>,
    config: ServeConfig,
    pub clients: Arc<Clients>,
    /// Sockets waiting to authenticate.
    pub(crate) pending: AtomicUsize,
    listener: watch::Sender<Option<Arc<TcpListener>>>,
    bound: Mutex<SocketAddr>,
    stop: Notify,
    stopping: AtomicBool,
}

/// The database `dir` keeps, created, migrated and seeded as the app does.
pub fn open_database(dir: &Path) -> Result<PathBuf, StartError> {
    let db = dir.join("vorn.db");
    let mut options = crate::native::config::options();
    options.owner_name = owner_name();
    options.seed_workflows = seed_workflows();
    match vorn_store::Store::open(&db, options) {
        Ok((_, vorn_store::Opened::Ok)) => Ok(db),
        Ok((_, vorn_store::Opened::Recovered { backup })) => {
            warn!(backup = %backup.display(), "the database was corrupt and has been reset");
            Ok(db)
        }
        Err(err) => Err(StartError::Database(err.to_string())),
    }
}

/// The name a new database's owner is given: this user's.
fn owner_name() -> String {
    ["USER", "USERNAME", "LOGNAME"]
        .iter()
        .find_map(|v| std::env::var(v).ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "owner".to_owned())
}

/// The workflows every install is given once.
fn seed_workflows() -> Vec<vorn_store::SeedWorkflow> {
    #[derive(serde::Deserialize)]
    struct Seed {
        flag: String,
        workflow: vorn_protocol::WorkflowDefinition,
    }
    let seeds: Vec<Seed> = serde_json::from_str(include_str!("seed-workflows.json"))
        .expect("the seeded workflows are workflows");
    seeds
        .into_iter()
        .map(|s| vorn_store::SeedWorkflow {
            flag: s.flag,
            workflow: s.workflow,
        })
        .collect()
}

impl Serving {
    /// Takes the data directory, opens the database and binds; nothing is
    /// published until [`Serving::publish`].
    pub async fn start(
        config: ServeConfig,
        held: files::Held,
        native: &Arc<Native>,
        credential: Vec<u8>,
    ) -> Result<Arc<Serving>, StartError> {
        let defaults = defaults(native).await;
        let network = defaults["networkAccessEnabled"].as_bool() == Some(true);
        let remembered = defaults["serverPort"]
            .as_u64()
            .and_then(|p| u16::try_from(p).ok());
        let host = config.host.unwrap_or_else(|| port::host(network));
        let wanted = port::wanted(config.port, remembered);
        let (listener, fell_back) = port::bind(host, wanted).await.map_err(StartError::Bind)?;
        let addr = listener.local_addr().map_err(StartError::Bind)?;
        if fell_back {
            warn!(
                wanted,
                taken = addr.port(),
                "the wanted port is held by something else; anything paired to it must be pointed at the new one"
            );
        }
        if port::remember(config.port, remembered, fell_back) && remembered != Some(addr.port()) {
            remember_port(native, addr.port()).await;
        }
        let (listener, _) = watch::channel(Some(Arc::new(listener)));
        let clients = Arc::new(Clients::default());
        native.set_clients(Arc::clone(&clients));
        Ok(Arc::new(Serving {
            held,
            credential,
            config,
            clients,
            pending: AtomicUsize::new(0),
            listener,
            bound: Mutex::new(addr),
            stop: Notify::new(),
            stopping: AtomicBool::new(false),
        }))
    }

    /// Where vornd listens now.
    pub fn addr(&self) -> SocketAddr {
        *self.bound.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The listeners to accept on, as they change with Network Access.
    pub fn listeners(&self) -> watch::Receiver<Option<Arc<TcpListener>>> {
        self.listener.subscribe()
    }

    pub fn credential(&self) -> &[u8] {
        &self.credential
    }

    pub fn data_dir(&self) -> &Path {
        self.held.dir()
    }

    /// How long it waits idle before stopping, if it stops on its own.
    pub fn idle_window(&self) -> Option<Duration> {
        self.config.idle
    }

    pub fn web(&self) -> Option<&Path> {
        self.config.web.as_deref()
    }

    /// Announces this server: the credential, then the port.
    pub fn publish(&self) {
        self.held.publish_credential(&self.credential);
        self.held.publish_port(self.addr().port());
    }

    /// Takes back what [`Serving::publish`] published.
    pub fn withdraw(&self) {
        self.held.withdraw(&self.credential);
    }

    /// The greeting every socket is sent first.
    pub fn hello(&self) -> Value {
        json!({
            "jsonrpc": "2.0",
            "method": "server:hello",
            "params": {
                "protocolVersion": 1,
                "capabilities": { "auth": 1, "subscribe": 1, "terminalBytes": 1, "terminalResync": 1 },
            },
        })
    }

    /// Who this server is, sent to a socket on this machine before it
    /// authenticates, for a desktop deciding whether to adopt it.
    pub fn identity(&self, sessions: Option<u64>) -> Value {
        json!({
            "jsonrpc": "2.0",
            "method": "server:identity",
            "params": {
                "appVersion": self.config.app_version,
                "dataDir": self.data_dir().to_string_lossy(),
                "pid": std::process::id(),
                "buildChannel": self.config.build_channel,
                "sessions": sessions,
            },
        })
    }

    /// Asks vornd to stop.
    pub fn request_stop(&self) {
        self.stopping.store(true, Ordering::Release);
        self.stop.notify_waiters();
        self.stop.notify_one();
    }

    /// Resolves once something asked vornd to stop.
    pub async fn stopped(&self) {
        if self.stopping.load(Ordering::Acquire) {
            return;
        }
        self.stop.notified().await;
    }

    /// Follows the configuration, however it changes: vornd's own saves, and
    /// the signal file a change anywhere else writes. Rebinds when Network
    /// Access changes, rearms the schedules and, for a change made elsewhere,
    /// tells every client.
    pub async fn follow_config(self: Arc<Self>, native: Weak<Native>) {
        let Some(changes) = native.upgrade() else {
            return;
        };
        let signal = self.data_dir().join(".db-signal");
        let mut seen = std::fs::read(&signal).ok();
        let mut every = tokio::time::interval(SIGNAL_EVERY);
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let elsewhere = tokio::select! {
                () = changes.config_change() => false,
                _ = every.tick() => {
                    let now = std::fs::read(&signal).ok();
                    if now == seen {
                        continue;
                    }
                    seen = now;
                    true
                }
                () = self.stopped() => return,
            };
            let Some(native) = native.upgrade() else {
                return;
            };
            if elsewhere {
                crate::native::config::announce(&native).await;
            }
            if let Some(work) = native.work() {
                work.workflows_changed_elsewhere();
            }
            if self.config.host.is_none() {
                let network =
                    defaults(&native).await["networkAccessEnabled"].as_bool() == Some(true);
                self.rebind(&native, port::host(network)).await;
            }
        }
    }

    /// Listens on `host` instead, on the same port, if that is a change.
    async fn rebind(&self, native: &Native, host: IpAddr) {
        let old = self.addr();
        if old.ip() == host {
            return;
        }
        info!(from = %old.ip(), to = %host, port = old.port(), "rebinding");
        let previous = self.listener.send_replace(None).map(|l| Arc::downgrade(&l));
        // The port is free only once the accept loop has let go of it.
        let deadline = Instant::now() + Duration::from_secs(2);
        while previous.as_ref().is_some_and(|w| w.strong_count() > 0) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let bound = match TcpListener::bind(SocketAddr::new(host, old.port())).await {
            Ok(listener) => listener,
            Err(err) => {
                error!(%err, %host, "could not rebind; listening where it was");
                match TcpListener::bind(old).await {
                    Ok(listener) => listener,
                    Err(err) => {
                        error!(%err, "could not listen again where it was");
                        return;
                    }
                }
            }
        };
        let addr = bound.local_addr().unwrap_or(old);
        *self.bound.lock().unwrap_or_else(|e| e.into_inner()) = addr;
        self.listener.send_replace(Some(Arc::new(bound)));
        if let Some(link) = native.link() {
            link.set_server_host(addr.ip().to_string());
        }
    }

    /// Stops vornd once nothing has used it for the window.
    pub async fn watch_idle(self: Arc<Self>, native: Weak<Native>, window: Duration) {
        let mut watch = idle::Watch::new(window);
        let mut every = tokio::time::interval(watch.every());
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = every.tick() => {}
                () = self.stopped() => return,
            }
            let Some(native) = native.upgrade() else {
                return;
            };
            let snapshot = self.snapshot(&native).await;
            if watch.tick(&snapshot, Instant::now()) {
                info!("nothing left to do; stopping");
                self.request_stop();
                return;
            }
        }
    }

    /// What could hold the server open now.
    async fn snapshot(&self, native: &Native) -> idle::Snapshot {
        let live = native
            .registry_live()
            .unwrap_or_else(|| json!({ "sessions": 0, "headless": 0 }));
        let hooks = native.hooks_activity().unwrap_or(Value::Null);
        let since_hook = hooks["msSinceActivity"]
            .as_u64()
            .map_or(Duration::MAX, Duration::from_millis);
        let schedules = match native.work() {
            Some(work) => work.armed_schedules().await as u64,
            None => 0,
        };
        idle::Snapshot {
            sessions: live["sessions"].as_u64().unwrap_or(0),
            headless: live["headless"].as_u64().unwrap_or(0),
            since_client: self.clients.quiet_for(),
            since_hook,
            pending_permissions: hooks["pendingPermissions"].as_u64().unwrap_or(0),
            pending_pairings: native.pending_pairings() as u64,
            connector_leases: connector_leases(native).await,
            schedules,
            serves_others: self.addr().ip().is_unspecified(),
        }
    }
}

/// The configuration's defaults, or nothing when they cannot be read.
async fn defaults(native: &Arc<Native>) -> Value {
    let n = Arc::clone(native);
    let loaded = crate::native::config::blocking("config:load", move || {
        crate::native::config::with_store(&n, |s| s.load_config())
    })
    .await;
    loaded.map(|c| c["defaults"].clone()).unwrap_or(Value::Null)
}

/// Writes `port` as the one this install keeps, so the next start is the same origin.
async fn remember_port(native: &Arc<Native>, port: u16) {
    let n = Arc::clone(native);
    let saved = crate::native::config::blocking("config:save", move || {
        crate::native::config::with_store(&n, |s| {
            let mut config = s.load_config()?;
            config["defaults"]["serverPort"] = json!(port);
            s.save_config(&config, &[])
        })
    })
    .await;
    if let Err(err) = saved {
        warn!(%err, "could not remember the port; it may change next start");
    }
}

/// Connector work claimed and not finished.
async fn connector_leases(native: &Native) -> u64 {
    let Some(db) = native.database().map(Path::to_path_buf) else {
        return 0;
    };
    tokio::task::spawn_blocking(move || {
        let store = vorn_store::Store::open_beside(&db).ok()??;
        store
            .db_count_active_connector_inbox_leases(&vorn_store::now_iso())
            .ok()
    })
    .await
    .ok()
    .flatten()
    .map_or(0, |n| u64::try_from(n).unwrap_or(0))
}

/// Accepts connections on whichever listener is current until `shutdown`.
pub async fn accept(
    mut listeners: watch::Receiver<Option<Arc<TcpListener>>>,
    daemon: Arc<crate::proxy::Daemon>,
    shutdown: impl std::future::Future<Output = ()>,
) {
    tokio::pin!(shutdown);
    loop {
        let current = listeners.borrow_and_update().clone();
        let Some(listener) = current else {
            tokio::select! {
                changed = listeners.changed() => if changed.is_err() { return },
                () = &mut shutdown => return,
            }
            continue;
        };
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => crate::proxy::serve_connection(&daemon, stream, peer),
                Err(err) => warn!(%err, "accept failed"),
            },
            changed = listeners.changed() => if changed.is_err() { return },
            () = &mut shutdown => return,
        }
    }
}
