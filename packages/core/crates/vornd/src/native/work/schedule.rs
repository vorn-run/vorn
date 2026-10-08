//! The scheduler and the connector inbox, run by vornd.
//!
//! Each second the scheduler plans what fired since it last looked
//! ([`vorn_work::schedule`]) and delivers each occurrence through
//! `effect_receipts` ([`vorn_work::receipts`]), so an occurrence delivered
//! twice runs once. Where it got to is kept in `vornd/schedule.json`
//! beside the database: a vornd that starts again plans from there, so the
//! fires it missed while it was down are caught up, at most
//! [`CATCH_UP_MS`] of them. A first start plans from now.
//!
//! A connector poll is fetched by the server, which holds the connectors
//! ([`crate::native::connectors::Connectors::poll`]), and its items, like webhook requests, are run from
//! the inbox here: leased, run, and settled by the run.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use jiff::tz::TimeZone;
use serde_json::{json, Value};
use tracing::{info, warn};
use vorn_work::receipts::{deliver_due, CATCH_UP_MS};
use vorn_work::schedule::Schedule;

use super::host::now_ms;
use super::Work;

/// How often the scheduler looks.
const TICK: Duration = Duration::from_secs(1);

/// How often it reads the workflows again without being told they changed.
const RELOAD_MS: i64 = 10_000;

/// How long the server gets to poll a connection.
const POLL_LIMIT: Duration = Duration::from_secs(10 * 60);

/// Where the scheduler keeps how far it got.
pub fn mark_file(db: &Path) -> PathBuf {
    db.parent()
        .unwrap_or(Path::new("."))
        .join("vornd")
        .join("schedule.json")
}

/// Where planning starts: from where the last vornd got, at most
/// [`CATCH_UP_MS`] back, or from now on a first start.
pub fn start_from(mark: Option<i64>, now_ms: i64) -> i64 {
    match mark {
        Some(until) => until.clamp(now_ms - CATCH_UP_MS, now_ms),
        None => now_ms,
    }
}

fn read_mark(file: &Path) -> Option<i64> {
    let text = std::fs::read_to_string(file).ok()?;
    serde_json::from_str::<Value>(&text)
        .ok()?
        .get("until")?
        .as_i64()
}

fn write_mark(file: &Path, until: i64) {
    let kept = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(file, json!({ "until": until }).to_string()));
    if let Err(err) = kept {
        warn!(%err, "the scheduler could not keep how far it got");
    }
}

impl Work {
    /// Runs the scheduler until vornd stops.
    pub(crate) async fn schedule(self: Arc<Self>) {
        let mark = mark_file(self.db.path());
        let zone = TimeZone::system();
        let now = now_ms();
        let mut until = start_from(read_mark(&mark), now);
        let mut armed: Option<(Schedule, i64)> = None;
        let mut ticks = tokio::time::interval(TICK);
        loop {
            tokio::select! {
                _ = ticks.tick() => {}
                () = self.rearm.notified() => armed = None,
            }
            let now = now_ms();
            if armed.as_ref().is_none_or(|(_, at)| now - at >= RELOAD_MS) {
                let at = until;
                let zone = zone.clone();
                let read = self.db.run(move |store| {
                    let list = store.call("dbListWorkflows", json!([])).ok()?;
                    Some(Schedule::arm(list.as_array()?, &zone, at))
                });
                if let Some(Some(schedule)) = read.await {
                    armed = Some((schedule, now));
                }
            }
            let Some((schedule, _)) = &armed else {
                continue;
            };
            let schedule = schedule.clone();
            let after = until;
            let fired = self
                .db
                .run(move |store| deliver_due(store, &schedule, after, now, |_| {}))
                .await
                .unwrap_or_default();
            until = now;
            write_mark(&mark, until);
            for occurrence in fired {
                info!(workflow = %occurrence.workflow_id, minute = occurrence.minute, "a schedule fired");
                self.fire(&occurrence.workflow_id, None).await;
            }
        }
    }

    /// `executeWorkflow` of the scheduler: a connector poll fans out
    /// through the inbox; anything else runs once with `inputs`.
    pub(crate) async fn fire(
        self: &Arc<Self>,
        workflow_id: &str,
        inputs: Option<serde_json::Map<String, Value>>,
    ) {
        let id = workflow_id.to_owned();
        let workflow = self
            .db
            .run(move |s| s.call("dbGetWorkflow", json!([id])).ok())
            .await
            .flatten();
        let Some(workflow) = workflow.filter(Value::is_object) else {
            return;
        };
        let polls = vorn_work::model::Workflow::from_json(&workflow)
            .is_some_and(|w| w.trigger_type() == Some("connectorPoll"));
        if !polls {
            self.engine.run_scheduled(workflow_id, inputs).await;
            return;
        }
        if !self
            .polls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(workflow_id.to_owned())
        {
            info!(workflow = %workflow_id, "a poll is already in flight");
            return;
        }
        let work = Arc::clone(self);
        let id = workflow_id.to_owned();
        tokio::spawn(async move {
            let connectors = work.native.upgrade().and_then(|n| n.connectors().cloned());
            match connectors {
                Some(connectors) => {
                    let polled = tokio::time::timeout(POLL_LIMIT, connectors.poll(&id)).await;
                    if polled.is_err() {
                        warn!(workflow = %id, "a connector poll took too long");
                    }
                }
                None => warn!(workflow = %id, "no connectors to poll with"),
            }
            work.polls
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
            work.drain.notify_one();
        });
    }

    /// Runs what the inbox holds, now and whenever a row is added or
    /// settled, and every half minute regardless.
    pub(crate) async fn deliver_inbox(self: Arc<Self>) {
        self.db
            .run(|s| vorn_work::inbox::release_leases(s, now_ms()))
            .await;
        let mut every =
            tokio::time::interval(Duration::from_millis(vorn_work::inbox::DRAIN_INTERVAL_MS));
        loop {
            tokio::select! {
                _ = every.tick() => {}
                () = self.drain.notified() => {}
            }
            let due = self
                .db
                .run(|s| vorn_work::inbox::claim_due(s, now_ms()))
                .await
                .unwrap_or_default();
            for row in due {
                let engine = self.engine.clone();
                let existing = row.existing.and_then(|e| serde_json::from_value(e).ok());
                tokio::spawn(async move {
                    engine
                        .run_connector_item(&row.workflow_id, row.item, existing)
                        .await;
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restart_catches_up_from_where_it_got_and_no_further_back() {
        let now = 1_000_000_000;
        assert_eq!(start_from(None, now), now);
        assert_eq!(start_from(Some(now - 60_000), now), now - 60_000);
        assert_eq!(
            start_from(Some(now - 10 * CATCH_UP_MS), now),
            now - CATCH_UP_MS
        );
        assert_eq!(start_from(Some(now + 5_000), now), now);
    }

    #[test]
    fn keeps_and_reads_its_mark() {
        let dir = tempfile::tempdir().unwrap();
        let file = mark_file(&dir.path().join("vorn.db"));
        assert_eq!(read_mark(&file), None);
        write_mark(&file, 42);
        assert_eq!(read_mark(&file), Some(42));
    }
}
