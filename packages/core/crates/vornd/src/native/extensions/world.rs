//! What vornd puts behind the bridge's gate: the core methods a program may
//! call, the network `net.fetch` reaches, and the vault its credentials come
//! from. The gate has already checked every call against the manifest.

use std::path::PathBuf;
use std::sync::Weak;
use std::time::Duration;

use serde_json::{json, Value};
use vorn_connectors::fetch::{Http, Request as HttpRequest};
use vorn_extensions::api::types::{Header, Request, Response};
use vorn_extensions::api::ExtError;
use vorn_extensions::gate::{Answer, Caller, World};

use crate::native::{secrets::Known, Native};

/// How long one `net.fetch` may take.
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// vornd's world: its session records, git, the network and the vault.
pub struct Wired {
    pub native: Weak<Native>,
}

fn stopping() -> ExtError {
    ExtError::new("stopping", "vornd is stopping")
}

impl World for Wired {
    fn call(&self, caller: &Caller, method: &str, args: Value) -> Answer<Value> {
        let native = self.native.upgrade();
        let method = method.to_owned();
        let project = args
            .get("project")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| caller.project.clone());
        Box::pin(async move {
            let native = native.ok_or_else(stopping)?;
            match method.as_str() {
                "sessions.list" => {
                    let sessions = native
                        .registry
                        .get()
                        .and_then(|r| {
                            r.read(|r| {
                                r.live_terminals()
                                    .map(|t| {
                                        json!({
                                            "id": t.id, "agent": t.agent_type,
                                            "projectPath": t.project_path,
                                            "worktreePath": t.worktree_path,
                                        })
                                    })
                                    .collect::<Vec<_>>()
                            })
                        })
                        .unwrap_or_default();
                    Ok(Value::Array(sessions))
                }
                "git.status" => {
                    let project = project.ok_or_else(|| {
                        ExtError::new("bad-request", "git.status takes the project to read")
                    })?;
                    let git = vorn_git::repo::Git {
                        bin: native.env.git_bin(),
                        env: native.env.get(),
                        ssh: None,
                    };
                    tokio::task::spawn_blocking(move || {
                        let dir = PathBuf::from(&project);
                        json!({ "branch": git.branch(&dir), "dirty": git.is_worktree_dirty(&dir) })
                    })
                    .await
                    .map_err(|e| ExtError::new("failed", e.to_string()))
                }
                other => Err(ExtError::new(
                    "unknown-method",
                    format!("Vorn has no method {other}"),
                )),
            }
        })
    }

    fn fetch(&self, _caller: &Caller, request: Request) -> Answer<Response> {
        Box::pin(async move {
            let sent = HttpRequest {
                method: request.method,
                url: request.url,
                headers: request
                    .headers
                    .into_iter()
                    .map(|h| (h.name, h.value))
                    .collect(),
                body: request.body,
            };
            let answer = tokio::task::spawn_blocking(move || Http.send(&sent, FETCH_TIMEOUT)).await;
            match answer {
                Ok(Ok(r)) => Ok(Response {
                    status: r.status,
                    headers: r
                        .headers
                        .into_iter()
                        .map(|(name, value)| Header { name, value })
                        .collect(),
                    body: r.body,
                }),
                Ok(Err(message)) => Err(ExtError::new("network", message)),
                Err(e) => Err(ExtError::new("failed", e.to_string())),
            }
        })
    }

    fn secret(&self, caller: &Caller, connection: &str, field: &str) -> Answer<Option<String>> {
        let native = self.native.upgrade();
        let (owner, connection, field) = (
            caller.extension.clone(),
            connection.to_owned(),
            field.to_owned(),
        );
        Box::pin(async move {
            let native = native.ok_or_else(stopping)?;
            let connectors = native
                .connectors()
                .cloned()
                .ok_or_else(|| ExtError::new("signed-out", "vornd holds no connections"))?;
            if connectors.owner_of(&connection).await.as_deref() != Some(owner.as_str()) {
                return Err(ExtError::new(
                    "denied",
                    format!("the connection {connection} is not {owner}'s"),
                ));
            }
            let found =
                tokio::task::spawn_blocking(move || match native.secrets.lookup(&connection) {
                    Known::Fields(fields) => fields.get(&field).map(|v| v.expose().to_owned()),
                    Known::None | Known::Unknown => None,
                })
                .await
                .map_err(|e| ExtError::new("failed", e.to_string()))?;
            Ok(found)
        })
    }

    fn published(&self, caller: &Caller, collection: &str) {
        if let Some(native) = self.native.upgrade() {
            native.broadcast_to(
                "extension:rowsChanged",
                json!({ "extensionId": caller.extension, "collection": collection }),
                None,
            );
        }
    }
}
