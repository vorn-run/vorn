//! A Vorn server in memory, for the tools' tests: it keeps the config blob
//! that `config:load` and `config:save` carry, answers other methods from a
//! table, and records every call.

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
        match method {
            "config:load" => Ok(self.config()),
            "config:save" => {
                *self.config.lock().unwrap() = params.unwrap_or(Value::Null);
                Ok(Value::Null)
            }
            other => self
                .answers
                .lock()
                .unwrap()
                .get(other)
                .cloned()
                .unwrap_or_else(|| Err(RpcError(format!("Method not found: {other}")))),
        }
    }
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
