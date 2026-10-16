//! A version 27 file is brought up to the current schema, rows kept.

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use common::{db_file, open, raw};

const SCHEMA_27: &str = include_str!("fixtures/schema-27.sql");

/// Every table's columns (name, type, not null, default, primary key) and
/// every index's columns, by name.
fn shape(conn: &rusqlite::Connection) -> BTreeMap<String, Vec<String>> {
    let names: Vec<(String, String)> = conn
        .prepare(
            "SELECT type, name FROM sqlite_master
              WHERE type IN ('table', 'index') AND name NOT LIKE 'sqlite_%'",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    names
        .into_iter()
        .map(|(kind, name)| {
            let pragma = if kind == "table" {
                format!("PRAGMA table_info({name})")
            } else {
                format!("PRAGMA index_info({name})")
            };
            let mut stmt = conn.prepare(&pragma).unwrap();
            let count = stmt.column_count();
            let rows = stmt
                .query_map([], |row| {
                    let cells: Vec<String> = (0..count)
                        .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                        .collect();
                    Ok(cells.join(" "))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            (format!("{kind} {name}"), rows)
        })
        .collect()
}

fn seed_27(dir: &Path) {
    raw(dir).execute_batch(SCHEMA_27).unwrap();
}

fn text(dir: &Path, sql: &str) -> Option<String> {
    raw(dir).query_row(sql, [], |row| row.get(0)).unwrap()
}

fn new_database() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    drop(open(dir.path()));
    dir
}

#[test]
fn a_version_27_file_ends_up_shaped_like_a_new_one() {
    let fresh = new_database();
    let upgraded = tempfile::tempdir().unwrap();
    seed_27(upgraded.path());
    drop(open(upgraded.path()));

    let want = shape(&raw(fresh.path()));
    let got = shape(&raw(upgraded.path()));
    for (name, columns) in &want {
        assert_eq!(got.get(name), Some(columns), "{name}");
    }
    assert_eq!(
        got.keys().collect::<Vec<_>>(),
        want.keys().collect::<Vec<_>>()
    );
}

#[test]
fn the_upgrade_keeps_rows_and_records_the_new_version() {
    let dir = tempfile::tempdir().unwrap();
    seed_27(dir.path());
    let store = open(dir.path());
    assert_eq!(store.schema_version().unwrap(), 28);
    drop(store);

    let d = dir.path();
    let cases = [
        (
            "SELECT value FROM schema_meta WHERE key = 'config_revision'",
            "12",
        ),
        ("SELECT id FROM users", "u1"),
        (
            "SELECT headless_args FROM agent_commands WHERE agent_type = 'claude'",
            r#"["-p"]"#,
        ),
        (
            "SELECT agent_session_id FROM sessions WHERE id = 's1'",
            "a1",
        ),
        ("SELECT title FROM tasks WHERE id = 't1'", "A task"),
        (
            "SELECT name FROM source_connections WHERE id = 'c1'",
            "Issues",
        ),
    ];
    for (sql, want) in cases {
        assert_eq!(text(d, sql).as_deref(), Some(want), "{sql}");
    }

    let store = open(dir.path());
    assert_eq!(store.schema_version().unwrap(), 28);
}

/// Upgrades a copy of a real database, named by `VORN_UPGRADE_DB` (a
/// `vorn.db` in a directory of its own, never the live file), in place.
#[test]
fn a_copy_of_a_real_database_upgrades() {
    let Ok(copy) = std::env::var("VORN_UPGRADE_DB") else {
        return;
    };
    let copy = Path::new(&copy);
    let dir = copy.parent().unwrap();
    assert_eq!(db_file(dir), copy, "the copy must be named vorn.db");
    let count = |table: &str| -> i64 {
        raw(dir)
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    };
    let tables = [
        "sessions",
        "tasks",
        "workflows",
        "workflow_runs",
        "source_connections",
    ];
    let before: Vec<i64> = tables.iter().map(|t| count(t)).collect();

    let store = open(dir);
    assert_eq!(store.schema_version().unwrap(), 28);
    drop(store);

    let after: Vec<i64> = tables.iter().map(|t| count(t)).collect();
    assert_eq!(before, after);
    let fresh = new_database();
    assert_eq!(shape(&raw(dir)), shape(&raw(fresh.path())));
}
