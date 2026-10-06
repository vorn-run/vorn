//! The napi face of `vorn-store`: what `database.ts` calls when the Native
//! store switch is on.
//!
//! One object per open database, and one call, `call`, that takes the
//! TypeScript function's name and its arguments as JSON and returns its result
//! as JSON. The calls are synchronous, as libsql's are: the store answers from
//! a local file in the time the TypeScript one does, and every caller in the
//! server expects the answer in hand.

use napi_derive::napi;
use serde::Deserialize;
use serde_json::{Map, Value};

/// What the server knows that the store needs, as JSON (`StoreOptions`).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Options {
    default_shell: String,
    default_agent_commands: Map<String, Value>,
    default_workspace: vorn_protocol::WorkspaceConfig,
    owner_name: String,
    #[serde(default)]
    seed_workflows: Vec<Seed>,
}

#[derive(Deserialize)]
struct Seed {
    flag: String,
    workflow: vorn_protocol::WorkflowDefinition,
}

fn options(json: &str) -> napi::Result<vorn_store::StoreOptions> {
    let o: Options = serde_json::from_str(json)
        .map_err(|err| napi::Error::from_reason(format!("store options: {err}")))?;
    Ok(vorn_store::StoreOptions {
        default_shell: o.default_shell,
        default_agent_commands: o.default_agent_commands,
        default_workspace: o.default_workspace,
        owner_name: o.owner_name,
        seed_workflows: o
            .seed_workflows
            .into_iter()
            .map(|s| vorn_store::SeedWorkflow {
                flag: s.flag,
                workflow: s.workflow,
            })
            .collect(),
    })
}

fn to_napi(err: vorn_store::Error) -> napi::Error {
    napi::Error::from_reason(err.to_string())
}

#[napi]
pub struct NativeStore {
    /// `None` once closed.
    inner: Option<vorn_store::Store>,
    /// Where a corrupt file was copied before being replaced, if it was.
    recovered: Option<String>,
}

#[napi]
impl NativeStore {
    /// Opens the database file, creating and migrating it as needed.
    #[napi(factory, catch_unwind)]
    pub fn open(path: String, options_json: String) -> napi::Result<Self> {
        let (store, opened) =
            vorn_store::Store::open(std::path::Path::new(&path), options(&options_json)?)
                .map_err(to_napi)?;
        let recovered = match opened {
            vorn_store::Opened::Ok => None,
            vorn_store::Opened::Recovered { backup } => Some(backup.display().to_string()),
        };
        Ok(Self {
            inner: Some(store),
            recovered,
        })
    }

    /// A database in memory, for tests.
    #[napi(factory, catch_unwind)]
    pub fn open_in_memory(options_json: String) -> napi::Result<Self> {
        let store = vorn_store::Store::open_in_memory(options(&options_json)?).map_err(to_napi)?;
        Ok(Self {
            inner: Some(store),
            recovered: None,
        })
    }

    /// The backup of a corrupt file this open replaced, or null.
    #[napi(getter)]
    pub fn recovered(&self) -> Option<String> {
        self.recovered.clone()
    }

    /// Calls the store function named `call` with `args`, a JSON array in
    /// signature order, and returns its result as JSON (`null` for nothing).
    #[napi(catch_unwind)]
    pub fn call(&mut self, call: String, args: String) -> napi::Result<String> {
        let store = self.inner.as_mut().ok_or_else(|| {
            napi::Error::from_reason("Database not initialized. Call initDatabase() first.")
        })?;
        let args: Value = serde_json::from_str(&args)
            .map_err(|err| napi::Error::from_reason(format!("{call}: {err}")))?;
        let result = store.call(&call, args).map_err(to_napi)?;
        Ok(result.to_string())
    }

    /// Closes the database. Every call after this fails.
    #[napi(catch_unwind)]
    pub fn close(&mut self) {
        self.inner = None;
    }
}
