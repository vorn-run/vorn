//! Connections and connectors: every `connection:*` and `connector:*` call,
//! `credentials:import` and `http:request`, and the polls connector-poll
//! workflows run.
//!
//! Connections are rows of `vorn.db`; their secrets are in the vault
//! ([`super::secrets`]), and a row holds [`IN_VAULT`] where a secret was. A
//! connection runs as a child: an MCP server spoken to through the MCP SDK
//! ([`super::mcp`]), or a package spoken to in the connector protocol
//! ([`vorn_connectors::sdk`]). The packs, the catalog and the HTTP connector
//! are [`vorn_connectors`]'s; this module wires them to the store, the vault,
//! the desktop ([`crate::bridge`]) and the clients.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use serde_json::{json, Map, Value};
use tracing::{info, warn};
use vorn_connectors::auth::{self, Runner};
use vorn_connectors::catalog::Catalog;
use vorn_connectors::child::Launch;
use vorn_connectors::connections::{self as conns, filter, Item, IN_VAULT, MCP, SDK};
use vorn_connectors::fetch::Http;
use vorn_connectors::install::{Installer, Source as PackSource};
use vorn_connectors::pack::PackStore;
use vorn_connectors::poll::{self, McpPoll, Page};
use vorn_connectors::sdk::{self, Children, SdkClient, SdkLaunch, Source as LaunchSource};
use vorn_store::Store;
use vorn_vault::Secret;

use super::secrets::{Fields, Known};
use super::{absolute_str, mcp, Answer, Native};
use crate::bridge::Bridge;

/// Every call this module answers.
pub const METHODS: &[&str] = &[
    "connection:list",
    "connection:create",
    "connection:update",
    "connection:delete",
    "connection:getSourceLink",
    "connection:listMcpTools",
    "connection:listActions",
    "connection:preflight",
    "connection:refreshMcpTools",
    "connection:executeAction",
    "connection:browserAuth",
    "connection:signedIn",
    "connection:signedOut",
    "connection:listKeys",
    "connection:rotateSecret",
    "connection:backfill",
    "connection:upsertFromItem",
    "connector:list",
    "connector:get",
    "connector:poll",
    "connector:probeSdk",
    "connector:catalog",
    "connector:catalogRefresh",
    "connector:inspectPack",
    "connector:installPack",
    "connector:removePack",
    "connector:rollbackPack",
    "connector:listPacks",
    "connector:seedWorkflow",
    "connector:status",
    "connector:probeAuth",
    "connector:detectRepo",
    "credentials:import",
    "http:request",
];

/// One call through a signed-in window.
const WINDOW_CALL_TIMEOUT: Duration = Duration::from_secs(20);
/// The requests of one tool call kept to explain it failing.
const KEPT_CALLS: usize = 50;
const MAX_WINDOW_REQUEST: usize = 1024 * 1024;
/// An MCP connection's tools are first looked for this long after it is made.
const FIRST_DISCOVERY: Duration = Duration::from_millis(1500);

/// A browser connector's child's grant to call through its connection's window.
#[derive(Debug)]
struct Grant {
    token: String,
    browser: Value,
    /// The window requests of each tool call in flight, by the call's key.
    calls: Mutex<HashMap<String, Vec<Value>>>,
}

/// What answers the connection and connector calls.
pub struct Connectors {
    native: Weak<Native>,
    installer: Installer,
    catalog: Catalog,
    children: Children,
    grants: Mutex<HashMap<String, Arc<Grant>>>,
    /// How a checkout signs in, read once by probing it; `None` when it would not start.
    checkout_auth: Mutex<HashMap<String, Option<auth::Source>>>,
    bridge: Arc<dyn Bridge>,
    http: Http,
    /// Where a browser connector's child reaches its window, vornd's own address.
    origin: String,
    version: String,
    repo_root: Option<PathBuf>,
}

impl std::fmt::Debug for Connectors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connectors")
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// A random token for a window grant or a call.
fn random_token(bytes: usize) -> String {
    let mut raw = vec![0u8; bytes];
    for chunk in raw.chunks_mut(16) {
        let id = uuid::Uuid::new_v4();
        chunk.copy_from_slice(&id.as_bytes()[..chunk.len()]);
    }
    raw.iter().map(|b| format!("{b:02x}")).collect()
}

fn failure(error: impl Into<String>) -> Value {
    json!({ "success": false, "error": error.into() })
}

/// The row as JavaScript would print it: whole numbers without `.0`.
fn from_store(mut value: Value) -> Value {
    fn walk(v: &mut Value) {
        match v {
            Value::Number(n) => {
                if let Some(f) = n.as_f64().filter(|_| n.is_f64()) {
                    if f.fract() == 0.0 && f.abs() < 9_007_199_254_740_992.0 {
                        *v = json!(f as i64);
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(walk),
            Value::Object(map) => map.values_mut().for_each(walk),
            _ => {}
        }
    }
    walk(&mut value);
    value
}

/// `discoveredTools`, the tools discovery stored on an MCP connection's row.
fn discovered_tools(conn: &Value) -> Vec<Value> {
    conn.pointer("/filters/discoveredTools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// The vault's fields as plain strings.
fn plain(fields: &Fields) -> HashMap<String, String> {
    fields
        .iter()
        .map(|(k, v)| (k.clone(), v.expose().to_owned()))
        .collect()
}

/// Takes the secret fields of `filters` into `secrets`, leaving the marker.
fn take_secrets(connector: &str, filters: &mut Map<String, Value>, secrets: &mut Fields) {
    for field in conns::password_fields(connector) {
        let Some(key) = field.get("key").and_then(Value::as_str) else {
            continue;
        };
        match filters.get(key).and_then(Value::as_str) {
            Some(v) if !v.is_empty() && v != IN_VAULT => {
                secrets.insert(key.to_owned(), Secret::from(v));
                filters.insert(key.to_owned(), json!(IN_VAULT));
            }
            _ => {}
        }
    }
}

/// How a child of vornd's runs a program, for the CLI sign-in probes.
struct ProcessRunner {
    env: Vec<(String, String)>,
    source: Vec<(String, String)>,
}

impl Runner for ProcessRunner {
    fn resolve(&self, name: &str) -> Option<PathBuf> {
        let path = self
            .env
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("PATH"))
            .map(|(_, v)| v.as_str())?;
        super::env::find_on_path(name, path)
    }

    fn run(
        &self,
        file: &Path,
        args: &[String],
        env: &[(String, String)],
        timeout: Duration,
    ) -> Result<(String, String), String> {
        let mut child = vorn_spawn::command(file)
            .args(args)
            .env_clear()
            .envs(env.iter().map(|(k, v)| (k, v)))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let started = std::time::Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let out = child.wait_with_output().map_err(|e| e.to_string())?;
                    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
                    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
                    return if status.success() {
                        Ok((stdout, stderr))
                    } else {
                        Err(format!(
                            "Command failed: {} {} ({status})",
                            file.display(),
                            args.join(" ")
                        ))
                    };
                }
                Ok(None) if started.elapsed() > timeout => {
                    let _ = child.kill();
                    return Err(format!("Command timed out: {}", file.display()));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(e) => return Err(e.to_string()),
            }
        }
    }

    fn var(&self, name: &str) -> Option<String> {
        self.source
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    }

    fn safe_env(&self) -> Vec<(String, String)> {
        self.env.clone()
    }
}

impl Connectors {
    pub fn new(
        native: &Arc<Native>,
        data_dir: &Path,
        origin: String,
        bridge: Arc<dyn Bridge>,
    ) -> Arc<Connectors> {
        let home = PathBuf::from(super::shell::home_dir());
        Arc::new(Connectors {
            native: Arc::downgrade(native),
            installer: Installer::new(PackStore::new(data_dir.join("connectors"))),
            catalog: Catalog::new(
                home.join(".vorn").join("connector-catalog.json"),
                std::env::var_os("VORN_CONNECTORS_ROOT").map(PathBuf::from),
            ),
            children: Children::default(),
            grants: Mutex::default(),
            checkout_auth: Mutex::default(),
            bridge,
            http: Http,
            origin,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            repo_root: std::env::var_os("VORN_CONNECTORS_ROOT").map(PathBuf::from),
        })
    }

    fn native(&self) -> Result<Arc<Native>, String> {
        self.native
            .upgrade()
            .ok_or_else(|| "vornd is stopping".to_owned())
    }

    /// Runs `f` on the store on a blocking thread.
    async fn store<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Store) -> vorn_store::Result<T> + Send + 'static,
    ) -> Result<T, String> {
        let native = self.native()?;
        tokio::task::spawn_blocking(move || {
            let mut store = native.store().ok_or("vornd has no database")?;
            f(&mut store).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())?
    }

    async fn connection(&self, id: &str) -> Result<Option<Value>, String> {
        let id = id.to_owned();
        let conn = self
            .store(move |s| s.call("dbGetSourceConnection", json!([id])))
            .await?;
        Ok((!conn.is_null()).then(|| from_store(conn)))
    }

    /// Tells every client and every process watching that the data changed.
    fn changed(&self) {
        if let Ok(native) = self.native() {
            native.signal_change();
        }
    }

    /// A connection's secrets, read off the runtime: the vault may block.
    async fn secrets_of(&self, id: &str) -> Option<Fields> {
        let native = self.native().ok()?;
        let id = id.to_owned();
        tokio::task::spawn_blocking(move || match native.secrets.lookup(&id) {
            Known::Fields(fields) => Some(fields),
            Known::None | Known::Unknown => None,
        })
        .await
        .ok()
        .flatten()
    }

    async fn secret(&self, id: &str, field: &str) -> Option<String> {
        self.secrets_of(id)
            .await?
            .get(field)
            .map(|s| s.expose().to_owned())
    }

    /// Answers `method`.
    pub async fn answer(self: &Arc<Self>, method: &str, params: Value) -> Answer {
        let result = match method {
            "connection:list" => self.list(&params).await,
            "connection:create" => self.create(&params).await,
            "connection:update" => self.update(&params).await,
            "connection:delete" => self.delete(&params).await,
            "connection:getSourceLink" => {
                let task = params.as_str().unwrap_or("").to_owned();
                self.store(move |s| s.call("dbGetTaskSourceLink", json!([task])))
                    .await
                    .map(|v| Some(from_store(v)))
            }
            "connection:listMcpTools" => self.list_mcp_tools(&params).await,
            "connection:listActions" => self.list_actions(&params).await,
            "connection:preflight" => self.preflight(&params).await,
            "connection:refreshMcpTools" => {
                Ok(Some(self.refresh(params.as_str().unwrap_or("")).await))
            }
            "connection:executeAction" => self.execute(&params).await.map(Some),
            "connection:browserAuth" => self.browser_auth(&params).await,
            "connection:signedIn" => self.signed_in(&params).await,
            "connection:signedOut" => self.signed_out(params.as_str().unwrap_or("")).await,
            "connection:listKeys" => self.list_keys().await,
            "connection:rotateSecret" => self.rotate(&params).await,
            "connection:backfill" => Ok(Some(self.backfill(&params).await)),
            "connection:upsertFromItem" => self.upsert_from_item(&params).await,
            "connector:list" => Ok(Some(Value::Array(conns::builtins()))),
            "connector:get" => Ok(Some(
                params
                    .as_str()
                    .and_then(conns::builtin)
                    .map(|mut c| {
                        if let Some(o) = c.as_object_mut() {
                            o.remove("addable");
                        }
                        c
                    })
                    .unwrap_or(Value::Null),
            )),
            "connector:poll" => {
                let id = params
                    .get("workflowId")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                Ok(Some(json!({ "pages": self.poll(&id).await })))
            }
            "connector:probeSdk" => Ok(Some(self.probe_sdk(&params).await)),
            "connector:catalog" => Ok(Some(self.catalog_snapshot())),
            "connector:catalogRefresh" => Ok(Some(self.catalog_refresh().await)),
            "connector:inspectPack" => Ok(Some(self.inspect_pack(&params).await)),
            "connector:installPack" => Ok(Some(self.install_pack(&params).await)),
            "connector:removePack" => {
                Ok(Some(self.remove_pack(params.as_str().unwrap_or("")).await))
            }
            "connector:rollbackPack" => Ok(Some(
                self.rollback_pack(params.as_str().unwrap_or("")).await,
            )),
            "connector:listPacks" => Ok(Some(json!(self.installer.store().list()))),
            "connector:seedWorkflow" => self.seed_workflow(&params).await,
            "connector:status" => Ok(Some(self.status().await)),
            "connector:probeAuth" => Ok(Some(self.probe_auth(params.as_str().unwrap_or("")).await)),
            "connector:detectRepo" => Ok(Some(self.detect_repo(&params))),
            "credentials:import" => self.import(&params).await,
            "http:request" => Ok(Some(self.http_request(&params).await)),
            _ => return Answer::Unanswered,
        };
        match result {
            Ok(Some(value)) => Answer::Result(value),
            Ok(None) => Answer::Void,
            Err(message) => Answer::Error(message),
        }
    }

    // ---- rows ----

    async fn list(&self, params: &Value) -> Result<Option<Value>, String> {
        let connector = params
            .get("connectorId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let mut list = self
            .store(move |s| s.call("dbListSourceConnections", json!([connector])))
            .await
            .map(from_store)?;
        if let Value::Array(rows) = &mut list {
            rows.retain(|c| text(c, "connectorId") != "webhook");
        }
        Ok(Some(list))
    }

    async fn create(self: &Arc<Self>, params: &Value) -> Result<Option<Value>, String> {
        let mut params = params
            .as_object()
            .cloned()
            .ok_or("connection:create needs a connection")?;
        let connector = params
            .get("connectorId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let mut filters = params
            .get("filters")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mut secrets = Fields::new();
        take_secrets(&connector, &mut filters, &mut secrets);
        params.insert("filters".into(), Value::Object(filters));
        let id = uuid();
        if !secrets.is_empty() {
            self.native()?.secrets.set(&id, secrets);
        }
        let new_id = id.clone();
        let conn = self
            .store(move |s| {
                conns::create(
                    s,
                    conns::NewConnection {
                        id: new_id,
                        params: &params,
                        now: now_iso(),
                    },
                )
            })
            .await?;
        self.changed();
        if connector == MCP {
            let me = Arc::clone(self);
            tokio::spawn(async move {
                tokio::time::sleep(FIRST_DISCOVERY).await;
                me.refresh(&id).await;
            });
        }
        Ok(Some(conn))
    }

    async fn update(&self, params: &Value) -> Result<Option<Value>, String> {
        let id = params
            .get("id")
            .and_then(Value::as_str)
            .ok_or("connection:update needs an id")?
            .to_owned();
        let mut updates = params
            .get("updates")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if let Some(Value::Object(mut filters)) = updates.remove("filters") {
            let conn = self.connection(&id).await?;
            let connector = conn
                .as_ref()
                .map(|c| text(c, "connectorId").to_owned())
                .unwrap_or_default();
            let mut secrets = self.secrets_of(&id).await.unwrap_or_default();
            let before = secrets.len();
            let had = secrets.clone();
            take_secrets(&connector, &mut filters, &mut secrets);
            if secrets.len() != before || secrets != had {
                self.native()?.secrets.set(&id, secrets);
            }
            updates.insert("filters".into(), Value::Object(filters));
        }
        let row = id.clone();
        self.store(move |s| conns::update(s, &row, updates, &[]))
            .await?;
        self.stop_children(&id).await;
        self.changed();
        let conn = self.connection(&id).await?;
        Ok(Some(conn.unwrap_or(Value::Null)))
    }

    async fn delete(&self, params: &Value) -> Result<Option<Value>, String> {
        let id = params
            .as_str()
            .ok_or("connection:delete needs an id")?
            .to_owned();
        if let Some(conn) = self.connection(&id).await? {
            if conns::is_implicit(&conn) {
                return Err(format!(
                    "{} came with its connector. Remove the pack instead.",
                    text(&conn, "name")
                ));
            }
        }
        self.remove_connection(&id).await?;
        Ok(None)
    }

    async fn remove_connection(&self, id: &str) -> Result<(), String> {
        let row = id.to_owned();
        self.store(move |s| conns::delete(s, &row)).await?;
        if let Ok(native) = self.native() {
            native.secrets.forget(id);
        }
        let bridge = Arc::clone(&self.bridge);
        let forget = id.to_owned();
        tokio::spawn(async move {
            let _ = bridge
                .request("session:forget", json!(forget), WINDOW_CALL_TIMEOUT)
                .await;
        });
        self.stop_children(id).await;
        self.changed();
        Ok(())
    }

    async fn stop_children(&self, id: &str) {
        self.children.stop(id).await;
        self.grants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        if let Ok(native) = self.native() {
            native.mcp.stop(id);
        }
    }

    async fn list_mcp_tools(&self, params: &Value) -> Result<Option<Value>, String> {
        let conn = self.connection(params.as_str().unwrap_or("")).await?;
        Ok(Some(match conn.filter(|c| text(c, "connectorId") == MCP) {
            Some(conn) => Value::Array(discovered_tools(&conn)),
            None => json!([]),
        }))
    }

    async fn list_actions(&self, params: &Value) -> Result<Option<Value>, String> {
        let Some(conn) = self.connection(params.as_str().unwrap_or("")).await? else {
            return Ok(Some(json!([])));
        };
        Ok(Some(match text(&conn, "connectorId") {
            MCP => Value::Array(
                discovered_tools(&conn)
                    .iter()
                    .filter_map(mcp::tool_action)
                    .collect(),
            ),
            SDK => {
                let manifest = self.sdk_manifest(&conn).await?;
                Value::Array(
                    manifest
                        .get("actions")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .map(conns::sdk_action_def)
                        .collect(),
                )
            }
            other => conns::builtin(other)
                .and_then(|c| c.pointer("/manifest/actions").cloned())
                .unwrap_or_else(|| json!([])),
        }))
    }

    async fn preflight(&self, params: &Value) -> Result<Option<Value>, String> {
        let id = params.as_str().unwrap_or("").to_owned();
        let conn = self
            .connection(&id)
            .await?
            .ok_or_else(|| format!("connection {id} not found"))?;
        Ok(Some(match text(&conn, "connectorId") {
            conns::HTTP => {
                let filters = conns::filters_of(&conn);
                let secret = self.secret(&id, "secret").await;
                if let Some(locked) =
                    vorn_connectors::http::locked_error(&filters, secret.is_some())
                {
                    json!({ "ok": false, "message": locked })
                } else {
                    let profile = vorn_connectors::http::Profile::of(&filters, secret.as_deref());
                    let http = self.http.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        vorn_connectors::http::execute(&http, "test", &profile, &Map::new())
                    })
                    .await
                    .unwrap_or_else(|e| failure(e.to_string()));
                    if result["success"] != true {
                        json!({ "ok": false, "message": result.get("error") })
                    } else {
                        let status = result
                            .pointer("/output/status")
                            .and_then(Value::as_u64)
                            .unwrap_or(500);
                        json!({ "ok": status < 400, "message": format!("HTTP {status}") })
                    }
                }
            }
            SDK => match self.sdk_client(&conn).await {
                Ok(client) => match client.preflight().await {
                    Ok(answer) => answer,
                    Err(err) => json!({ "ok": false, "message": err.message }),
                },
                Err(err) => json!({ "ok": false, "message": err }),
            },
            _ => json!({ "ok": null }),
        }))
    }

    // ---- MCP ----

    /// The MCP child of `conn`, started or reused.
    async fn mcp_peer(
        &self,
        conn: &Value,
    ) -> Result<rmcp::service::Peer<rmcp::service::RoleClient>, String> {
        let native = self.native()?;
        let launch = self.spawn_spec(conn).await?;
        let launch = mcp::Launch {
            command: launch.launch.program.to_string_lossy().into_owned(),
            args: launch.launch.args.clone(),
            env: launch.own_env,
        };
        native
            .mcp
            .client(text(conn, "id"), &launch, native.child_env())
            .await
    }

    async fn invoke_mcp(&self, conn: &Value, tool: &str, args: &Map<String, Value>) -> Value {
        match self.mcp_peer(conn).await {
            Ok(peer) => mcp::invoke(&peer, tool, args, &discovered_tools(conn)).await,
            Err(error) => failure(error),
        }
    }

    /// `runMcpDiscovery`: lists an MCP connection's tools onto its row.
    async fn refresh(&self, id: &str) -> Value {
        let conn = match self.connection(id).await {
            Ok(Some(conn)) if text(&conn, "connectorId") == MCP => conn,
            _ => return json!({ "ok": false, "error": "Not an MCP connection" }),
        };
        let discovered = match self.mcp_peer(&conn).await {
            Ok(peer) => mcp::discover(&peer).await,
            Err(error) => Err(error),
        };
        let row = id.to_owned();
        let mut filters = conns::filters_of(&conn);
        let answer = match discovered {
            Ok(tools) => {
                let count = tools.len();
                filters.insert("discoveredTools".into(), Value::Array(tools));
                let mut updates = Map::new();
                updates.insert("filters".into(), Value::Object(filters));
                updates.insert("lastSyncAt".into(), json!(now_iso()));
                let written = self
                    .store(move |s| conns::update(s, &row, updates, &["lastSyncError"]))
                    .await;
                match written {
                    Ok(()) => json!({ "ok": true, "count": count }),
                    Err(error) => json!({ "ok": false, "error": error }),
                }
            }
            Err(error) => {
                let mut updates = Map::new();
                updates.insert("lastSyncError".into(), json!(error));
                let _ = self
                    .store(move |s| conns::update(s, &row, updates, &[]))
                    .await;
                json!({ "ok": false, "error": error })
            }
        };
        self.changed();
        answer
    }

    // ---- launching ----

    /// How `sdk_id` signs in: its checkout's manifest, probed once, else its
    /// installed pack's (`resolveConnectorAuth`).
    async fn connector_auth(&self, sdk_id: &str) -> Option<auth::Source> {
        if sdk_id.is_empty() {
            return None;
        }
        let local = vorn_connectors::catalog::local_launch_spec(sdk_id, self.repo_root.as_deref());
        let Some((command, args)) = local else {
            let pack = self.installer.store().describe(sdk_id)?;
            return Some(auth::Source {
                auth: pack
                    .auth
                    .as_ref()
                    .and_then(|a| serde_json::to_value(a).ok()),
                declared: pack.env.iter().map(|e| e.name.clone()).collect(),
                trusted: false,
            });
        };
        if let Some(cached) = self
            .checkout_auth
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(sdk_id)
        {
            return cached.clone();
        }
        let native = self.native().ok()?;
        let program = super::mcp::resolve(&command, &native.child_env());
        let probed = sdk::probe(
            &program.to_string_lossy(),
            &args,
            native.child_env(),
            std::env::temp_dir(),
            &self.version,
            sdk::Timeouts::default(),
        )
        .await;
        let source = (probed["ok"] == true).then(|| auth::Source {
            auth: probed.pointer("/manifest/auth").cloned(),
            declared: probed
                .pointer("/manifest/env")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|e| e.get("name").and_then(Value::as_str).map(str::to_owned))
                .collect(),
            trusted: false,
        });
        if source.is_none() {
            warn!(
                "[auth] could not read {sdk_id} from its checkout: {}",
                probed["error"]
            );
        }
        self.checkout_auth
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(sdk_id.to_owned(), source.clone());
        source
    }

    fn runner(&self) -> Result<ProcessRunner, String> {
        let native = self.native()?;
        Ok(ProcessRunner {
            env: native.child_env(),
            source: std::env::vars().collect(),
        })
    }

    /// Everything a connection's child starts with (`buildSpawnConfig`).
    async fn spawn_spec(&self, conn: &Value) -> Result<Spawn, String> {
        let native = self.native()?;
        let filters = conns::filters_of(conn);
        let sdk_id = conns::sdk_id_of(conn);
        let mut source = LaunchSource::Command;
        let mut protocol = None;
        let mut name = String::new();
        let mut program_args: Option<(String, Vec<String>)> = None;
        if !sdk_id.is_empty() {
            if let Some(local) =
                vorn_connectors::catalog::local_launch_spec(&sdk_id, self.repo_root.as_deref())
            {
                source = LaunchSource::Checkout;
                program_args = Some(local);
            } else if let Some(pack) = self.installer.store().describe(&sdk_id) {
                source = LaunchSource::Pack;
                protocol = pack.protocol;
                name = pack.name.clone();
                program_args = Some((
                    "node".into(),
                    vec![pack.entry().to_string_lossy().into_owned()],
                ));
            }
        }
        let (command, args) = match program_args {
            Some(found) => found,
            None => {
                let command = filters.get("command").map_or(String::new(), |c| {
                    vorn_connectors::js::trim(&vorn_connectors::js::to_string(c)).to_owned()
                });
                if command.is_empty() {
                    return Err("MCP connection is missing a command".into());
                }
                let args = match filters
                    .get("args")
                    .and_then(Value::as_str)
                    .and_then(|a| serde_json::from_str::<Value>(a).ok())
                {
                    Some(Value::Array(items)) => {
                        items.iter().map(vorn_connectors::js::to_string).collect()
                    }
                    _ => Vec::new(),
                };
                (command, args)
            }
        };
        let mut own: Vec<(String, String)> = Vec::new();
        let mut put = |k: String, v: String| match own.iter_mut().find(|(key, _)| *key == k) {
            Some(slot) => slot.1 = v,
            None => own.push((k, v)),
        };
        let auth = self.connector_auth(&sdk_id).await;
        if let Some(auth) = &auth {
            let runner = self.runner()?;
            let source = auth.clone();
            let borrowed =
                tokio::task::spawn_blocking(move || auth::borrowed_secrets(&source, &runner))
                    .await
                    .unwrap_or_default();
            for (k, v) in borrowed {
                put(k, v);
            }
        }
        for (k, v) in mcp::parse_json_object(filters.get("env")) {
            put(k, v.as_str().unwrap_or("").to_owned());
        }
        if let Some(secrets) = self.secrets_of(text(conn, "id")).await {
            for (k, v) in super::secrets::secret_env(&secrets) {
                put(k, v.as_str().unwrap_or("").to_owned());
            }
        }
        let browser = auth
            .as_ref()
            .and_then(|a| a.auth.as_ref())
            .filter(|a| a.get("rung").and_then(Value::as_str) == Some("browser"))
            .and_then(|a| a.get("browser").cloned());
        let mut env = native.child_env();
        for (k, v) in &own {
            env.retain(|(key, _)| key != k);
            env.push((k.clone(), v.clone()));
        }
        let program = super::mcp::resolve(&command, &env);
        Ok(Spawn {
            launch: Launch {
                program,
                args,
                cwd: std::env::temp_dir(),
                env,
            },
            own_env: own,
            source,
            protocol,
            name,
            browser,
        })
    }

    /// A package connection's child, started on first use.
    async fn sdk_client(&self, conn: &Value) -> Result<Arc<SdkClient>, String> {
        let id = text(conn, "id").to_owned();
        let conn = conn.clone();
        self.children
            .get_or_start(&id, || async {
                let mut spawn = self.spawn_spec(&conn).await?;
                if let Some(browser) = spawn.browser.clone() {
                    let token = random_token(32);
                    spawn.launch.env.push((
                        "VORN_BROWSER_HOST".into(),
                        format!("{}/connections/{id}/browser", self.origin),
                    ));
                    spawn
                        .launch
                        .env
                        .push(("VORN_BROWSER_TOKEN".into(), token.clone()));
                    self.grants
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(
                            id.clone(),
                            Arc::new(Grant {
                                token,
                                browser,
                                calls: Mutex::default(),
                            }),
                        );
                }
                let key = if spawn.name.is_empty() {
                    text(&conn, "name").to_owned()
                } else {
                    spawn.name.clone()
                };
                let launch = SdkLaunch {
                    launch: spawn.launch,
                    source: spawn.source,
                    protocol: spawn.protocol,
                };
                sdk::open(&launch, &key, &self.version, sdk::Timeouts::default())
                    .await
                    .map_err(|e| e.to_string())
            })
            .await
    }

    /// What a package connection declares: its installed pack, or what its child says.
    async fn sdk_manifest(&self, conn: &Value) -> Result<Value, String> {
        let sdk_id = conns::sdk_id_of(conn);
        let local = vorn_connectors::catalog::local_launch_spec(&sdk_id, self.repo_root.as_deref());
        if !sdk_id.is_empty() && local.is_none() {
            if let Some(pack) = self.installer.store().describe(&sdk_id) {
                return Ok(json!(pack));
            }
        }
        let client = self.sdk_client(conn).await?;
        client
            .manifest()
            .await
            .map(|m| json!(m))
            .map_err(|e| e.message)
    }

    // ---- actions ----

    async fn execute(&self, params: &Value) -> Result<Value, String> {
        let id = params
            .get("connectionId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let action = params
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let args = params
            .get("args")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let Some(conn) = self.connection(&id).await? else {
            return Ok(failure(format!("Connection {id} not found")));
        };
        Ok(self.run_action(&conn, &action, &args).await)
    }

    async fn run_action(&self, conn: &Value, action: &str, args: &Map<String, Value>) -> Value {
        let id = text(conn, "id");
        match text(conn, "connectorId") {
            MCP => self.invoke_mcp(conn, action, args).await,
            SDK => {
                let mut params = args.clone();
                params.retain(|_, v| !v.is_null() || true);
                self.through_window(conn, |client, call| {
                    let action = action.to_owned();
                    let params = params.clone();
                    async move {
                        client
                            .action(&action, &params, call.as_deref())
                            .await
                            .map(Value::Object)
                    }
                })
                .await
                .map_or_else(
                    |failed| failed,
                    |(output, calls)| {
                        let mut result = json!({ "success": true, "output": output });
                        if !calls.is_empty() {
                            result["sessionCalls"] = Value::Array(calls);
                        }
                        result
                    },
                )
            }
            conns::HTTP => {
                let filters = conns::filters_of(conn);
                let secret = self.secret(id, "secret").await;
                if let Some(locked) =
                    vorn_connectors::http::locked_error(&filters, secret.is_some())
                {
                    return failure(locked);
                }
                let mut merged = filters.clone();
                if let Some(secret) = &secret {
                    merged.insert("secret".into(), json!(secret));
                }
                merged.extend(args.clone());
                let profile = vorn_connectors::http::Profile::of(&merged, secret.as_deref());
                let (http, action) = (self.http.clone(), action.to_owned());
                tokio::task::spawn_blocking(move || {
                    vorn_connectors::http::execute(&http, &action, &profile, &merged)
                })
                .await
                .unwrap_or_else(|e| failure(e.to_string()))
            }
            other => failure(format!("Connector {other} does not support actions")),
        }
    }

    /// Runs a call on a package connection's child, through its signed-in
    /// window when it has one; a failure reads the way an action's does.
    async fn through_window<F, Fut>(
        &self,
        conn: &Value,
        run: F,
    ) -> Result<(Value, Vec<Value>), Value>
    where
        F: FnOnce(Arc<SdkClient>, Option<String>) -> Fut,
        Fut: std::future::Future<Output = Result<Value, sdk::SdkError>>,
    {
        let id = text(conn, "id").to_owned();
        let client = match self.sdk_client(conn).await {
            Ok(client) => client,
            Err(error) => return Err(failure(error)),
        };
        let grant = self
            .grants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .cloned();
        let key = grant.as_ref().map(|g| {
            let key = random_token(12);
            g.calls
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(key.clone(), Vec::new());
            key
        });
        let answered = run(client, key.clone()).await;
        let calls = match (&grant, &key) {
            (Some(g), Some(k)) => g
                .calls
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(k)
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        match answered {
            Ok(value) => Ok((value, calls)),
            Err(err) => {
                let mut result = failure(err.message.clone());
                match err.data.kind.as_deref() {
                    Some("signed-out") => result["errorKind"] = json!("needs-sign-in"),
                    Some("app-offline") => result["errorKind"] = json!("app-offline"),
                    _ => {}
                }
                if !calls.is_empty() {
                    result["sessionCalls"] = Value::Array(calls.clone());
                }
                match grant {
                    Some(g) => Err(self.window_outcome(conn, &g, calls, result).await),
                    None => Err(result),
                }
            }
        }
    }

    /// A failed call through a window, told apart: Vorn closed, or the site signed it out.
    async fn window_outcome(
        &self,
        conn: &Value,
        grant: &Grant,
        calls: Vec<Value>,
        mut result: Value,
    ) -> Value {
        let name = text(conn, "name");
        if calls.iter().any(|c| c["status"] == "app-offline") {
            result["errorKind"] = json!("app-offline");
            result["error"] = json!(format!(
                "Open Vorn on the desktop {name} signed in on, then run this step again."
            ));
            return result;
        }
        let refused = calls
            .iter()
            .any(|c| c["status"] == 401 || c["status"] == 403);
        if refused && self.still_signed_in(text(conn, "id"), &grant.browser).await == Some(false) {
            self.mark_signed_out(text(conn, "id")).await;
            result["errorKind"] = json!("needs-sign-in");
            result["error"] = json!(format!(
                "{name} was signed out. Sign in again, and this step runs again."
            ));
        }
        result
    }

    async fn still_signed_in(&self, id: &str, browser: &Value) -> Option<bool> {
        if !self.bridge.connected() {
            return None;
        }
        let answer = self
            .bridge
            .request(
                "session:check",
                json!({ "connectionId": id, "browser": browser }),
                WINDOW_CALL_TIMEOUT,
            )
            .await
            .ok()?;
        answer.get("signedIn").and_then(Value::as_bool)
    }

    /// `POST /connections/<id>/browser/fetch`: a browser connector's child
    /// calling through its window. The status and body to answer with.
    pub async fn window_fetch(
        &self,
        id: &str,
        bearer: Option<&str>,
        call_key: Option<&str>,
        body: &[u8],
    ) -> (u16, Value) {
        let refuse = |status: u16, message: &str| (status, json!({ "error": message }));
        let grant = self
            .grants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned();
        let known = grant.as_ref().zip(bearer).is_some_and(|(g, t)| {
            vorn_reach::token::constant_time_eq(t.as_bytes(), g.token.as_bytes())
        });
        let Some(grant) = grant.filter(|_| known) else {
            return refuse(401, "This endpoint does not know that caller");
        };
        if body.len() > MAX_WINDOW_REQUEST {
            return refuse(413, "That request is too large");
        }
        let Some(request) = read_window_request(body) else {
            return refuse(400, "Send { url, method, headers?, body? }");
        };
        let method = text(&request, "method").to_owned();
        if !matches!(
            method.as_str(),
            "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE"
        ) {
            return refuse(
                405,
                &format!("{method} is not a method a signed-in call may use"),
            );
        }
        let url = text(&request, "url").to_owned();
        let origins: Vec<String> = grant
            .browser
            .get("origins")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        if !vorn_connectors::manifest::within_origins(&origins, &url) {
            return refuse(
                403,
                &format!("{url} is not on one of this connection's origins"),
            );
        }
        let path = url::Url::parse(&url)
            .map(|u| u.path().to_owned())
            .unwrap_or_default();
        let record = |status: Value| {
            if let Some(key) = call_key {
                if let Some(calls) = grant
                    .calls
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get_mut(key)
                {
                    calls.push(json!({ "method": method, "path": path, "status": status }));
                    if calls.len() > KEPT_CALLS {
                        let excess = calls.len() - KEPT_CALLS;
                        calls.drain(..excess);
                    }
                }
            }
        };
        if !self.bridge.connected() {
            record(json!("app-offline"));
            return refuse(503, "Open Vorn on the desktop this connection signed in on");
        }
        let asked = self
            .bridge
            .request(
                "session:fetch",
                json!({ "connectionId": id, "origins": origins, "request": request }),
                WINDOW_CALL_TIMEOUT,
            )
            .await;
        match asked {
            Ok(answer) => {
                record(answer.get("status").cloned().unwrap_or(Value::Null));
                (200, answer)
            }
            Err(error) => {
                record(json!("failed"));
                refuse(503, &error)
            }
        }
    }

    async fn browser_auth(&self, params: &Value) -> Result<Option<Value>, String> {
        let Some(conn) = self.connection(params.as_str().unwrap_or("")).await? else {
            return Ok(Some(Value::Null));
        };
        let sdk_id = conns::sdk_id_of(&conn);
        let auth = self.connector_auth(&sdk_id).await.and_then(|s| s.auth);
        Ok(Some(match auth {
            Some(a)
                if a.get("rung").and_then(Value::as_str) == Some("browser")
                    && a.get("browser").is_some_and(|b| !b.is_null()) =>
            {
                json!({ "name": conn.get("name"), "browser": a.get("browser") })
            }
            _ => Value::Null,
        }))
    }

    async fn signed_in(&self, params: &Value) -> Result<Option<Value>, String> {
        let id = params
            .get("connectionId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let identity = params.get("identity").cloned().unwrap_or(Value::Null);
        let row = id.clone();
        self.store(move |s| s.call("dbSetConnectionSignIn", json!([row, identity, now_iso()])))
            .await?;
        self.changed();
        if let Some(work) = self.native()?.work().cloned() {
            tokio::spawn(async move { work.signed_in(&id).await });
        }
        Ok(None)
    }

    async fn signed_out(&self, id: &str) -> Result<Option<Value>, String> {
        self.mark_signed_out(id).await;
        Ok(None)
    }

    async fn mark_signed_out(&self, id: &str) {
        let row = id.to_owned();
        let _ = self
            .store(move |s| s.call("dbSetConnectionSignIn", json!([row, null, null])))
            .await;
        self.changed();
    }

    // ---- keys ----

    async fn list_keys(&self) -> Result<Option<Value>, String> {
        let (conns_list, workflows) = self
            .store(|s| {
                Ok((
                    s.call("dbListSourceConnections", json!([null]))?,
                    s.call("dbListWorkflows", json!([]))?,
                ))
            })
            .await?;
        let rows: Vec<Value> = conns_list
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| text(c, "connectorId") != "webhook")
            .cloned()
            .collect();
        let workflows = workflows.as_array().cloned().unwrap_or_default();
        let mut held: HashMap<String, HashMap<String, String>> = HashMap::new();
        for row in &rows {
            if let Some(fields) = self.secrets_of(text(row, "id")).await {
                held.insert(text(row, "id").to_owned(), plain(&fields));
            }
        }
        let keys = conns::list_keys(&rows, conns::auth_fields, &workflows, |id| {
            held.get(id).cloned()
        });
        Ok(Some(Value::Array(keys)))
    }

    async fn rotate(&self, params: &Value) -> Result<Option<Value>, String> {
        let id = params
            .get("connectionId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let field = params
            .get("field")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let plaintext = params
            .get("plaintext")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let Some(conn) = self.connection(&id).await? else {
            return Ok(Some(
                json!({ "ok": false, "error": format!("connection {id} not found") }),
            ));
        };
        let secret = conns::password_fields(text(&conn, "connectorId"))
            .iter()
            .any(|f| f.get("key").and_then(Value::as_str) == Some(field.as_str()));
        if !secret {
            return Ok(Some(
                json!({ "ok": false, "error": format!("{field} is not a secret on this connection") }),
            ));
        }
        if plaintext.trim().is_empty() {
            return Ok(Some(
                json!({ "ok": false, "error": "A replacement value is required" }),
            ));
        }
        let mut filters = conns::filters_of(&conn);
        filters.insert(field.clone(), json!(IN_VAULT));
        let mut updates = Map::new();
        updates.insert("filters".into(), Value::Object(filters));
        let row = id.clone();
        self.store(move |s| conns::update(s, &row, updates, &[]))
            .await?;
        let native = self.native()?;
        let (row, key) = (id.clone(), field.clone());
        tokio::task::spawn_blocking(move || native.secrets.merge(&row, &key, &plaintext))
            .await
            .map_err(|e| e.to_string())?;
        self.stop_children(&id).await;
        self.changed();
        Ok(Some(json!({ "ok": true })))
    }

    /// `credentials:import`: what the desktop decrypted of the secrets it
    /// sealed, filed in the vault once, each row then holding the marker.
    async fn import(&self, params: &Value) -> Result<Option<Value>, String> {
        let native = self.native()?;
        let mut imported = 0u64;
        for (id, fields) in params
            .get("connections")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let Some(fields) = fields.as_object() else {
                continue;
            };
            let Some(conn) = self.connection(id).await? else {
                continue;
            };
            let mut secrets = self.secrets_of(id).await.unwrap_or_default();
            let mut filters = conns::filters_of(&conn);
            for (key, value) in fields {
                let Some(value) = value.as_str().filter(|v| !v.is_empty()) else {
                    continue;
                };
                secrets.insert(key.clone(), Secret::from(value));
                filters.insert(key.clone(), json!(IN_VAULT));
            }
            native.secrets.set(id, secrets);
            let mut updates = Map::new();
            updates.insert("filters".into(), Value::Object(filters));
            let row = id.clone();
            self.store(move |s| conns::update(s, &row, updates, &[]))
                .await?;
            imported += 1;
        }
        if imported > 0 {
            info!(imported, "secrets the desktop sealed are in the vault");
            self.changed();
        }
        let (n, p) = (Arc::clone(&native), params.clone());
        let ssh = tokio::task::spawn_blocking(move || {
            super::config::with_store(&n, |s| Ok(super::credential::import(&n, s, &p)))?
        })
        .await
        .map_err(|e| e.to_string())?;
        let (ssh_keys, host_passwords) = ssh?;
        if host_passwords > 0 {
            super::config::announce(&native).await;
        }
        Ok(Some(json!({
            "connections": imported,
            "sshKeys": ssh_keys,
            "hostPasswords": host_passwords,
        })))
    }

    // ---- items ----

    async fn upsert_from_item(&self, params: &Value) -> Result<Option<Value>, String> {
        let id = params
            .get("connectionId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let conn = self
            .connection(&id)
            .await?
            .ok_or_else(|| format!("connection {id} not found"))?;
        let item = params.get("item").cloned().unwrap_or(Value::Null);
        let now = now_iso();
        let s = |v: Option<&Value>| v.and_then(Value::as_str).unwrap_or("").to_owned();
        let raw = item.get("raw");
        let item_fields = Item {
            external_id: s(item.get("externalId")),
            title: s(item.get("title")),
            description: s(item.get("body")),
            external_url: s(item.get("externalUrl")),
            status_raw: s(raw.and_then(|r| r.get("status"))),
            updated_at: raw
                .and_then(|r| r.get("updatedAt"))
                .and_then(Value::as_str)
                .map_or_else(|| now.clone(), str::to_owned),
        };
        let project = params
            .get("project")
            .filter(|p| vorn_connectors::js::truthy(p))
            .or_else(|| {
                conn.get("executionProject")
                    .filter(|p| vorn_connectors::js::truthy(p))
            })
            .or_else(|| conn.get("name"))
            .map(vorn_connectors::js::to_string)
            .unwrap_or_default();
        let status = params.get("initialStatus").cloned().unwrap_or(Value::Null);
        let result = self
            .store(move |st| {
                let r = conns::upsert_item(st, &conn, &item_fields, &project, &status, &now, uuid)?;
                let mut updates = Map::new();
                updates.insert("lastSyncAt".into(), json!(now));
                conns::update(st, &id, updates, &[])?;
                Ok(r)
            })
            .await?;
        self.changed();
        Ok(Some(result))
    }

    /// `connection:backfill`: everything a connection's trigger returns, from the start, on the board.
    async fn backfill(&self, params: &Value) -> Value {
        let id = params
            .get("connectionId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let Ok(Some(conn)) = self.connection(&id).await else {
            return json!({ "imported": 0, "updated": 0, "error": "Connection not found" });
        };
        let mut items: Vec<Item> = Vec::new();
        let drained = self.drain_all(&conn, &mut items).await;
        let now = now_iso();
        let project = conn
            .get("executionProject")
            .filter(|p| vorn_connectors::js::truthy(p))
            .or_else(|| conn.get("name"))
            .map(vorn_connectors::js::to_string)
            .unwrap_or_default();
        let mapping = conn.get("statusMapping").cloned().unwrap_or(Value::Null);
        let c = conn.clone();
        let written = self
            .store(move |s| {
                let (mut imported, mut updated) = (0u64, 0u64);
                for item in &items {
                    let status = mapping
                        .get(&item.status_raw)
                        .filter(|v| vorn_connectors::js::truthy(v))
                        .cloned()
                        .unwrap_or_else(|| json!("todo"));
                    let r = conns::upsert_item(s, &c, item, &project, &status, &now, uuid)?;
                    if r["created"] == true {
                        imported += 1;
                    } else {
                        updated += 1;
                    }
                }
                Ok((imported, updated, now))
            })
            .await;
        let (imported, updated, now) = match written {
            Ok(counts) => counts,
            Err(error) => return json!({ "imported": 0, "updated": 0, "error": error }),
        };
        let row = id.clone();
        let answer = match drained {
            Ok(()) => {
                let mut updates = Map::new();
                updates.insert("lastSyncAt".into(), json!(now));
                let _ = self
                    .store(move |s| conns::update(s, &row, updates, &["lastSyncError"]))
                    .await;
                json!({ "imported": imported, "updated": updated })
            }
            Err(error) => {
                let mut updates = Map::new();
                updates.insert("lastSyncError".into(), json!(error));
                let _ = self
                    .store(move |s| conns::update(s, &row, updates, &[]))
                    .await;
                json!({ "imported": imported, "updated": updated, "error": error })
            }
        };
        self.changed();
        answer
    }

    async fn drain_all(&self, conn: &Value, items: &mut Vec<Item>) -> Result<(), String> {
        let name = text(conn, "name");
        match text(conn, "connectorId") {
            MCP => {
                let filters = conns::filters_of(conn);
                let cfg = McpPoll::of(&filters);
                if cfg.tool.is_none() {
                    return Err(format!("Connection \"{name}\" has no poll tool configured, so there is nothing to import."));
                }
                let mut from_start = conn.clone();
                from_start["filters"] = Value::Object(cfg.without_seed_cursor(&filters));
                let mut cursor: Option<String> = None;
                for _ in 0..poll::MAX_BACKFILL_PAGES {
                    let using = if cursor.is_none() { &from_start } else { conn };
                    let page = self.mcp_page(using, cursor.as_deref()).await?;
                    items.extend(page.events.iter().map(poll::event_item));
                    if cfg.cursor_arg.is_none() {
                        return Ok(());
                    }
                    match page.next_cursor {
                        Some(next) if !page.events.is_empty() && Some(&next) != cursor.as_ref() => {
                            cursor = Some(next)
                        }
                        _ => return Ok(()),
                    }
                }
                Err(format!(
                    "Connection \"{name}\" exceeded {} backfill pages",
                    poll::MAX_BACKFILL_PAGES
                ))
            }
            SDK => {
                let trigger = conns::sdk_trigger_of(conn);
                if trigger.is_empty() {
                    return Err(format!(
                        "Connection \"{name}\" has no trigger, so there is nothing to import."
                    ));
                }
                let who = format!("Connection \"{name}\"");
                let mut cursor: Option<String> = None;
                for _ in 0..poll::MAX_BACKFILL_PAGES {
                    let page = self.sdk_raw_page(conn, &trigger, cursor.as_deref()).await?;
                    let now = now_iso();
                    items.extend(
                        page.get("items")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .map(|i| poll::sdk_item(i, &now)),
                    );
                    if page.get("hasMore") != Some(&Value::Bool(true)) {
                        return Ok(());
                    }
                    let next = page
                        .get("nextCursor")
                        .and_then(Value::as_str)
                        .filter(|n| !n.is_empty())
                        .map(str::to_owned);
                    if next.is_none() || next == cursor {
                        return Err(format!("{who} did not advance its backfill cursor"));
                    }
                    cursor = next;
                }
                Err(format!(
                    "{who} exceeded {} backfill pages",
                    poll::MAX_BACKFILL_PAGES
                ))
            }
            other => Err(format!("Connector {other} does not support listItems()")),
        }
    }

    async fn mcp_page(&self, conn: &Value, cursor: Option<&str>) -> Result<Page, String> {
        let cfg = McpPoll::of(&conns::filters_of(conn));
        let Some(tool) = cfg.tool.clone() else {
            return Ok(Page::default());
        };
        let args = cfg.arguments(cursor)?;
        let result = self.invoke_mcp(conn, &tool, &args).await;
        if result["success"] != true {
            let error = result
                .get("error")
                .and_then(Value::as_str)
                .filter(|e| !e.is_empty())
                .map(str::to_owned);
            return Err(error.unwrap_or_else(|| format!("MCP poll tool {tool} failed")));
        }
        cfg.page(
            cursor,
            result.get("output").unwrap_or(&Value::Null),
            &now_iso(),
        )
    }

    async fn sdk_raw_page(
        &self,
        conn: &Value,
        trigger: &str,
        cursor: Option<&str>,
    ) -> Result<Map<String, Value>, String> {
        let answered = self
            .through_window(conn, |client, call| {
                let (trigger, cursor) = (trigger.to_owned(), cursor.map(str::to_owned));
                async move {
                    client
                        .poll(&trigger, cursor.as_deref(), call.as_deref())
                        .await
                        .map(Value::Object)
                }
            })
            .await;
        match answered {
            Ok((Value::Object(page), _)) => Ok(page),
            Ok(_) => Err(format!("Polling {} failed", text(conn, "name"))),
            Err(failed) => Err(failed.get("error").and_then(Value::as_str).map_or_else(
                || format!("Polling {} failed", text(conn, "name")),
                str::to_owned,
            )),
        }
    }

    // ---- polls ----

    /// `connector:poll`: a connector-poll workflow's new items into its inbox,
    /// page by page, each page and its cursor kept together. The pages read.
    pub async fn poll(&self, workflow_id: &str) -> u64 {
        let wf = workflow_id.to_owned();
        let Ok(workflow) = self
            .store(move |s| s.call("dbGetWorkflow", json!([wf])))
            .await
        else {
            return 0;
        };
        let trigger = workflow
            .get("nodes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|n| n.get("type").and_then(Value::as_str) == Some("trigger"))
            .and_then(|n| n.get("config"))
            .filter(|c| c.get("triggerType").and_then(Value::as_str) == Some("connectorPoll"))
            .cloned();
        let Some(trigger) = trigger else {
            return 0;
        };
        let connection_id = text(&trigger, "connectionId").to_owned();
        let event = text(&trigger, "event").to_owned();
        let Ok(Some(conn)) = self.connection(&connection_id).await else {
            warn!("[scheduler] connectorPoll: connection {connection_id} not found — skipping");
            return 0;
        };
        let connector = text(&conn, "connectorId").to_owned();
        let sdk_trigger = match connector.as_str() {
            MCP if event != conns::POLL_EVENT => {
                warn!("[scheduler] connectorPoll: connection {connection_id} got unexpected event \"{event}\" — skipping");
                return 0;
            }
            MCP => None,
            SDK => {
                let t = if event == conns::POLL_EVENT {
                    conns::sdk_trigger_of(&conn)
                } else {
                    event.clone()
                };
                if t.is_empty() {
                    warn!("[scheduler] connectorPoll: connection {connection_id} has no trigger to poll — skipping");
                    return 0;
                }
                Some(t)
            }
            _ => {
                warn!("[scheduler] connectorPoll: connection {connection_id} has no poll() — skipping");
                return 0;
            }
        };
        let (wf, cid) = (workflow_id.to_owned(), connection_id.clone());
        let mut cursor: Option<String> = self
            .store(move |s| s.call("dbGetConnectorPollCursor", json!([wf, cid])))
            .await
            .ok()
            .and_then(|c| c.as_str().map(str::to_owned));
        let now = now_iso();
        let mut pages = 0u64;
        let failed: Option<String> = 'pages: {
            for _ in 0..poll::MAX_PAGES_PER_POLL {
                let page = match &sdk_trigger {
                    Some(t) => self
                        .sdk_raw_page(&conn, t, cursor.as_deref())
                        .await
                        .map(|p| poll::sdk_page(&p, &now_iso())),
                    None => self.mcp_page(&conn, cursor.as_deref()).await,
                };
                let page = match page {
                    Ok(page) => page,
                    Err(error) => break 'pages Some(error),
                };
                let next = page.next_cursor.clone().or_else(|| cursor.clone());
                if page.has_more && next == cursor {
                    break 'pages Some(format!(
                        "{connector}.poll({event}) returned hasMore without advancing its cursor"
                    ));
                }
                let events = poll::inbox_events(&conn, &page.events);
                let record = json!({
                    "workflowId": workflow_id,
                    "connectionId": connection_id,
                    "connectorId": connector,
                    "cursor": next,
                    "polledAt": now,
                    "events": events,
                });
                if let Err(error) = self
                    .store(move |s| s.call("dbRecordConnectorPollPage", json!([record])))
                    .await
                {
                    break 'pages Some(error);
                }
                pages += 1;
                cursor = next;
                if !page.has_more {
                    break;
                }
            }
            None
        };
        if let Some(error) = failed {
            warn!("[scheduler] connectorPoll: {connector}.poll({event}) failed: {error}");
            let record = json!({ "workflowId": workflow_id, "connectionId": connection_id, "error": error, "polledAt": now });
            let _ = self
                .store(move |s| s.call("dbRecordConnectorPollError", json!([record])))
                .await;
        }
        pages
    }

    // ---- connector-wide ----

    async fn probe_sdk(&self, params: &Value) -> Value {
        let Ok(native) = self.native() else {
            return json!({ "ok": false, "error": "vornd is stopping" });
        };
        let command = params.get("command").and_then(Value::as_str).unwrap_or("");
        let args: Vec<String> = params
            .get("args")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        let mut env = native.child_env();
        for (k, v) in params
            .get("env")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            if let Some(v) = v.as_str() {
                env.retain(|(key, _)| key != k);
                env.push((k.clone(), v.to_owned()));
            }
        }
        let program = super::mcp::resolve(vorn_connectors::js::trim(command), &env);
        let shown = if program.as_os_str().is_empty() {
            command.to_owned()
        } else {
            program.to_string_lossy().into_owned()
        };
        sdk::probe(
            &shown,
            &args,
            env,
            std::env::temp_dir(),
            &self.version,
            sdk::Timeouts::default(),
        )
        .await
    }

    fn catalog_snapshot(self: &Arc<Self>) -> Value {
        let (snapshot, stale) = self.catalog.snapshot(now_ms());
        if stale {
            let me = Arc::clone(self);
            tokio::spawn(async move {
                me.catalog_refresh().await;
            });
        }
        snapshot
    }

    async fn catalog_refresh(self: &Arc<Self>) -> Value {
        let me = Arc::clone(self);
        let changed = tokio::task::spawn_blocking(move || me.catalog.refresh(&me.http, now_ms()).1)
            .await
            .unwrap_or(false);
        let (snapshot, _) = self.catalog.snapshot(now_ms());
        if changed {
            if let Ok(native) = self.native() {
                native.broadcast("connector:catalogChanged", snapshot.clone());
            }
        }
        snapshot
    }

    async fn inspect_pack(self: &Arc<Self>, params: &Value) -> Value {
        let source = match PackSource::parse(params) {
            Ok(s) => s,
            Err(error) => return json!({ "ok": false, "error": error }),
        };
        let me = Arc::clone(self);
        tokio::task::spawn_blocking(move || me.installer.inspect(&source, &me.http))
            .await
            .unwrap_or_else(|e| json!({ "ok": false, "error": e.to_string() }))
    }

    async fn install_pack(self: &Arc<Self>, params: &Value) -> Value {
        let source = match PackSource::parse(params) {
            Ok(s) => s,
            Err(error) => return json!({ "ok": false, "error": error }),
        };
        let me = Arc::clone(self);
        let native = self.native.clone();
        let result = tokio::task::spawn_blocking(move || {
            let progress = move |event: Value| {
                if let Some(native) = native.upgrade() {
                    native.broadcast("connector:installProgress", event);
                }
            };
            me.installer.install(&source, &me.http, &progress)
        })
        .await
        .unwrap_or_else(|e| json!({ "ok": false, "error": e.to_string() }));
        if result["ok"] == true {
            if let Some(id) = result.pointer("/pack/id").and_then(Value::as_str) {
                self.pack_changed(id).await;
            }
            self.changed();
        }
        result
    }

    async fn remove_pack(&self, id: &str) -> Value {
        let connections = self
            .connections_of(id)
            .await
            .iter()
            .filter(|c| !conns::is_implicit(c))
            .count();
        let mut result = self.installer.remove(id);
        if result["ok"] == true {
            self.pack_changed(id).await;
            self.changed();
        }
        result["connections"] = json!(connections);
        result
    }

    async fn rollback_pack(&self, id: &str) -> Value {
        let result = self.installer.rollback(id);
        if result["ok"] == true {
            self.pack_changed(id).await;
            self.changed();
        }
        result
    }

    async fn connections_of(&self, connector_id: &str) -> Vec<Value> {
        let all = self
            .store(|s| s.call("dbListSourceConnections", json!([null])))
            .await
            .unwrap_or(Value::Null);
        all.as_array()
            .into_iter()
            .flatten()
            .filter(|c| conns::connector_id_of(c) == connector_id)
            .cloned()
            .collect()
    }

    /// After a pack changed, no child runs its old files, and the connection
    /// a pack brings with it comes or goes with it.
    async fn pack_changed(&self, id: &str) {
        let existing = self.connections_of(id).await;
        for conn in &existing {
            self.stop_children(text(conn, "id")).await;
        }
        let pack = self.installer.store().describe(id);
        let wants_implicit = pack.as_ref().is_some_and(|p| {
            !p.is_extension()
                && p.auth
                    .as_ref()
                    .and_then(|a| serde_json::to_value(a).ok())
                    .is_some_and(|a| a["rung"] == "none")
        });
        if !wants_implicit {
            for conn in existing.iter().filter(|c| conns::is_implicit(c)) {
                if self.remove_connection(text(conn, "id")).await.is_ok() {
                    info!(
                        "[packs] withdrew the implicit connection {}",
                        text(conn, "id")
                    );
                }
            }
            return;
        }
        if !existing.is_empty() {
            return;
        }
        let Some(pack) = pack else { return };
        let pack_json = json!(pack);
        let mut filters = conns::sdk_connection_filters(&pack_json, None);
        filters.insert(filter::IMPLICIT.into(), json!(true));
        let params = json!({
            "connectorId": SDK,
            "name": pack.name,
            "filters": filters,
            "syncIntervalMinutes": 0,
            "statusMapping": {},
        });
        let params = params.as_object().cloned().unwrap_or_default();
        let made = self
            .store(move |s| {
                conns::create(
                    s,
                    conns::NewConnection {
                        id: uuid(),
                        params: &params,
                        now: now_iso(),
                    },
                )
            })
            .await;
        if made.is_ok() {
            self.changed();
        }
    }

    /// Settles every installed pack's implicit connection, as the server did at start.
    pub async fn reconcile(&self) {
        for pack in self.installer.store().list() {
            self.pack_changed(&pack.id).await;
        }
    }

    async fn seed_workflow(&self, params: &Value) -> Result<Option<Value>, String> {
        let id = params
            .get("connectionId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let event = params
            .get("event")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let conn = self
            .connection(&id)
            .await?
            .ok_or_else(|| format!("connection {id} not found"))?;
        let connector = text(&conn, "connectorId").to_owned();
        let manifest = conns::builtin(&connector)
            .and_then(|c| c.get("manifest").cloned())
            .ok_or_else(|| format!("connector {connector} not registered"))?;
        let event_def = manifest
            .get("defaultWorkflows")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|e| text(e, "event") == event)
            .cloned()
            .ok_or_else(|| format!("event {event} not defined by connector {connector}"))?;
        let wf_id = conns::seeded_workflow_id(&id, &event);
        let wf = conns::seeded_workflow(&conn, &manifest, &event_def);
        let check = wf_id.clone();
        let created = self
            .store(move |s| {
                if !s.call("dbGetWorkflow", json!([check]))?.is_null() {
                    return Ok(false);
                }
                s.call("dbInsertWorkflow", json!([wf]))?;
                Ok(true)
            })
            .await?;
        if created {
            self.changed();
        }
        Ok(Some(json!({ "workflowId": wf_id, "created": created })))
    }

    async fn auth_source(&self, connector_id: &str) -> auth::Source {
        self.connector_auth(connector_id).await.unwrap_or_default()
    }

    async fn probe_auth(&self, connector_id: &str) -> Value {
        let source = self.auth_source(connector_id).await;
        let Ok(runner) = self.runner() else {
            return json!({ "ok": null });
        };
        tokio::task::spawn_blocking(move || auth::probe(&source, &runner))
            .await
            .unwrap_or_else(|_| json!({ "ok": null }))
    }

    async fn status(&self) -> Value {
        let mut results = Vec::new();
        for connector in conns::builtins() {
            let id = text(&connector, "id").to_owned();
            let report = self.probe_auth(&id).await;
            let authed = report["ok"] != false;
            let mut entry = json!({ "connectorId": id, "authed": authed });
            if !authed {
                let message: Vec<&str> = [report.get("message"), report.get("installHint")]
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter(|m| !m.is_empty())
                    .collect();
                if !message.is_empty() {
                    entry["message"] = json!(message.join("\n"));
                }
            }
            results.push(entry);
        }
        Value::Array(results)
    }

    fn detect_repo(&self, params: &Value) -> Value {
        let (Some(dir), Ok(native)) = (absolute_str(params), self.native()) else {
            return Value::Null;
        };
        let git = vorn_git::repo::Git {
            bin: native.env.git_bin(),
            env: native.env.get(),
            ssh: None,
        };
        git.github_origin(Path::new(dir))
            .map_or(Value::Null, |r| json!({ "owner": r.owner, "repo": r.repo }))
    }

    async fn http_request(&self, params: &Value) -> Value {
        let s = |k: &str| {
            params
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        let mut profile = vorn_connectors::http::Profile::default();
        let profile_id = s("profileConnectionId");
        if !profile_id.is_empty() {
            let conn = match self.connection(&profile_id).await {
                Ok(Some(conn)) => conn,
                _ => return failure(format!("Connection {profile_id} not found")),
            };
            let filters = conns::filters_of(&conn);
            let secret = self.secret(&profile_id, "secret").await;
            if let Some(problem) = vorn_connectors::http::profile_error(
                text(&conn, "connectorId"),
                &filters,
                secret.is_some(),
            ) {
                return failure(problem);
            }
            profile = vorn_connectors::http::Profile::of(&filters, secret.as_deref());
        }
        let headers: Vec<(String, String)> = params
            .get("headers")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(k, v)| (k.clone(), vorn_connectors::js::to_string(v)))
            .collect();
        let body = params
            .get("body")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let (method, url, http) = (s("method"), s("url"), self.http.clone());
        tokio::task::spawn_blocking(move || {
            vorn_connectors::http::perform(&http, &profile, &method, &url, headers, body)
        })
        .await
        .unwrap_or_else(|e| failure(e.to_string()))
    }

    /// Stops every child, as vornd stops.
    pub async fn stop(&self) {
        self.children.stop_all().await;
    }
}

/// What a connection's child starts with.
struct Spawn {
    launch: Launch,
    /// The connection's own variables, which win over the safe environment.
    own_env: Vec<(String, String)>,
    source: LaunchSource,
    protocol: Option<u64>,
    /// The name an installed pack gives itself.
    name: String,
    /// A browser connector's sign-in, when it acts through a window.
    browser: Option<Value>,
}

/// A window request the child sent: `{url, method, headers?, body?, binaryBody?}`.
fn read_window_request(body: &[u8]) -> Option<Value> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let v = value.as_object()?;
    let url = v.get("url")?.as_str()?;
    let method = v.get("method")?.as_str()?.to_uppercase();
    if v.get("body").is_some_and(|b| !b.is_string()) {
        return None;
    }
    let mut out = Map::new();
    out.insert("url".into(), json!(url));
    out.insert("method".into(), json!(method));
    if let Some(headers) = v.get("headers").and_then(Value::as_object) {
        let kept: BTreeMap<&String, &Value> =
            headers.iter().filter(|(_, v)| v.is_string()).collect();
        out.insert("headers".into(), json!(kept));
    }
    if let Some(body) = v.get("body") {
        out.insert("body".into(), body.clone());
    }
    if v.get("binaryBody") == Some(&Value::Bool(true)) {
        out.insert("binaryBody".into(), json!(true));
    }
    Some(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_a_package_action_as_the_step_editor_reads_it() {
        let action = json!({ "type": "create", "label": "Create", "inputs": [
            { "key": "kind", "label": "Kind", "type": "select", "required": true, "options": [{ "value": "a" }] },
            { "key": "data", "label": "Data", "type": "json", "required": false },
        ], "outputs": [{ "key": "id", "type": "string" }] });
        let def = conns::sdk_action_def(&action);
        assert_eq!(def["configFields"][0]["type"], "select");
        assert_eq!(
            def["configFields"][0]["options"][0],
            json!({ "value": "a", "label": "a" })
        );
        assert_eq!(def["configFields"][1]["type"], "textarea");
        assert_eq!(
            def["outputSchema"]["properties"]["id"],
            json!({ "type": "string" })
        );
    }

    #[test]
    fn reads_a_window_request_and_refuses_what_is_not_one() {
        let req = read_window_request(br#"{"url":"https://a.example/x","method":"post","headers":{"a":"1","b":2},"body":"x"}"#).unwrap();
        assert_eq!(req["method"], "POST");
        assert_eq!(req["headers"], json!({ "a": "1" }));
        assert!(read_window_request(br#"{"url":"u","method":"GET","body":3}"#).is_none());
        assert!(read_window_request(b"[]").is_none());
    }

    #[test]
    fn takes_secrets_out_of_a_row() {
        let mut filters = json!({ "secretEnv": "{\"K\":\"v\"}", "env": "{}" })
            .as_object()
            .unwrap()
            .clone();
        let mut secrets = Fields::new();
        take_secrets(MCP, &mut filters, &mut secrets);
        assert_eq!(filters["secretEnv"], IN_VAULT);
        assert_eq!(secrets["secretEnv"].expose(), "{\"K\":\"v\"}");
        let mut again = filters.clone();
        let mut none = Fields::new();
        take_secrets(MCP, &mut again, &mut none);
        assert!(none.is_empty());
    }
}
