//! What the Tasks built-in reaches Vorn through, and nothing else: the
//! tables it has kept since before extensions had a prefix of their own,
//! the public events, the core's calls (the task list among them), the rows
//! its templates draw, and the clients it tells what changed.
//!
//! The daemon that hosts the built-in implements [`Host`]; tests implement
//! it over a store of their own.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use serde_json::Value;
use vorn_store::Store;

/// A future a host answers with.
pub type Pending<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// Work on the built-in's own tables, run where blocking is allowed.
pub type TableWork = Box<dyn FnOnce(&mut Store) -> vorn_store::Result<()> + Send + 'static>;

/// What a write did to the board, as clients are told it.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// These rows, whole, as `task:get` reads them: new or changed.
    Rows(Vec<Value>),
    /// These tasks are gone.
    Removed(Vec<String>),
}

/// What the built-in asks of Vorn.
pub trait Host: Send + Sync + 'static {
    /// Runs `work` on the board's tables: `tasks` and `task_source_links`.
    fn tables(&self, work: TableWork) -> Pending<Result<(), String>>;

    /// The folder `vorn.db` is in, under which each task's images are kept.
    fn data_dir(&self) -> Option<PathBuf>;

    /// A public event (`task.created`, `task.statusChanged`), which the
    /// workflows' triggers and any extension that subscribes hear.
    fn emit(&self, name: &str, data: Value);

    /// Tells every client which tasks changed, once they are stored.
    fn changed(&self, change: Change) -> Pending<()>;

    /// A core call, as a client makes it: the task list and the agents that
    /// are on, for the tools.
    fn call(&self, method: &str, params: Value) -> Pending<Result<Value, String>>;

    /// Row ops for `collection`, which the built-in's templates draw.
    fn publish(&self, collection: &str, ops: Vec<vorn_extensions::api::types::RowOp>);
}
