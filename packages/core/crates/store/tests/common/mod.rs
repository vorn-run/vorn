//! The options the store is opened with in tests, and calls by their call name.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};
use vorn_store::{Opened, SeedWorkflow, Store, StoreOptions};

/// The workflows vornd seeds, as the Workflows built-in ships them: `(flag, file)`.
const SEED_WORKFLOWS: [(&str, &str); 2] = [
    (
        "hasSeededDefaultTaskWorkflow",
        include_str!("../../../workflows/workflows/default-task.toml"),
    ),
    (
        "hasSeededDevServerWorkflow",
        include_str!("../../../workflows/workflows/dev-server.toml"),
    ),
];

/// Seeded workflows' ids.
pub const DEFAULT_TASK_WORKFLOW_ID: &str = "system:default-task-workflow";
pub const DEV_SERVER_WORKFLOW_ID: &str = "system:dev-server-on-restore";

/// The agents Vorn ships switched on, in the launcher's order: the ones
/// whose answers the recorded store answers name.
pub fn shipped() -> Vec<vorn_agents::Agent> {
    vorn_adapters::shipped()
        .iter()
        .filter(|s| s.on_by_default)
        .filter_map(|s| vorn_agents::from_manifest_text(s.manifest).ok().flatten())
        .collect()
}

/// The id of the `n`th agent Vorn ships.
pub fn agent(n: usize) -> String {
    shipped()[n].id().to_owned()
}

/// The app's agent commands (`DEFAULT_AGENT_COMMANDS`): each shipped agent's default.
pub fn default_agent_commands() -> Map<String, Value> {
    shipped()
        .iter()
        .map(|a| {
            let c = a.default_command();
            let mut entry = json!({ "command": c.command, "args": c.args });
            if let Some(h) = c.headless_args {
                entry["headlessArgs"] = json!(h);
            }
            (a.id().to_owned(), entry)
        })
        .collect()
}

fn seed_workflows() -> Vec<SeedWorkflow> {
    SEED_WORKFLOWS
        .iter()
        .map(|(flag, text)| {
            let read = vorn_workflow_file::read(text, &Default::default()).expect("a seed reads");
            SeedWorkflow {
                flag: (*flag).to_owned(),
                workflow: serde_json::from_value(read.workflow).expect("a seed is a workflow"),
            }
        })
        .collect()
}

/// The options vornd opens the store with, with the machine's shell and user name given.
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

/// A store in memory with the test options.
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

/// The call named `name`, with `args` in signature order, whole floats read
/// as integers; panics on an error.
pub fn call(store: &mut Store, name: &str, args: Value) -> Value {
    let result = store
        .call(name, args)
        .unwrap_or_else(|err| panic!("{name}: {err}"));
    with_whole_floats_as_integers(result)
}

/// `value` with every whole float as an integer: a field typed `number` is an
/// f64 (`15.0`), and these tests compare it as `15`.
pub fn with_whole_floats_as_integers(value: Value) -> Value {
    match value {
        Value::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() && f.fract() == 0.0 && f.abs() <= 9_007_199_254_740_992.0 => {
                Value::from(f as i64)
            }
            _ => Value::Number(n),
        },
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(with_whole_floats_as_integers)
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, with_whole_floats_as_integers(v)))
                .collect(),
        ),
        other => other,
    }
}

/// `saveConfig`, with the `defaults` keys to clear as `deleted`.
pub fn save_config(store: &mut Store, config: Value, deleted: &[&str]) {
    call(store, "saveConfig", json!([config, deleted]));
}

pub fn load_config(store: &mut Store) -> Value {
    call(store, "loadConfig", json!([]))
}

/// `dbUpdateTask`, with `present` naming every key the update had, including
/// cleared ones the JSON of `updates` drops.
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

/// Whether `value` has no `key` at all.
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

/// Makes `workflows` the only ones the store has, through the workflow
/// calls: the configuration no longer carries them.
pub fn put_workflows(store: &mut Store, workflows: Vec<Value>) {
    for w in call(store, "dbListWorkflows", json!([]))
        .as_array()
        .cloned()
        .unwrap_or_default()
    {
        call(store, "dbDeleteWorkflow", json!([w["id"]]));
    }
    for w in workflows {
        call(store, "dbInsertWorkflow", json!([w]));
    }
}
