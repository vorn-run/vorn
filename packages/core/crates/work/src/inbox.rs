//! The connector inbox, which carries connector items and webhook requests
//! to the workflows that run on them (the inbox half of `scheduler.ts`, and
//! `webhook-trigger.ts`).
//!
//! A poll or a webhook request lands in the inbox first, so an event
//! received while nothing can run it is still there later. Rows are leased
//! to a run while it works on them, and settled when it ends: processed,
//! tried again after a backoff, or deferred.

use serde_json::{json, Map, Value};
use vorn_store::Store;

use crate::artifacts::call;
use crate::js;
use crate::receipts::{receive, Delivery};

/// How long a run holds a row before another may take it.
pub const LEASE_MS: i64 = 5 * 60_000;

/// How often the inbox is looked at with nothing else to prompt it.
pub const DRAIN_INTERVAL_MS: u64 = 30_000;

/// Rows leased at once.
pub const BATCH: i64 = 50;

/// How long a deferred row waits.
const DEFER_MS: i64 = 30_000;

/// The connector id a webhook's events are filed under.
pub const WEBHOOK_CONNECTOR: &str = "webhook";

/// Auth-bearing headers stay out of run records.
const HEADER_DENYLIST: &[&str] = &["authorization", "cookie", "proxy-authorization", "x-api-key"];

/// One row to run, and the run already working on it, if one is.
#[derive(Clone, Debug, PartialEq)]
pub struct Due {
    pub workflow_id: String,
    /// The row's item with its inbox id and lease.
    pub item: Value,
    /// A run still going on this row from before.
    pub existing: Option<Value>,
}

/// Lets every lease go: the runs that held them ended with the process.
pub fn release_leases(store: &mut Store, now_ms: i64) {
    let _ = call(store, "dbReleaseConnectorInboxLeases", json!([js::iso(now_ms)]));
}

/// `deliverPendingConnectorInbox`, up to running: leases what is due and
/// can be taken, settles rows whose run already finished, and returns the
/// rest.
pub fn claim_due(store: &mut Store, now_ms: i64) -> Vec<Due> {
    let now = js::iso(now_ms);
    let active = call(store, "dbCountActiveConnectorInboxLeases", json!([now]))
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as i64;
    let slots = BATCH - active;
    if slots <= 0 {
        return Vec::new();
    }
    let claimed = call(
        store,
        "dbClaimConnectorInbox",
        json!([{ "now": now, "leaseUntil": js::iso(now_ms + LEASE_MS), "limit": slots }]),
    )
    .unwrap_or(json!([]));
    let mut due = Vec::new();
    for row in claimed.as_array().into_iter().flatten() {
        let id = row["id"].clone();
        let lease = row["leaseToken"].clone();
        let existing = call(store, "dbGetWorkflowRunByConnectorInboxId", json!([id])).unwrap_or(Value::Null);
        let finished = existing.is_object()
            && existing["status"] != "running"
            && (existing["connectorInboxDisposition"] == "processed"
                || existing["status"] == "success"
                || existing["status"] == "cancelled");
        if finished {
            let _ = call(store, "dbCompleteConnectorInbox", json!([id, lease, js::iso(now_ms)]));
            continue;
        }
        let mut item = row["connectorItem"].as_object().cloned().unwrap_or_default();
        item.insert("inboxId".into(), id);
        item.insert("inboxLeaseToken".into(), lease);
        due.push(Due {
            workflow_id: row["workflowId"].as_str().unwrap_or("").to_owned(),
            item: Value::Object(item),
            existing: (existing.is_object() && existing["status"] == "running").then_some(existing),
        });
    }
    due
}

/// `completeConnectorInbox`: a run settles its row.
pub fn complete(store: &mut Store, id: i64, lease: &str, disposition: &str, error: Option<&str>, now_ms: i64) {
    let now = js::iso(now_ms);
    let _ = match disposition {
        "processed" => call(store, "dbCompleteConnectorInbox", json!([id, lease, now])),
        "retry" => call(
            store,
            "dbRetryConnectorInbox",
            json!([{ "id": id, "leaseToken": lease, "error": error.filter(|e| !e.is_empty()).unwrap_or("Connector workflow failed"), "now": now }]),
        ),
        _ => call(store, "dbDeferConnectorInbox", json!([id, lease, js::iso(now_ms + DEFER_MS)])),
    };
}

/// Extends a row's lease; false once the row is no longer the caller's.
pub fn renew(store: &mut Store, id: i64, lease: &str, now_ms: i64) -> bool {
    call(store, "dbRenewConnectorInboxLease", json!([id, lease, js::iso(now_ms + LEASE_MS)]))
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// A request to a workflow's webhook.
#[derive(Clone, Debug, Default)]
pub struct Request {
    pub method: String,
    /// Header names as received.
    pub headers: Vec<(String, String)>,
    /// The query's string values.
    pub query: Map<String, Value>,
    /// The parsed body, `null` without one.
    pub body: Value,
    /// A key the sender repeats when it sends the same event again.
    pub delivery_id: Option<String>,
}

/// What a webhook request came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Received {
    /// Queued for the workflow: 202.
    Queued,
    /// A repeat of an event already queued: 202, nothing queued.
    Repeat,
    /// No such workflow, token or method: one 404 for every miss.
    NotFound,
}

/// The webhook trigger of a stored workflow, when it has one.
fn webhook_trigger(workflow: &Value) -> Option<&Value> {
    let trigger = workflow
        .get("nodes")?
        .as_array()?
        .iter()
        .find(|n| n.get("type").and_then(Value::as_str) == Some("trigger"))?
        .get("config")?;
    (trigger.get("triggerType").and_then(Value::as_str) == Some("webhook")).then_some(trigger)
}

/// Files a request to `/wf-hooks/<workflow>/<token>` in the inbox. A
/// sender's repeat of one delivery id is received once.
pub fn receive_webhook(store: &mut Store, workflow_id: &str, token: &str, request: &Request, event_id: &str, now_ms: i64) -> Received {
    let workflow = call(store, "dbGetWorkflow", json!([workflow_id])).unwrap_or(Value::Null);
    let enabled = crate::is_truthy(workflow.get("enabled"));
    let Some(trigger) = webhook_trigger(&workflow).filter(|_| enabled) else {
        return Received::NotFound;
    };
    let token_ok = trigger.get("token").and_then(Value::as_str).is_some_and(|t| crate::gates::same_token(t, token));
    if !token_ok || trigger.get("method").and_then(Value::as_str) != Some(request.method.as_str()) {
        return Received::NotFound;
    }
    if let Some(key) = request.delivery_id.as_deref().filter(|k| !k.is_empty()) {
        if receive(store, &format!("webhook/{workflow_id}/{key}"), now_ms) == Delivery::Repeat {
            return Received::Repeat;
        }
    }
    let mut headers = Map::new();
    for (name, value) in &request.headers {
        if HEADER_DENYLIST.contains(&name.to_ascii_lowercase().as_str()) {
            continue;
        }
        let key = name.to_ascii_lowercase();
        match headers.get_mut(&key) {
            Some(Value::String(existing)) => {
                existing.push_str(", ");
                existing.push_str(value);
            }
            _ => {
                headers.insert(key, Value::String(value.clone()));
            }
        }
    }
    let received = js::iso(now_ms);
    let _ = call(
        store,
        "dbEnqueueWebhookEvent",
        json!([{
            "workflowId": workflow_id,
            "eventId": event_id,
            "receivedAt": received,
            "item": {
                "connectionId": format!("{WEBHOOK_CONNECTOR}:{workflow_id}"),
                "connectorId": WEBHOOK_CONNECTOR,
                "externalId": event_id,
                "title": format!("Webhook {}", request.method),
                "raw": {
                    "body": request.body,
                    "headers": headers,
                    "query": request.query,
                    "method": request.method,
                    "receivedAt": received,
                }
            }
        }]),
    );
    Received::Queued
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn store(dir: &Path) -> Store {
        let options = vorn_store::StoreOptions {
            default_shell: String::new(),
            default_agent_commands: Map::new(),
            default_workspace: serde_json::from_value(json!({ "id": "personal", "name": "Personal", "icon": "User", "iconColor": "#6b7280", "order": 0 })).unwrap(),
            owner_name: "owner".into(),
            seed_workflows: Vec::new(),
        };
        Store::open(&dir.join("vorn.db"), options).unwrap().0
    }

    fn hooked(s: &mut Store) {
        call(s, "dbInsertWorkflow", json!([{
            "id": "wf", "name": "Hook", "icon": "x", "iconColor": "#000", "enabled": true, "edges": [],
            "nodes": [{ "id": "t", "type": "trigger", "label": "T", "position": { "x": 0, "y": 0 }, "config": { "triggerType": "webhook", "method": "POST", "token": "tok" } }]
        }])).unwrap();
    }

    fn post(delivery: Option<&str>) -> Request {
        Request {
            method: "POST".into(),
            headers: vec![("Authorization".into(), "secret".into()), ("X-Id".into(), "1".into()), ("x-id".into(), "2".into())],
            query: serde_json::from_value(json!({ "q": "z" })).unwrap(),
            body: json!({ "name": "n" }),
            delivery_id: delivery.map(str::to_owned),
        }
    }

    #[test]
    fn a_webhook_request_is_queued_and_run_once_per_delivery() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        hooked(&mut s);
        let now = 1_700_000_000_000;
        assert_eq!(receive_webhook(&mut s, "wf", "tok", &post(Some("d1")), "e1", now), Received::Queued);
        assert_eq!(receive_webhook(&mut s, "wf", "tok", &post(Some("d1")), "e2", now), Received::Repeat);
        assert_eq!(receive_webhook(&mut s, "wf", "bad", &post(None), "e3", now), Received::NotFound);
        let get = Request { method: "GET".into(), ..post(None) };
        assert_eq!(receive_webhook(&mut s, "wf", "tok", &get, "e4", now), Received::NotFound);
        assert_eq!(receive_webhook(&mut s, "nope", "tok", &post(None), "e5", now), Received::NotFound);

        let due = claim_due(&mut s, now + 1);
        assert_eq!(due.len(), 1);
        let item = &due[0].item;
        assert_eq!(item["raw"]["headers"], json!({ "x-id": "1, 2" }));
        assert_eq!(item["raw"]["body"], json!({ "name": "n" }));
        assert!(item["inboxLeaseToken"].is_string());
        // Leased: not handed out again.
        assert!(claim_due(&mut s, now + 2).is_empty());
        let (id, lease) = (item["inboxId"].as_i64().unwrap(), item["inboxLeaseToken"].as_str().unwrap().to_owned());
        assert!(renew(&mut s, id, &lease, now + 3));
        complete(&mut s, id, &lease, "processed", None, now + 4);
        assert!(!renew(&mut s, id, &lease, now + 5));
    }

    #[test]
    fn a_released_lease_is_delivered_again_and_a_deferred_row_waits() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        hooked(&mut s);
        let now = 1_700_000_000_000;
        receive_webhook(&mut s, "wf", "tok", &post(None), "e1", now);
        assert_eq!(claim_due(&mut s, now).len(), 1);
        release_leases(&mut s, now + 1);
        let again = claim_due(&mut s, now + 2);
        assert_eq!(again.len(), 1);
        let (id, lease) = (again[0].item["inboxId"].as_i64().unwrap(), again[0].item["inboxLeaseToken"].as_str().unwrap().to_owned());
        complete(&mut s, id, &lease, "defer", None, now + 3);
        assert!(claim_due(&mut s, now + 4).is_empty());
        assert_eq!(claim_due(&mut s, now + 3 + DEFER_MS + 1).len(), 1);
    }
}
