//! The Connectors built-in: every `connection:*` and `connector:*` call,
//! `credentials:import` and `http:request`, and the polls connector-poll
//! workflows run, on the public API ([`crate::host`]).
//!
//! Connections are rows of its own tables; their secrets are in the vault,
//! and a row holds [`IN_VAULT`] where a secret was. A connection runs as a
//! child: an MCP server spoken to through the MCP SDK ([`crate::mcp`]), or a
//! package spoken to in the connector protocol
//! ([`vorn_extensions::connector`]) under the bridge's lifecycle. Tasks it
//! imports are written by the board (`task.importItems`), polled items go to
//! the Workflows inbox (`inbox.recordPage`), and a window signed in again
//! wakes the steps waiting for it (`signin.completed`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Map, Value};
use tracing::{info, warn};
use vorn_extensions::child::Launch;
use vorn_extensions::connector::{self as sdk, SdkClient, SdkLaunch, Source as LaunchSource};
use vorn_extensions::lifecycle::{Lease, ServiceKey};
use vorn_extensions::netproxy::{hosts_in, ProxyGrant};
use vorn_extensions::pack::PackStore;
use vorn_extensions::signin::{Grant, Windows};
use vorn_store::Store;
use vorn_vault::Secret;

use crate::auth::{self, Runner};
use crate::catalog::Catalog;
use crate::connections::{self as conns, filter, Item, IN_VAULT, MCP, SDK};
use crate::fetch::Http;
use crate::host::{Fields, Host, Services};
use crate::mcp;
use crate::poll::{self, McpPoll, Page};

/// The id the built-in goes by.
pub const ID: &str = "vorn.connectors";

/// Its manifest: what it declares it uses and the MCP tools it adds.
pub const MANIFEST: &str = include_str!("../vorn-extension.toml");

/// What `describe_workflow_nodes` tells of the node types it declares, by id.
pub const NODE_DOCS: &str = include_str!("../generated/nodes.json");

/// The trigger type whose polls it answers.
pub const POLL_TRIGGER: &str = "connectorPoll";

/// Every call this built-in answers.
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

/// What a call came to.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Result(Value),
    /// Answered with nothing.
    Void,
    Error(String),
    /// Not a call this built-in answers.
    Unanswered,
}

/// The calls answered with the built-in off: data the person moves, and
/// requests a workflow's HTTP step makes without a connection.
pub const WHILE_OFF: &[&str] = &["credentials:import", "http:request"];

/// An MCP connection's tools are first looked for this long after it is made.
const FIRST_DISCOVERY: Duration = Duration::from_millis(1500);

/// What answers the connection and connector calls.
pub struct Connectors {
    host: Arc<dyn Host>,
    services: Services,
    catalog: Catalog,
    mcp: mcp::McpClients,
    /// The proxy grant each sandboxed child runs with, by connection.
    proxied: Mutex<HashMap<String, ProxyGrant>>,
    /// How a checkout signs in, read once by probing it; `None` when it would not start.
    checkout_auth: Mutex<HashMap<String, Option<auth::Source>>>,
    http: Http,
    /// The connection the Connections page is adding, while it adds one.
    pub(crate) draft: Mutex<Option<crate::page::Draft>>,
}

impl std::fmt::Debug for Connectors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connectors")
            .field("services", &self.services)
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

fn failure(error: impl Into<String>) -> Value {
    json!({ "success": false, "error": error.into() })
}

/// The row with whole numbers written without `.0`.
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

/// The values of a connection's `secretEnv` field: its plaintext, a JSON
/// object, as `parseJsonObject` reads it.
pub fn secret_env(fields: &Fields) -> Map<String, Value> {
    let raw = fields
        .get("secretEnv")
        .map(|s| Value::String(s.expose().to_owned()));
    mcp::parse_json_object(raw.as_ref())
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
    host: Arc<dyn Host>,
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
        self.host.find_on_path(name, path)
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
    /// The built-in on `host`, running its connectors' programs with `services`.
    pub fn new(host: Arc<dyn Host>, services: Services) -> Arc<Connectors> {
        Arc::new(Connectors {
            catalog: Catalog::new(services.catalog_cache.clone(), services.repo_root.clone()),
            host,
            services,
            mcp: mcp::McpClients::default(),
            proxied: Mutex::default(),
            checkout_auth: Mutex::default(),
            http: Http,
            draft: Mutex::default(),
        })
    }

    fn packs(&self) -> &PackStore {
        self.services.installs.store()
    }

    fn windows(&self) -> &Windows {
        &self.services.windows
    }

    fn repo_root(&self) -> Option<&Path> {
        self.services.repo_root.as_deref()
    }

    /// Runs `f` on its own tables, off the runtime.
    async fn store<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Store) -> vorn_store::Result<T> + Send + 'static,
    ) -> Result<T, String> {
        let slot: Arc<Mutex<Option<T>>> = Arc::default();
        let out = Arc::clone(&slot);
        self.host
            .tables(Box::new(move |s| {
                let value = f(s)?;
                *out.lock().unwrap_or_else(|e| e.into_inner()) = Some(value);
                Ok(())
            }))
            .await?;
        let value = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        value.ok_or_else(|| "the store answered nothing".to_owned())
    }

    /// A core method of the public API.
    async fn api(&self, method: &str, args: Value) -> Result<Value, String> {
        self.host.call(method, args).await
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
        self.host.changed();
    }

    /// A connection's secrets.
    async fn secrets_of(&self, id: &str) -> Option<Fields> {
        self.host.secrets(id).await
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
                self.api("task.sourceLink", json!({ "taskId": task }))
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
            "connector:listPacks" => Ok(Some(json!(self.packs().list()))),
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
            self.host.set_secrets(&id, secrets);
        }
        let new_id = id.clone();
        let (conn, seeded) = self
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
        if !seeded.is_empty() {
            self.api("workflow.seed", json!({ "workflows": seeded }))
                .await?;
        }
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
                self.host.set_secrets(&id, secrets);
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
        self.api(
            "workflow.unseed",
            json!({ "prefix": conns::seeded_workflow_prefix(id) }),
        )
        .await?;
        let row = id.to_owned();
        self.store(move |s| conns::delete(s, &row)).await?;
        self.host.forget_secrets(id);
        let host = Arc::clone(&self.host);
        let forget = id.to_owned();
        tokio::spawn(async move {
            let _ = host.call("signin.forget", json!({ "key": forget })).await;
        });
        self.stop_children(id).await;
        self.changed();
        Ok(())
    }

    async fn stop_children(&self, id: &str) {
        let scope = connection_scope(id);
        self.services
            .lifecycle
            .stop_where(|k| k.scope == scope)
            .await;
        self.windows().revoke(id);
        self.proxied
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        self.mcp.stop(id);
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
                if let Some(locked) = crate::http::locked_error(&filters, secret.is_some()) {
                    json!({ "ok": false, "message": locked })
                } else {
                    let profile = crate::http::Profile::of(&filters, secret.as_deref());
                    let http = self.http.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        crate::http::execute(&http, "test", &profile, &Map::new())
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
        let launch = self.spawn_spec(conn).await?;
        let launch = mcp::Launch {
            command: launch.launch.program.to_string_lossy().into_owned(),
            args: launch.launch.args.clone(),
            env: launch.own_env,
        };
        self.mcp
            .client(text(conn, "id"), &launch, self.host.child_env())
            .await
    }

    async fn invoke_mcp(&self, conn: &Value, tool: &str, args: &Map<String, Value>) -> Value {
        match self.mcp_peer(conn).await {
            Ok(peer) => mcp::invoke(&peer, tool, args, &discovered_tools(conn)).await,
            Err(error) => failure(error),
        }
    }

    /// Lists an MCP connection's tools onto its row.
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
    /// installed pack's.
    async fn connector_auth(&self, sdk_id: &str) -> Option<auth::Source> {
        if sdk_id.is_empty() {
            return None;
        }
        let local = crate::catalog::local_launch_spec(sdk_id, self.repo_root());
        let Some((command, args)) = local else {
            let pack = self.packs().inspect_unmeasured(sdk_id).ok()?;
            return Some(auth::Source {
                auth: pack
                    .manifest
                    .auth
                    .as_ref()
                    .and_then(|a| serde_json::to_value(a).ok()),
                declared: pack.manifest.env.iter().map(|e| e.name.clone()).collect(),
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
        let env = self.host.child_env();
        let program = mcp::resolve(&command, &env);
        let probed = sdk::probe(
            &program.to_string_lossy(),
            &args,
            env,
            std::env::temp_dir(),
            &self.services.version,
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
        Ok(ProcessRunner {
            host: Arc::clone(&self.host),
            env: self.host.child_env(),
            source: std::env::vars().collect(),
        })
    }

    /// Everything a connection's child starts with.
    async fn spawn_spec(&self, conn: &Value) -> Result<Spawn, String> {
        let filters = conns::filters_of(conn);
        let sdk_id = conns::sdk_id_of(conn);
        let mut source = LaunchSource::Command;
        let mut protocol = None;
        let mut name = String::new();
        let mut cwd = std::env::temp_dir();
        let mut program_args: Option<(String, Vec<String>)> = None;
        if !sdk_id.is_empty() {
            if let Some(local) = crate::catalog::local_launch_spec(&sdk_id, self.repo_root()) {
                source = LaunchSource::Checkout;
                program_args = Some(local);
            } else if let Ok(pack) = self.packs().inspect_unmeasured(&sdk_id) {
                source = LaunchSource::Pack;
                protocol = pack.manifest.protocol;
                name = pack.name().to_owned();
                // A package's program runs in its own folder.
                cwd = pack.path.clone();
                program_args = pack
                    .service_command()
                    .map(|(program, args)| (program.to_string_lossy().into_owned(), args));
            }
        }
        let (command, args) = match program_args {
            Some(found) => found,
            None => {
                let command = filters.get("command").map_or(String::new(), |c| {
                    vorn_extensions::js::trim(&vorn_extensions::js::to_string(c)).to_owned()
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
                        items.iter().map(vorn_extensions::js::to_string).collect()
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
            for (k, v) in secret_env(&secrets) {
                put(k, v.as_str().unwrap_or("").to_owned());
            }
        }
        let browser = auth
            .as_ref()
            .and_then(|a| a.auth.as_ref())
            .filter(|a| a.get("rung").and_then(Value::as_str) == Some("browser"))
            .and_then(|a| a.get("browser").cloned());
        let mut env = self.host.child_env();
        for (k, v) in &own {
            env.retain(|(key, _)| key != k);
            env.push((k.clone(), v.clone()));
        }
        let program = mcp::resolve(&command, &env);
        Ok(Spawn {
            launch: Launch {
                program,
                args,
                cwd,
                env,
            },
            own_env: own,
            source,
            protocol,
            name,
            browser,
        })
    }

    /// A package connection's child, started on first use under the bridge's lifecycle.
    async fn sdk_client(&self, conn: &Value) -> Result<Lease<SdkClient>, String> {
        let id = text(conn, "id").to_owned();
        let sdk_id = conns::sdk_id_of(conn);
        let owner = if sdk_id.is_empty() {
            text(conn, "name").to_owned()
        } else {
            sdk_id
        };
        let key = ServiceKey::new(owner, connection_scope(&id));
        let conn = conn.clone();
        self.services
            .lifecycle
            .get_or_start(&key, || async {
                let mut spawn = self.spawn_spec(&conn).await?;
                if let Some(browser) = spawn.browser.clone() {
                    let token = self.windows().grant(&id, browser)?;
                    spawn.launch.env.push((
                        "VORN_BROWSER_HOST".into(),
                        format!("{}/connections/{id}/browser", self.services.origin),
                    ));
                    spawn.launch.env.push(("VORN_BROWSER_TOKEN".into(), token));
                }
                let key = if spawn.name.is_empty() {
                    text(&conn, "name").to_owned()
                } else {
                    spawn.name.clone()
                };
                let (held, grant) =
                    self.held(spawn.launch, &conns::sdk_id_of(&conn), &spawn.own_env);
                if let Some(grant) = grant {
                    self.proxied
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(id.clone(), grant);
                }
                let launch = SdkLaunch {
                    launch: held,
                    source: spawn.source,
                    protocol: spawn.protocol,
                };
                sdk::open(
                    &launch,
                    &key,
                    &self.services.version,
                    sdk::Timeouts::default(),
                )
                .await
                .map_err(|e| e.to_string())
            })
            .await
    }

    /// `launch` under the macOS sandbox profile when the person turned it
    /// on for package `sdk_id`: no network but the proxy's, which lets it
    /// through to the hosts its manifest declares and those the connection's
    /// settings (`own`) name where the manifest says so.
    fn held(
        &self,
        launch: Launch,
        sdk_id: &str,
        own: &[(String, String)],
    ) -> (Launch, Option<ProxyGrant>) {
        let store = self.packs();
        if !vorn_extensions::pack::is_safe_id(sdk_id)
            || !vorn_extensions::sandbox::available()
            || !vorn_extensions::sandbox::is_on(&store.root().join(sdk_id))
        {
            return (launch, None);
        }
        let Ok(pack) = store.inspect_unmeasured(sdk_id) else {
            return (launch, None);
        };
        let declares = &pack.manifest.declares;
        let mut hosts = declares.network.clone();
        for (name, value) in own {
            if declares.network_from.contains(name) {
                hosts.extend(hosts_in(value));
            }
        }
        let grant =
            self.services
                .proxy
                .as_ref()
                .and_then(|proxy| match proxy.grant(sdk_id, hosts) {
                    Ok(grant) => Some(grant),
                    Err(err) => {
                        warn!("[connectors] {sdk_id} runs without a network: {err}");
                        None
                    }
                });
        let mut launch = launch;
        if let Some(grant) = &grant {
            for (k, v) in grant.env() {
                launch.env.retain(|(key, _)| *key != k);
                launch.env.push((k, v));
            }
        }
        let port = grant.as_ref().map(ProxyGrant::port);
        (
            vorn_extensions::sandbox::wrap_through(launch, &pack.path, port),
            grant,
        )
    }

    /// Runs `action` in a process of its own when the connection's package
    /// gives it a command: started, greeted, asked once and stopped.
    async fn one_shot(
        &self,
        conn: &Value,
        action: &str,
        args: &Map<String, Value>,
    ) -> Option<Result<Value, sdk::SdkError>> {
        let sdk_id = conns::sdk_id_of(conn);
        let pack = self.packs().inspect_unmeasured(&sdk_id).ok()?;
        let run = pack.manifest.run(action)?;
        let (program, run_args) = pack.resolve(&run.command)?;
        let mut spawn = match self.spawn_spec(conn).await {
            Ok(spawn) => spawn,
            Err(message) => {
                return Some(Err(sdk::SdkError {
                    message,
                    data: Default::default(),
                }))
            }
        };
        spawn.launch.program = program;
        spawn.launch.args = run_args;
        let (held, _grant) = self.held(spawn.launch, &sdk_id, &spawn.own_env);
        let launch = SdkLaunch {
            launch: held,
            source: spawn.source,
            protocol: spawn.protocol,
        };
        let name = pack.name().to_owned();
        let client = match sdk::open(
            &launch,
            &name,
            &self.services.version,
            sdk::Timeouts::default(),
        )
        .await
        {
            Ok(client) => client,
            Err(err) => {
                return Some(Err(sdk::SdkError {
                    message: err.to_string(),
                    data: Default::default(),
                }))
            }
        };
        let answered = client.action(action, args, None).await.map(Value::Object);
        tokio::spawn(async move { client.close().await });
        Some(answered)
    }

    /// What a package connection declares: its installed pack, or what its child says.
    async fn sdk_manifest(&self, conn: &Value) -> Result<Value, String> {
        let sdk_id = conns::sdk_id_of(conn);
        let local = crate::catalog::local_launch_spec(&sdk_id, self.repo_root());
        if !sdk_id.is_empty() && local.is_none() {
            if let Some(pack) = self.packs().describe(&sdk_id) {
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
                if let Some(answered) = self.one_shot(conn, action, args).await {
                    return match answered {
                        Ok(output) => json!({ "success": true, "output": output }),
                        Err(err) => failure(err.message),
                    };
                }
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
                if let Some(locked) = crate::http::locked_error(&filters, secret.is_some()) {
                    return failure(locked);
                }
                let mut merged = filters.clone();
                if let Some(secret) = &secret {
                    merged.insert("secret".into(), json!(secret));
                }
                merged.extend(args.clone());
                let profile = crate::http::Profile::of(&merged, secret.as_deref());
                let (http, action) = (self.http.clone(), action.to_owned());
                tokio::task::spawn_blocking(move || {
                    crate::http::execute(&http, &action, &profile, &merged)
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
        let lease = match self.sdk_client(conn).await {
            Ok(lease) => lease,
            Err(error) => return Err(failure(error)),
        };
        let client = lease.shared();
        let begun = self.windows().begin(&id);
        let key = begun.as_ref().map(|(_, k)| k.clone());
        let answered = run(client, key).await;
        drop(lease);
        let (grant, calls) = match begun {
            Some((g, k)) => {
                let calls = Windows::end(&g, &k);
                (Some(g), calls)
            }
            None => (None, Vec::new()),
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
        let answer = self
            .api("signin.check", json!({ "key": id, "browser": browser }))
            .await
            .ok()?;
        answer.get("signedIn").and_then(Value::as_bool)
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
        let host = Arc::clone(&self.host);
        tokio::spawn(async move {
            if let Err(err) = host.call("signin.completed", json!({ "key": id })).await {
                warn!("[connectors] could not wake the steps waiting on {id}: {err}");
            }
        });
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
        let conns_list = self
            .store(|s| s.call("dbListSourceConnections", json!([null])))
            .await?;
        let workflows = self.api("workflow.list", json!({})).await?;
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
        self.host.merge_secret(&id, &field, &plaintext).await?;
        self.stop_children(&id).await;
        self.changed();
        Ok(Some(json!({ "ok": true })))
    }

    /// `credentials:import`: what the desktop decrypted of the secrets it
    /// sealed, filed in the vault once, each row then holding the marker.
    async fn import(&self, params: &Value) -> Result<Option<Value>, String> {
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
            self.host.set_secrets(id, secrets);
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
        let (ssh_keys, host_passwords) = self.host.import_core_credentials(params.clone()).await?;
        Ok(Some(json!({
            "connections": imported,
            "sshKeys": ssh_keys,
            "hostPasswords": host_passwords,
        })))
    }

    // ---- items ----

    async fn upsert_from_item(&self, params: &Value) -> Result<Option<Value>, String> {
        let args = self.item_import(params).await?;
        let answered = self.api("task.importItems", args.clone()).await?;
        let result = answered
            .as_array()
            .and_then(|a| a.last())
            .cloned()
            .unwrap_or(Value::Null);
        self.synced(&args).await?;
        Ok(Some(result))
    }

    /// `connection.itemImport`: what `task.importItems` takes to put one item
    /// of a connection on the board, in the step's project or the
    /// connection's, read without writing anything. Tasks imports it itself,
    /// so its own step never waits on a call back into it.
    pub async fn item_import(&self, params: &Value) -> Result<Value, String> {
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
            .filter(|p| vorn_extensions::js::truthy(p))
            .or_else(|| {
                conn.get("executionProject")
                    .filter(|p| vorn_extensions::js::truthy(p))
            })
            .or_else(|| conn.get("name"))
            .map(vorn_extensions::js::to_string)
            .unwrap_or_default();
        let status = params.get("initialStatus").cloned().unwrap_or(Value::Null);
        Ok(import_args(&conn, &project, vec![(item_fields, status)]))
    }

    /// `connection.synced`: the connection whose items `args` imported was synced now.
    pub async fn synced(&self, args: &Value) -> Result<(), String> {
        let id = text(args, "connectionId").to_owned();
        let mut updates = Map::new();
        updates.insert("lastSyncAt".into(), json!(now_iso()));
        self.store(move |st| conns::update(st, &id, updates, &[]))
            .await?;
        self.changed();
        Ok(())
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
            .filter(|p| vorn_extensions::js::truthy(p))
            .or_else(|| conn.get("name"))
            .map(vorn_extensions::js::to_string)
            .unwrap_or_default();
        let mapping = conn.get("statusMapping").cloned().unwrap_or(Value::Null);
        let with_status: Vec<(Item, Value)> = items
            .into_iter()
            .map(|item| {
                let status = mapping
                    .get(&item.status_raw)
                    .filter(|v| vorn_extensions::js::truthy(v))
                    .cloned()
                    .unwrap_or_else(|| json!("todo"));
                (item, status)
            })
            .collect();
        let written = if with_status.is_empty() {
            Ok(Vec::new())
        } else {
            self.import_items(&conn, &project, with_status).await
        };
        let (imported, updated) = match written {
            Ok(answers) => {
                let created = answers.iter().filter(|r| r["created"] == true).count() as u64;
                (created, answers.len() as u64 - created)
            }
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

    /// Puts items of `conn` on the board through the board's own method:
    /// each answer `{taskId, created}` in order.
    async fn import_items(
        &self,
        conn: &Value,
        project: &str,
        items: Vec<(Item, Value)>,
    ) -> Result<Vec<Value>, String> {
        let answered = self
            .api("task.importItems", import_args(conn, project, items))
            .await?;
        Ok(answered.as_array().cloned().unwrap_or_default())
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
        let Ok(workflow) = self
            .api("workflow.get", json!({ "workflowId": workflow_id }))
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
        let mut cursor: Option<String> = self
            .api(
                "inbox.cursor",
                json!({ "workflowId": workflow_id, "connectionId": connection_id }),
            )
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
                if let Err(error) = self.api("inbox.recordPage", record).await {
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
            let _ = self.api("inbox.recordError", record).await;
        }
        pages
    }

    // ---- connector-wide ----

    async fn probe_sdk(&self, params: &Value) -> Value {
        let command = params.get("command").and_then(Value::as_str).unwrap_or("");
        let args: Vec<String> = params
            .get("args")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        let mut env = self.host.child_env();
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
        let program = mcp::resolve(vorn_extensions::js::trim(command), &env);
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
            &self.services.version,
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
            self.host
                .broadcast("connector:catalogChanged", snapshot.clone());
        }
        snapshot
    }

    /// `connector:inspectPack`: clones a connector's repository and
    /// describes it, its declared hosts and borrowed sign-in included, running
    /// nothing of it.
    async fn inspect_pack(self: &Arc<Self>, params: &Value) -> Value {
        let source = match GitSource::parse(params) {
            Ok(s) => s,
            Err(error) => return json!({ "ok": false, "error": error }),
        };
        let me = Arc::clone(self);
        let inspected = tokio::task::spawn_blocking(move || me.inspect_git(&source))
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
        match inspected {
            Ok(preview) => json!({ "ok": true, "preview": preview }),
            Err(error) => {
                warn!("[connectors] inspecting a connector refused: {error}");
                json!({ "ok": false, "error": error })
            }
        }
    }

    /// The preview the confirm sheet shows: the manifest's fields and the install sheet's.
    fn inspect_git(&self, source: &GitSource) -> Result<Value, String> {
        let installs = &self.services.installs;
        let sheet = installs.inspect_git_at(
            &source.url,
            source.branch.as_deref(),
            source.path.as_deref(),
        )?;
        let manifest = installs
            .staged_manifest(&sheet.token)
            .ok_or("That install sheet expired before it was read")?;
        let mut preview = match serde_json::to_value(&sheet) {
            Ok(Value::Object(map)) => map,
            _ => Map::new(),
        };
        if let Some(icon) = &manifest.icon {
            preview.insert("icon".into(), json!(icon));
        }
        if let Some(auth) = &manifest.auth {
            preview.insert("auth".into(), json!(auth));
        }
        preview.insert("triggers".into(), json!(manifest.triggers));
        preview.insert("actions".into(), json!(manifest.actions));
        preview.insert("env".into(), json!(manifest.env));
        if let Some(installed) = self.packs().describe(&manifest.id) {
            preview.insert("installedVersion".into(), json!(installed.version));
        }
        Ok(Value::Object(preview))
    }

    /// `connector:installPack`: puts in place what a sheet described, or
    /// clones and installs a repository at once; a click on the sheet is
    /// the confirmation, as it was for a pack.
    async fn install_pack(self: &Arc<Self>, params: &Value) -> Value {
        let token = match params.get("kind").and_then(Value::as_str) {
            Some("staged") => match params.get("token").and_then(Value::as_str) {
                Some(t) if !t.is_empty() => t.to_owned(),
                _ => return json!({ "ok": false, "error": "That pack has expired" }),
            },
            _ => {
                let inspected = self.inspect_pack(params).await;
                match inspected.pointer("/preview/token").and_then(Value::as_str) {
                    Some(t) => t.to_owned(),
                    None => return inspected,
                }
            }
        };
        let installs = Arc::clone(&self.services.installs);
        let Some(manifest) = installs.staged_manifest(&token) else {
            return json!({ "ok": false, "error": "That connector was checked too long ago; open it again to install it" });
        };
        let id = manifest.id.clone();
        let progress = |event: Value| {
            let mut event = event;
            event["id"] = json!(id);
            self.host.broadcast("connector:installProgress", event);
        };
        progress(json!({ "phase": "installing", "version": manifest.version }));
        let typed = id.clone();
        let placed = tokio::task::spawn_blocking(move || installs.confirm(&token, &typed))
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
        match placed {
            Ok(pack) => {
                info!("[connectors] installed {}@{}", pack.id, pack.version);
                progress(json!({ "phase": "installed", "version": pack.version }));
                self.pack_changed(&pack.id).await;
                self.changed();
                json!({ "ok": true, "pack": pack })
            }
            Err(error) => {
                warn!("[connectors] installing {id} refused: {error}");
                progress(json!({ "phase": "failed", "error": error }));
                json!({ "ok": false, "error": error })
            }
        }
    }

    async fn remove_pack(&self, id: &str) -> Value {
        let connections = self
            .connections_of(id)
            .await
            .iter()
            .filter(|c| !conns::is_implicit(c))
            .count();
        let installs = Arc::clone(&self.services.installs);
        let owned = id.to_owned();
        let removed = tokio::task::spawn_blocking(move || installs.remove(&owned))
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
        let mut result = match removed {
            Ok(()) => {
                self.pack_changed(id).await;
                self.changed();
                json!({ "ok": true })
            }
            Err(error) => json!({ "ok": false, "error": error }),
        };
        result["connections"] = json!(connections);
        result
    }

    async fn rollback_pack(&self, id: &str) -> Value {
        let installs = Arc::clone(&self.services.installs);
        let owned = id.to_owned();
        let rolled = tokio::task::spawn_blocking(move || installs.rollback(&owned))
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
        match rolled {
            Ok(pack) => {
                self.pack_changed(id).await;
                self.changed();
                json!({ "ok": true, "pack": pack })
            }
            Err(error) => json!({ "ok": false, "error": error }),
        }
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

    /// The package connection `id` belongs to, if it is one.
    pub async fn owner_of(&self, id: &str) -> Option<String> {
        let conn = self.connection(id).await.ok()??;
        let sdk_id = conns::sdk_id_of(&conn);
        (!sdk_id.is_empty()).then_some(sdk_id)
    }

    /// A package installed or linked through the extension bridge.
    pub async fn package_changed(&self, id: &str) {
        self.pack_changed(id).await;
        self.changed();
    }

    /// A package removed through the extension bridge takes every connection
    /// made with it, and their secrets: nothing of it is left.
    pub async fn package_removed(&self, id: &str) {
        for conn in self.connections_of(id).await {
            let conn_id = text(&conn, "id");
            if let Err(err) = self.remove_connection(conn_id).await {
                warn!("[packs] could not remove {conn_id} with {id}: {err}");
            }
        }
        self.pack_changed(id).await;
        self.changed();
    }

    /// After a pack changed, no child runs its old files, and the connection
    /// a pack brings with it comes or goes with it.
    async fn pack_changed(&self, id: &str) {
        let existing = self.connections_of(id).await;
        for conn in &existing {
            self.stop_children(text(conn, "id")).await;
        }
        let pack = self.packs().describe(id);
        let wants_implicit = pack.as_ref().is_some_and(|p| {
            p.manifest.kind == vorn_extensions::manifest::Kind::Connector
                && p.manifest
                    .auth
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
            "name": pack.name(),
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

    /// Settles every installed pack's implicit connection; run at start.
    pub async fn reconcile(&self) {
        for pack in self.packs().list() {
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
        let seeded = self
            .api("workflow.seed", json!({ "workflows": [wf] }))
            .await?;
        let created = seeded.pointer("/created/0") == Some(&Value::Bool(true));
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
        let Some(dir) = params
            .as_str()
            .filter(|d| d.starts_with('/') || Path::new(d).is_absolute())
        else {
            return Value::Null;
        };
        self.host.github_origin(Path::new(dir)).map_or(
            Value::Null,
            |(owner, repo)| json!({ "owner": owner, "repo": repo }),
        )
    }

    async fn http_request(&self, params: &Value) -> Value {
        let s = |k: &str| {
            params
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        let mut profile = crate::http::Profile::default();
        let profile_id = s("profileConnectionId");
        if !profile_id.is_empty() {
            let conn = match self.connection(&profile_id).await {
                Ok(Some(conn)) => conn,
                _ => return failure(format!("Connection {profile_id} not found")),
            };
            let filters = conns::filters_of(&conn);
            let secret = self.secret(&profile_id, "secret").await;
            if let Some(problem) =
                crate::http::profile_error(text(&conn, "connectorId"), &filters, secret.is_some())
            {
                return failure(problem);
            }
            profile = crate::http::Profile::of(&filters, secret.as_deref());
        }
        let headers: Vec<(String, String)> = params
            .get("headers")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(k, v)| (k.clone(), vorn_extensions::js::to_string(v)))
            .collect();
        let body = params
            .get("body")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let (method, url, http) = (s("method"), s("url"), self.http.clone());
        tokio::task::spawn_blocking(move || {
            crate::http::perform(&http, &profile, &method, &url, headers, body)
        })
        .await
        .unwrap_or_else(|e| failure(e.to_string()))
    }

    /// The installed package `id`, unmeasured, as its JSON describes it.
    pub(crate) fn installed(&self, id: &str) -> Option<Value> {
        self.packs().inspect_unmeasured(id).ok().map(|p| json!(p))
    }

    /// What a host the built-in runs on answers to a core method.
    pub(crate) async fn core(&self, method: &str, args: Value) -> Result<Value, String> {
        self.api(method, args).await
    }

    /// Stops every child, as vornd stops.
    pub async fn stop(&self) {
        self.services
            .lifecycle
            .stop_where(|k| k.scope.starts_with(CONNECTION_SCOPE))
            .await;
    }
}

/// A connector's repository, as a client names it to install from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitSource {
    pub url: String,
    pub branch: Option<String>,
    /// The connector's folder when the repository holds several.
    pub path: Option<String>,
}

impl GitSource {
    /// Reads `{kind: "git", url, branch?, path?}`; any other kind is refused.
    pub fn parse(raw: &Value) -> Result<GitSource, String> {
        let text = |k: &str| {
            raw.get(k)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        match raw.get("kind").and_then(Value::as_str) {
            Some("git") => Ok(GitSource {
                url: text("url").ok_or("A git URL is required")?,
                branch: text("branch"),
                path: text("path"),
            }),
            _ => Err("That is not a connector to install".to_owned()),
        }
    }
}

/// Where a connection's child sits in the lifecycle's keys.
const CONNECTION_SCOPE: &str = "connection:";

fn connection_scope(id: &str) -> String {
    format!("{CONNECTION_SCOPE}{id}")
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

/// What `task.importItems` takes to put `items` of connection `conn` on the board in `project`.
fn import_args(conn: &Value, project: &str, items: Vec<(Item, Value)>) -> Value {
    let items: Vec<Value> = items
        .into_iter()
        .map(|(i, status)| {
            json!({
                "externalId": i.external_id, "title": i.title, "description": i.description,
                "externalUrl": i.external_url, "statusRaw": i.status_raw,
                "updatedAt": i.updated_at, "status": status,
            })
        })
        .collect();
    json!({
        "connectionId": text(conn, "id"),
        "connectorId": conn.get("connectorId").cloned().unwrap_or(Value::Null),
        "sourceConnectorId": conns::connector_id_of(conn),
        "project": project,
        "items": items,
    })
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
