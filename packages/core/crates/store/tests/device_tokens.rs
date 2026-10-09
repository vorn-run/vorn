//! The seeded owner and the device token rows minted for them.

mod common;

use common::{call, memory};
use serde_json::{json, Value};
use vorn_store::Store;

fn owner_id(store: &mut Store) -> String {
    call(store, "dbGetOwnerUser", json!([]))["id"]
        .as_str()
        .expect("a seeded owner")
        .to_owned()
}

/// Inserts the owner's token `id`, named `name`, created at `created_at`.
fn insert(store: &mut Store, id: &str, name: &str, created_at: &str) {
    let user = owner_id(store);
    call(
        store,
        "dbInsertDeviceToken",
        json!([{
            "id": id, "userId": user, "name": name,
            "tokenHash": "0".repeat(64), "createdAt": created_at
        }]),
    );
}

fn list(store: &mut Store) -> Vec<Value> {
    call(store, "dbListDeviceTokens", json!([]))
        .as_array()
        .cloned()
        .expect("a list")
}

fn has_tokens(store: &mut Store) -> Value {
    call(store, "dbHasDeviceTokens", json!([]))
}

fn revoke(store: &mut Store, id: &str) -> Value {
    call(
        store,
        "dbRevokeDeviceToken",
        json!([id, "2026-10-01T02:00:00.000Z"]),
    )
}

#[test]
fn a_fresh_database_has_an_owner() {
    let mut store = memory();
    let owner = call(&mut store, "dbGetOwnerUser", json!([]));
    assert_eq!(owner["role"], "owner");
    assert!(!owner["name"].as_str().unwrap_or_default().is_empty());
}

#[test]
fn the_secret_read_carries_the_hash_and_the_listing_does_not() {
    let mut store = memory();
    insert(&mut store, "t1", "iPhone", "2026-10-01T00:00:00.000Z");
    let secret = call(&mut store, "dbGetDeviceTokenSecret", json!(["t1"]));
    assert_eq!(secret["tokenHash"].as_str().map(str::len), Some(64));
    assert!(list(&mut store)
        .iter()
        .all(|t| t.get("tokenHash").is_none()));
}

#[test]
fn an_unknown_token_has_no_secret() {
    let mut store = memory();
    assert!(call(&mut store, "dbGetDeviceTokenSecret", json!(["nope"])).is_null());
}

#[test]
fn has_tokens_is_false_on_a_fresh_database_and_true_once_one_exists() {
    let mut store = memory();
    assert_eq!(has_tokens(&mut store), false);
    insert(&mut store, "t1", "iPhone", "2026-10-01T00:00:00.000Z");
    assert_eq!(has_tokens(&mut store), true);
}

#[test]
fn a_revoked_token_still_counts_since_it_still_exists() {
    let mut store = memory();
    insert(&mut store, "t1", "iPhone", "2026-10-01T00:00:00.000Z");
    revoke(&mut store, "t1");
    assert_eq!(has_tokens(&mut store), true);
}

#[test]
fn the_listing_is_empty_on_a_fresh_database() {
    let mut store = memory();
    assert!(list(&mut store).is_empty());
}

#[test]
fn tokens_list_in_creation_order_and_show_revocation() {
    let mut store = memory();
    insert(&mut store, "a", "first", "2026-10-01T00:00:00.000Z");
    insert(&mut store, "b", "second", "2026-10-01T00:00:01.000Z");
    let names: Vec<Value> = list(&mut store).iter().map(|t| t["name"].clone()).collect();
    assert_eq!(names, [json!("first"), json!("second")]);

    revoke(&mut store, "a");
    let listed = list(&mut store);
    let by_id = |id: &str| listed.iter().find(|t| t["id"] == id).expect("listed");
    assert!(by_id("a")["revokedAt"].is_string());
    assert!(by_id("b")["revokedAt"].is_null());
}

#[test]
fn revoking_an_unknown_id_reports_false() {
    let mut store = memory();
    assert_eq!(revoke(&mut store, "not-a-token"), false);
}

#[test]
fn revoking_twice_reports_false_the_second_time() {
    let mut store = memory();
    insert(&mut store, "t1", "iPhone", "2026-10-01T00:00:00.000Z");
    assert_eq!(revoke(&mut store, "t1"), true);
    assert_eq!(revoke(&mut store, "t1"), false);
}

#[test]
fn a_first_sighting_is_recorded() {
    let mut store = memory();
    insert(&mut store, "t1", "iPhone", "2026-10-01T00:00:00.000Z");
    assert!(list(&mut store)[0]["lastSeenAt"].is_null());
    call(
        &mut store,
        "dbTouchDeviceToken",
        json!(["t1", "2026-10-01T01:00:00.000Z"]),
    );
    assert_eq!(
        list(&mut store)[0]["lastSeenAt"],
        "2026-10-01T01:00:00.000Z"
    );
}

#[test]
fn touching_an_unknown_id_is_not_an_error() {
    let mut store = memory();
    call(
        &mut store,
        "dbTouchDeviceToken",
        json!(["not-a-token", "2026-10-01T01:00:00.000Z"]),
    );
}
