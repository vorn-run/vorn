//! The Widget: a small window over the others listing every agent's status
//! and the tools they wait to use, with Allow and Deny.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use futures_util::FutureExt;
use serde_json::{json, Value};
use vorn_extensions::api::types::{Event, RowOp};
use vorn_extensions::api::ExtError;
use vorn_extensions::events::{
    AGENT_PERMISSION, AGENT_PERMISSION_CLOSED, SESSION_CREATED, SESSION_EXITED, SESSION_STATUS,
};

use super::{arg, unknown, Api, Builtin};

const NAME: &str = "Widget";
const AGENTS: &str = "agents";
const PERMISSIONS: &str = "permissions";
/// The longest tool input shown, in characters.
const INPUT_MAX: usize = 160;

pub struct Widget;

/// Every running session, read again: a status change carries no name.
async fn agents(api: &Api) {
    let Ok(Value::Array(sessions)) = api.call("sessions.list", json!({})).await else {
        return;
    };
    let mut ops = vec![RowOp::Clear];
    ops.extend(sessions.iter().map(|s| {
        RowOp::Upsert(json!({
            "id": s["id"],
            "name": s["name"],
            "agent": s["agent"],
            "status": s["status"],
        }))
    }));
    api.publish(AGENTS, ops);
}

/// What a request asks, in a line: its description, else its command or file.
fn asked(data: &Value) -> String {
    let said = data["description"].as_str().unwrap_or_default().trim();
    if !said.is_empty() {
        return said.to_owned();
    }
    let input = &data["toolInput"];
    let line = ["command", "file_path", "path", "url", "pattern"]
        .iter()
        .find_map(|k| input[*k].as_str())
        .map_or_else(|| input.to_string(), str::to_owned);
    if line.chars().count() > INPUT_MAX {
        let cut: String = line.chars().take(INPUT_MAX - 1).collect();
        return format!("{cut}…");
    }
    line
}

fn permission(api: &Api, data: &Value) {
    let Some(id) = data["requestId"].as_str() else {
        return;
    };
    let terminal = data["terminalId"].as_str().unwrap_or_default();
    let agent = api
        .rows(AGENTS)
        .into_iter()
        .find(|a| a["id"] == terminal)
        .and_then(|a| a["name"].as_str().map(str::to_owned))
        .or_else(|| data["projectName"].as_str().map(str::to_owned))
        .unwrap_or_default();
    let row = json!({
        "id": id,
        "tool": data["toolName"].as_str().unwrap_or("Tool"),
        "agent": agent,
        "description": asked(data),
    });
    api.publish(PERMISSIONS, vec![RowOp::Upsert(row)]);
}

async fn answer(api: &Api, action: &str, args: &Value) -> Result<Value, ExtError> {
    let id = arg(args, "id", action)?;
    let allow = action == "allow";
    api.call(
        "permissions.resolve",
        json!({ "requestId": id, "allow": allow }),
    )
    .await?;
    api.publish(PERMISSIONS, vec![RowOp::Delete(id.to_owned())]);
    Ok(Value::Null)
}

impl Builtin for Widget {
    fn manifest(&self) -> &'static str {
        include_str!("widget.toml")
    }

    fn views(&self) -> &'static [(&'static str, &'static str)] {
        &[("views/widget.kdl", include_str!("views/widget.kdl"))]
    }

    fn events(&self) -> &'static [&'static str] {
        &[
            SESSION_CREATED,
            SESSION_STATUS,
            SESSION_EXITED,
            AGENT_PERMISSION,
            AGENT_PERMISSION_CLOSED,
        ]
    }

    fn on_by_default(&self) -> bool {
        false
    }

    fn start<'a>(&'a self, api: &'a Api) -> BoxFuture<'a, ()> {
        agents(api).boxed()
    }

    fn handle<'a>(
        &'a self,
        api: &'a Api,
        action: &'a str,
        args: Value,
    ) -> BoxFuture<'a, Result<Value, ExtError>> {
        async move {
            match action {
                "allow" | "deny" => answer(api, action, &args).await,
                other => Err(unknown(NAME, other)),
            }
        }
        .boxed()
    }

    fn on_events<'a>(&'a self, api: &'a Api, batch: Vec<Arc<Event>>) -> BoxFuture<'a, ()> {
        async move {
            let mut sessions = false;
            for event in &batch {
                match event.name.as_str() {
                    AGENT_PERMISSION => permission(api, &event.data),
                    AGENT_PERMISSION_CLOSED => {
                        if let Some(id) = event.data["requestId"].as_str() {
                            api.publish(PERMISSIONS, vec![RowOp::Delete(id.to_owned())]);
                        }
                    }
                    _ => sessions = true,
                }
            }
            if sessions {
                agents(api).await;
            }
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::asked;

    #[test]
    fn a_request_reads_as_its_description_or_its_input() {
        assert_eq!(
            asked(&json!({ "description": " Run the tests " })),
            "Run the tests"
        );
        assert_eq!(
            asked(&json!({ "toolInput": { "command": "cargo test" } })),
            "cargo test"
        );
        let long = "x".repeat(400);
        assert_eq!(
            asked(&json!({ "toolInput": { "command": long } }))
                .chars()
                .count(),
            160
        );
    }
}
