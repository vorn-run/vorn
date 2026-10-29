//! Status, the built-in: a sidebar grouping that puts each running session
//! under what it is doing, published as rows of `status` and read again
//! whenever a session starts, changes status or ends.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use futures_util::FutureExt;
use serde_json::{json, Value};
use vorn_extensions::api::types::{Event, RowOp};
use vorn_extensions::api::ExtError;
use vorn_extensions::events::{SESSION_CREATED, SESSION_EXITED, SESSION_STATUS};

use super::{unknown, Api, Builtin};

/// The collection the grouping's rows are in.
const ROWS: &str = "status";

/// The built-in, as an entry of the built-ins' registry.
#[derive(Debug, Default)]
pub struct Status;

/// A status's heading, and where it sorts: what waits on a person first.
fn heading(status: &str) -> (u8, &'static str) {
    match status {
        "waiting" => (0, "Waiting"),
        "running" => (1, "Running"),
        "error" => (2, "Failed"),
        _ => (3, "Idle"),
    }
}

/// The rows for `sessions` as `sessions.list` lists them, headings in order.
fn rows(sessions: &Value) -> Vec<RowOp> {
    let mut by: Vec<(u8, &str, &'static str)> = sessions
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let id = s.get("id")?.as_str()?;
            let (rank, group) = heading(s.get("status").and_then(Value::as_str).unwrap_or(""));
            Some((rank, id, group))
        })
        .collect();
    by.sort_by_key(|(rank, ..)| *rank);
    std::iter::once(RowOp::Clear)
        .chain(
            by.into_iter()
                .map(|(_, id, group)| RowOp::Upsert(json!({ "id": id, "group": group }))),
        )
        .collect()
}

async fn publish(api: &Api) {
    if let Ok(sessions) = api.call("sessions.list", Value::Null).await {
        api.publish(ROWS, rows(&sessions));
    }
}

impl Builtin for Status {
    fn manifest(&self) -> &'static str {
        include_str!("status.toml")
    }

    fn events(&self) -> &'static [&'static str] {
        &[SESSION_CREATED, SESSION_STATUS, SESSION_EXITED]
    }

    fn on_by_default(&self) -> bool {
        true
    }

    fn start<'a>(&'a self, api: &'a Api) -> BoxFuture<'a, ()> {
        publish(api).boxed()
    }

    fn handle<'a>(
        &'a self,
        _api: &'a Api,
        action: &'a str,
        _args: Value,
    ) -> BoxFuture<'a, Result<Value, ExtError>> {
        async move { Err(unknown("Status grouping", action)) }.boxed()
    }

    // A batch is read as one list, however many sessions changed in it.
    fn on_events<'a>(&'a self, api: &'a Api, _batch: Vec<Arc<Event>>) -> BoxFuture<'a, ()> {
        publish(api).boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn its_manifest_reads_with_a_grouping() {
        let m =
            vorn_extensions::manifest::file::parse_builtin(include_str!("status.toml")).unwrap();
        let listed = vorn_extensions::points::listed(&m, &Default::default());
        assert!(
            listed
                .iter()
                .any(|p| p["kind"] == "groupings" && p["rows"] == "status"),
            "{listed:?}"
        );
    }

    #[test]
    fn sessions_waiting_come_first_under_their_heading() {
        let ops = rows(&json!([
            { "id": "a", "status": "idle" },
            { "id": "b", "status": "waiting" },
            { "id": "c", "status": "running" },
            { "status": "running" },
        ]));
        assert!(matches!(ops[0], RowOp::Clear));
        let got: Vec<Value> = ops[1..]
            .iter()
            .map(|op| match op {
                RowOp::Upsert(row) => row.clone(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            got,
            [
                json!({ "id": "b", "group": "Waiting" }),
                json!({ "id": "c", "group": "Running" }),
                json!({ "id": "a", "group": "Idle" }),
            ]
        );
    }
}
