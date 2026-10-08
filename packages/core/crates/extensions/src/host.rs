//! The extension hosts: one child per extension and project, started when
//! first needed, each with a token of its own, started again when it
//! crashes and stopped when its project has no session left.
//!
//! A child's token is how the bridge knows who calls it: it is minted at
//! each start, handed to the child in its environment and to nobody else,
//! and forgotten when the child ends.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tracing::{info, warn};

use crate::child::{Child, Launch};
use crate::footer::{read_items, Item};
use crate::js;
use crate::links::MAX_CLICKED_TEXT;
use crate::pack::{InstalledPack, PackStore};
use crate::token;

/// The protocols this host speaks.
pub const SUPPORTED_PROTOCOLS: [u64; 1] = [1];
const UNSUPPORTED_PROTOCOL: i64 = -32001;
const HELLO_TIMEOUT: Duration = Duration::from_secs(15);
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// Crashes within [`CRASH_WINDOW`] after which a host waits to be asked again.
const MAX_CRASHES: usize = 5;
const CRASH_WINDOW: Duration = Duration::from_secs(60);
const FIRST_RESTART_DELAY: Duration = Duration::from_secs(1);

/// What a pack that names no protocol is told.
pub fn outdated_message(name: &str) -> String {
    format!(
        "{name} was built for an older Vorn. Update it in Settings → Connectors, or rebuild it with @vornrun/connector-sdk 0.7.1-beta.3 or later."
    )
}

fn needs_newer_vorn(name: &str, protocol: Option<u64>) -> String {
    let spoken = protocol.map_or_else(
        || "a newer connector protocol".to_owned(),
        |p| format!("connector protocol {p}"),
    );
    format!("{name} speaks {spoken}, which needs a newer Vorn")
}

/// Which extension, for which project.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HostKey {
    pub extension_id: String,
    pub project_path: String,
}

/// The session a footer or handler is asked about.
#[derive(Debug, Clone, Copy)]
pub struct Asked<'a> {
    pub session_id: &'a str,
    pub worktree_path: &'a str,
    pub agent: &'a str,
}

/// A running extension, past its hello.
#[derive(Debug)]
pub struct Host {
    pub key: HostKey,
    token: String,
    child: Child,
    name: String,
}

impl Host {
    /// A footer's items now.
    pub async fn footer(&self, footer: &str, of: Asked<'_>) -> Result<Vec<Item>, String> {
        let answer = self
            .call(
                "extension/footer",
                json!({ "footer": footer, "sessionId": of.session_id, "worktreePath": of.worktree_path, "agent": of.agent }),
            )
            .await?;
        if !answer.get("items").is_some_and(Value::is_array) {
            return Err(format!(
                "{} answered extension/footer without items",
                self.name
            ));
        }
        read_items(&answer).map_err(str::to_owned)
    }

    /// What a link handler made of `url`: the pane it asks to open, if any.
    pub async fn handler(
        &self,
        handler: &str,
        of: Asked<'_>,
        url: &str,
    ) -> Result<Option<String>, String> {
        let answer = self
            .call(
                "extension/handler",
                json!({
                    "handler": handler, "sessionId": of.session_id, "worktreePath": of.worktree_path,
                    "agent": of.agent, "url": js::slice16(url, MAX_CLICKED_TEXT),
                }),
            )
            .await?;
        Ok(answer
            .get("openPane")
            .and_then(Value::as_str)
            .filter(|p| !p.is_empty())
            .map(str::to_owned))
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let answer = self
            .child
            .request(method, params, CALL_TIMEOUT)
            .await
            .map_err(|e| e.to_string())?;
        if answer.is_object() {
            Ok(answer)
        } else {
            let what = if method == "extension/footer" {
                "items"
            } else {
                "an object"
            };
            Err(format!("{} answered {method} without {what}", self.name))
        }
    }

    /// The token it was started with.
    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn exited(&self) -> bool {
        self.child.exited()
    }
}

/// What every host is started with.
#[derive(Debug, Clone)]
pub struct HostSettings {
    /// The program that runs a pack's entry: `node`.
    pub program: PathBuf,
    /// The environment every child starts from.
    pub base_env: Vec<(String, String)>,
    /// Where the bridge is served, e.g. `http://127.0.0.1:50090`.
    pub bridge_origin: String,
    /// The version the hello names.
    pub version: String,
}

#[derive(Default)]
struct Slot {
    /// Held while starting, so callers that arrive meanwhile share one start.
    gate: tokio::sync::Mutex<()>,
    running: Mutex<Option<Arc<Host>>>,
    crashes: Mutex<Vec<Instant>>,
}

/// Every host, by extension and project.
pub struct Supervisor {
    store: PackStore,
    settings: HostSettings,
    slots: Mutex<HashMap<HostKey, Arc<Slot>>>,
    me: Weak<Supervisor>,
}

fn guard<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Supervisor {
    pub fn new(store: PackStore, settings: HostSettings) -> Arc<Supervisor> {
        Arc::new_cyclic(|me| Supervisor {
            store,
            settings,
            slots: Mutex::new(HashMap::new()),
            me: me.clone(),
        })
    }

    pub fn store(&self) -> &PackStore {
        &self.store
    }

    /// The running host for `key`, started if it is not.
    pub async fn get_or_start(&self, key: &HostKey) -> Result<Arc<Host>, String> {
        let slot = Arc::clone(guard(&self.slots).entry(key.clone()).or_default());
        let _starting = slot.gate.lock().await;
        if let Some(host) = guard(&slot.running).as_ref().filter(|h| !h.exited()) {
            return Ok(Arc::clone(host));
        }
        let host = Arc::new(self.start(key).await?);
        let current = guard(&self.slots)
            .get(key)
            .is_some_and(|s| Arc::ptr_eq(s, &slot));
        if !current {
            // Stopped while it started.
            host.child.close().await;
            return Err(format!("{} was stopped before it started", host.name));
        }
        *guard(&slot.running) = Some(Arc::clone(&host));
        self.watch(Arc::clone(&host), &slot);
        Ok(host)
    }

    async fn start(&self, key: &HostKey) -> Result<Host, String> {
        let id = &key.extension_id;
        let pack = self
            .store
            .describe(id)
            .filter(InstalledPack::is_extension)
            .ok_or_else(|| format!("No extension \"{id}\" is installed"))?;
        if !pack.entry().is_file() {
            return Err(format!("The extension \"{id}\" has no files to run"));
        }
        if self.settings.bridge_origin.is_empty() {
            return Err("The extension bridge has no address yet".into());
        }
        let name = pack.name.clone();
        match pack.protocol {
            None => return Err(outdated_message(&name)),
            Some(p) if !SUPPORTED_PROTOCOLS.contains(&p) => {
                return Err(needs_newer_vorn(&name, Some(p)))
            }
            Some(_) => {}
        }
        let token = token::mint().map_err(|e| format!("{name} has no token: {e}"))?;
        let mut env = self.settings.base_env.clone();
        set(
            &mut env,
            "VORN_EXTENSION_HOST",
            format!("{}/extensions/{id}/bridge", self.settings.bridge_origin),
        );
        set(&mut env, "VORN_EXTENSION_TOKEN", token.clone());
        let launch = Launch {
            program: self.settings.program.clone(),
            args: vec![pack.entry().to_string_lossy().into_owned()],
            cwd: PathBuf::from(&key.project_path),
            env,
        };
        let child = Child::start(&launch, format!("[extensions] {name}"))?;
        let hello = child
            .request(
                "vorn/hello",
                json!({ "protocols": SUPPORTED_PROTOCOLS, "host": { "name": "vorn", "version": self.settings.version } }),
                HELLO_TIMEOUT,
            )
            .await;
        let agreed = match hello {
            Ok(answer) => answer.get("protocol").and_then(Value::as_f64),
            Err(err) => {
                child.close().await;
                return Err(match err.code() {
                    Some(UNSUPPORTED_PROTOCOL) => needs_newer_vorn(&name, None),
                    _ => format!("{name} did not answer vorn/hello: {err}"),
                });
            }
        };
        match agreed {
            Some(p) if p.fract() == 0.0 && SUPPORTED_PROTOCOLS.contains(&(p as u64)) => {}
            Some(p) => {
                child.close().await;
                return Err(match (p.fract() == 0.0 && p >= 0.0).then_some(p as u64) {
                    Some(p) => needs_newer_vorn(&name, Some(p)),
                    None => {
                        format!("{name} speaks connector protocol {p}, which needs a newer Vorn")
                    }
                });
            }
            None => {
                child.close().await;
                return Err(format!("{name} answered vorn/hello without a protocol"));
            }
        }
        info!("[extensions] {name} started for {}", key.project_path);
        Ok(Host {
            key: key.clone(),
            token,
            child,
            name,
        })
    }

    /// Starts `host` again when it ends without being stopped, unless it
    /// has crashed [`MAX_CRASHES`] times within [`CRASH_WINDOW`].
    fn watch(&self, host: Arc<Host>, slot: &Arc<Slot>) {
        let me = self.me.clone();
        let slot = Arc::downgrade(slot);
        tokio::spawn(async move {
            let exit = host.child.wait().await;
            let (Some(me), Some(slot)) = (me.upgrade(), slot.upgrade()) else {
                return;
            };
            {
                let mut running = guard(&slot.running);
                if !running.as_ref().is_some_and(|h| Arc::ptr_eq(h, &host)) {
                    return;
                }
                *running = None;
            }
            let still_wanted = guard(&me.slots)
                .get(&host.key)
                .is_some_and(|s| Arc::ptr_eq(s, &slot));
            if !still_wanted {
                return;
            }
            let crashes = {
                let mut crashes = guard(&slot.crashes);
                let now = Instant::now();
                crashes.retain(|at| now.duration_since(*at) < CRASH_WINDOW);
                crashes.push(now);
                crashes.len()
            };
            if crashes > MAX_CRASHES {
                warn!(
                    "{} crashed {crashes} times in a minute; it starts again when next needed",
                    host.name
                );
                return;
            }
            let delay = FIRST_RESTART_DELAY * (1 << (crashes - 1));
            warn!(
                "{} ended ({exit}); starting it again in {} s",
                host.name,
                delay.as_secs()
            );
            tokio::time::sleep(delay).await;
            let still_wanted = guard(&me.slots)
                .get(&host.key)
                .is_some_and(|s| Arc::ptr_eq(s, &slot));
            if still_wanted {
                if let Err(err) = me.get_or_start(&host.key).await {
                    warn!(
                        "[extensions] {} did not start again: {err}",
                        host.key.extension_id
                    );
                }
            }
        });
    }

    /// Who holds `offered` among `extension_id`'s running hosts. Every one
    /// is compared, so how long it takes says nothing about which matched.
    pub fn by_token(&self, extension_id: &str, offered: &str) -> Option<HostKey> {
        let mut found = None;
        for host in self.running() {
            if host.key.extension_id == extension_id && token::same(&host.token, offered) {
                found = Some(host.key.clone());
            }
        }
        found
    }

    /// The token of `key`'s running host.
    pub fn token_for(&self, key: &HostKey) -> Option<String> {
        self.running()
            .into_iter()
            .find(|h| h.key == *key)
            .map(|h| h.token.clone())
    }

    /// Every running host.
    pub fn running(&self) -> Vec<Arc<Host>> {
        guard(&self.slots)
            .values()
            .filter_map(|slot| guard(&slot.running).clone())
            .filter(|h| !h.exited())
            .collect()
    }

    /// Stops every host `matches` picks.
    pub async fn stop_where(&self, matches: impl Fn(&HostKey) -> bool) {
        let removed: Vec<Arc<Slot>> = {
            let mut slots = guard(&self.slots);
            let keys: Vec<HostKey> = slots.keys().filter(|k| matches(k)).cloned().collect();
            keys.iter().filter_map(|k| slots.remove(k)).collect()
        };
        let closing = removed
            .iter()
            .filter_map(|slot| guard(&slot.running).take());
        futures_join(closing.map(|host| async move { host.child.close().await })).await;
    }

    pub async fn stop_extension(&self, extension_id: &str) {
        self.stop_where(|k| k.extension_id == extension_id).await;
    }

    pub async fn stop_project(&self, project_path: &str) {
        self.stop_where(|k| k.project_path == project_path).await;
    }

    pub async fn stop_all(&self) {
        self.stop_where(|_| true).await;
    }
}

/// Runs every future to its end at once.
async fn futures_join<F: std::future::Future<Output = ()> + Send + 'static>(
    all: impl Iterator<Item = F>,
) {
    let handles: Vec<_> = all.map(tokio::spawn).collect();
    for handle in handles {
        let _ = handle.await;
    }
}

fn set(env: &mut Vec<(String, String)>, key: &str, value: String) {
    match env.iter_mut().find(|(k, _)| k == key) {
        Some(slot) => slot.1 = value,
        None => env.push((key.to_owned(), value)),
    }
}
