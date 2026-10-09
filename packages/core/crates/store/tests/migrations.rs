//! Opening a database an older Vorn wrote: the migrations that rewrite it, and recovery from a corrupt file.

mod common;

use std::path::Path;

use common::{call, db_file, migrate, open, options, raw};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use vorn_store::{Opened, Store};

fn dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temp dir")
}

fn schema_version(dir: &Path) -> i64 {
    let version: Option<String> = raw(dir)
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .optional()
        .expect("schema_meta reads");
    version.map_or(0, |v| v.parse().expect("a number"))
}

fn set_schema_version(conn: &rusqlite::Connection, version: i64) {
    conn.execute(
        "INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('schema_version', ?1)",
        params![version.to_string()],
    )
    .expect("the version is set");
}

fn owner(store: &mut Store) -> Value {
    call(store, "dbGetOwnerUser", json!([]))
}

mod identity {
    use super::*;

    /// The version identity landed on: later migrations may move past it.
    const IDENTITY_SCHEMA_VERSION: i64 = 14;

    fn count_users(dir: &Path) -> i64 {
        raw(dir)
            .query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))
            .expect("users count")
    }

    #[test]
    fn a_fresh_database_lands_on_14_with_one_owner() {
        let dir = dir();
        migrate(dir.path());
        assert!(schema_version(dir.path()) >= IDENTITY_SCHEMA_VERSION);
        assert_eq!(count_users(dir.path()), 1);
    }

    #[test]
    fn migrating_a_version_13_database_seeds_the_owner() {
        let dir = dir();
        migrate(dir.path());
        {
            let conn = raw(dir.path());
            conn.execute("DELETE FROM users", []).expect("users go");
            set_schema_version(&conn, 13);
        }
        assert_eq!(count_users(dir.path()), 0);
        assert_eq!(schema_version(dir.path()), IDENTITY_SCHEMA_VERSION - 1);

        let owner = owner(&mut open(dir.path()));

        assert_eq!(owner["role"], "owner");
        assert!(schema_version(dir.path()) >= IDENTITY_SCHEMA_VERSION);
        assert_eq!(count_users(dir.path()), 1);
    }

    #[test]
    fn reopening_seeds_no_second_owner() {
        let dir = dir();
        let first = owner(&mut open(dir.path()));
        let second = owner(&mut open(dir.path()));
        assert_eq!(second["id"], first["id"]);
        assert_eq!(count_users(dir.path()), 1);
    }

    #[test]
    fn a_renamed_owner_is_not_seeded_again() {
        let dir = dir();
        migrate(dir.path());
        raw(dir.path())
            .execute("UPDATE users SET name = ?1", ["renamed"])
            .expect("the owner is renamed");

        let owner = owner(&mut open(dir.path()));

        assert_eq!(owner["name"], "renamed");
        assert_eq!(count_users(dir.path()), 1);
    }

    /// What `mintOwnerToken` refused on: a migrated database that lost its owner has none, rather than a new one seeded behind the user's back.
    #[test]
    fn a_migrated_database_that_lost_its_owner_answers_none() {
        let dir = dir();
        migrate(dir.path());
        raw(dir.path())
            .execute_batch("DELETE FROM device_tokens; DELETE FROM users;")
            .expect("the owner goes");

        assert!(owner(&mut open(dir.path())).is_null());
    }

    #[test]
    fn an_unreadable_file_is_backed_up_and_replaced_rather_than_failing_to_open() {
        let dir = dir();
        std::fs::write(db_file(dir.path()), "this is not a sqlite database")
            .expect("the file is written");

        let (mut store, opened) =
            Store::open(&db_file(dir.path()), options()).expect("the store opens");
        let owner = owner(&mut store);
        drop(store);

        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .expect("the dir lists")
            .map(|e| e.expect("an entry").path())
            .filter(|p| p.to_string_lossy().contains(".corrupt-"))
            .collect();
        assert_eq!(backups.len(), 1, "{backups:?}");
        assert_eq!(
            opened,
            Opened::Recovered {
                backup: backups[0].clone()
            }
        );
        assert_eq!(
            std::fs::read_to_string(&backups[0]).expect("the backup reads"),
            "this is not a sqlite database"
        );
        // Rebuilt, migrated and seeded, not merely opened.
        assert!(schema_version(dir.path()) >= IDENTITY_SCHEMA_VERSION);
        assert_eq!(owner["role"], "owner");
    }

    #[test]
    fn the_database_lands_in_the_directory_given() {
        let dir = dir();
        migrate(dir.path());
        assert!(db_file(dir.path()).exists());
    }
}

mod packaged_task_sources {
    //! Migration 17: a packaged connector's tasks were recorded under `mcp`; they move to the connector they really came from.
    use super::*;

    const INSERT_TASK: &str = r#"INSERT INTO tasks
        (id, title, status, "order", project_name, created_at, updated_at, source_connector_id, source_external_id)
        VALUES (?1, ?2, 'todo', ?3, 'Novum', '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z', ?4, ?5)"#;

    /// A database before the migration: a packaged connection, a task from it recorded under `mcp`, and the link between them.
    fn seed_packaged_task(dir: &Path, with_connection: bool) {
        let conn = raw(dir);
        if with_connection {
            conn.execute(
                "INSERT INTO source_connections
                   (id, connector_id, name, filters, sync_interval_minutes, status_mapping, created_at)
                 VALUES ('conn-1', 'mcp', 'Pack Demo', ?1, 5, '{}', '2026-09-01T00:00:00Z')",
                [json!({ "sdkConnectorId": "packdemo" }).to_string()],
            )
            .expect("the connection is written");
        }
        conn.execute(INSERT_TASK, params!["task-1", "Tick 7", 1, "mcp", "7"])
            .expect("the task is written");
        if with_connection {
            conn.execute(
                "INSERT INTO task_source_links
                   (task_id, connection_id, connector_id, external_id, external_url,
                    source_status_raw, source_updated_at, last_synced_at)
                 VALUES ('task-1', 'conn-1', 'mcp', '7', '', 'open', '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z')",
                [],
            )
            .expect("the link is written");
        }
        set_schema_version(&conn, 16);
    }

    fn task_connector(dir: &Path, id: &str) -> Option<String> {
        raw(dir)
            .query_row(
                "SELECT source_connector_id FROM tasks WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .expect("the task reads")
    }

    fn link_connector(dir: &Path) -> Option<String> {
        raw(dir)
            .query_row(
                "SELECT connector_id FROM task_source_links WHERE task_id = 'task-1'",
                [],
                |r| r.get(0),
            )
            .optional()
            .expect("the link reads")
    }

    #[test]
    fn mcp_is_rewritten_to_the_connector_the_task_came_from() {
        let dir = dir();
        migrate(dir.path());
        seed_packaged_task(dir.path(), true);

        migrate(dir.path());

        assert_eq!(
            task_connector(dir.path(), "task-1").as_deref(),
            Some("packdemo")
        );
        assert_eq!(link_connector(dir.path()).as_deref(), Some("packdemo"));
    }

    #[test]
    fn a_task_is_left_alone_when_nothing_says_which_connector_it_was() {
        let dir = dir();
        migrate(dir.path());
        seed_packaged_task(dir.path(), false);

        migrate(dir.path());

        assert_eq!(task_connector(dir.path(), "task-1").as_deref(), Some("mcp"));
    }

    #[test]
    fn a_built_in_connectors_task_is_left_exactly_as_it_was() {
        let dir = dir();
        migrate(dir.path());
        {
            let conn = raw(dir.path());
            conn.execute(INSERT_TASK, params!["task-2", "Issue 3", 2, "github", "3"])
                .expect("the task is written");
            set_schema_version(&conn, 16);
        }

        migrate(dir.path());

        assert_eq!(
            task_connector(dir.path(), "task-2").as_deref(),
            Some("github")
        );
    }
}

mod package_connections_to_sdk {
    //! Migration 21: connections to a connector package move from `mcp` to `sdk` under the id they had, losing what only MCP needed.
    use super::*;

    const ICON: &str = r#"{"viewBox":"0 0 24 24","paths":["M0 0h24v24H0z"]}"#;

    fn poll_filters() -> Value {
        json!({
            "itemsPath": "items", "idField": "externalId", "timestampField": "updatedAt",
            "titleField": "title", "urlField": "url", "cursorArg": "cursor",
            "cursorPath": "nextCursor"
        })
    }

    fn tools() -> Value {
        json!([{ "name": "vorn_connector_manifest" }, { "name": "echo" }])
    }

    fn launch(package: &str) -> Value {
        json!({ "command": "npx", "args": format!(r#"["-y","{package}"]"#), "env": "{}" })
    }

    fn merged(parts: &[Value]) -> Value {
        let mut out = serde_json::Map::new();
        for part in parts {
            for (k, v) in part.as_object().expect("an object") {
                out.insert(k.clone(), v.clone());
            }
        }
        Value::Object(out)
    }

    struct Row {
        id: &'static str,
        name: &'static str,
        filters: Value,
        signed_in: bool,
    }

    /// Five package connections as the database held them before the switch, and two that must not move.
    fn rows() -> Vec<Row> {
        vec![
            Row {
                id: "github-1",
                name: "GitHub: New issue",
                filters: merged(&[
                    launch("@vornrun/connector-github"),
                    json!({
                        "sdkConnectorId": "github", "sdkVersion": "0.2.0", "sdkIcon": ICON,
                        "pollTool": "poll_issueCreated"
                    }),
                    poll_filters(),
                    json!({ "discoveredTools": tools() }),
                ]),
                signed_in: false,
            },
            Row {
                id: "midjourney-1",
                name: "Midjourney",
                filters: json!({
                    "command": "node", "args": r#"["/packs/midjourney/index.js"]"#, "env": "{}",
                    "sdkConnectorId": "midjourney", "sdkVersion": "0.1.0", "sdkIcon": ICON,
                    "discoveredTools": tools()
                }),
                signed_in: true,
            },
            Row {
                id: "substack-1",
                name: "Substack: New post",
                filters: merged(&[
                    launch("@vornrun/connector-substack"),
                    json!({
                        "secretEnv": "Y2lwaGVydGV4dA==", "sdkConnectorId": "substack",
                        "sdkVersion": "0.2.0", "sdkIcon": ICON, "pollTool": "poll_newPost"
                    }),
                    poll_filters(),
                    json!({ "discoveredTools": tools() }),
                ]),
                signed_in: true,
            },
            Row {
                id: "rss-implicit",
                name: "RSS",
                filters: json!({
                    "sdkConnectorId": "rss", "sdkVersion": "0.1.1", "sdkIcon": ICON,
                    "implicit": true, "discoveredTools": tools()
                }),
                signed_in: false,
            },
            Row {
                id: "rss-1",
                name: "RSS: New item",
                filters: merged(&[
                    launch("@vornrun/connector-rss"),
                    json!({
                        "sdkConnectorId": "rss", "sdkVersion": "0.1.1", "sdkIcon": ICON,
                        "pollTool": "poll_newItem"
                    }),
                    poll_filters(),
                    json!({ "discoveredTools": tools() }),
                ]),
                signed_in: false,
            },
            Row {
                id: "catalog-server",
                name: "Filesystem",
                filters: merged(&[
                    launch("@modelcontextprotocol/server-filesystem"),
                    json!({ "sdkConnectorId": "filesystem" }),
                ]),
                signed_in: false,
            },
            Row {
                id: "raw-mcp",
                name: "Tickets",
                filters: json!({
                    "command": "uvx", "args": r#"["tickets-mcp"]"#,
                    "pollTool": "list_things", "itemsPath": "rows"
                }),
                signed_in: false,
            },
        ]
    }

    fn seed(dir: &Path) {
        let conn = raw(dir);
        // Inbox and cursor rows need no workflow behind them to show where they moved.
        conn.execute_batch("PRAGMA foreign_keys = OFF")
            .expect("foreign keys off");
        for row in rows() {
            let at = row.signed_in.then_some("2026-09-12T00:00:00Z");
            let who = row.signed_in.then_some("Javier");
            conn.execute(
                "INSERT INTO source_connections
                   (id, connector_id, name, filters, sync_interval_minutes, status_mapping, created_at, signed_in_as, signed_in_at)
                 VALUES (?1, 'mcp', ?2, ?3, 5, '{}', '2026-09-01T00:00:00Z', ?4, ?5)",
                params![row.id, row.name, row.filters.to_string(), who, at],
            )
            .expect("the connection is written");
        }
        for (workflow, connection) in [("wf-substack", "substack-1"), ("wf-raw", "raw-mcp")] {
            conn.execute(
                "INSERT INTO connector_inbox
                   (workflow_id, connection_id, connector_id, event_id, event_type, event_timestamp, payload, available_at, created_at)
                 VALUES (?1, ?2, 'mcp', 'e1', 'mcpPoll', '2026-09-12T00:00:00Z', '{}', '2026-09-12T00:00:00Z', '2026-09-12T00:00:00Z')",
                params![workflow, connection],
            )
            .expect("the inbox row is written");
        }
        conn.execute(
            "INSERT INTO connector_poll_state (workflow_id, connection_id, cursor)
             VALUES ('wf-substack', 'substack-1', 'c-42')",
            [],
        )
        .expect("the cursor is written");
        set_schema_version(&conn, 20);
    }

    #[derive(Debug, PartialEq)]
    struct Stored {
        connector_id: String,
        filters: Value,
        signed_in_as: Option<String>,
    }

    fn stored(dir: &Path, id: &str) -> Stored {
        raw(dir)
            .query_row(
                "SELECT connector_id, filters, signed_in_as FROM source_connections WHERE id = ?1",
                [id],
                |r| {
                    let filters: String = r.get(1)?;
                    Ok(Stored {
                        connector_id: r.get(0)?,
                        filters: serde_json::from_str(&filters).expect("filters are JSON"),
                        signed_in_as: r.get(2)?,
                    })
                },
            )
            .expect("the connection reads")
    }

    fn inbox_connector(dir: &Path, connection: &str) -> String {
        raw(dir)
            .query_row(
                "SELECT connector_id FROM connector_inbox WHERE connection_id = ?1",
                [connection],
                |r| r.get(0),
            )
            .expect("the inbox row reads")
    }

    /// A file migrated past 21 with the old rows written into it and the version wound back, then opened again.
    fn migrated() -> tempfile::TempDir {
        let dir = dir();
        migrate(dir.path());
        seed(dir.path());
        migrate(dir.path());
        dir
    }

    #[test]
    fn every_package_connection_moves_to_sdk_under_its_id() {
        let dir = migrated();
        for id in [
            "github-1",
            "midjourney-1",
            "substack-1",
            "rss-implicit",
            "rss-1",
        ] {
            assert_eq!(stored(dir.path(), id).connector_id, "sdk", "{id}");
        }
    }

    #[test]
    fn the_trigger_is_named_from_the_tool_it_used_to_call() {
        let dir = migrated();
        let filters = |id| stored(dir.path(), id).filters;
        assert_eq!(filters("github-1")["sdkTrigger"], "issueCreated");
        assert_eq!(filters("substack-1")["sdkTrigger"], "newPost");
        assert_eq!(filters("rss-1")["sdkTrigger"], "newItem");
        assert!(filters("midjourney-1").get("sdkTrigger").is_none());
        assert!(filters("rss-implicit").get("sdkTrigger").is_none());
    }

    #[test]
    fn what_only_mcp_needed_is_dropped_and_the_rest_kept() {
        let dir = migrated();
        let substack = stored(dir.path(), "substack-1").filters;
        let mut mcp_only = vec!["discoveredTools".to_owned(), "pollTool".to_owned()];
        mcp_only.extend(
            poll_filters()
                .as_object()
                .expect("an object")
                .keys()
                .cloned(),
        );
        for key in &mcp_only {
            assert!(substack.get(key).is_none(), "{key}");
        }
        assert_eq!(
            substack,
            merged(&[
                launch("@vornrun/connector-substack"),
                json!({
                    "secretEnv": "Y2lwaGVydGV4dA==", "sdkConnectorId": "substack",
                    "sdkVersion": "0.2.0", "sdkIcon": ICON, "sdkTrigger": "newPost"
                }),
            ])
        );
        assert_eq!(
            stored(dir.path(), "rss-implicit").filters,
            json!({
                "sdkConnectorId": "rss", "sdkVersion": "0.1.1", "sdkIcon": ICON,
                "implicit": true
            })
        );
    }

    #[test]
    fn who_a_connection_is_signed_in_as_is_kept() {
        let dir = migrated();
        for id in ["midjourney-1", "substack-1"] {
            assert_eq!(
                stored(dir.path(), id).signed_in_as.as_deref(),
                Some("Javier")
            );
        }
    }

    #[test]
    fn a_catalog_server_and_a_raw_mcp_server_are_left_exactly_as_they_were() {
        let dir = migrated();
        for row in rows()
            .into_iter()
            .filter(|r| r.id == "catalog-server" || r.id == "raw-mcp")
        {
            assert_eq!(
                stored(dir.path(), row.id),
                Stored {
                    connector_id: "mcp".to_owned(),
                    filters: row.filters,
                    signed_in_as: None,
                }
            );
        }
    }

    #[test]
    fn only_a_package_connections_inbox_rows_move_and_cursors_stay() {
        let dir = migrated();
        assert_eq!(inbox_connector(dir.path(), "substack-1"), "sdk");
        assert_eq!(inbox_connector(dir.path(), "raw-mcp"), "mcp");
        let cursor: String = raw(dir.path())
            .query_row(
                "SELECT cursor FROM connector_poll_state WHERE workflow_id = 'wf-substack'",
                [],
                |r| r.get(0),
            )
            .expect("the cursor reads");
        assert_eq!(cursor, "c-42");
    }

    #[test]
    fn the_new_version_is_recorded_and_running_again_changes_nothing() {
        let dir = migrated();
        assert!(schema_version(dir.path()) >= 21);
        let all = || -> Vec<Stored> {
            rows()
                .iter()
                .map(|row| stored(dir.path(), row.id))
                .collect()
        };
        let before = all();
        migrate(dir.path());
        assert_eq!(all(), before);
    }
}

mod artifacts {
    //! Migration 25: the published artifact tables, and the gate edit columns a version could have been stamped without.
    use super::*;

    fn tables(dir: &Path) -> Vec<String> {
        let conn = raw(dir);
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
            .expect("the query prepares");
        let names = stmt
            .query_map([], |r| r.get(0))
            .expect("tables list")
            .collect::<Result<_, _>>()
            .expect("names read");
        names
    }

    fn node_columns(dir: &Path) -> Vec<String> {
        let conn = raw(dir);
        let mut stmt = conn
            .prepare("PRAGMA table_info(workflow_run_nodes)")
            .expect("the pragma prepares");
        let names = stmt
            .query_map([], |r| r.get("name"))
            .expect("columns list")
            .collect::<Result<_, _>>()
            .expect("names read");
        names
    }

    #[test]
    fn the_artifact_tables_are_created_on_a_database_that_stopped_at_24() {
        let dir = dir();
        migrate(dir.path());
        raw(dir.path())
            .execute_batch(
                "DROP TABLE artifact_comments; DROP TABLE artifact_versions; DROP TABLE artifacts;
                 UPDATE schema_meta SET value = '24' WHERE key = 'schema_version';",
            )
            .expect("wound back to 24");

        migrate(dir.path());

        let tables = tables(dir.path());
        for table in ["artifacts", "artifact_versions", "artifact_comments"] {
            assert!(tables.iter().any(|t| t == table), "{table} in {tables:?}");
        }
        assert!(schema_version(dir.path()) >= 25);
    }

    #[test]
    fn the_gate_edit_columns_are_repaired_when_a_version_was_stamped_without_them() {
        let dir = dir();
        migrate(dir.path());
        raw(dir.path())
            .execute_batch("ALTER TABLE workflow_run_nodes DROP COLUMN edited_text")
            .expect("the column goes");

        migrate(dir.path());

        let columns = node_columns(dir.path());
        for column in ["editable_text", "edited_text"] {
            assert!(
                columns.iter().any(|c| c == column),
                "{column} in {columns:?}"
            );
        }
    }
}
