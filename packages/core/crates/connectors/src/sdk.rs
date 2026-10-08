//! A connector package's child, opened in the connector protocol: started,
//! greeted with `vorn/hello`, then asked for its manifest, a preflight, a
//! select's options, a page of a trigger, or an action.
//!
//! An installed pack names the protocol it speaks; a checkout or a stored
//! command is asked. A child that never heard of the hello was built for the
//! MCP-era SDK, and is told so rather than run.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Map, Value};
use tokio::sync::Mutex;

use crate::child::{CallError, Child, ErrorData, Launch, CONNECTOR_ERROR};
use crate::manifest::{self, Manifest};

/// Every protocol this build speaks, offered in `vorn/hello`.
pub const SUPPORTED_PROTOCOLS: &[u64] = &[1];

/// The protocol's error codes this side reads.
const METHOD_NOT_FOUND: i64 = -32601;
const UNSUPPORTED_PROTOCOL: i64 = -32001;

/// How long each kind of call may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    pub hello: Duration,
    /// `npx` may download the package before the child can say anything.
    pub npx_hello: Duration,
    pub manifest: Duration,
    pub call: Duration,
    /// A call through a signed-in window waits on the desktop too.
    pub session_call: Duration,
}

impl Default for Timeouts {
    fn default() -> Timeouts {
        Timeouts {
            hello: Duration::from_secs(15),
            npx_hello: Duration::from_secs(90),
            manifest: Duration::from_secs(15),
            call: Duration::from_secs(60),
            session_call: Duration::from_secs(120),
        }
    }
}

impl Timeouts {
    /// The hello's wait for `command`.
    pub fn hello_for(&self, command: &str) -> Duration {
        let name = command
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(command)
            .to_lowercase();
        if matches!(name.as_str(), "npx" | "npx.cmd" | "npx.exe") {
            self.npx_hello
        } else {
            self.hello
        }
    }
}

/// Where a connection's child comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Checkout,
    Pack,
    Command,
}

/// What to start, where it came from, and the protocol an installed pack names.
#[derive(Debug, Clone)]
pub struct SdkLaunch {
    pub launch: Launch,
    pub source: Source,
    pub protocol: Option<u64>,
}

/// Why a child could not be opened in a protocol this build speaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// It speaks a newer protocol than this build.
    Unsupported(String),
    /// It was built for the MCP-era SDK.
    Outdated(String),
    /// It would not start or would not answer the hello.
    Failed(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::Unsupported(m) | OpenError::Outdated(m) | OpenError::Failed(m) => {
                f.write_str(m)
            }
        }
    }
}

impl std::error::Error for OpenError {}

/// What a connector built on the MCP-era SDK is told, wherever it would have run.
pub fn outdated_message(name: &str) -> String {
    format!("{name} was built for an older Vorn. Update it in Settings → Connectors, or rebuild it with @vornrun/connector-sdk 0.7.1-beta.3 or later.")
}

fn needs_newer_vorn(key: &str, protocol: Option<u64>) -> OpenError {
    let spoken = match protocol {
        Some(p) => format!("connector protocol {p}"),
        None => "a newer connector protocol".to_owned(),
    };
    OpenError::Unsupported(format!("{key} speaks {spoken}, which needs a newer Vorn"))
}

/// A failed call, read the way an action's failure is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdkError {
    pub message: String,
    pub data: ErrorData,
}

impl std::fmt::Display for SdkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<CallError> for SdkError {
    fn from(err: CallError) -> SdkError {
        match err {
            CallError::Answered { message, data, .. } => SdkError { message, data },
            CallError::Transport(message) => SdkError {
                message,
                data: ErrorData::default(),
            },
        }
    }
}

/// One open connector child.
#[derive(Debug)]
pub struct SdkClient {
    child: Child,
    hello: Value,
    key: String,
    timeouts: Timeouts,
    /// Asked once per child: a manifest does not change while its files run.
    manifest: Mutex<Option<Map<String, Value>>>,
}

/// Starts `launch` and greets it; `key` is how messages name it.
pub async fn open(
    launch: &SdkLaunch,
    key: &str,
    host_version: &str,
    timeouts: Timeouts,
) -> Result<SdkClient, OpenError> {
    let probe = match (launch.source, launch.protocol) {
        (Source::Pack, None) => return Err(OpenError::Outdated(outdated_message(key))),
        (Source::Pack, Some(p)) if !SUPPORTED_PROTOCOLS.contains(&p) => {
            return Err(needs_newer_vorn(key, Some(p)))
        }
        (Source::Pack, Some(_)) => false,
        _ => true,
    };
    let child = Child::start(&launch.launch, key.to_owned()).map_err(OpenError::Failed)?;
    let command = launch.launch.program.to_string_lossy().into_owned();
    let hello = child
        .request(
            "vorn/hello",
            json!({
                "protocols": SUPPORTED_PROTOCOLS,
                "host": { "name": "vorn", "version": host_version },
            }),
            timeouts.hello_for(&command),
        )
        .await;
    let hello = match hello {
        Ok(hello) => hello,
        Err(err) => {
            child.close().await;
            return Err(match err.code() {
                Some(METHOD_NOT_FOUND) if probe => OpenError::Outdated(outdated_message(key)),
                Some(UNSUPPORTED_PROTOCOL) => needs_newer_vorn(key, None),
                _ => OpenError::Failed(format!("{key} did not answer vorn/hello: {err}")),
            });
        }
    };
    let agreed = hello.get("protocol").and_then(Value::as_f64);
    match agreed {
        Some(p) if p.fract() == 0.0 && p >= 0.0 && SUPPORTED_PROTOCOLS.contains(&(p as u64)) => {}
        Some(p) => {
            child.close().await;
            return Err(needs_newer_vorn(
                key,
                (p.fract() == 0.0).then_some(p as u64),
            ));
        }
        None => {
            child.close().await;
            return Err(OpenError::Failed(format!(
                "{key} answered vorn/hello without a protocol"
            )));
        }
    }
    Ok(SdkClient {
        child,
        hello,
        key: key.to_owned(),
        timeouts,
        manifest: Mutex::new(None),
    })
}

impl SdkClient {
    /// What the child said in its hello.
    pub fn hello(&self) -> &Value {
        &self.hello
    }

    pub fn exited(&self) -> bool {
        self.child.exited()
    }

    pub async fn close(&self) {
        self.child.close().await;
    }

    async fn call(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        valid: impl Fn(&Map<String, Value>) -> bool,
        what: &str,
    ) -> Result<Map<String, Value>, SdkError> {
        let result = self.child.request(method, params, timeout).await?;
        match result {
            Value::Object(map) if valid(&map) => Ok(map),
            _ => Err(SdkError {
                message: format!("{} answered {method} without {what}", self.key),
                data: ErrorData::default(),
            }),
        }
    }

    fn call_timeout(&self, session_call: Option<&str>) -> Duration {
        if session_call.is_some() {
            self.timeouts.session_call
        } else {
            self.timeouts.call
        }
    }

    /// The manifest as the child reports it, read once per child.
    pub async fn raw_manifest(&self) -> Result<Map<String, Value>, SdkError> {
        let mut held = self.manifest.lock().await;
        if let Some(manifest) = held.as_ref() {
            return Ok(manifest.clone());
        }
        let manifest = self
            .call(
                "connector/manifest",
                json!({}),
                self.timeouts.manifest,
                |r| r.get("id").is_some_and(Value::is_string),
                "a manifest",
            )
            .await?;
        *held = Some(manifest.clone());
        Ok(manifest)
    }

    /// The manifest, checked as an install checks it.
    pub async fn manifest(&self) -> Result<Manifest, SdkError> {
        let raw = self.raw_manifest().await?;
        manifest::read(&raw).map_err(|e| SdkError {
            message: e.to_string(),
            data: ErrorData::default(),
        })
    }

    /// `connector/preflight`: `{ok, message?}`, `ok` null when it declares none.
    pub async fn preflight(&self) -> Result<Value, SdkError> {
        let result = self
            .call(
                "connector/preflight",
                json!({}),
                self.timeouts.call,
                |r| matches!(r.get("ok"), Some(Value::Bool(_) | Value::Null)),
                "an ok",
            )
            .await?;
        let mut out = Map::new();
        out.insert(
            "ok".into(),
            result.get("ok").cloned().unwrap_or(Value::Null),
        );
        if let Some(message) = result.get("message").and_then(Value::as_str) {
            if !message.is_empty() {
                out.insert("message".into(), json!(message));
            }
        }
        Ok(Value::Object(out))
    }

    /// `connector/options` for the select `name`.
    pub async fn options(&self, name: &str, session_call: Option<&str>) -> Result<Value, SdkError> {
        let mut params = json!({ "name": name });
        if let Some(call) = session_call {
            params["sessionCall"] = json!(call);
        }
        self.call(
            "connector/options",
            params,
            self.call_timeout(session_call),
            |r| r.get("options").is_some_and(Value::is_array),
            "options",
        )
        .await
        .map(Value::Object)
    }

    /// One page of `trigger` after `cursor`: `{items, nextCursor?, hasMore}`.
    pub async fn poll(
        &self,
        trigger: &str,
        cursor: Option<&str>,
        session_call: Option<&str>,
    ) -> Result<Map<String, Value>, SdkError> {
        let mut params = json!({ "trigger": trigger });
        if let Some(cursor) = cursor {
            params["cursor"] = json!(cursor);
        }
        if let Some(call) = session_call {
            params["sessionCall"] = json!(call);
        }
        self.call(
            "trigger/poll",
            params,
            self.call_timeout(session_call),
            |r| {
                r.get("items").is_some_and(Value::is_array)
                    && r.get("hasMore").is_some_and(Value::is_boolean)
            },
            "a page of items",
        )
        .await
    }

    /// `action/run`: whatever object the action answers.
    pub async fn action(
        &self,
        action: &str,
        args: &Map<String, Value>,
        session_call: Option<&str>,
    ) -> Result<Map<String, Value>, SdkError> {
        let mut params = json!({ "action": action, "args": args });
        if let Some(call) = session_call {
            params["sessionCall"] = json!(call);
        }
        self.call(
            "action/run",
            params,
            self.call_timeout(session_call),
            |_| true,
            "an output object",
        )
        .await
    }
}

/// A one-shot read of a package's manifest before any connection exists
/// (`probeSdkConnector`): started, asked, and stopped.
pub async fn probe(
    command: &str,
    args: &[String],
    env: Vec<(String, String)>,
    cwd: PathBuf,
    host_version: &str,
    timeouts: Timeouts,
) -> Value {
    let command = crate::js::trim(command);
    if command.is_empty() {
        return json!({ "ok": false, "error": "A command is required" });
    }
    let key = args
        .iter()
        .rfind(|a| !a.starts_with('-'))
        .map_or(command, String::as_str)
        .to_owned();
    let launch = SdkLaunch {
        launch: Launch {
            program: PathBuf::from(command),
            args: args.to_vec(),
            cwd,
            env,
        },
        source: Source::Command,
        protocol: None,
    };
    let client = match open(&launch, &key, host_version, timeouts).await {
        Ok(client) => client,
        Err(err) => return json!({ "ok": false, "error": err.to_string() }),
    };
    let read = client.manifest().await;
    client.close().await;
    match read {
        Ok(manifest) => json!({ "ok": true, "manifest": manifest }),
        Err(err) => json!({ "ok": false, "error": err.message }),
    }
}

/// One key's child, once started.
type Slot = Arc<Mutex<Option<Arc<SdkClient>>>>;

/// Children kept one per key (a connection), started on first use.
#[derive(Debug, Default)]
pub struct Children {
    live: std::sync::Mutex<std::collections::HashMap<String, Slot>>,
}

impl Children {
    fn slot(&self, key: &str) -> Slot {
        Arc::clone(
            self.live
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry(key.to_owned())
                .or_default(),
        )
    }

    /// `key`'s child, or one started by `start`; two callers share a start.
    pub async fn get_or_start<F, Fut>(&self, key: &str, start: F) -> Result<Arc<SdkClient>, String>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<SdkClient, String>>,
    {
        let slot = self.slot(key);
        let mut held = slot.lock().await;
        if let Some(client) = held.as_ref().filter(|c| !c.exited()) {
            return Ok(Arc::clone(client));
        }
        let client = Arc::new(start().await?);
        *held = Some(Arc::clone(&client));
        Ok(client)
    }

    /// The running child of `key`, if any.
    pub async fn get(&self, key: &str) -> Option<Arc<SdkClient>> {
        let slot = self.slot(key);
        let held = slot.lock().await;
        held.as_ref().filter(|c| !c.exited()).cloned()
    }

    /// Stops `key`'s child, if it has one.
    pub async fn stop(&self, key: &str) {
        let slot = self
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key);
        if let Some(slot) = slot {
            if let Some(client) = slot.lock().await.take() {
                client.close().await;
            }
        }
    }

    /// Stops every child.
    pub async fn stop_all(&self) {
        let keys: Vec<String> = self
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect();
        for key in keys {
            self.stop(&key).await;
        }
    }
}

/// An error code a connector answered with, or the protocol's own when none.
pub fn code_of(err: &CallError) -> i64 {
    err.code().unwrap_or(CONNECTOR_ERROR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_longer_for_npx() {
        let t = Timeouts::default();
        assert_eq!(t.hello_for("npx"), t.npx_hello);
        assert_eq!(t.hello_for("/usr/local/bin/npx"), t.npx_hello);
        assert_eq!(t.hello_for("C:\\node\\NPX.CMD"), t.npx_hello);
        assert_eq!(t.hello_for("node"), t.hello);
    }

    #[test]
    fn words_what_cannot_be_opened_as_the_app_does() {
        assert_eq!(
            needs_newer_vorn("x", Some(2)).to_string(),
            "x speaks connector protocol 2, which needs a newer Vorn"
        );
        assert_eq!(
            needs_newer_vorn("x", None).to_string(),
            "x speaks a newer connector protocol, which needs a newer Vorn"
        );
        assert!(outdated_message("Kusto").starts_with("Kusto was built for an older Vorn."));
    }

    #[tokio::test]
    async fn refuses_a_pack_without_a_protocol_before_starting_it() {
        let launch = SdkLaunch {
            launch: Launch {
                program: PathBuf::from("/nonexistent"),
                args: vec![],
                cwd: std::env::temp_dir(),
                env: vec![],
            },
            source: Source::Pack,
            protocol: None,
        };
        let err = open(&launch, "Pack", "1", Timeouts::default())
            .await
            .unwrap_err();
        assert!(matches!(err, OpenError::Outdated(_)));
        let newer = SdkLaunch {
            protocol: Some(9),
            ..launch
        };
        assert_eq!(
            open(&newer, "Pack", "1", Timeouts::default())
                .await
                .unwrap_err(),
            needs_newer_vorn("Pack", Some(9))
        );
    }

    #[tokio::test]
    async fn a_missing_command_is_a_failed_probe() {
        let probed = probe(
            "  ",
            &[],
            vec![],
            std::env::temp_dir(),
            "1",
            Timeouts::default(),
        )
        .await;
        assert_eq!(
            probed,
            json!({ "ok": false, "error": "A command is required" })
        );
        let probed = probe(
            "vorn-no-such-program",
            &["-y".into(), "pkg".into()],
            vec![],
            std::env::temp_dir(),
            "1",
            Timeouts::default(),
        )
        .await;
        assert_eq!(probed["ok"], false);
        assert!(
            probed["error"].as_str().unwrap().contains("ENOENT"),
            "{probed}"
        );
    }
}
