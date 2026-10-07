//! Receiving a workflow trigger once.
//!
//! Triggers are delivered at least once: a scheduler that restarts replays
//! the minutes it may have missed, and a webhook sender retries what it did
//! not see acknowledged. The receiver claims each trigger's effect id in
//! `effect_receipts` before it runs the step, so a second delivery of the
//! same trigger finds the claim and is dropped.

use vorn_store::Store;

use crate::schedule::{Occurrence, Schedule};

/// The `kind` a trigger's receipt is recorded under.
pub const TRIGGER_KIND: &str = "workflow-trigger";

/// How far back a scheduler replays on start. A minute missed longer ago
/// than this is not run late.
pub const CATCH_UP_MS: i64 = 15 * 60_000;

/// What a delivery of a trigger came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// Not received before: run the step.
    First,
    /// Received before: drop it.
    Repeat,
}

/// Claims `effect_id` at `now_ms`. A claim the store cannot record runs the
/// step: a trigger run twice is the lesser loss than one never run.
pub fn receive(store: &Store, effect_id: &str, now_ms: i64) -> Delivery {
    match store.claim_effect(effect_id, TRIGGER_KIND, Some(now_ms as f64)) {
        Ok(false) => Delivery::Repeat,
        Ok(true) | Err(_) => Delivery::First,
    }
}

/// Delivers every occurrence of `schedule` due after `after_ms` up to
/// `now_ms`, running `step` for each one received for the first time, and
/// returns those.
pub fn deliver_due(
    store: &Store,
    schedule: &Schedule,
    after_ms: i64,
    now_ms: i64,
    mut step: impl FnMut(&Occurrence),
) -> Vec<Occurrence> {
    let mut ran = Vec::new();
    for occurrence in schedule.due(after_ms, now_ms) {
        if receive(store, &occurrence.effect_id(), now_ms) == Delivery::First {
            step(&occurrence);
            ran.push(occurrence);
        }
    }
    ran
}

/// Where a scheduler starting at `now_ms` replays from: the occurrences it
/// may have missed while it was down, which the receipts of those it did
/// run keep from running twice.
pub fn catch_up_from(now_ms: i64) -> i64 {
    now_ms - CATCH_UP_MS
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::tz::TimeZone;
    use serde_json::json;
    use std::path::Path;
    use vorn_store::StoreOptions;

    fn open(path: &Path) -> Store {
        let options = StoreOptions {
            default_shell: String::new(),
            default_agent_commands: serde_json::Map::new(),
            default_workspace: serde_json::from_value(json!({
                "id": "personal", "name": "Personal", "icon": "User", "iconColor": "#6b7280", "order": 0
            }))
            .unwrap(),
            owner_name: "owner".into(),
            seed_workflows: Vec::new(),
        };
        Store::open(path, options).unwrap().0
    }

    fn every_minute(now: i64) -> Schedule {
        let wf = json!({ "id": "wf", "enabled": true, "nodes": [{ "id": "t", "type": "trigger", "config": { "triggerType": "recurring", "cron": "* * * * *" } }] });
        Schedule::arm(&[wf], &TimeZone::UTC, now)
    }

    #[test]
    fn a_trigger_delivered_twice_runs_its_step_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(&dir.path().join("vorn.db"));
        let now = 29_000_000 * 60_000;
        let schedule = every_minute(now);
        let mut runs = 0;
        let first = deliver_due(&store, &schedule, now - 60_000, now, |_| runs += 1);
        let second = deliver_due(&store, &schedule, now - 60_000, now, |_| runs += 1);
        assert_eq!(runs, 1);
        assert_eq!(first.len(), 1);
        assert!(second.is_empty());
        assert_eq!(
            receive(&store, &first[0].effect_id(), now),
            Delivery::Repeat
        );
    }

    #[test]
    fn a_second_receiver_on_the_same_database_drops_the_repeat() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vorn.db");
        let one = open(&path);
        let other = Store::open_beside(&path).unwrap().unwrap();
        assert_eq!(receive(&one, "webhook/wf/abc", 1), Delivery::First);
        assert_eq!(receive(&other, "webhook/wf/abc", 2), Delivery::Repeat);
        assert_eq!(receive(&other, "webhook/wf/abd", 3), Delivery::First);
    }

    #[test]
    fn a_restart_runs_the_missed_minutes_once_and_the_rest_not_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vorn.db");
        let start = 29_000_000 * 60_000;
        let base = start / 60_000;
        let mut before = Vec::new();
        {
            let store = open(&path);
            let schedule = every_minute(start);
            let up_until = start + 3 * 60_000;
            deliver_due(&store, &schedule, catch_up_from(start), start, |o| {
                before.push(o.minute)
            });
            deliver_due(&store, &schedule, start, up_until, |o| {
                before.push(o.minute)
            });
        }
        assert_eq!(before, (base - 14..=base + 3).collect::<Vec<_>>());
        // Down for two minutes, then up again: the replay overlaps what ran.
        let now = start + 5 * 60_000;
        let store = open(&path);
        let mut after = Vec::new();
        deliver_due(&store, &every_minute(now), catch_up_from(now), now, |o| {
            after.push(o.minute)
        });
        assert_eq!(after, [base + 4, base + 5]);
    }

    #[test]
    fn a_store_that_cannot_record_the_claim_runs_the_step() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vorn.db");
        let store = open(&path);
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute("DROP TABLE effect_receipts", [])
            .unwrap();
        assert_eq!(receive(&store, "x", 1), Delivery::First);
    }
}
