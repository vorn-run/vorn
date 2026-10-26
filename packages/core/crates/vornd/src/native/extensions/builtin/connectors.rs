//! The Connectors built-in in vornd: [`vorn_connectors::builtin`] on the
//! public API, an entry of the built-ins' registry ([`super::Builtins`])
//! like the others. vornd starts it before the extension host, as its calls
//! answer the app from the first moment; the registry keeps its switch, and
//! this module hands the built-in what [`Host`] asks for and refuses its
//! calls while it is off.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use futures_util::future::BoxFuture;
use futures_util::FutureExt;
use serde_json::Value;
use tracing::{info, warn};
use vorn_connectors::builtin::{self, Answer as BuiltinAnswer, Connectors};
use vorn_connectors::host::{Fields, Host, Pending, Services, TableWork};
use vorn_connectors::page;
use vorn_extensions::api::types::Event;
use vorn_extensions::api::ExtError;
use vorn_extensions::gate::{self, Caller, World};
use vorn_extensions::manifest::{file, Manifest};

use super::{Api, Builtin};
use crate::native::{secrets::Known, Answer, Native};

/// What a call to Connectors is told while it is off.
pub const OFF: &str = "Connectors is off. Turn it on in Settings → Extensions.";

/// The built-in, and whether its switch is on.
pub struct ConnectorsBuiltin {
    inner: Arc<Connectors>,
    on: AtomicBool,
    polls: Arc<vorn_extensions::polls::Polls>,
}

impl std::fmt::Debug for ConnectorsBuiltin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectorsBuiltin")
            .field("on", &self.on)
            .finish_non_exhaustive()
    }
}

/// The text of the built-in's manifest.
pub const MANIFEST: &str = builtin::MANIFEST;

/// The built-in's manifest, which it is held to.
pub fn manifest() -> Manifest {
    file::parse_builtin(MANIFEST)
        .expect("the Connectors built-in's manifest is checked by its tests")
}

impl ConnectorsBuiltin {
    /// The built-in on `native`, with the bridge's `services`, `on` as its switch was left.
    pub fn new(native: &Arc<Native>, services: Services, on: bool) -> Arc<ConnectorsBuiltin> {
        let host = Arc::new(Wired {
            native: Arc::downgrade(native),
            manifest: manifest(),
        });
        let polls = Arc::clone(&services.polls);
        let me = Arc::new(ConnectorsBuiltin {
            inner: Connectors::new(host, services),
            on: AtomicBool::new(on),
            polls,
        });
        if on {
            me.polls.offer(builtin::POLL_TRIGGER, builtin::ID);
        }
        me
    }

    /// The built-in itself, whatever its switch says.
    pub fn inner(&self) -> &Arc<Connectors> {
        &self.inner
    }

    pub fn is_on(&self) -> bool {
        self.on.load(Ordering::Acquire)
    }

    /// Refuses its calls and offers no polls from now on, as it is being
    /// switched off; [`ConnectorsBuiltin::set_on`] then stops what runs.
    fn refuse(&self) {
        self.polls.withdraw(builtin::ID);
        self.on.store(false, Ordering::Release);
    }

    /// Turns it on or off, as its switch in the registry says. Off stops
    /// every connection's child and every poll, and keeps the connections,
    /// their secrets and their inbox as they are.
    pub async fn set_on(&self, on: bool) {
        if self.on.swap(on, Ordering::AcqRel) == on {
            if !on {
                // Refused already when the switch moved; what runs still stops.
                self.inner.stop().await;
            }
            return;
        }
        if on {
            info!("[connectors] switched on");
            self.polls.offer(builtin::POLL_TRIGGER, builtin::ID);
            self.inner.reconcile().await;
        } else {
            info!("[connectors] switched off; connections are kept");
            self.polls.withdraw(builtin::ID);
            self.inner.stop().await;
        }
    }

    /// Answers one of the built-in's calls; while it is off, only those it answers off.
    pub async fn answer(&self, method: &str, params: Value) -> Answer {
        if !self.is_on() && !builtin::WHILE_OFF.contains(&method) {
            return Answer::Error(OFF.to_owned());
        }
        match self.inner.answer(method, params).await {
            BuiltinAnswer::Result(v) => Answer::Result(v),
            BuiltinAnswer::Void => Answer::Void,
            BuiltinAnswer::Error(e) => Answer::Error(e),
            BuiltinAnswer::Unanswered => Answer::Unanswered,
        }
    }

    /// `connection.itemImport` and `connection.synced`, which Tasks' step
    /// calls to put a connection's item on the board itself; refused while off.
    pub async fn items(&self, method: &str, args: &Value) -> Result<Value, ExtError> {
        if !self.is_on() {
            return Err(ExtError::new("off", OFF));
        }
        let done = if method == "connection.synced" {
            self.inner.synced(args).await.map(|()| Value::Null)
        } else {
            self.inner.item_import(args).await
        };
        done.map_err(|e| ExtError::new("failed", e))
    }

    /// A workflow step of a node type Connectors declares; refused while off.
    pub async fn run_node(&self, node_type: &str, request: &Value) -> Result<Value, String> {
        if !self.is_on() {
            return Err(OFF.to_owned());
        }
        let call = |method: &'static str, params: Value| async move {
            match self.answer(method, params).await {
                Answer::Result(v) => Ok(v),
                Answer::Void | Answer::Unanswered => Ok(Value::Null),
                Answer::Error(e) => Err(e),
            }
        };
        vorn_connectors::nodes::run(node_type, request, call)
            .await
            .ok_or_else(|| format!("Connectors runs no {node_type} steps"))
    }

    /// Settles every installed connector's implicit connection, if it is on.
    pub async fn reconcile(&self) {
        if self.is_on() {
            self.inner.reconcile().await;
        }
    }

    /// Stops every connection's child, as vornd stops.
    pub async fn stop(&self) {
        self.inner.stop().await;
    }
}

/// Connectors as an entry of the built-ins' registry.
pub fn entry(connectors: &Arc<ConnectorsBuiltin>) -> Arc<dyn Builtin> {
    Arc::new(Entry(Arc::clone(connectors)))
}

struct Entry(Arc<ConnectorsBuiltin>);

impl Builtin for Entry {
    fn manifest(&self) -> &'static str {
        MANIFEST
    }

    fn events(&self) -> &'static [&'static str] {
        &[]
    }

    fn on_by_default(&self) -> bool {
        true
    }

    fn start<'a>(&'a self, _api: &'a Api) -> BoxFuture<'a, ()> {
        self.0.set_on(true).boxed()
    }

    fn handle<'a>(
        &'a self,
        _api: &'a Api,
        action: &'a str,
        args: Value,
    ) -> BoxFuture<'a, Result<Value, ExtError>> {
        async move {
            page::act(&self.0.inner, action, &args)
                .await
                .map_err(|e| ExtError::new("failed", e))
        }
        .boxed()
    }

    fn on_events<'a>(&'a self, _api: &'a Api, _batch: Vec<Arc<Event>>) -> BoxFuture<'a, ()> {
        async {}.boxed()
    }

    fn switched(&self, on: bool) {
        if !on {
            self.0.refuse();
        }
    }

    fn stop<'a>(&'a self, _api: &'a Api) -> BoxFuture<'a, ()> {
        self.0.set_on(false).boxed()
    }

    fn mcp_tools(&self) -> Vec<Value> {
        vorn_connectors::tools::listed().to_vec()
    }

    fn mcp_call<'a>(&'a self, name: &'a str, arguments: Value) -> BoxFuture<'a, Option<Value>> {
        vorn_connectors::tools::call(&self.0.inner, name, arguments).boxed()
    }
}

/// The tools Connectors adds while vornd hosts no extensions, when it is on.
pub fn mcp_tools(native: &Native) -> Vec<Value> {
    match native.connectors() {
        Some(c) if c.is_on() => vorn_connectors::tools::listed().to_vec(),
        _ => Vec::new(),
    }
}

/// An agent's call to a tool Connectors adds, while vornd hosts no extensions.
pub async fn mcp_call(native: &Native, name: &str, arguments: Value) -> Option<Value> {
    let connectors = native.connectors().filter(|c| c.is_on())?;
    vorn_connectors::tools::call(connectors.inner(), name, arguments).await
}

/// vornd, as the built-in reaches it.
struct Wired {
    native: Weak<Native>,
    manifest: Manifest,
}

fn stopping() -> String {
    "vornd is stopping".to_owned()
}

impl Host for Wired {
    fn call(&self, method: &str, args: Value) -> Pending<Result<Value, String>> {
        let admitted = gate::admit_call(&self.manifest, method).map(|_| ());
        let world = crate::native::extensions::world::Wired {
            native: self.native.clone(),
        };
        let caller = Caller::builtin(&self.manifest);
        let method = method.to_owned();
        Box::pin(async move {
            admitted.map_err(|e| e.message)?;
            world
                .call(&caller, &method, args)
                .await
                .map_err(|e| e.message)
        })
    }

    fn tables(&self, work: TableWork) -> Pending<Result<(), String>> {
        let native = self.native.upgrade();
        Box::pin(async move {
            let native = native.ok_or_else(stopping)?;
            tokio::task::spawn_blocking(move || {
                let mut store = native.store().ok_or("vornd has no database")?;
                work(&mut store).map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| e.to_string())?
        })
    }

    fn secrets(&self, id: &str) -> Pending<Option<Fields>> {
        let (native, id) = (self.native.upgrade(), id.to_owned());
        Box::pin(async move {
            let native = native?;
            tokio::task::spawn_blocking(move || match native.secrets.lookup(&id) {
                Known::Fields(fields) => Some(fields),
                Known::None | Known::Unknown => None,
            })
            .await
            .ok()
            .flatten()
        })
    }

    fn set_secrets(&self, id: &str, fields: Fields) {
        if let Some(native) = self.native.upgrade() {
            native.secrets.set(id, fields);
        }
    }

    fn merge_secret(&self, id: &str, field: &str, value: &str) -> Pending<Result<(), String>> {
        let native = self.native.upgrade();
        let (id, field, value) = (id.to_owned(), field.to_owned(), value.to_owned());
        Box::pin(async move {
            let native = native.ok_or_else(stopping)?;
            tokio::task::spawn_blocking(move || native.secrets.merge(&id, &field, &value))
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn forget_secrets(&self, id: &str) {
        if let Some(native) = self.native.upgrade() {
            native.secrets.forget(id);
        }
    }

    fn child_env(&self) -> Vec<(String, String)> {
        self.native
            .upgrade()
            .map(|n| n.child_env())
            .unwrap_or_default()
    }

    fn find_on_path(&self, name: &str, path: &str) -> Option<PathBuf> {
        crate::native::env::find_on_path(name, path)
    }

    fn github_origin(&self, dir: &Path) -> Option<(String, String)> {
        let native = self.native.upgrade()?;
        let git = vorn_git::repo::Git {
            bin: native.env.git_bin(),
            env: native.env.get(),
            ssh: None,
        };
        git.github_origin(dir).map(|r| (r.owner, r.repo))
    }

    fn changed(&self) {
        if let Some(native) = self.native.upgrade() {
            native.signal_change();
        }
    }

    fn broadcast(&self, method: &str, params: Value) {
        if let Some(native) = self.native.upgrade() {
            native.broadcast(method, params);
        }
    }

    fn import_core_credentials(&self, params: Value) -> Pending<Result<(u64, u64), String>> {
        let native = self.native.upgrade();
        Box::pin(async move {
            let native = native.ok_or_else(stopping)?;
            let n = Arc::clone(&native);
            let imported = tokio::task::spawn_blocking(move || {
                crate::native::config::with_store(&n, |s| {
                    Ok(crate::native::credential::import(&n, s, &params))
                })?
            })
            .await
            .map_err(|e| e.to_string())??;
            if imported.1 > 0 {
                crate::native::config::announce(&native).await;
            }
            Ok(imported)
        })
    }
}

/// The bridge's services for the built-in, started once vornd has a
/// database and its own address.
pub async fn services(native: &Arc<Native>, data_dir: &Path, origin: String) -> Services {
    let proxy = if vorn_extensions::sandbox::available() {
        match vorn_extensions::netproxy::NetProxy::start().await {
            Ok(p) => Some(p),
            Err(err) => {
                warn!("[connectors] sandboxed connectors have no network: {err}");
                None
            }
        }
    } else {
        None
    };
    let home = PathBuf::from(crate::native::shell::home_dir());
    Services {
        lifecycle: Arc::clone(native.lifecycle()),
        installs: native.installs(data_dir),
        windows: Arc::clone(native.windows()),
        polls: Arc::clone(native.polls()),
        proxy,
        origin,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        catalog_cache: home.join(".vorn").join("connector-catalog.json"),
        repo_root: std::env::var_os("VORN_CONNECTORS_ROOT").map(PathBuf::from),
    }
}
