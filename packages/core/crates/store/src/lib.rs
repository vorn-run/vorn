//! Vorn's store on rusqlite.
//!
//! One database file (`vorn.db`) in WAL mode, so more than one process can
//! have it open at once. Every call takes and returns the protocol types of
//! `vorn-protocol`.
//!
//! [`Store::call`] answers a call by its name with JSON arguments, which is
//! how a host that speaks JSON drives it.
//! [`ProjectHosts`] and [`AgentSettings`] are the reads a second process
//! makes without opening the store: which projects are on a remote host, and
//! how the agents are configured.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};
use serde_json::{Map, Value};
use vorn_protocol::WorkspaceConfig;

mod agents;
mod artifacts;
mod catalog;
mod config;
mod connectors;
mod dispatch;
pub mod extension;
mod hosts;
mod runs;
mod schema;
mod sessions;
mod sql;
mod tasks;
mod tokens;
mod worktrees;

pub use agents::AgentSettings;
pub use config::VIEWER_SETTING_KEYS;
pub use connectors::MAX_INBOX_ATTEMPTS;
pub use hosts::{remote_host, Placement, ProjectHost, ProjectHosts};
pub use sql::now_iso;
pub use tokens::DeviceTokens;
pub use worktrees::WorktreeSettings;

/// What the store needs from its host that is not in the database: defaults
/// the app defines, and facts about the machine.
#[derive(Clone, Debug)]
pub struct StoreOptions {
    /// `defaults.shell` when the user has not chosen one.
    pub default_shell: String,
    /// `agentCommands` when the table is empty, by agent type.
    pub default_agent_commands: Map<String, Value>,
    /// The workspace every database has.
    pub default_workspace: WorkspaceConfig,
    /// The name the owner is seeded with on a new database.
    pub owner_name: String,
    /// Workflows seeded once each, the first time the store opens.
    pub seed_workflows: Vec<SeedWorkflow>,
}

/// A workflow the app ships, inserted once: `flag` in `defaults` records that
/// it was, so deleting it sticks.
#[derive(Clone, Debug)]
pub struct SeedWorkflow {
    pub flag: String,
    pub workflow: vorn_protocol::WorkflowDefinition,
}

#[derive(Debug)]
pub enum Error {
    Sqlite(rusqlite::Error),
    /// A column holding JSON did not parse, or a value did not serialize.
    Json(serde_json::Error),
    /// A call named something the store does not answer.
    UnknownCall(String),
    /// A call's arguments did not have the shape its signature has.
    BadArguments {
        call: String,
        error: serde_json::Error,
    },
    /// A call refused because a precondition does not hold, or the file
    /// cannot be opened (`Artifact not found: <id>`).
    Refused(String),
    Io(std::io::Error),
    /// Statement `index` (from 0) of a transaction failed, so none of it held.
    Statement {
        index: usize,
        error: Box<Error>,
    },
}

impl Error {
    /// Whether it is a refusal, in a statement of a transaction or not.
    pub fn is_refused(&self) -> bool {
        match self {
            Error::Refused(_) => true,
            Error::Statement { error, .. } => error.is_refused(),
            _ => false,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Sqlite(err) => write!(f, "{err}"),
            Error::Json(err) => write!(f, "{err}"),
            Error::UnknownCall(call) => write!(f, "the store has no call {call}"),
            Error::BadArguments { call, error } => write!(f, "{call}: {error}"),
            Error::Refused(message) => f.write_str(message),
            Error::Io(err) => write!(f, "{err}"),
            Error::Statement { index, error } => write!(f, "statement {index}: {error}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Sqlite(err) => Some(err),
            Error::Json(err) | Error::BadArguments { error: err, .. } => Some(err),
            Error::Io(err) => Some(err),
            Error::Statement { error, .. } => Some(error.as_ref()),
            Error::UnknownCall(_) | Error::Refused(_) => None,
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(err: rusqlite::Error) -> Self {
        Error::Sqlite(err)
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::Json(err)
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Io(err)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// One open database.
pub struct Store {
    conn: Connection,
    /// `None` for a store opened beside the server ([`Store::open_beside`]).
    options: Option<StoreOptions>,
    /// The file, or `None` in memory.
    path: Option<PathBuf>,
}

/// How [`Store::open`] went.
#[derive(Debug, PartialEq, Eq)]
pub enum Opened {
    /// The file opened (or was created) and is current.
    Ok,
    /// The file was not a database. It was copied to `backup` and replaced
    /// with a new one.
    Recovered { backup: PathBuf },
}

impl Store {
    /// Opens `path` (`vorn.db` in the data directory), creating it if needed,
    /// and brings its schema up to date.
    pub fn open(path: &Path, options: StoreOptions) -> Result<(Store, Opened)> {
        match Store::open_file(path, options.clone()) {
            Ok(store) => Ok((store, Opened::Ok)),
            Err(err) if is_corrupt(&err) => {
                let backup = schema::set_aside_corrupt(path)?;
                let store = Store::open_file(path, options)?;
                Ok((store, Opened::Recovered { backup }))
            }
            Err(err) => Err(err),
        }
    }

    /// A database in memory with the schema but none of the seeded
    /// workflows.
    pub fn open_in_memory(options: StoreOptions) -> Result<Store> {
        let conn = Connection::open_in_memory()?;
        let mut store = Store {
            conn,
            options: Some(options),
            path: None,
        };
        store.prepare_connection()?;
        schema::create(&mut store)?;
        Ok(store)
    }

    fn open_file(path: &Path, options: StoreOptions) -> Result<Store> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let mut store = Store {
            conn,
            options: Some(options),
            path: Some(path.to_owned()),
        };
        store.prepare_connection()?;
        schema::create(&mut store)?;
        schema::seed_system_defaults(&mut store)?;
        Ok(store)
    }

    /// Opens `path` for a process other than the server (vornd), which reads
    /// and updates rows the server's schema already has: nothing is
    /// created, migrated or seeded, and the journal mode is left as the
    /// server set it. `None` when there is no such file yet.
    ///
    /// Only for calls on rows. The app's defaults are the server's to supply,
    /// so a call that falls back on them (`loadConfig`) is not for this store.
    pub fn open_beside(path: &Path) -> Result<Option<Store>> {
        if !path.exists() {
            return Ok(None);
        }
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(Some(Store {
            conn,
            options: None,
            path: Some(path.to_owned()),
        }))
    }

    /// Gives a store opened beside the server the app's defaults, so the
    /// calls that fall back on them (`loadConfig`, `saveConfig`) can be made.
    pub fn with_defaults(mut self, options: StoreOptions) -> Store {
        self.options = Some(options);
        self
    }

    fn prepare_connection(&mut self) -> Result<()> {
        // Another process (the MCP server, the CLI) can hold the file; wait
        // for it rather than fail the call.
        self.conn.busy_timeout(std::time::Duration::from_secs(5))?;
        self.conn.pragma_update(None, "journal_mode", "WAL")?;
        self.conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(())
    }

    /// The database file, or `None` in memory.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    pub(crate) fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// A transaction for a write; see [`write_transaction`].
    pub(crate) fn write_transaction(&mut self) -> Result<Transaction<'_>> {
        write_transaction(&mut self.conn)
    }

    /// The app's defaults; refused on a store opened beside the server.
    pub(crate) fn options(&self) -> Result<&StoreOptions> {
        self.options.as_ref().ok_or_else(|| {
            Error::Refused("a store opened beside the server has none of the app's defaults".into())
        })
    }

    /// Answers the call named `call`, with its arguments as a
    /// JSON array in signature order, and returns what it returns.
    /// `undefined` is `null` both ways.
    pub fn call(&mut self, call: &str, args: Value) -> Result<Value> {
        dispatch::call(self, call, args)
    }

    /// The schema version recorded in the file.
    pub fn schema_version(&self) -> Result<i64> {
        schema::version(&self.conn)
    }
}

/// Whether opening failed because the file is not a database, the case
/// [`Store::open`] recovers from by starting a new one.
fn is_corrupt(err: &Error) -> bool {
    match err {
        Error::Sqlite(rusqlite::Error::SqliteFailure(failure, _)) => matches!(
            failure.code,
            rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
        ),
        _ => false,
    }
}

/// Begins a transaction that takes the write lock up front (`BEGIN IMMEDIATE`).
///
/// More than one process writes the file. A deferred transaction that
/// reads and then writes holds a snapshot, and once the other process
/// commits, SQLite refuses to upgrade that snapshot to a write
/// (`SQLITE_BUSY_SNAPSHOT`) without calling the busy handler, so
/// `busy_timeout` cannot help and the call fails with "database is locked".
/// Taking the lock first makes it wait its turn within the timeout instead.
pub(crate) fn write_transaction(conn: &mut Connection) -> Result<Transaction<'_>> {
    Ok(conn.transaction_with_behavior(TransactionBehavior::Immediate)?)
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub fn options() -> StoreOptions {
        StoreOptions {
            default_shell: "/bin/zsh".into(),
            default_agent_commands: serde_json::from_str(
                r#"{"acme":{"command":"acme","args":[]}}"#,
            )
            .expect("valid JSON"),
            default_workspace: serde_json::from_value(serde_json::json!({
                "id": "personal",
                "name": "Personal",
                "icon": "User",
                "iconColor": "#6b7280",
                "order": 0
            }))
            .expect("a workspace"),
            owner_name: "owner".into(),
            seed_workflows: Vec::new(),
        }
    }

    pub fn store() -> Store {
        Store::open_in_memory(options()).expect("an in-memory store opens")
    }
}
