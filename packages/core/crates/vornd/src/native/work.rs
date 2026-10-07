//! The work model's calls in shadow mode: workflows, their runs, the
//! schedule log and artifacts, read from the server's database
//! ([`vorn_work::reads`]), and the schedules the server fires, watched.
//!
//! The server still runs every workflow. vornd computes which occurrences
//! fire from the same rows ([`vorn_work::schedule`]) and checks each
//! against the tick lock the server writes when it fires one; it writes
//! nothing, so a schedule never fires twice for being compared.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use jiff::tz::TimeZone;
use serde_json::Value;
use tracing::{debug, warn};
use vorn_store::Store;
use vorn_work::reads::{self, Host, Reply};
use vorn_work::schedule::{parse_lock_name, Schedule, FIRE_GRACE_MS};

use super::{Answer, Native};
use crate::groups::{Counted, Groups};

/// What a fire is counted as in the `scheduler` group.
pub const FIRE_METHOD: &str = "scheduler:fire";

/// How often the watch looks for the server's tick locks.
const TICK: Duration = Duration::from_secs(1);

/// How often it reads the workflows again; the server re-arms on a change.
const RELOAD_MS: i64 = 10_000;

/// How long a lock vornd did not plan waits for a plan: a workflow the
/// server armed on a change vornd has yet to read.
const UNPLANNED_GRACE_MS: i64 = RELOAD_MS + FIRE_GRACE_MS;

/// Answers `method` with `params` from the database, as the server would.
pub fn call(native: &Native, method: &str, params: &Value) -> Answer {
    let Some(data_dir) = native.db.get().and_then(|db| db.parent()) else {
        return Answer::Forward;
    };
    let Some(mut store) = native.store() else {
        return Answer::Forward;
    };
    let zone = TimeZone::system();
    let host = Host {
        data_dir,
        server_port: native.reach.server_port(),
        now_ms: now_ms(),
        zone: &zone,
    };
    match reads::read(&mut store, &host, method, params) {
        Reply::Value(value) => Answer::Result(value),
        Reply::Void => Answer::Void,
        Reply::Server => Answer::Forward,
    }
}

fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

/// What one look at the lock directory found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seen {
    /// The server fired an occurrence vornd planned.
    Fired { workflow_id: String, minute: i64 },
    /// vornd planned an occurrence the server has not fired in time.
    Missed { workflow_id: String, minute: i64 },
    /// The server fired an occurrence vornd did not plan.
    Unplanned { workflow_id: String, minute: i64 },
}

/// The schedules vornd computes, beside the fires the server records.
#[derive(Debug)]
pub struct Watch {
    schedule: Schedule,
    /// Up to when occurrences have been planned, Unix milliseconds.
    planned_until: i64,
    /// Planned occurrences not yet seen fired, by workflow and minute, with
    /// when they count as missed.
    planned: BTreeMap<(String, i64), i64>,
    /// Locks seen without a plan, and when they count as unplanned: the
    /// server can fire a moment before vornd plans.
    unplanned: BTreeMap<(String, i64), i64>,
    /// The lock names already looked at.
    seen: HashSet<String>,
}

impl Watch {
    /// A watch from `now_ms`, with `locks` the names already in the lock
    /// directory: fires from before vornd started are not compared.
    pub fn new(now_ms: i64, locks: impl IntoIterator<Item = String>) -> Watch {
        Watch {
            schedule: Schedule::default(),
            planned_until: now_ms,
            planned: BTreeMap::new(),
            unplanned: BTreeMap::new(),
            seen: locks.into_iter().collect(),
        }
    }

    /// Arms what the server arms for `workflows` at `now_ms`, from now on.
    /// A lock waiting for a plan gets one when the new arming fires then.
    pub fn rearm(&mut self, workflows: &[Value], zone: &TimeZone, now_ms: i64) {
        let next = Schedule::arm(workflows, zone, now_ms);
        let since = self.planned_until - UNPLANNED_GRACE_MS;
        for o in next.due(since, self.planned_until) {
            let key = (o.workflow_id, o.minute);
            if self.unplanned.contains_key(&key) {
                self.planned.insert(key, o.at_ms + FIRE_GRACE_MS);
            }
        }
        self.schedule = next;
    }

    /// Plans what fires up to `now_ms`, then matches `locks`, the names in
    /// the lock directory now, against the plan.
    pub fn tick(&mut self, now_ms: i64, locks: &[String]) -> Vec<Seen> {
        for o in self.schedule.due(self.planned_until, now_ms) {
            self.planned
                .insert((o.workflow_id, o.minute), o.at_ms + FIRE_GRACE_MS);
        }
        self.planned_until = self.planned_until.max(now_ms);

        let mut out = Vec::new();
        for name in locks {
            if self.seen.contains(name) {
                continue;
            }
            if let Some((id, minute)) = parse_lock_name(name) {
                self.unplanned
                    .insert((id.to_owned(), minute), now_ms + UNPLANNED_GRACE_MS);
            }
        }
        // The server removes a workflow's older locks as it fires.
        self.seen = locks.iter().cloned().collect();

        let fired: Vec<(String, i64)> = self
            .unplanned
            .keys()
            .filter(|key| self.planned.contains_key(*key))
            .cloned()
            .collect();
        for key in fired {
            self.planned.remove(&key);
            self.unplanned.remove(&key);
            out.push(Seen::Fired {
                workflow_id: key.0,
                minute: key.1,
            });
        }
        for ((workflow_id, minute), _) in take_expired(&mut self.planned, now_ms) {
            out.push(Seen::Missed {
                workflow_id,
                minute,
            });
        }
        for ((workflow_id, minute), _) in take_expired(&mut self.unplanned, now_ms) {
            out.push(Seen::Unplanned {
                workflow_id,
                minute,
            });
        }
        out
    }
}

fn take_expired(map: &mut BTreeMap<(String, i64), i64>, now_ms: i64) -> Vec<((String, i64), i64)> {
    let expired: Vec<(String, i64)> = map
        .iter()
        .filter(|(_, deadline)| **deadline < now_ms)
        .map(|(key, _)| key.clone())
        .collect();
    expired
        .into_iter()
        .filter_map(|key| map.remove_entry(&key))
        .collect()
}

/// Where the server writes its tick locks: `~/.vorn`, as `os.homedir()` reads it.
pub fn lock_dir() -> PathBuf {
    PathBuf::from(super::shell::home_dir()).join(".vorn")
}

/// The names in the server's lock directory; none when it cannot be read.
fn lock_names(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|name| name.starts_with("scheduler-"))
        .collect()
}

/// The workflows as stored, or `None` when the database cannot be read.
fn workflows(db: &Path) -> Option<Vec<Value>> {
    let mut store = Store::open_beside(db).ok()??;
    match store.call("dbListWorkflows", Value::Array(Vec::new())) {
        Ok(Value::Array(list)) => Some(list),
        Ok(_) => None,
        Err(err) => {
            debug!(%err, "could not read the workflows to watch their schedules");
            None
        }
    }
}

/// Watches the server fire the schedules in `db`'s workflows, counting
/// each fire vornd foresaw, each it missed and each it did not plan, in
/// the `scheduler` group. `lock_dir` is where the server writes its tick
/// locks (`~/.vorn`).
pub async fn watch(db: PathBuf, lock_dir: PathBuf, groups: Arc<Groups>) {
    let start = {
        let dir = lock_dir.clone();
        tokio::task::spawn_blocking(move || lock_names(&dir))
            .await
            .unwrap_or_default()
    };
    let mut state = Some((Watch::new(now_ms(), start), None::<i64>));
    let zone = TimeZone::system();
    let mut ticks = tokio::time::interval(TICK);
    loop {
        ticks.tick().await;
        let Some((mut watch, mut loaded_at)) = state.take() else {
            return;
        };
        let (db, dir, zone) = (db.clone(), lock_dir.clone(), zone.clone());
        let looked = tokio::task::spawn_blocking(move || {
            let now = now_ms();
            if loaded_at.is_none_or(|at| now - at >= RELOAD_MS) {
                if let Some(list) = workflows(&db) {
                    // What was due under the old arming is planned first.
                    let seen = watch.tick(now, &lock_names(&dir));
                    watch.rearm(&list, &zone, now);
                    loaded_at = Some(now);
                    return (watch, loaded_at, seen);
                }
            }
            let seen = watch.tick(now, &lock_names(&dir));
            (watch, loaded_at, seen)
        })
        .await;
        let Ok((watch, loaded_at, seen)) = looked else {
            warn!("the schedule watch stopped");
            return;
        };
        for outcome in seen {
            count(&groups, &outcome);
        }
        state = Some((watch, loaded_at));
    }
}

fn count(groups: &Groups, seen: &Seen) {
    match seen {
        Seen::Fired { .. } => groups.count(FIRE_METHOD, Counted::ShadowMatched),
        Seen::Missed {
            workflow_id,
            minute,
        } => {
            warn!(workflow = %workflow_id, minute, "the server did not fire a schedule vornd planned");
            groups.count(FIRE_METHOD, Counted::ShadowMismatched);
        }
        Seen::Unplanned {
            workflow_id,
            minute,
        } => {
            warn!(workflow = %workflow_id, minute, "the server fired a schedule vornd did not plan");
            groups.count(FIRE_METHOD, Counted::ShadowMismatched);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use vorn_work::schedule::lock_name;

    const MINUTE: i64 = 60_000;

    fn every_minute(id: &str) -> Value {
        json!({ "id": id, "enabled": true, "nodes": [{ "id": "t", "type": "trigger", "config": { "triggerType": "recurring", "cron": "* * * * *" } }] })
    }

    #[test]
    fn a_fire_vornd_planned_matches_and_one_it_did_not_is_reported() {
        let start = 29_000_000 * MINUTE + 30_000;
        let base = start / MINUTE;
        let old = lock_name("a", base);
        let mut watch = Watch::new(start, [old.clone()]);
        watch.rearm(&[every_minute("a")], &TimeZone::UTC, start);

        // The server fires `a` at the next minute, and `b`, which vornd does not arm.
        let next = (base + 1) * MINUTE;
        let locks = [old, lock_name("a", base + 1), lock_name("b", base + 1)];
        let seen = watch.tick(next + 500, &locks);
        assert_eq!(
            seen,
            [Seen::Fired {
                workflow_id: "a".into(),
                minute: base + 1
            }]
        );
        // `b` is given the grace a slow read of the workflows would need, then reported.
        assert!(watch.tick(next + 15_000, &locks).is_empty());
        assert_eq!(
            watch.tick(next + 500 + UNPLANNED_GRACE_MS + 1, &locks),
            [Seen::Unplanned {
                workflow_id: "b".into(),
                minute: base + 1
            }]
        );
    }

    #[test]
    fn a_planned_fire_the_server_skips_is_missed_after_the_grace() {
        let start = 29_000_000 * MINUTE;
        let base = start / MINUTE;
        let mut watch = Watch::new(start, []);
        watch.rearm(&[every_minute("a")], &TimeZone::UTC, start);
        let at = (base + 1) * MINUTE;
        assert!(watch.tick(at + 1_000, &[]).is_empty());
        assert!(watch.tick(at + FIRE_GRACE_MS, &[]).is_empty());
        assert_eq!(
            watch.tick(at + FIRE_GRACE_MS + 1, &[]),
            [Seen::Missed {
                workflow_id: "a".into(),
                minute: base + 1
            }]
        );
    }

    #[test]
    fn a_lock_seen_before_its_plan_still_matches() {
        let start = 29_000_000 * MINUTE;
        let base = start / MINUTE;
        let mut watch = Watch::new(start, []);
        let locks = [lock_name("a", base + 1)];
        // vornd reads the new workflow only after the server fired it.
        let fired = (base + 1) * MINUTE;
        assert!(watch.tick(fired + 100, &locks).is_empty());
        watch.rearm(&[every_minute("a")], &TimeZone::UTC, fired + 8_000);
        assert_eq!(
            watch.tick(fired + 12_000, &locks),
            [Seen::Fired {
                workflow_id: "a".into(),
                minute: base + 1
            }]
        );
    }

    #[test]
    fn reads_the_lock_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("scheduler-a-5.lock"), "1").unwrap();
        std::fs::write(dir.path().join("other.txt"), "1").unwrap();
        assert_eq!(lock_names(dir.path()), ["scheduler-a-5.lock"]);
        assert!(lock_names(&dir.path().join("missing")).is_empty());
    }
}
