//! The extensions of 0.8, which add to a session's card: one child per
//! extension and project in the connector protocol, each with a token of its
//! own, kept by the shared [`Lifecycle`]: started when first needed, started
//! again on the next use after a crash, and stopped when its project has no
//! session left or it sits idle.
//!
//! A child's token is how the bridge knows who calls it: it is minted at
//! each start, handed to the child in its environment and to nobody else,
//! and forgotten when the child ends.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tracing::info;

use crate::child::{Child, Launch};
use crate::footer::{read_items, Item};
use crate::js;
use crate::lifecycle::{Lease, Lifecycle, Running, ServiceKey};
use crate::links::MAX_CLICKED_TEXT;
use crate::pack::{InstalledPack, PackStore};
use crate::token;

/// The protocols this host speaks.
pub const SUPPORTED_PROTOCOLS: [u64; 1] = [1];
const UNSUPPORTED_PROTOCOL: i64 = -32001;
const HELLO_TIMEOUT: Duration = Duration::from_secs(15);
const CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// What a pack that names no protocol is told.
pub fn outdated_message(name: &str) -> String {
    format!(
        "{name} was built for an older Vorn. Update it in Settings › Extensions, or rebuild it with @vornrun/connector-sdk 0.7.1-beta.3 or later."
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

impl Running for Host {
    fn child(&self) -> &Child {
        &self.child
    }
}

/// What every host is started with.
#[derive(Debug, Clone)]
pub struct HostSettings {
    /// The environment every child starts from.
    pub base_env: Vec<(String, String)>,
    /// Where the bridge is served, e.g. `http://127.0.0.1:50090`.
    pub bridge_origin: String,
    /// The version the hello names.
    pub version: String,
}

/// The services of this kind are scoped by project in the lifecycle.
fn service_key(key: &HostKey) -> ServiceKey {
    ServiceKey::new(key.extension_id.clone(), project_scope(&key.project_path))
}

fn project_scope(project: &str) -> String {
    format!("project:{project}")
}

/// The extension hosts, by extension and project, on the shared lifecycle.
pub struct Hosts {
    lifecycle: Arc<Lifecycle>,
    store: PackStore,
    settings: HostSettings,
}

impl std::fmt::Debug for Hosts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hosts")
            .field("root", &self.store.root())
            .finish_non_exhaustive()
    }
}

impl Hosts {
    pub fn new(lifecycle: Arc<Lifecycle>, store: PackStore, settings: HostSettings) -> Arc<Hosts> {
        Arc::new(Hosts {
            lifecycle,
            store,
            settings,
        })
    }

    pub fn store(&self) -> &PackStore {
        &self.store
    }

    /// The running host for `key`, started if it is not.
    pub async fn get_or_start(&self, key: &HostKey) -> Result<Lease<Host>, String> {
        self.lifecycle
            .get_or_start(&service_key(key), || self.start(key))
            .await
    }

    async fn start(&self, key: &HostKey) -> Result<Host, String> {
        let id = &key.extension_id;
        let pack = self
            .store
            .inspect_unmeasured(id)
            .map(Some)?
            .filter(InstalledPack::is_extension)
            .ok_or_else(|| format!("No extension \"{id}\" is installed"))?;
        let (program, args) = pack
            .service_command()
            .ok_or_else(|| format!("The extension \"{id}\" has no files to run"))?;
        if self.settings.bridge_origin.is_empty() {
            return Err("The extension bridge has no address yet".into());
        }
        let name = pack.name().to_owned();
        match pack.manifest.protocol {
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
            program,
            args,
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

    /// Every running host.
    pub fn running(&self) -> Vec<Arc<Host>> {
        self.lifecycle
            .running_as::<Host>()
            .into_iter()
            .map(|(_, host)| host)
            .collect()
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

    pub async fn stop_extension(&self, extension_id: &str) {
        self.lifecycle
            .stop_where(|k| k.extension == extension_id && k.scope.starts_with("project:"))
            .await;
    }

    pub async fn stop_project(&self, project_path: &str) {
        let scope = project_scope(project_path);
        self.lifecycle.stop_where(|k| k.scope == scope).await;
    }

    pub async fn stop_all(&self) {
        self.lifecycle
            .stop_where(|k| k.scope.starts_with("project:"))
            .await;
    }
}

fn set(env: &mut Vec<(String, String)>, key: &str, value: String) {
    match env.iter_mut().find(|(k, _)| k == key) {
        Some(slot) => slot.1 = value,
        None => env.push((key.to_owned(), value)),
    }
}
