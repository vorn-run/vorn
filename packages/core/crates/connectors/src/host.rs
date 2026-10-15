//! What the Connectors built-in reaches Vorn through, and nothing else: the
//! public API's core methods, held to its manifest; the tables it has kept
//! since before extensions had a prefix of their own; the vault its
//! connections' secrets are in; and the bridge's services, handed in as
//! [`Services`].
//!
//! The daemon that hosts the built-in implements [`Host`]; tests implement it
//! over an in-memory store.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;
use vorn_extensions::install::Installs;
use vorn_extensions::lifecycle::Lifecycle;
use vorn_extensions::netproxy::NetProxy;
use vorn_extensions::polls::Polls;
use vorn_extensions::signin::Windows;
use vorn_store::Store;
use vorn_vault::Secret;

/// A connection's secrets, by field.
pub type Fields = BTreeMap<String, Secret>;

/// A future a host answers with.
pub type Pending<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// Work on the built-in's own tables.
pub type TableWork = Box<dyn FnOnce(&mut Store) -> vorn_store::Result<()> + Send + 'static>;

/// What the built-in asks of Vorn.
pub trait Host: Send + Sync + 'static {
    /// A core method of the public API (`task.importItems`), refused unless
    /// the built-in's manifest declares the permission it needs.
    fn call(&self, method: &str, args: Value) -> Pending<Result<Value, String>>;

    /// Runs `work` on the tables the built-in keeps: `source_connections`,
    /// `connector_poll_state` and the pages it adds to the inbox.
    fn tables(&self, work: TableWork) -> Pending<Result<(), String>>;

    /// Connection `id`'s secrets, `None` when it has none or the vault cannot say.
    fn secrets(&self, id: &str) -> Pending<Option<Fields>>;
    /// Replaces connection `id`'s secrets.
    fn set_secrets(&self, id: &str, fields: Fields);
    /// Sets one secret of connection `id`.
    fn merge_secret(&self, id: &str, field: &str, value: &str) -> Pending<Result<(), String>>;
    /// Forgets every secret of connection `id`.
    fn forget_secrets(&self, id: &str);

    /// The environment every program Vorn starts begins from.
    fn child_env(&self) -> Vec<(String, String)>;
    /// `name` on the search path `path`, as a shell finds it.
    fn find_on_path(&self, name: &str, path: &str) -> Option<PathBuf>;
    /// The GitHub owner and repository of the checkout at `dir`.
    fn github_origin(&self, dir: &Path) -> Option<(String, String)>;

    /// Tells every client, and the schedules, that connections or the
    /// workflows they seed changed.
    fn changed(&self);
    /// A notification to every client.
    fn broadcast(&self, method: &str, params: Value);

    /// The SSH keys and host passwords of a one-time import of what the
    /// desktop sealed, which are the core's: `(ssh keys, host passwords)`.
    fn import_core_credentials(&self, params: Value) -> Pending<Result<(u64, u64), String>>;
}

/// The bridge's services the built-in runs its connectors' programs with.
#[derive(Clone)]
pub struct Services {
    /// Every program's lifecycle: started when needed, stopped when idle.
    pub lifecycle: Arc<Lifecycle>,
    /// Installing, rolling back and removing packages, and the store they are in.
    pub installs: Arc<Installs>,
    /// The sign-in windows programs call through.
    pub windows: Arc<Windows>,
    /// Who answers polls; the built-in offers `connectorPoll`'s.
    pub polls: Arc<Polls>,
    /// The network a sandboxed program reaches its declared hosts through.
    pub proxy: Option<Arc<NetProxy>>,
    /// vornd's own address, where a program reaches its window.
    pub origin: String,
    /// The version a program is greeted with.
    pub version: String,
    /// Where the catalog is cached.
    pub catalog_cache: PathBuf,
    /// A checkout of the connectors repository to run connectors from, for their authors.
    pub repo_root: Option<PathBuf>,
}

impl std::fmt::Debug for Services {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Services")
            .field("origin", &self.origin)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}
