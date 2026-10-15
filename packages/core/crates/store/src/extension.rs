//! An extension's own tables in `vorn.db`, behind SQLite's authorizer.
//!
//! Every table an extension makes is named with its prefix, `x_<id>_`, the id
//! escaped so that no two ids share one ([`table_prefix`]). A statement it
//! sends is prepared with an authorizer that allows reading and writing only
//! tables with that prefix and denies the rest by name: another extension's
//! tables, the core's, the schema, pragmas, attachments and transactions. A
//! small key-value store and the record of which migrations ran sit beside
//! them, in tables only the host reaches. The schema itself may be read: it
//! holds no rows, SQLite reads it to change a table, and it cannot be written
//! directly.

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::types::{Value as Sql, ValueRef};
use rusqlite::{params_from_iter, Connection, OpenFlags, OptionalExtension};
use serde_json::{Map, Number, Value};

use crate::{Error, Result};

/// The most rows one query answers.
pub const MAX_ROWS: usize = 10_000;

/// How long a statement waits on another writer before it fails.
const BUSY: Duration = Duration::from_secs(5);

/// The prefix of every table extension `id` owns: `x_`, the id escaped, `_`.
///
/// Letters and digits stand for themselves; `.`, `-` and `_` become `_0`,
/// `_1` and `_2`, and a capital `_3` and its lowercase, since SQLite folds
/// case. After the prefix an extension's own name starts with a letter, so no
/// id's prefix begins another's tables.
pub fn table_prefix(id: &str) -> String {
    let mut out = String::with_capacity(id.len() + 4);
    out.push_str("x_");
    for c in id.chars() {
        match c {
            'a'..='z' | '0'..='9' => out.push(c),
            'A'..='Z' => {
                out.push_str("_3");
                out.push(c.to_ascii_lowercase());
            }
            '.' => out.push_str("_0"),
            '-' => out.push_str("_1"),
            _ => out.push_str("_2"),
        }
    }
    out.push('_');
    out
}

/// Whether `table` is one an extension with `prefix` made itself.
fn owns(prefix: &str, table: &str) -> bool {
    let table = table.to_ascii_lowercase();
    table
        .strip_prefix(prefix)
        .and_then(|rest| rest.chars().next())
        .is_some_and(|c| c.is_ascii_lowercase())
}

/// The host's own tables beside an extension's: past the prefix they start with `_`.
fn kv_table(prefix: &str) -> String {
    format!("{prefix}_kv")
}

fn migrations_table(prefix: &str) -> String {
    format!("{prefix}_migrations")
}

fn in_main(ctx: &AuthContext<'_>) -> bool {
    ctx.database_name.is_none_or(|d| d == "main")
}

/// What the authorizer decides for `ctx`, and why when it refuses.
fn decide(prefix: &str, ctx: &AuthContext<'_>) -> std::result::Result<(), String> {
    let schema = |t: &str| {
        t.eq_ignore_ascii_case("sqlite_master") || t.eq_ignore_ascii_case("sqlite_schema")
    };
    let mine = |t: &str| owns(prefix, t) && in_main(ctx);
    // A change to its own table writes the schema, which SQLite allows no statement to write directly.
    let internal = |t: &str| schema(t);
    match ctx.action {
        AuthAction::Select | AuthAction::Recursive | AuthAction::Function { .. } => Ok(()),
        // The schema holds no rows, and Vorn's own is public; SQLite reads it to change a table.
        AuthAction::Read { table_name, .. } if mine(table_name) || schema(table_name) => Ok(()),
        AuthAction::Read { table_name, .. } => Err(format!("read the table {table_name}")),
        AuthAction::Insert { table_name }
        | AuthAction::Delete { table_name }
        | AuthAction::Update { table_name, .. }
            if mine(table_name) || internal(table_name) =>
        {
            Ok(())
        }
        AuthAction::Insert { table_name }
        | AuthAction::Delete { table_name }
        | AuthAction::Update { table_name, .. } => Err(format!("write the table {table_name}")),
        AuthAction::CreateTable { table_name }
        | AuthAction::DropTable { table_name }
        | AuthAction::AlterTable { table_name, .. }
        | AuthAction::Analyze { table_name }
            if mine(table_name) =>
        {
            Ok(())
        }
        AuthAction::CreateIndex {
            index_name,
            table_name,
        }
        | AuthAction::DropIndex {
            index_name,
            table_name,
        } if mine(table_name) && owns(prefix, index_name) => Ok(()),
        AuthAction::CreateTable { table_name }
        | AuthAction::DropTable { table_name }
        | AuthAction::AlterTable { table_name, .. }
        | AuthAction::Analyze { table_name }
        | AuthAction::CreateIndex { table_name, .. }
        | AuthAction::DropIndex { table_name, .. } => {
            Err(format!("change the schema of {table_name}"))
        }
        AuthAction::Pragma { pragma_name, .. } => Err(format!("run PRAGMA {pragma_name}")),
        AuthAction::Attach { .. } | AuthAction::Detach { .. } => {
            Err("attach a database".to_owned())
        }
        AuthAction::Transaction { .. } | AuthAction::Savepoint { .. } => {
            Err("open a transaction; each call is one".to_owned())
        }
        AuthAction::Reindex { index_name } if owns(prefix, index_name) => Ok(()),
        _ => Err("run that statement".to_owned()),
    }
}

/// One extension's tables, on a connection of its own.
pub struct ExtensionTables {
    conn: Connection,
    id: String,
    prefix: String,
    /// What the authorizer refused last, to name it in the error.
    refused: Arc<Mutex<Option<String>>>,
}

impl std::fmt::Debug for ExtensionTables {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtensionTables")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

fn to_sql(v: &Value) -> Sql {
    match v {
        Value::Null => Sql::Null,
        Value::Bool(b) => Sql::Integer(i64::from(*b)),
        Value::Number(n) => n
            .as_i64()
            .map(Sql::Integer)
            .unwrap_or_else(|| Sql::Real(n.as_f64().unwrap_or(f64::NAN))),
        Value::String(s) => Sql::Text(s.clone()),
        other => Sql::Text(other.to_string()),
    }
}

fn from_sql(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::from(i),
        ValueRef::Real(f) => Number::from_f64(f).map_or(Value::Null, Value::Number),
        ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Value::Array(b.iter().map(|x| Value::from(*x)).collect()),
    }
}

impl ExtensionTables {
    /// Opens `db` for extension `id`, its statements held to its own tables.
    pub fn open(db: &Path, id: &str) -> Result<ExtensionTables> {
        let conn = Connection::open_with_flags(
            db,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(BUSY)?;
        let prefix = table_prefix(id);
        let (kv, migrations) = (kv_table(&prefix), migrations_table(&prefix));
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS \"{kv}\" (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS \"{migrations}\" (name TEXT PRIMARY KEY, applied_at TEXT NOT NULL);"
        ))?;
        let tables = ExtensionTables {
            conn,
            id: id.to_owned(),
            prefix,
            refused: Arc::default(),
        };
        tables.guard(true);
        Ok(tables)
    }

    /// The prefix every table of its own carries.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Puts the authorizer on, or takes it off for the host's own statements.
    fn guard(&self, on: bool) {
        if !on {
            self.conn
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
            return;
        }
        let (prefix, refused) = (self.prefix.clone(), Arc::clone(&self.refused));
        self.conn.authorizer(Some(move |ctx: AuthContext<'_>| {
            match decide(&prefix, &ctx) {
                Ok(()) => Authorization::Allow,
                Err(why) => {
                    *refused.lock().unwrap_or_else(PoisonError::into_inner) = Some(why);
                    Authorization::Deny
                }
            }
        }));
    }

    /// An error the authorizer caused, worded with what it refused.
    fn named(&self, err: rusqlite::Error) -> Error {
        let refused = self
            .refused
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        match (refused, err.sqlite_error_code()) {
            (Some(why), Some(rusqlite::ErrorCode::AuthorizationForStatementDenied)) => {
                Error::Refused(format!(
                    "{} may not {why}: its tables are named {}…",
                    self.id, self.prefix
                ))
            }
            _ => Error::Sqlite(err),
        }
    }

    /// The rows `sql` answers, each an object keyed by column.
    pub fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Value>> {
        let mut stmt = self.conn.prepare(sql).map_err(|e| self.named(e))?;
        let names: Vec<String> = stmt
            .column_names()
            .iter()
            .map(|n| (*n).to_owned())
            .collect();
        let mut rows = stmt
            .query(params_from_iter(params.iter().map(to_sql)))
            .map_err(|e| self.named(e))?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(|e| self.named(e))? {
            if out.len() == MAX_ROWS {
                return Err(Error::Refused(format!(
                    "a query answers at most {MAX_ROWS} rows; page it with LIMIT"
                )));
            }
            let mut object = Map::with_capacity(names.len());
            for (i, name) in names.iter().enumerate() {
                object.insert(name.clone(), from_sql(row.get_ref(i)?));
            }
            out.push(Value::Object(object));
        }
        Ok(out)
    }

    /// Runs one statement; answers how many rows it changed.
    pub fn exec(&self, sql: &str, params: &[Value]) -> Result<u64> {
        let mut stmt = self.conn.prepare(sql).map_err(|e| self.named(e))?;
        let changed = stmt
            .execute(params_from_iter(params.iter().map(to_sql)))
            .map_err(|e| self.named(e))?;
        Ok(u64::try_from(changed).unwrap_or(u64::MAX))
    }

    /// Runs the `.sql` files in `dir` not yet run, in name order, each in a
    /// transaction of its own; answers the ones it ran.
    pub fn migrate(&self, dir: &Path) -> Result<Vec<String>> {
        let mut files: Vec<String> = std::fs::read_dir(dir)?
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.ends_with(".sql"))
            .collect();
        files.sort();
        let table = migrations_table(&self.prefix);
        let mut ran = Vec::new();
        for name in files {
            self.guard(false);
            let done: Option<String> = self
                .conn
                .query_row(
                    &format!("SELECT name FROM \"{table}\" WHERE name = ?"),
                    [&name],
                    |r| r.get(0),
                )
                .optional()?;
            if done.is_some() {
                continue;
            }
            let sql = std::fs::read_to_string(dir.join(&name))?;
            self.conn.execute_batch("BEGIN IMMEDIATE")?;
            self.guard(true);
            let applied = self.conn.execute_batch(&sql).map_err(|e| self.named(e));
            self.guard(false);
            let recorded = applied.and_then(|()| {
                self.conn
                    .execute(
                        &format!("INSERT INTO \"{table}\" (name, applied_at) VALUES (?, ?)"),
                        [&name, &crate::now_iso()],
                    )
                    .map_err(Error::from)
            });
            match recorded {
                Ok(_) => self.conn.execute_batch("COMMIT")?,
                Err(err) => {
                    let _ = self.conn.execute_batch("ROLLBACK");
                    self.guard(true);
                    return Err(match err {
                        Error::Refused(why) => Error::Refused(format!("{name}: {why}")),
                        other => Error::Refused(format!("{name}: {other}")),
                    });
                }
            }
            ran.push(name);
        }
        self.guard(true);
        Ok(ran)
    }

    /// The value kept under `key`.
    pub fn get(&self, key: &str) -> Result<Option<Value>> {
        self.guard(false);
        let text: rusqlite::Result<Option<String>> = self
            .conn
            .query_row(
                &format!(
                    "SELECT value FROM \"{}\" WHERE key = ?",
                    kv_table(&self.prefix)
                ),
                [key],
                |r| r.get(0),
            )
            .optional();
        self.guard(true);
        Ok(text?.map(|t| serde_json::from_str(&t)).transpose()?)
    }

    /// Keeps `value` under `key`; `null` removes it.
    pub fn set(&self, key: &str, value: &Value) -> Result<()> {
        let kv = kv_table(&self.prefix);
        self.guard(false);
        let done = if value.is_null() {
            self.conn
                .execute(&format!("DELETE FROM \"{kv}\" WHERE key = ?"), [key])
        } else {
            self.conn.execute(
                &format!("INSERT OR REPLACE INTO \"{kv}\" (key, value) VALUES (?, ?)"),
                [key, &value.to_string()],
            )
        };
        self.guard(true);
        done?;
        Ok(())
    }

    /// Drops every table of extension `id`, its key-value store and its migrations with them.
    pub fn forget(db: &Path, id: &str) -> Result<usize> {
        let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        conn.busy_timeout(BUSY)?;
        let prefix = table_prefix(id);
        let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?;
        let tables: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .filter(|t| t.to_ascii_lowercase().starts_with(&prefix))
            .collect();
        drop(stmt);
        for t in &tables {
            conn.execute_batch(&format!(
                "DROP TABLE IF EXISTS \"{}\"",
                t.replace('"', "\"\"")
            ))?;
        }
        Ok(tables.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn db() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vorn.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE tasks (id TEXT PRIMARY KEY, title TEXT);
             INSERT INTO tasks VALUES ('t1', 'secret');",
        )
        .unwrap();
        (dir, path)
    }

    #[test]
    fn escapes_ids_so_no_two_share_a_prefix() {
        assert_eq!(table_prefix("example.pr-checks"), "x_example_0pr_1checks_");
        assert_ne!(table_prefix("a.b"), table_prefix("a_b"));
        assert_ne!(table_prefix("a-b"), table_prefix("a.b"));
        assert_ne!(table_prefix("A"), table_prefix("a"));
        let a = table_prefix("a");
        assert!(owns(&a, "x_a_rows"));
        assert!(owns(&a, "X_A_Rows"));
        // `a`'s prefix starts `a_0x`'s, but its own names start with a letter.
        assert!(!owns(&a, &format!("{}rows", table_prefix("a.x"))));
        assert!(!owns(&a, "x_a__kv"));
        assert!(!owns(&a, "tasks"));
    }

    #[test]
    fn reads_and_writes_its_own_tables_and_nothing_else() {
        let (_dir, path) = db();
        let t = ExtensionTables::open(&path, "test.x").unwrap();
        let rows = format!("{}rows", t.prefix());
        t.exec(
            &format!("CREATE TABLE {rows} (id INTEGER PRIMARY KEY, name TEXT, n REAL)"),
            &[],
        )
        .unwrap();
        t.exec(&format!("CREATE INDEX {rows}_name ON {rows} (name)"), &[])
            .unwrap();
        assert_eq!(
            t.exec(
                &format!("INSERT INTO {rows} (name, n) VALUES (?, ?)"),
                &[json!("a"), json!(1.5)]
            )
            .unwrap(),
            1
        );
        t.exec(&format!("ALTER TABLE {rows} ADD COLUMN extra TEXT"), &[])
            .unwrap();
        assert_eq!(
            t.query(
                &format!("SELECT name, n, extra FROM {rows} WHERE name = ?"),
                &[json!("a")]
            )
            .unwrap(),
            [json!({ "name": "a", "n": 1.5, "extra": null })]
        );
        for (sql, why) in [
            (
                "SELECT title FROM tasks",
                "test.x may not read the table tasks",
            ),
            (
                "INSERT INTO tasks VALUES ('t2', 'x')",
                "test.x may not write the table tasks",
            ),
            ("DELETE FROM tasks", "test.x may not write the table tasks"),
            (
                "DROP TABLE tasks",
                "test.x may not change the schema of tasks",
            ),
            (
                "PRAGMA table_info(tasks)",
                "test.x may not run PRAGMA table_info",
            ),
            (
                "ATTACH DATABASE ':memory:' AS other",
                "test.x may not attach a database",
            ),
            (
                "BEGIN",
                "test.x may not open a transaction; each call is one",
            ),
            (
                "CREATE TABLE notmine (a)",
                "test.x may not change the schema of notmine",
            ),
        ] {
            let err = t.exec(sql, &[]).unwrap_err().to_string();
            assert!(err.starts_with(why), "{sql}: {err}");
        }
        let err = t
            .exec(
                &format!("CREATE VIEW {}v AS SELECT * FROM tasks", t.prefix()),
                &[],
            )
            .unwrap_err();
        assert!(err.to_string().contains("may not"), "{err}");
        let other = ExtensionTables::open(&path, "test.y").unwrap();
        let err = other
            .query(&format!("SELECT * FROM {rows}"), &[])
            .unwrap_err();
        assert!(
            err.to_string()
                .starts_with(&format!("test.y may not read the table {rows}")),
            "{err}"
        );
    }

    #[test]
    fn keeps_values_runs_each_migration_once_and_forgets_everything() {
        let (dir, path) = db();
        let t = ExtensionTables::open(&path, "test.x").unwrap();
        t.set("cursor", &json!({ "at": 3 })).unwrap();
        assert_eq!(t.get("cursor").unwrap(), Some(json!({ "at": 3 })));
        t.set("cursor", &Value::Null).unwrap();
        assert_eq!(t.get("cursor").unwrap(), None);

        let migrations = dir.path().join("migrations");
        std::fs::create_dir(&migrations).unwrap();
        std::fs::write(
            migrations.join("001_rows.sql"),
            "CREATE TABLE x_test_0x_rows (a); INSERT INTO x_test_0x_rows VALUES (1);",
        )
        .unwrap();
        assert_eq!(t.migrate(&migrations).unwrap(), ["001_rows.sql"]);
        assert!(t.migrate(&migrations).unwrap().is_empty());
        std::fs::write(
            migrations.join("002_bad.sql"),
            "CREATE TABLE x_test_0x_more (a); DELETE FROM tasks;",
        )
        .unwrap();
        let err = t.migrate(&migrations).unwrap_err().to_string();
        assert_eq!(
            err,
            "002_bad.sql: test.x may not write the table tasks: its tables are named x_test_0x_…"
        );
        // The failed migration left nothing behind.
        assert!(t.query("SELECT * FROM x_test_0x_more", &[]).is_err());
        assert_eq!(
            t.query("SELECT a FROM x_test_0x_rows", &[]).unwrap(),
            [json!({ "a": 1 })]
        );

        drop(t);
        assert_eq!(ExtensionTables::forget(&path, "test.x").unwrap(), 3);
        let conn = Connection::open(&path).unwrap();
        let left: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name LIKE 'x_%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(left, 0);
        let title: String = conn
            .query_row("SELECT title FROM tasks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(title, "secret");
    }
}
