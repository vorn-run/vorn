//! A workflow's trigger, as the scheduler reads it: the `config` of the
//! first node whose `type` is `"trigger"`, read field by field, with a
//! missing field or one of another type left absent.

use std::collections::BTreeMap;

use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use serde_json::Value;

/// How a trigger type fires, as the extension that declares it says: the
/// core's scheduler arms the timed ones and its webhook route the webhooks.
/// No trigger type is known here by name; the workflow registry answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timing {
    /// On the config's `cron`, in its `timezone` or the system's.
    Cron,
    /// Once, at the config's `runAt`.
    Once,
    /// By a request to the workflow's webhook address, its `method` and `token`.
    Webhook,
}

/// The timing of each trigger type that has one, by type id.
pub type Timings = BTreeMap<String, Timing>;

/// What the scheduler makes of a workflow's trigger.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Trigger<'a> {
    /// No trigger node, or one without a config.
    Missing,
    /// A [`Timing::Cron`] type: fires on `cron`, in `timezone` or the
    /// system's. The fields are as stored; either may be absent.
    Cron {
        cron: Option<&'a Value>,
        timezone: Option<&'a Value>,
    },
    /// A [`Timing::Once`] type: fires at `runAt`, as stored.
    Once { run_at: Option<&'a Value> },
    /// A trigger the scheduler does not arm.
    Other,
    /// Nodes that cannot be searched: not an array, or a `null` before the
    /// trigger.
    Unreadable,
}

impl<'a> Trigger<'a> {
    /// `getTriggerConfig(workflow)`, from the workflow's `nodes`, its type
    /// timed as `timings` says.
    pub fn of(workflow: &'a Value, timings: &Timings) -> Trigger<'a> {
        let Some(nodes) = workflow.get("nodes").and_then(Value::as_array) else {
            return Trigger::Unreadable;
        };
        for node in nodes {
            match node {
                Value::Null => return Trigger::Unreadable,
                Value::Object(n) if n.get("type").and_then(Value::as_str) == Some("trigger") => {
                    return Trigger::from_config(n.get("config"), timings);
                }
                _ => {}
            }
        }
        Trigger::Missing
    }

    fn from_config(config: Option<&'a Value>, timings: &Timings) -> Trigger<'a> {
        let config = match config {
            None | Some(Value::Null) => return Trigger::Missing,
            Some(c) => c,
        };
        let kind = config.get("triggerType").and_then(Value::as_str);
        match kind.and_then(|k| timings.get(k)) {
            Some(Timing::Cron) => Trigger::Cron {
                cron: config.get("cron"),
                timezone: config.get("timezone"),
            },
            Some(Timing::Once) => Trigger::Once {
                run_at: config.get("runAt"),
            },
            _ => Trigger::Other,
        }
    }
}

/// The trigger's `timezone`, or `system` when that is falsy, as a zone;
/// `None` for an unknown name, which never fires.
pub fn zone_of(timezone: Option<&Value>, system: &TimeZone) -> Option<TimeZone> {
    match timezone {
        None | Some(Value::Null | Value::Bool(false)) => Some(system.clone()),
        Some(Value::String(s)) if s.is_empty() => Some(system.clone()),
        Some(Value::Number(n)) if n.as_f64() == Some(0.0) => Some(system.clone()),
        Some(Value::String(name)) => TimeZone::get(name).ok(),
        Some(_) => None,
    }
}

/// Unix milliseconds of an ISO date-time string: `YYYY-MM-DD` (UTC), or
/// with `THH:mm`, seconds and a fraction, and `Z` or `±HH:mm` (local time in
/// `system` without one). `None` for anything else.
pub fn js_date(text: &str, system: &TimeZone) -> Option<i64> {
    let (date, time) = match text.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (text, None),
    };
    let [y, m, d] = split_digits::<3>(date, '-', &[4, 2, 2])?;
    let Some(time) = time else {
        let civil = DateTime::new(y as i16, m as i8, d as i8, 0, 0, 0, 0).ok()?;
        return civil
            .to_zoned(TimeZone::UTC)
            .ok()
            .map(|z| z.timestamp().as_millisecond());
    };
    let (clock, offset) = split_offset(time)?;
    let (hm_s, fraction) = match clock.split_once('.') {
        Some((c, f)) if !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()) => (c, Some(f)),
        Some(_) => return None,
        None => (clock, None),
    };
    let (h, mi, s) = match hm_s.len() {
        5 if fraction.is_none() => {
            let [h, mi] = split_digits::<2>(hm_s, ':', &[2, 2])?;
            (h, mi, 0)
        }
        8 => {
            let [h, mi, s] = split_digits::<3>(hm_s, ':', &[2, 2, 2])?;
            (h, mi, s)
        }
        _ => return None,
    };
    // Only the first three digits are milliseconds.
    let ms = fraction.map_or(0, |f| {
        let three: String = f.chars().chain("00".chars()).take(3).collect();
        three.parse::<i32>().unwrap_or(0)
    });
    let civil = DateTime::new(
        y as i16,
        m as i8,
        d as i8,
        h as i8,
        mi as i8,
        s as i8,
        ms * 1_000_000,
    )
    .ok()?;
    let zone = match offset {
        Some(seconds) => TimeZone::fixed(jiff::tz::Offset::from_seconds(seconds).ok()?),
        None => system.clone(),
    };
    civil
        .to_zoned(zone)
        .ok()
        .map(|z| z.timestamp().as_millisecond())
}

/// `N` runs of exactly `widths` ASCII digits joined by `sep`.
fn split_digits<const N: usize>(text: &str, sep: char, widths: &[usize; N]) -> Option<[i64; N]> {
    let mut out = [0i64; N];
    let mut parts = text.split(sep);
    for (slot, width) in out.iter_mut().zip(widths) {
        let part = parts.next()?;
        if part.len() != *width || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *slot = part.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}

/// The clock and its offset in seconds: `Z`, `±HH:mm`, or none.
fn split_offset(time: &str) -> Option<(&str, Option<i32>)> {
    if let Some(clock) = time.strip_suffix('Z') {
        return Some((clock, Some(0)));
    }
    let at = time.rfind(['+', '-']);
    let Some(at) = at else {
        return Some((time, None));
    };
    let sign = if time.as_bytes()[at] == b'-' { -1 } else { 1 };
    let [h, m] = split_digits::<2>(&time[at + 1..], ':', &[2, 2])?;
    if h > 23 || m > 59 {
        return None;
    }
    Some((&time[..at], Some(sign * (h as i32 * 3600 + m as i32 * 60))))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    fn trigger_config(config: Value) -> Value {
        json!({ "nodes": [{ "id": "a", "type": "step" }, { "id": "t", "type": "trigger", "config": config }] })
    }

    /// The timings Vorn's own trigger types declare.
    pub(crate) fn timings() -> Timings {
        [
            ("recurring", Timing::Cron),
            ("connectorPoll", Timing::Cron),
            ("once", Timing::Once),
            ("webhook", Timing::Webhook),
        ]
        .into_iter()
        .map(|(k, t)| (k.to_owned(), t))
        .collect()
    }

    #[test]
    fn reads_the_first_trigger_nodes_config() {
        let wf = trigger_config(json!({ "triggerType": "recurring", "cron": "* * * * *" }));
        assert_eq!(
            Trigger::of(&wf, &timings()),
            Trigger::Cron {
                cron: Some(&json!("* * * * *")),
                timezone: None
            }
        );
        let wf = trigger_config(
            json!({ "triggerType": "connectorPoll", "cron": "0 * * * *", "timezone": "UTC" }),
        );
        assert!(matches!(
            Trigger::of(&wf, &timings()),
            Trigger::Cron {
                timezone: Some(_),
                ..
            }
        ));
        let wf = trigger_config(json!({ "triggerType": "once", "runAt": "2030-01-01T00:00:00Z" }));
        assert!(matches!(
            Trigger::of(&wf, &timings()),
            Trigger::Once { run_at: Some(_) }
        ));
        let wf = trigger_config(json!({ "triggerType": "manual" }));
        assert_eq!(Trigger::of(&wf, &timings()), Trigger::Other);
        assert_eq!(
            Trigger::of(&trigger_config(Value::Null), &timings()),
            Trigger::Missing
        );
        assert_eq!(
            Trigger::of(&json!({ "nodes": [] }), &timings()),
            Trigger::Missing
        );
        assert_eq!(
            Trigger::of(&json!({ "nodes": [1, "x"] }), &timings()),
            Trigger::Missing
        );
    }

    #[test]
    fn nodes_javascript_cannot_search_are_unreadable() {
        assert_eq!(Trigger::of(&json!({}), &timings()), Trigger::Unreadable);
        assert_eq!(
            Trigger::of(&json!({ "nodes": {} }), &timings()),
            Trigger::Unreadable
        );
        assert_eq!(
            Trigger::of(
                &json!({ "nodes": [null, { "type": "trigger" }] }),
                &timings()
            ),
            Trigger::Unreadable
        );
    }

    #[test]
    fn a_falsy_zone_is_the_systems_and_an_unknown_one_none() {
        let system = TimeZone::get("Europe/Madrid").unwrap();
        for falsy in [
            None,
            Some(json!(null)),
            Some(json!("")),
            Some(json!(false)),
            Some(json!(0)),
        ] {
            assert_eq!(zone_of(falsy.as_ref(), &system), Some(system.clone()));
        }
        assert_eq!(
            zone_of(Some(&json!("Asia/Tokyo")), &system),
            TimeZone::get("Asia/Tokyo").ok()
        );
        assert_eq!(zone_of(Some(&json!("Not/AZone")), &system), None);
        assert_eq!(zone_of(Some(&json!(true)), &system), None);
    }

    #[test]
    fn reads_iso_dates() {
        let tokyo = TimeZone::get("Asia/Tokyo").unwrap();
        let utc = |s: &str| s.parse::<jiff::Timestamp>().unwrap().as_millisecond();
        assert_eq!(
            js_date("2030-01-02", &tokyo),
            Some(utc("2030-01-02T00:00:00Z"))
        );
        assert_eq!(
            js_date("2030-01-02T03:04:05.678Z", &tokyo),
            Some(utc("2030-01-02T03:04:05.678Z"))
        );
        assert_eq!(
            js_date("2030-01-02T03:04:05.6789+02:00", &tokyo),
            Some(utc("2030-01-02T01:04:05.678Z"))
        );
        assert_eq!(
            js_date("2030-01-02T03:04:05.5Z", &tokyo),
            Some(utc("2030-01-02T03:04:05.500Z"))
        );
        // No offset on a date-time: local time.
        assert_eq!(
            js_date("2030-01-02T09:00", &tokyo),
            Some(utc("2030-01-02T00:00:00Z"))
        );
        assert_eq!(
            js_date("2030-01-02T09:00-05:30", &tokyo),
            Some(utc("2030-01-02T14:30:00Z"))
        );
        for other in [
            "",
            "tomorrow",
            "2030-1-2",
            "2030-01-02T9:00",
            "2030-02-30",
            "2030-01-02T09:00:00.Z",
            "2030-01-02T25:00Z",
        ] {
            assert_eq!(js_date(other, &tokyo), None, "{other}");
        }
    }
}
