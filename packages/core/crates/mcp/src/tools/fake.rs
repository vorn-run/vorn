//! A Vorn server in memory, for the tools' tests: it keeps the config blob
//! that `config:load` and `config:save` carry, stores the workflow methods'
//! writes in it, answers other methods from a table (which also overrides
//! those), and records every call.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};

use crate::rpc::{Caller, Rpc, RpcError};

#[derive(Default)]
pub struct FakeRpc {
    config: Mutex<Value>,
    answers: Mutex<HashMap<String, Result<Value, RpcError>>>,
    calls: Mutex<Vec<(String, Option<Value>)>>,
}

impl FakeRpc {
    pub fn new(config: Value) -> FakeRpc {
        FakeRpc {
            config: Mutex::new(config),
            ..FakeRpc::default()
        }
    }

    /// Answers `method` with `answer` from now on.
    pub fn answer(&self, method: &str, answer: Value) -> &FakeRpc {
        self.answers
            .lock()
            .unwrap()
            .insert(method.into(), Ok(answer));
        self
    }

    /// Fails `method` with `message` from now on.
    pub fn fail(&self, method: &str, message: &str) -> &FakeRpc {
        self.answers
            .lock()
            .unwrap()
            .insert(method.into(), Err(RpcError(message.into())));
        self
    }

    pub fn config(&self) -> Value {
        self.config.lock().unwrap().clone()
    }

    /// Every call so far, in order.
    pub fn calls(&self) -> Vec<(String, Option<Value>)> {
        self.calls.lock().unwrap().clone()
    }

    /// The params of the last call to `method`.
    pub fn last(&self, method: &str) -> Option<Value> {
        self.calls()
            .into_iter()
            .rev()
            .find(|(m, _)| m == method)
            .and_then(|(_, p)| p)
    }

    fn respond(&self, method: &str, params: Option<Value>) -> Result<Value, RpcError> {
        self.calls
            .lock()
            .unwrap()
            .push((method.into(), params.clone()));
        if let Some(answer) = self.answers.lock().unwrap().get(method) {
            return answer.clone();
        }
        let params = params.unwrap_or(Value::Null);
        let mut config = self.config.lock().unwrap();
        match method {
            "config:load" => Ok(config.clone()),
            "config:save" => {
                *config = params;
                Ok(Value::Null)
            }
            "workflow:create" => {
                let workflow = params["workflow"].clone();
                workflows(&mut config).push(workflow.clone());
                Ok(workflow)
            }
            "workflow:update" => {
                let row = workflows(&mut config)
                    .iter_mut()
                    .find(|w| w["id"] == params["id"]);
                let ok = row.is_some();
                if let (Some(Value::Object(row)), Value::Object(updates)) =
                    (row, &params["updates"])
                {
                    row.extend(updates.clone());
                }
                Ok(json!({ "ok": ok }))
            }
            "workflow:delete" => {
                let rows = workflows(&mut config);
                let before = rows.len();
                rows.retain(|w| w["id"] != params["id"]);
                Ok(json!({ "ok": rows.len() < before }))
            }
            other => Err(RpcError(format!("Method not found: {other}"))),
        }
    }
}

/// The config's workflow rows, made a list if they were not one.
fn workflows(config: &mut Value) -> &mut Vec<Value> {
    if !config["workflows"].is_array() {
        config["workflows"] = json!([]);
    }
    config["workflows"]
        .as_array_mut()
        .expect("made a list above")
}

impl Rpc for FakeRpc {
    async fn call(
        &self,
        method: &str,
        params: Option<Value>,
        _: Duration,
    ) -> Result<Value, RpcError> {
        self.respond(method, params)
    }

    async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), RpcError> {
        self.calls.lock().unwrap().push((method.into(), params));
        Ok(())
    }
}

/// A caller in `/work/app`, in session `s1`.
pub fn caller() -> Caller {
    Caller {
        cwd: "/work/app".into(),
        session: Some("s1".into()),
    }
}

/// A config with one project, one task and one workflow.
pub fn sample_config() -> Value {
    json!({
        "version": 1,
        "projects": [
            { "name": "app", "path": "/work/app", "preferredAgents": ["claude"], "workspaceId": "personal" }
        ],
        "tasks": [
            { "id": "t1", "projectName": "app", "title": "First", "description": "", "status": "in_progress",
              "order": 1, "createdAt": "2026-01-01T00:00:00.000Z", "updatedAt": "2026-01-01T00:00:00.000Z" }
        ],
        "workflows": [],
        "workspaces": [{ "id": "personal", "name": "Personal", "order": 0 }]
    })
}

/// Runs a future to completion on a current-thread runtime.
pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("a current-thread runtime builds")
        .block_on(future)
}
