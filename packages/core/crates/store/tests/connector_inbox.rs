//! The durable connector inbox: poll pages and their cursors, leases, retries and deferrals.

mod common;

use common::{call, load_config, memory, save_config};
use serde_json::{json, Value};
use vorn_store::Store;

const CONNECTION: &str = "conn-1";

fn workflow(id: &str) -> Value {
    json!({
        "id": id, "name": id, "icon": "Plug", "iconColor": "#fff",
        "enabled": true, "nodes": [], "edges": []
    })
}

fn connection(id: &str, name: &str) -> Value {
    json!({
        "id": id, "connectorId": "github", "name": name,
        "filters": { "owner": "owner", "repo": "repo" }, "syncIntervalMinutes": 5,
        "statusMapping": {}, "createdAt": "2026-08-04T00:00:00.000Z"
    })
}

fn item(connection: &str, external: &str) -> Value {
    json!({
        "connectionId": connection, "connectorId": "github", "externalId": external,
        "title": format!("Item {external}"),
        "raw": { "externalId": external, "title": format!("Item {external}") }
    })
}

fn event(connection: &str, external: &str, at: &str) -> Value {
    json!({
        "eventId": external, "eventType": "issueCreated", "eventTimestamp": at,
        "connectorItem": item(connection, external)
    })
}

/// The store with one connection and two workflows on it.
fn store() -> Store {
    let mut store = memory();
    call(
        &mut store,
        "dbInsertSourceConnection",
        json!([connection(CONNECTION, "owner/repo")]),
    );
    call(
        &mut store,
        "dbInsertWorkflow",
        json!([workflow("wf-issues")]),
    );
    call(&mut store, "dbInsertWorkflow", json!([workflow("wf-prs")]));
    store
}

/// Records one page from `CONNECTION`; answers how many events were new.
fn record(store: &mut Store, workflow: &str, cursor: &str, externals: &[&str]) -> Value {
    let events: Vec<Value> = externals
        .iter()
        .map(|id| event(CONNECTION, id, "2026-08-04T00:30:00.000Z"))
        .collect();
    call(
        store,
        "dbRecordConnectorPollPage",
        json!([{
            "workflowId": workflow, "connectionId": CONNECTION, "connectorId": "github",
            "cursor": cursor, "polledAt": "2026-08-04T01:00:00.000Z", "events": events
        }]),
    )
}

fn cursor(store: &mut Store, workflow: &str, connection: &str) -> Value {
    call(
        store,
        "dbGetConnectorPollCursor",
        json!([workflow, connection]),
    )
}

fn claim(store: &mut Store, now: &str, lease_until: &str) -> Vec<Value> {
    let claimed = call(
        store,
        "dbClaimConnectorInbox",
        json!([{ "now": now, "leaseUntil": lease_until, "limit": 10 }]),
    );
    claimed.as_array().cloned().expect("a list")
}

fn externals(claimed: &[Value]) -> Vec<&str> {
    claimed
        .iter()
        .map(|e| e["connectorItem"]["externalId"].as_str().expect("an id"))
        .collect()
}

fn complete(store: &mut Store, entry: &Value, token: &str, at: &str) -> Value {
    call(
        store,
        "dbCompleteConnectorInbox",
        json!([entry["id"], token, at]),
    )
}

#[test]
fn events_and_their_cursor_are_kept_as_one_page() {
    let mut store = store();
    assert_eq!(record(&mut store, "wf-issues", "cursor-1", &["1", "2"]), 2);
    assert_eq!(cursor(&mut store, "wf-issues", CONNECTION), "cursor-1");

    let claimed = claim(
        &mut store,
        "2026-08-04T01:00:00.000Z",
        "2026-08-04T02:00:00.000Z",
    );
    assert_eq!(externals(&claimed), ["1", "2"]);
    assert!(claimed.iter().all(|e| e["attempts"] == 1));
}

#[test]
fn an_overlapping_page_is_deduplicated_and_the_checkpoint_kept() {
    let mut store = store();
    assert_eq!(record(&mut store, "wf-issues", "cursor-1", &["1", "2"]), 2);
    assert_eq!(record(&mut store, "wf-issues", "cursor-2", &["2", "3"]), 1);
    assert_eq!(cursor(&mut store, "wf-issues", CONNECTION), "cursor-2");

    let claimed = claim(
        &mut store,
        "2026-08-04T01:00:00.000Z",
        "2026-08-04T02:00:00.000Z",
    );
    assert_eq!(externals(&claimed), ["1", "2", "3"]);
}

#[test]
fn workflows_on_one_connection_keep_their_own_cursors() {
    let mut store = store();
    record(&mut store, "wf-issues", "issue-cursor", &[]);
    record(&mut store, "wf-prs", "pr-cursor", &[]);
    assert_eq!(cursor(&mut store, "wf-issues", CONNECTION), "issue-cursor");
    assert_eq!(cursor(&mut store, "wf-prs", CONNECTION), "pr-cursor");
}

#[test]
fn changing_connection_resets_the_cursor_and_the_dedupe_scope() {
    let mut store = store();
    record(&mut store, "wf-issues", "cursor-1", &["1"]);
    call(
        &mut store,
        "dbInsertSourceConnection",
        json!([connection("conn-2", "owner/other")]),
    );

    assert!(cursor(&mut store, "wf-issues", "conn-2").is_null());
    let recorded = call(
        &mut store,
        "dbRecordConnectorPollPage",
        json!([{
            "workflowId": "wf-issues", "connectionId": "conn-2", "connectorId": "github",
            "cursor": "cursor-2", "polledAt": "2026-08-04T02:00:00.000Z",
            "events": [event("conn-2", "1", "2026-08-04T01:30:00.000Z")]
        }]),
    );
    assert_eq!(recorded, 1);
}

#[test]
fn a_completed_item_is_not_claimed_again() {
    let mut store = store();
    record(&mut store, "wf-issues", "cursor-1", &["1"]);
    let claimed = claim(
        &mut store,
        "2026-08-04T01:00:00.000Z",
        "2026-08-04T02:00:00.000Z",
    );
    let token = claimed[0]["leaseToken"].as_str().expect("a token");
    complete(&mut store, &claimed[0], token, "2026-08-04T01:01:00.000Z");
    assert!(claim(
        &mut store,
        "2026-08-04T03:00:00.000Z",
        "2026-08-04T04:00:00.000Z"
    )
    .is_empty());
}

#[test]
fn a_failed_item_is_retried_only_after_its_backoff() {
    let mut store = store();
    record(&mut store, "wf-issues", "cursor-1", &["1"]);
    let claimed = claim(
        &mut store,
        "2026-08-04T01:00:00.000Z",
        "2026-08-04T02:00:00.000Z",
    );
    call(
        &mut store,
        "dbRetryConnectorInbox",
        json!([{
            "id": claimed[0]["id"], "leaseToken": claimed[0]["leaseToken"],
            "error": "workflow failed", "now": "2026-08-04T01:00:00.000Z"
        }]),
    );

    assert!(claim(
        &mut store,
        "2026-08-04T01:00:59.000Z",
        "2026-08-04T02:00:00.000Z"
    )
    .is_empty());
    let retried = claim(
        &mut store,
        "2026-08-04T01:01:00.000Z",
        "2026-08-04T02:00:00.000Z",
    );
    assert_eq!(retried[0]["attempts"], 2);
}

#[test]
fn deferring_an_unaccepted_item_counts_no_attempt() {
    let mut store = store();
    record(&mut store, "wf-issues", "cursor-1", &["1"]);
    let claimed = claim(
        &mut store,
        "2026-08-04T01:00:00.000Z",
        "2026-08-05T01:00:00.000Z",
    );
    assert_eq!(claimed[0]["attempts"], 1);
    call(
        &mut store,
        "dbDeferConnectorInbox",
        json!([
            claimed[0]["id"],
            claimed[0]["leaseToken"],
            "2026-08-04T01:00:30.000Z"
        ]),
    );
    let reclaimed = claim(
        &mut store,
        "2026-08-04T01:00:30.000Z",
        "2026-08-05T01:00:00.000Z",
    );
    assert_eq!(reclaimed[0]["attempts"], 1);
}

#[test]
fn leases_are_released_at_once_after_a_restart() {
    let mut store = store();
    record(&mut store, "wf-issues", "cursor-1", &["1"]);
    claim(
        &mut store,
        "2026-08-04T01:00:00.000Z",
        "2026-08-04T02:00:00.000Z",
    );
    call(
        &mut store,
        "dbReleaseConnectorInboxLeases",
        json!(["2026-08-04T01:01:00.000Z"]),
    );
    let reclaimed = claim(
        &mut store,
        "2026-08-04T01:01:00.000Z",
        "2026-08-04T02:00:00.000Z",
    );
    assert_eq!(reclaimed[0]["attempts"], 2);
}

#[test]
fn acknowledgements_from_an_expired_lease_owner_are_ignored() {
    let mut store = store();
    record(&mut store, "wf-issues", "cursor-1", &["1"]);
    let first = claim(
        &mut store,
        "2026-08-04T01:00:00.000Z",
        "2026-08-04T01:01:00.000Z",
    );
    let second = claim(
        &mut store,
        "2026-08-04T01:01:00.000Z",
        "2026-08-04T01:06:00.000Z",
    );

    assert_ne!(second[0]["leaseToken"], first[0]["leaseToken"]);
    let deferred = call(
        &mut store,
        "dbDeferConnectorInbox",
        json!([
            first[0]["id"],
            first[0]["leaseToken"],
            "2026-08-04T01:01:30.000Z"
        ]),
    );
    assert_eq!(deferred, false);
    let token = second[0]["leaseToken"].as_str().expect("a token");
    assert_eq!(
        complete(&mut store, &second[0], token, "2026-08-04T01:02:00.000Z"),
        true
    );
}

#[test]
fn saving_an_existing_workflow_keeps_its_inbox_rows_and_cursor() {
    let mut store = store();
    record(&mut store, "wf-issues", "cursor-1", &["1"]);
    let mut config = load_config(&mut store);
    for entry in config["workflows"].as_array_mut().expect("workflows") {
        if entry["id"] == "wf-issues" {
            entry["name"] = json!("Renamed");
        }
    }
    save_config(&mut store, config, &[]);

    assert_eq!(cursor(&mut store, "wf-issues", CONNECTION), "cursor-1");
    assert_eq!(
        claim(
            &mut store,
            "2026-08-04T01:00:00.000Z",
            "2026-08-04T01:05:00.000Z"
        )
        .len(),
        1
    );
}

#[test]
fn only_the_current_owner_renews_a_lease() {
    let mut store = store();
    record(&mut store, "wf-issues", "cursor-1", &["1"]);
    let claimed = claim(
        &mut store,
        "2026-08-04T01:00:00.000Z",
        "2026-08-04T01:05:00.000Z",
    );
    let renew = |store: &mut Store, token: &Value, until: &str| {
        call(
            store,
            "dbRenewConnectorInboxLease",
            json!([claimed[0]["id"], token, until]),
        )
    };

    assert_eq!(
        renew(
            &mut store,
            &claimed[0]["leaseToken"],
            "2026-08-04T01:10:00.000Z"
        ),
        true
    );
    assert_eq!(
        renew(
            &mut store,
            &json!("stale-token"),
            "2026-08-04T01:20:00.000Z"
        ),
        false
    );
    // The renewal held the row past the original expiry.
    assert!(claim(
        &mut store,
        "2026-08-04T01:06:00.000Z",
        "2026-08-04T01:11:00.000Z"
    )
    .is_empty());
}

#[test]
fn only_unexpired_leases_count_against_capacity() {
    let mut store = store();
    record(&mut store, "wf-issues", "cursor-1", &["1", "2"]);
    claim(
        &mut store,
        "2026-08-04T01:00:00.000Z",
        "2026-08-04T01:05:00.000Z",
    );
    let count = |store: &mut Store, now: &str| {
        call(store, "dbCountActiveConnectorInboxLeases", json!([now]))
    };
    assert_eq!(count(&mut store, "2026-08-04T01:01:00.000Z"), 2);
    assert_eq!(count(&mut store, "2026-08-04T01:06:00.000Z"), 0);
}

#[test]
fn a_poll_failure_leaves_the_last_good_cursor() {
    let mut store = store();
    record(&mut store, "wf-issues", "cursor-1", &["1"]);
    call(
        &mut store,
        "dbRecordConnectorPollError",
        json!([{
            "workflowId": "wf-issues", "connectionId": CONNECTION,
            "error": "rate limited", "polledAt": "2026-08-04T01:05:00.000Z"
        }]),
    );

    assert_eq!(cursor(&mut store, "wf-issues", CONNECTION), "cursor-1");
    let connections = call(&mut store, "dbListSourceConnections", json!([null]));
    assert_eq!(connections[0]["lastSyncError"], "rate limited");
}

#[test]
fn a_failed_first_poll_on_a_new_connection_drops_the_old_cursor() {
    let mut store = store();
    record(&mut store, "wf-issues", "cursor-1", &["1"]);
    call(
        &mut store,
        "dbInsertSourceConnection",
        json!([connection("conn-2", "owner/other")]),
    );
    call(
        &mut store,
        "dbRecordConnectorPollError",
        json!([{
            "workflowId": "wf-issues", "connectionId": "conn-2",
            "error": "unauthorized", "polledAt": "2026-08-04T01:05:00.000Z"
        }]),
    );
    assert!(cursor(&mut store, "wf-issues", "conn-2").is_null());
}

#[test]
fn a_ready_row_is_leased_at_most_once_per_claim_window() {
    let mut store = store();
    record(&mut store, "wf-issues", "cursor-1", &["1"]);
    let first = claim(
        &mut store,
        "2026-08-04T01:00:00.000Z",
        "2026-08-04T01:05:00.000Z",
    );
    let second = claim(
        &mut store,
        "2026-08-04T01:00:00.000Z",
        "2026-08-04T01:05:00.000Z",
    );
    assert_eq!(first.len(), 1);
    assert!(second.is_empty());
    assert_eq!(first[0]["attempts"], 1);
}

#[test]
fn the_run_that_owns_an_inbox_row_is_found() {
    let mut store = store();
    call(
        &mut store,
        "saveWorkflowRun",
        json!([{
            "runId": "run-inbox-9", "workflowId": "wf-issues",
            "startedAt": "2026-08-04T01:00:00.000Z", "status": "running",
            "connectorInboxId": 9, "connectorInboxLeaseToken": "lease-9", "nodeStates": []
        }]),
    );
    let by_inbox =
        |store: &mut Store, id: i64| call(store, "dbGetWorkflowRunByConnectorInboxId", json!([id]));
    assert_eq!(by_inbox(&mut store, 9)["runId"], "run-inbox-9");
    assert!(by_inbox(&mut store, 404).is_null());
}
