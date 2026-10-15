//! The agent adapters Vorn ships, each a built-in with its own switch: its
//! manifest's `[agent]` section is read by [`crate::adapters`], and its
//! switch decides whether that agent is on.
//!
//! An adapter that draws templates gets one row per session of its agent in
//! its `sessions` collection, as its hooks report it: the model the session
//! was started with, its tool calls and the last tool it ran.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures_util::future::BoxFuture;
use futures_util::FutureExt;
use serde_json::{json, Value};
use vorn_adapters::Shipped;
use vorn_extensions::api::types::{Event, RowOp};
use vorn_extensions::api::ExtError;
use vorn_extensions::events::{AGENT_HOOK, SESSION_CREATED, SESSION_EXITED};

use super::{unknown, Api, Builtin};
use crate::adapters::Adapters;

/// The collection an adapter's templates read its sessions from.
pub const SESSIONS: &str = "sessions";

/// One shipped adapter as a built-in.
pub struct AgentAdapter {
    shipped: &'static Shipped,
    adapters: Arc<Adapters>,
    /// Its manifest's `id`.
    extension: String,
    /// Its agent's id.
    agent: Option<String>,
    views: Vec<(&'static str, &'static str)>,
    /// Its sessions' rows, by session id.
    rows: Mutex<HashMap<String, Value>>,
}

impl AgentAdapter {
    fn new(shipped: &'static Shipped, adapters: Arc<Adapters>) -> AgentAdapter {
        let table: toml::Table = toml::from_str(shipped.manifest).unwrap_or_default();
        let id = |t: Option<&toml::Value>| {
            t.and_then(|t| t.get("id"))
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
        };
        let extension = table
            .get("id")
            .and_then(toml::Value::as_str)
            .map_or_else(|| shipped.name.to_owned(), str::to_owned);
        let agent = id(table.get("agent"));
        AgentAdapter {
            shipped,
            adapters,
            extension,
            agent,
            views: shipped.views(),
            rows: Mutex::default(),
        }
    }

    fn rows(&self) -> std::sync::MutexGuard<'_, HashMap<String, Value>> {
        self.rows.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The rows `batch` changes, as the operations that publish them.
    fn fold(&self, batch: &[Arc<Event>]) -> Vec<RowOp> {
        let Some(agent) = self.agent.as_deref() else {
            return Vec::new();
        };
        let mut rows = self.rows();
        let mut ops = Vec::new();
        for event in batch {
            let data = &event.data;
            let Some(id) = data["sessionId"].as_str() else {
                continue;
            };
            match event.name.as_str() {
                SESSION_CREATED if data["agent"] == agent => {
                    let row = json!({
                        "id": id,
                        "model": data["model"].as_str().unwrap_or_default(),
                        "tool": "",
                        "tools": 0,
                    });
                    rows.insert(id.to_owned(), row.clone());
                    ops.push(RowOp::Upsert(row));
                }
                SESSION_EXITED => {
                    if rows.remove(id).is_some() {
                        ops.push(RowOp::Delete(id.to_owned()));
                    }
                }
                AGENT_HOOK if data["event"] == "PreToolUse" => {
                    let Some(row) = rows.get_mut(id) else {
                        continue;
                    };
                    row["tools"] = json!(row["tools"].as_u64().unwrap_or(0) + 1);
                    if let Some(tool) = data["tool"].as_str() {
                        row["tool"] = json!(tool);
                    }
                    ops.push(RowOp::Upsert(row.clone()));
                }
                _ => {}
            }
        }
        ops
    }
}

/// Every shipped adapter, as built-ins sharing `adapters`.
pub fn entries(adapters: &Arc<Adapters>) -> Vec<Arc<dyn Builtin>> {
    vorn_adapters::shipped()
        .iter()
        .map(|s| Arc::new(AgentAdapter::new(s, Arc::clone(adapters))) as Arc<dyn Builtin>)
        .collect()
}

impl Builtin for AgentAdapter {
    fn manifest(&self) -> &'static str {
        self.shipped.manifest
    }

    fn views(&self) -> &[(&'static str, &'static str)] {
        &self.views
    }

    fn events(&self) -> &'static [&'static str] {
        &[SESSION_CREATED, SESSION_EXITED, AGENT_HOOK]
    }

    fn on_by_default(&self) -> bool {
        self.shipped.on_by_default
    }

    fn start<'a>(&'a self, _api: &'a Api) -> BoxFuture<'a, ()> {
        async {}.boxed()
    }

    fn handle<'a>(
        &'a self,
        _api: &'a Api,
        action: &'a str,
        _args: Value,
    ) -> BoxFuture<'a, Result<Value, ExtError>> {
        let name = self.agent.clone().unwrap_or_default();
        async move { Err(unknown(&name, action)) }.boxed()
    }

    fn on_events<'a>(&'a self, api: &'a Api, batch: Vec<Arc<Event>>) -> BoxFuture<'a, ()> {
        async move {
            // Only an adapter that draws something keeps rows.
            if self.views.is_empty() {
                return;
            }
            let ops = self.fold(&batch);
            if !ops.is_empty() {
                api.publish(SESSIONS, ops);
            }
        }
        .boxed()
    }

    fn switched(&self, on: bool) {
        self.adapters.set_shipped(&self.extension, on);
    }

    fn stop<'a>(&'a self, _api: &'a Api) -> BoxFuture<'a, ()> {
        self.rows().clear();
        async {}.boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(name: &str, data: Value) -> Arc<Event> {
        Arc::new(Event {
            name: name.to_owned(),
            data,
        })
    }

    #[test]
    fn keeps_a_row_per_session_of_its_agent_from_its_hooks() {
        let adapters = crate::adapters::testing::adapters();
        let shipped = vorn_adapters::shipped()
            .iter()
            .find(|s| !s.views().is_empty())
            .expect("an adapter ships a template");
        let mut a = AgentAdapter::new(shipped, adapters);
        a.agent = Some("acme".to_owned());
        let ops = a.fold(&[
            event(SESSION_CREATED, json!({ "sessionId": "s", "agent": "acme", "model": "m1" })),
            event(SESSION_CREATED, json!({ "sessionId": "o", "agent": "beta" })),
            event(AGENT_HOOK, json!({ "sessionId": "s", "event": "PreToolUse", "tool": "edit" })),
            event(AGENT_HOOK, json!({ "sessionId": "o", "event": "PreToolUse", "tool": "x" })),
        ]);
        assert_eq!(
            ops.last(),
            Some(&RowOp::Upsert(
                json!({ "id": "s", "model": "m1", "tool": "edit", "tools": 1 })
            ))
        );
        assert_eq!(ops.len(), 2);
        assert_eq!(
            a.fold(&[event(SESSION_EXITED, json!({ "sessionId": "s" }))]),
            [RowOp::Delete("s".into())]
        );
    }
}
