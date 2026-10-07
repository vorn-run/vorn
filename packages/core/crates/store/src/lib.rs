//! Vorn's store on rusqlite.
//!
//! The same database file, schema and migrations as the server's
//! `database.ts`, so either can open what the other wrote, and both can have
//! it open at once (WAL). Every call takes and returns the protocol types of
//! `vorn-protocol`, generated from the TypeScript that defines them, and
//! behaves as the TypeScript function of the same name does: the same rows
//! written, the same fields left out of what it reads back.
//!
//! [`Store::call`] answers a call by its TypeScript name with JSON arguments,
//! which is how a host that speaks JSON (the server, through napi) drives it.
//! [`ProjectHosts`] and [`AgentSettings`] are the reads a second process
//! makes without opening the store: which projects are on a remote host, and
//! how the agents are configured.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde_json::{Map, Value};
use vorn_protocol::WorkspaceConfig;

mod agents;
mod artifacts;
mod catalog;
mod config;
mod connectors;
mod dispatch;
mod hosts;
mod runs;
mod schema;
mod sessions;
mod sql;
mod tasks;
mod tokens;
mod worktrees;

pub use agents::AgentSettings;
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
    /// A call's arguments did not have the shape its TypeScript signature has.
    BadArguments {
        call: String,
        error: serde_json::Error,
    },
    /// A precondition the TypeScript also refuses, worded as it words it
    /// (`Artifact not found: <id>`).
    Refused(String),
    Io(std::io::Error),
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
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Sqlite(err) => Some(err),
            Error::Json(err) | Error::BadArguments { error: err, .. } => Some(err),
            Error::Io(err) => Some(err),
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
    /// with a new one, as `database.ts` does.
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

    /// A database in memory, as `initTestDatabase` makes: schema and
    /// migrations, but none of the seeded workflows.
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

    /// Opens `path` for a process standing beside the server (vornd), which
    /// reads and updates rows the server's schema already has: nothing is
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

    fn prepare_connection(&mut self) -> Result<()> {
        // Another process (the MCP server, the other implementation) can hold
        // the file; wait for it rather than fail the call.
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

    /// The app's defaults; refused on a store opened beside the server.
    pub(crate) fn options(&self) -> Result<&StoreOptions> {
        self.options.as_ref().ok_or_else(|| {
            Error::Refused("a store opened beside the server has none of the app's defaults".into())
        })
    }

    /// Answers the `database.ts` call named `call`, with its arguments as a
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
/// `database.ts` recovers from by starting a new one.
fn is_corrupt(err: &Error) -> bool {
    match err {
        Error::Sqlite(rusqlite::Error::SqliteFailure(failure, _)) => matches!(
            failure.code,
            rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
        ),
        _ => false,
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub fn options() -> StoreOptions {
        StoreOptions {
            default_shell: "/bin/zsh".into(),
            default_agent_commands: serde_json::from_str(
                r#"{"claude":{"command":"claude","args":[]}}"#,
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
