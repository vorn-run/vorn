use std::sync::Mutex;

use serde_json::{json, Value};
use vorn_extensions::api::types::{Request, Response};
use vorn_extensions::api::ExtError;
use vorn_extensions::gate::{Answer, Caller, StorageCall, World};
use vorn_extensions::lifecycle::{Lifecycle, Policy};
use vorn_extensions::pack::PackStore;
use vorn_extensions::router::{Router, RouterSettings};

use super::*;

/// Answers the core methods the built-ins call, and keeps what they asked
/// and what they store, by extension and key.
#[derive(Default)]
struct Core {
    calls: Mutex<Vec<(String, Value)>>,
    kept: Mutex<BTreeMap<(String, String), Value>>,
}

impl World for Core {
    fn call(&self, caller: &Caller, method: &str, args: Value) -> Answer<Value> {
        guard(&self.calls).push((format!("{} {method}", caller.extension), args));
        let answer = match method {
            "sessions.list" => json!([
                { "id": "t1", "name": "api", "agent": "acme", "status": "waiting" }
            ]),
            _ => Value::Null,
        };
        Box::pin(async move { Ok(answer) })
    }

    fn fetch(&self, _: &Caller, _: Request) -> Answer<Response> {
        Box::pin(async { Err(ExtError::new("network", "no network here")) })
    }

    fn secret(&self, _: &Caller, _: &str, _: &str) -> Answer<Option<String>> {
        Box::pin(async { Ok(None) })
    }

    fn storage(
        &self,
        caller: &Caller,
        _: Option<vorn_extensions::pack::Migrations>,
        call: StorageCall,
    ) -> Answer<Value> {
        let mut kept = guard(&self.kept);
        let answer = match call {
            StorageCall::Get { key } => kept
                .get(&(caller.extension.clone(), key))
                .cloned()
                .unwrap_or(Value::Null),
            StorageCall::Set { key, value } => {
                kept.insert((caller.extension.clone(), key), value);
                Value::Null
            }
            _ => Value::Null,
        };
        Box::pin(async move { Ok(answer) })
    }
}

struct Setup {
    _dir: tempfile::TempDir,
    builtins: Builtins,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let core = Arc::new(Core::default());
    let router = Router::new(
        PackStore::new(dir.path().join("connectors")),
        Lifecycle::new(Policy::default()),
        core as Arc<dyn World>,
        RouterSettings::new(Vec::new(), "test"),
        vorn_extensions::events::Bus::new(),
    );
    let builtins = Builtins::new(router, dir.path().join("builtins.json"), Vec::new());
    Setup {
        _dir: dir,
        builtins,
    }
}

#[test]
fn lists_what_each_adds_and_uses_for_settings() {
    let s = setup();
    let list = s.builtins.list();
    let artifacts = list
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["id"] == "vorn.artifacts")
        .unwrap();
    assert_eq!(artifacts["on"], true);
    let uses = artifacts["declares"]["uses"].as_array().unwrap();
    assert!(uses.iter().any(|u| u == "sessions.read"), "{uses:?}");
}

#[test]
fn every_bundled_extension_s_id_is_kept_from_installs() {
    let ids = bundled_ids().expect("every bundled manifest reads");
    let s = setup();
    let mut registered: std::collections::BTreeSet<String> = s
        .builtins
        .list()
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_owned())
        .collect();
    registered.insert(file::parse_builtin(connectors::MANIFEST).unwrap().id);
    assert_eq!(ids, registered);
    // Tasks and Workflows are installed from their packs like any extension.
    assert_eq!(
        ids.into_iter().collect::<Vec<_>>(),
        [
            "vorn.artifacts",
            "vorn.connectors",
            "vorn.recent",
            "vorn.status"
        ]
    );
}
