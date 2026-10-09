//! The options the store is opened with in tests, and calls by their TypeScript name.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};
use vorn_store::{Opened, SeedWorkflow, Store, StoreOptions};

/// The workflows vornd seeds, as it seeds them.
const SEED_WORKFLOWS: &str = include_str!("../../../vornd/src/serve/seed-workflows.json");

/// Seeded workflows' ids, as `default-workflows.ts` named them.
pub const DEFAULT_TASK_WORKFLOW_ID: &str = "system:default-task-workflow";
pub const DEV_SERVER_WORKFLOW_ID: &str = "system:dev-server-on-restore";

/// The app's agent commands (`DEFAULT_AGENT_COMMANDS`).
pub fn default_agent_commands() -> Map<String, Value> {
    let commands = json!({
        "claude": { "command": "claude", "args": [], "headlessArgs": ["--dangerously-skip-permissions"] },
        "copilot": { "command": "copilot", "args": [], "headlessArgs": ["--allow-all"] },
        "codex": { "command": "codex", "args": [], "headlessArgs": ["-a", "never"] },
        "opencode": { "command": "opencode", "args": [] },
        "gemini": { "command": "gemini", "args": [], "headlessArgs": ["-y"] }
    });
    match commands {
        Value::Object(map) => map,
        _ => unreachable!("an object literal"),
    }
}

fn seed_workflows() -> Vec<SeedWorkflow> {
    #[derive(serde::Deserialize)]
    struct Seed {
        flag: String,
        workflow: vorn_protocol::WorkflowDefinition,
    }
    let seeds: Vec<Seed> = serde_json::from_str(SEED_WORKFLOWS).expect("seeded workflows parse");
    seeds
        .into_iter()
        .map(|s| SeedWorkflow {
            flag: s.flag,
            workflow: s.workflow,
        })
        .collect()
}

/// What the server passed the store (`nativeOptions()`), with the machine's shell and user name given.
pub fn options_with(default_shell: &str, owner_name: &str) -> StoreOptions {
    StoreOptions {
        default_shell: default_shell.to_owned(),
        default_agent_commands: default_agent_commands(),
        default_workspace: serde_json::from_value(json!({
            "id": "personal",
            "name": "Personal",
            "icon": "User",
            "iconColor": "#6b7280",
            "order": 0
        }))
        .expect("a workspace"),
        owner_name: owner_name.to_owned(),
        seed_workflows: seed_workflows(),
    }
}

pub fn options() -> StoreOptions {
    options_with("/bin/zsh", "owner")
}

/// A store in memory, as `initTestDatabase()` opened one.
pub fn memory() -> Store {
    Store::open_in_memory(options()).expect("an in-memory store opens")
}

/// `vorn.db` in `dir`.
pub fn db_file(dir: &Path) -> PathBuf {
    dir.join("vorn.db")
}

/// The store on `vorn.db` in `dir`, as `initDatabase(dir)` opened it.
pub fn open(dir: &Path) -> Store {
    let (store, opened) = Store::open(&db_file(dir), options()).expect("the store opens");
    assert_eq!(opened, Opened::Ok);
    store
}

/// Opens and closes the store on `dir`, which creates or migrates it.
pub fn migrate(dir: &Path) {
    drop(open(dir));
}

/// The file in `dir` on a connection of its own, outside the store.
pub fn raw(dir: &Path) -> rusqlite::Connection {
    rusqlite::Connection::open(db_file(dir)).expect("the file opens")
}

/// The call named `name`, with `args` in signature order, answered as the TypeScript caller read it; panics on an error.
pub fn call(store: &mut Store, name: &str, args: Value) -> Value {
    let result = store
        .call(name, args)
        .unwrap_or_else(|err| panic!("{name}: {err}"));
    as_javascript_read_it(result)
}

/// `value` with every whole float as an integer. The store answers a field typed `number` as an f64 (`15.0`); JavaScript has one number type, so the caller that parsed it saw `15`, and so do these tests.
pub fn as_javascript_read_it(value: Value) -> Value {
    match value {
        Value::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() && f.fract() == 0.0 && f.abs() <= 9_007_199_254_740_992.0 => {
                Value::from(f as i64)
            }
            _ => Value::Number(n),
        },
        Value::Array(items) => Value::Array(items.into_iter().map(as_javascript_read_it).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, as_javascript_read_it(v)))
                .collect(),
        ),
        other => other,
    }
}

/// `saveConfig(config)`: the wrapper sent the keys of `defaults` set to `undefined` beside the config, as `deleted`.
pub fn save_config(store: &mut Store, config: Value, deleted: &[&str]) {
    call(store, "saveConfig", json!([config, deleted]));
}

pub fn load_config(store: &mut Store) -> Value {
    call(store, "loadConfig", json!([]))
}

/// `dbUpdateTask(id, updates)`: the wrapper sent `Object.keys(updates)` too, which names a key set to `undefined` that the JSON of `updates` drops.
pub fn update_task(store: &mut Store, id: &str, updates: Value, present: &[&str]) {
    call(store, "dbUpdateTask", json!([id, updates, present]));
}

/// `dbUpdateSourceConnection(id, updates)`, with the same extra argument.
pub fn update_connection(store: &mut Store, id: &str, updates: Value, present: &[&str]) {
    call(
        store,
        "dbUpdateSourceConnection",
        json!([id, updates, present]),
    );
}

/// Whether `value` has no `key` at all: what `toBeUndefined()` and `not.toHaveProperty` asked of a value that crossed as JSON.
pub fn lacks(value: &Value, key: &str) -> bool {
    value.get(key).is_none()
}

/// The string `key` of each row in a list, in order.
pub fn ids(rows: &Value, key: &str) -> Vec<String> {
    rows.as_array()
        .expect("a list")
        .iter()
        .map(|row| row[key].as_str().expect("a string id").to_owned())
        .collect()
}
