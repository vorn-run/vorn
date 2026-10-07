//! The workflow schedules the server arms, and the occurrences they fire.
//!
//! The server arms every enabled workflow whose trigger is `recurring` or
//! `connectorPoll` with a cron node-cron accepts, and every `once` whose
//! `runAt` is still ahead. An occurrence is one workflow in one Unix minute:
//! the server writes one tick lock per workflow per minute, however many
//! seconds of that minute the cron names, so the minute is the unit both
//! sides count and the key a receiver deduplicates on.

use jiff::tz::TimeZone;
use serde_json::Value;

use crate::cron::Cron;
use crate::is_truthy;
use crate::trigger::{js_date, zone_of, Trigger};

/// How long a server takes to notice a minute it fires in; a planned
/// occurrence not seen by then is a difference.
pub const FIRE_GRACE_MS: i64 = 10_000;

/// One workflow's armed trigger.
#[derive(Clone, Debug, PartialEq)]
enum Arm {
    Cron(Cron, TimeZone),
    Once(i64),
}

/// The armed schedules of a set of workflows, as one server sync arms them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Schedule {
    armed: Vec<(String, Arm)>,
}

/// A workflow firing in one minute.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Occurrence {
    /// Unix minute it fires in.
    pub minute: i64,
    /// The workflow it fires.
    pub workflow_id: String,
    /// Unix milliseconds of its first fire in the minute.
    pub at_ms: i64,
}

impl Occurrence {
    /// The key a receiver claims, so a second delivery of it is dropped.
    pub fn effect_id(&self) -> String {
        format!("schedule/{}/{}", self.workflow_id, self.minute)
    }

    /// The server's tick lock for this occurrence, in its lock directory.
    pub fn lock_name(&self) -> String {
        lock_name(&self.workflow_id, self.minute)
    }
}

/// `scheduler-<workflowId>-<minute>.lock`, the server's tick lock name.
pub fn lock_name(workflow_id: &str, minute: i64) -> String {
    format!("scheduler-{workflow_id}-{minute}.lock")
}

/// The workflow and minute of a tick lock name, when it is one. The minute
/// is the trailing digits, since a workflow id may hold dashes.
pub fn parse_lock_name(name: &str) -> Option<(&str, i64)> {
    let rest = name.strip_prefix("scheduler-")?.strip_suffix(".lock")?;
    let (id, minute) = rest.rsplit_once('-')?;
    if id.is_empty() || minute.is_empty() || !minute.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((id, minute.parse().ok()?))
}

impl Schedule {
    /// What a sync at `now_ms` arms for `workflows` (the stored workflow
    /// rows), with `system` the zone a trigger without one runs in.
    pub fn arm(workflows: &[Value], system: &TimeZone, now_ms: i64) -> Schedule {
        let armed = workflows
            .iter()
            .filter(|wf| is_truthy(wf.get("enabled")))
            .filter_map(|wf| {
                let id = wf.get("id")?.as_str()?;
                let arm = match Trigger::of(wf) {
                    Trigger::Cron { cron, timezone } => {
                        let cron = Cron::parse(cron?.as_str()?).ok()?;
                        Arm::Cron(cron, zone_of(timezone, system)?)
                    }
                    Trigger::Once { run_at } => {
                        let at = js_date(run_at?.as_str()?, system)?;
                        (at > now_ms).then_some(Arm::Once(at))?
                    }
                    _ => return None,
                };
                Some((id.to_owned(), arm))
            })
            .collect();
        Schedule { armed }
    }

    /// Whether `workflow_id` is armed.
    pub fn arms(&self, workflow_id: &str) -> bool {
        self.armed.iter().any(|(id, _)| id == workflow_id)
    }

    /// The occurrences that fire after `after_ms` up to `until_ms`, both Unix
    /// milliseconds, in minute order.
    pub fn due(&self, after_ms: i64, until_ms: i64) -> Vec<Occurrence> {
        let mut due = Vec::new();
        if until_ms <= after_ms {
            return due;
        }
        let first = after_ms.div_euclid(60_000);
        let last = until_ms.div_euclid(60_000);
        for (id, arm) in &self.armed {
            match arm {
                Arm::Cron(cron, tz) => {
                    for minute in first..=last {
                        if let Some(at) = cron.fires_in(minute, tz) {
                            if at > after_ms && at <= until_ms {
                                due.push(occurrence(id, minute, at));
                            }
                        }
                    }
                }
                Arm::Once(at) if *at > after_ms && *at <= until_ms => {
                    due.push(occurrence(id, at.div_euclid(60_000), *at));
                }
                Arm::Once(_) => {}
            }
        }
        due.sort();
        due
    }
}

fn occurrence(id: &str, minute: i64, at_ms: i64) -> Occurrence {
    Occurrence {
        minute,
        workflow_id: id.to_owned(),
        at_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ms(text: &str) -> i64 {
        text.parse::<jiff::Timestamp>().unwrap().as_millisecond()
    }

    fn workflow(id: &str, enabled: bool, config: Value) -> Value {
        json!({ "id": id, "enabled": enabled, "nodes": [{ "id": "t", "type": "trigger", "config": config }] })
    }

    fn recurring(id: &str, cron: &str) -> Value {
        workflow(
            id,
            true,
            json!({ "triggerType": "recurring", "cron": cron }),
        )
    }

    #[test]
    fn arms_enabled_valid_schedules_only() {
        let now = ms("2030-01-01T00:00:00Z");
        let workflows = [
            recurring("every-minute", "* * * * *"),
            workflow(
                "off",
                false,
                json!({ "triggerType": "recurring", "cron": "* * * * *" }),
            ),
            recurring("bad", "61 * * * *"),
            workflow("manual", true, json!({ "triggerType": "manual" })),
            workflow(
                "bad-zone",
                true,
                json!({ "triggerType": "recurring", "cron": "* * * * *", "timezone": "Nowhere/Land" }),
            ),
            workflow(
                "number-cron",
                true,
                json!({ "triggerType": "recurring", "cron": 5 }),
            ),
            workflow(
                "past",
                true,
                json!({ "triggerType": "once", "runAt": "2029-12-31T00:00:00Z" }),
            ),
            workflow(
                "ahead",
                true,
                json!({ "triggerType": "once", "runAt": "2030-01-01T00:05:30Z" }),
            ),
            workflow(
                "poll",
                true,
                json!({ "triggerType": "connectorPoll", "cron": "*/2 * * * *" }),
            ),
            json!({ "id": "no-nodes", "enabled": true }),
        ];
        let schedule = Schedule::arm(&workflows, &TimeZone::UTC, now);
        let armed: Vec<&str> = schedule.armed.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(armed, ["every-minute", "ahead", "poll"]);
        assert!(schedule.arms("poll"));
        assert!(!schedule.arms("off"));
    }

    #[test]
    fn one_occurrence_per_workflow_per_minute() {
        let now = ms("2030-01-01T00:00:00Z");
        let workflows = [
            recurring("seconds", "*/10 * * * * *"),
            recurring("even", "*/2 * * * *"),
            workflow(
                "once",
                true,
                json!({ "triggerType": "once", "runAt": "2030-01-01T00:02:30Z" }),
            ),
        ];
        let schedule = Schedule::arm(&workflows, &TimeZone::UTC, now);
        let due = schedule.due(now, now + 3 * 60_000);
        let got: Vec<(i64, &str, i64)> = due
            .iter()
            .map(|o| {
                (
                    o.minute - now / 60_000,
                    o.workflow_id.as_str(),
                    o.at_ms - now,
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                (1, "seconds", 60_000),
                (2, "even", 120_000),
                (2, "once", 150_000),
                (2, "seconds", 120_000),
                (3, "seconds", 180_000),
            ]
        );
        assert_eq!(
            due[1].effect_id(),
            format!("schedule/even/{}", now / 60_000 + 2)
        );
        // A minute is the server's from its first fire, which this window starts after.
        assert!(due.iter().all(|o| o.minute != now / 60_000));
    }

    #[test]
    fn a_window_is_open_at_its_start_and_closed_at_its_end() {
        let now = ms("2030-01-01T00:00:00Z");
        let schedule = Schedule::arm(&[recurring("w", "* * * * *")], &TimeZone::UTC, now);
        assert!(schedule.due(now, now).is_empty());
        assert_eq!(schedule.due(now - 1, now).len(), 1);
        assert!(schedule.due(now, now + 59_999).is_empty());
        assert!(schedule.due(now + 5, now).is_empty());
    }

    #[test]
    fn fires_on_the_wall_clock_of_the_triggers_zone() {
        let now = ms("2030-01-01T00:00:00Z");
        let wf = workflow(
            "nine",
            true,
            json!({ "triggerType": "recurring", "cron": "0 9 * * *", "timezone": "Asia/Tokyo" }),
        );
        let schedule = Schedule::arm(&[wf], &TimeZone::UTC, now);
        let due = schedule.due(now, now + 86_400_000);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].at_ms, ms("2030-01-02T00:00:00Z"));
    }

    #[test]
    fn lock_names_round_trip() {
        let occurrence = occurrence("wf-with-dashes", 29_000_000, 0);
        assert_eq!(
            occurrence.lock_name(),
            "scheduler-wf-with-dashes-29000000.lock"
        );
        assert_eq!(
            parse_lock_name(&occurrence.lock_name()),
            Some(("wf-with-dashes", 29_000_000))
        );
        for other in [
            "scheduler--1.lock",
            "scheduler-x-.lock",
            "scheduler-x-1a.lock",
            "other-x-1.lock",
            "scheduler-x-1",
        ] {
            assert_eq!(parse_lock_name(other), None, "{other}");
        }
    }
}
