//! Footer bands: the readings each session's extensions publish, and when
//! each is read again.
//!
//! This is the bookkeeping only. The caller runs the timers and the calls,
//! and is told which pollers to start or stop and what to publish, so the
//! rules (a failing band slows down, an unchanged reading says nothing, a
//! band that went away takes its reading with it) are tested without time.

use std::collections::HashMap;

use serde::Serialize;
use serde_json::Value;

use crate::js;
use crate::manifest::MIN_FOOTER_SECONDS;

/// Most items a band draws.
const MAX_ITEMS: usize = 12;
/// Longest label or value drawn, in UTF-16 units.
const MAX_ITEM_TEXT: usize = 60;
/// How much slower a failing band is read, and how much faster once it recovers.
const FAILURE_BACKOFF: u64 = 5;

/// One label and value in a band.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Item {
    pub label: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
}

/// A band as a session's windows draw it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    pub extension_id: String,
    pub extension_name: String,
    pub footer_id: String,
    pub title: String,
    pub items: Vec<Item>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// ISO 8601, as `Date.prototype.toISOString` writes it.
    pub computed_at: String,
}

/// One band on one session.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    pub session: String,
    pub extension: String,
    pub footer: String,
}

/// What a band was declared with; a change to any of it starts it again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declared {
    pub every_ms: u64,
    pub title: String,
    pub version: String,
}

impl Declared {
    /// `every` seconds, never under the floor a manifest is held to.
    pub fn new(every: f64, title: String, version: String) -> Declared {
        // Finite and at least the floor, as the manifest reader keeps it.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let every_ms = (every.max(MIN_FOOTER_SECONDS) * 1000.0) as u64;
        Declared {
            every_ms,
            title,
            version,
        }
    }
}

#[derive(Debug)]
struct Poller {
    declared: Declared,
    every_ms: u64,
    failing: bool,
}

/// What [`Footers::sync`] asks of the caller.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Start (or start again) these, reading once now and then every so often.
    pub start: Vec<(Key, u64)>,
    /// Stop these.
    pub stop: Vec<Key>,
}

/// What a reading's outcome asks of the caller.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Tell the session's windows these readings.
    pub publish: Option<Vec<Reading>>,
    /// Read this band at this new interval from now on.
    pub retime: Option<u64>,
}

/// Every band's poller and last reading.
#[derive(Debug, Default)]
pub struct Footers {
    pollers: HashMap<Key, Poller>,
    readings: HashMap<Key, Reading>,
}

impl Footers {
    /// Makes `session`'s bands exactly `wanted`.
    pub fn sync(&mut self, session: &str, wanted: Vec<(Key, Declared)>) -> Plan {
        let mut plan = Plan::default();
        let keep: Vec<Key> = wanted.iter().map(|(k, _)| k.clone()).collect();
        for (key, declared) in wanted {
            if self
                .pollers
                .get(&key)
                .is_some_and(|p| p.declared == declared)
            {
                continue;
            }
            plan.start.push((key.clone(), declared.every_ms));
            self.pollers.insert(
                key,
                Poller {
                    every_ms: declared.every_ms,
                    declared,
                    failing: false,
                },
            );
        }
        let gone: Vec<Key> = self
            .pollers
            .keys()
            .filter(|k| k.session == session && !keep.contains(k))
            .cloned()
            .collect();
        for key in gone {
            self.pollers.remove(&key);
            self.readings.remove(&key);
            plan.stop.push(key);
        }
        plan
    }

    /// Forgets every band of `session`, returning the ones to stop.
    pub fn stop_session(&mut self, session: &str) -> Vec<Key> {
        let gone: Vec<Key> = self
            .pollers
            .keys()
            .filter(|k| k.session == session)
            .cloned()
            .collect();
        for key in &gone {
            self.pollers.remove(key);
        }
        self.readings.retain(|k, _| k.session != session);
        gone
    }

    /// Whether `key` is still wanted.
    pub fn has(&self, key: &Key) -> bool {
        self.pollers.contains_key(key)
    }

    /// Records what reading `key` gave: its items, or why it failed. A
    /// failure keeps the last good items beside the error.
    pub fn record(
        &mut self,
        key: &Key,
        extension_name: &str,
        answer: Result<Vec<Item>, String>,
        computed_at: String,
    ) -> Outcome {
        let Some(poller) = self.pollers.get_mut(key) else {
            return Outcome::default();
        };
        let title = poller.declared.title.clone();
        let failed = answer.is_err();
        let retime = if failed != poller.failing {
            poller.failing = failed;
            poller.every_ms = if failed {
                poller.every_ms * FAILURE_BACKOFF
            } else {
                poller.every_ms / FAILURE_BACKOFF
            };
            Some(poller.every_ms)
        } else {
            None
        };
        let (items, error) = match answer {
            Ok(items) => (items, None),
            Err(error) => (
                self.readings
                    .get(key)
                    .map(|r| r.items.clone())
                    .unwrap_or_default(),
                Some(error),
            ),
        };
        let reading = Reading {
            extension_id: key.extension.clone(),
            extension_name: extension_name.to_owned(),
            footer_id: key.footer.clone(),
            title,
            items,
            error,
            computed_at,
        };
        let unchanged = self
            .readings
            .get(key)
            .is_some_and(|r| r.error == reading.error && r.items == reading.items);
        let publish = (!unchanged).then(|| {
            self.readings.insert(key.clone(), reading);
            self.readings_of(&key.session)
        });
        Outcome { publish, retime }
    }

    /// `session`'s readings, by extension then footer.
    pub fn readings_of(&self, session: &str) -> Vec<Reading> {
        let mut found: Vec<(&Key, &Reading)> = self
            .readings
            .iter()
            .filter(|(k, _)| k.session == session)
            .collect();
        found.sort_by(|a, b| (&a.0.extension, &a.0.footer).cmp(&(&b.0.extension, &b.0.footer)));
        found.into_iter().map(|(_, r)| r.clone()).collect()
    }
}

/// Reads the items a footer answered with, or why the band cannot draw them.
pub fn read_items(answer: &Value) -> Result<Vec<Item>, &'static str> {
    let Some(raw) = answer.get("items").and_then(Value::as_array) else {
        return Err("a footer answers with items");
    };
    raw.iter()
        .take(MAX_ITEMS)
        .map(|entry| {
            let (Some(label), Some(value)) = (
                entry.get("label").and_then(Value::as_str),
                entry.get("value").and_then(Value::as_str),
            ) else {
                return Err("every item carries a label and a value");
            };
            let href = match entry.get("href").and_then(Value::as_str) {
                None | Some("") => None,
                Some(href) if is_web_link(href) => Some(href.to_owned()),
                Some(_) => return Err("an item links to http or https, or to nothing"),
            };
            Ok(Item {
                label: js::slice16(label, MAX_ITEM_TEXT).to_owned(),
                value: js::slice16(value, MAX_ITEM_TEXT).to_owned(),
                tone: entry
                    .get("tone")
                    .and_then(Value::as_str)
                    .filter(|t| matches!(*t, "default" | "ok" | "danger"))
                    .map(str::to_owned),
                href,
            })
        })
        .collect()
}

/// Whether `href` parses as an `http:` or `https:` URL with a host.
pub fn is_web_link(href: &str) -> bool {
    let href = js::trim(href);
    let Some((scheme, rest)) = href.split_once(':') else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return false;
    }
    let rest = rest.trim_start_matches(['/', '\\']);
    let authority = rest.split(['/', '\\', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = host.split(':').next().unwrap_or_default();
    !host.is_empty() && !host.contains(|c: char| c.is_whitespace() || "<>^|%".contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key(session: &str, ext: &str) -> Key {
        Key {
            session: session.into(),
            extension: ext.into(),
            footer: "f".into(),
        }
    }

    fn declared(every: f64) -> Declared {
        Declared::new(every, "T".into(), "1".into())
    }

    fn items(label: &str) -> Vec<Item> {
        read_items(&json!({ "items": [{ "label": label, "value": "v" }] })).unwrap()
    }

    #[test]
    fn starts_a_band_once_and_again_when_its_declaration_changes() {
        let mut f = Footers::default();
        let plan = f.sync("s", vec![(key("s", "a"), declared(5.0))]);
        assert_eq!(plan.start, vec![(key("s", "a"), 5000)]);
        assert_eq!(
            f.sync("s", vec![(key("s", "a"), declared(5.0))]),
            Plan::default()
        );
        assert_eq!(
            f.sync("s", vec![(key("s", "a"), declared(10.0))])
                .start
                .len(),
            1
        );
        let plan = f.sync("s", vec![]);
        assert_eq!(plan.stop, vec![key("s", "a")]);
        assert!(!f.has(&key("s", "a")));
    }

    #[test]
    fn publishes_only_a_reading_that_moved() {
        let mut f = Footers::default();
        f.sync("s", vec![(key("s", "a"), declared(5.0))]);
        let first = f.record(&key("s", "a"), "A", Ok(items("x")), "t1".into());
        assert_eq!(first.publish.unwrap()[0].items[0].label, "x");
        assert_eq!(
            f.record(&key("s", "a"), "A", Ok(items("x")), "t2".into()),
            Outcome::default()
        );
        assert!(f
            .record(&key("s", "b"), "B", Ok(items("x")), "t".into())
            .publish
            .is_none());
    }

    #[test]
    fn keeps_the_last_items_beside_an_error_and_slows_down() {
        let mut f = Footers::default();
        let k = key("s", "a");
        f.sync("s", vec![(k.clone(), declared(5.0))]);
        f.record(&k, "A", Ok(items("x")), "t".into());
        let failed = f.record(&k, "A", Err("boom".into()), "t".into());
        assert_eq!(failed.retime, Some(25_000));
        let reading = &failed.publish.unwrap()[0];
        assert_eq!(
            (reading.items.len(), reading.error.as_deref()),
            (1, Some("boom"))
        );
        assert_eq!(
            f.record(&k, "A", Err("boom".into()), "t".into()),
            Outcome::default()
        );
        assert_eq!(
            f.record(&k, "A", Ok(items("x")), "t".into()).retime,
            Some(5_000)
        );
    }

    #[test]
    fn orders_readings_and_forgets_a_session() {
        let mut f = Footers::default();
        f.sync(
            "s",
            vec![
                (key("s", "b"), declared(5.0)),
                (key("s", "a"), declared(5.0)),
            ],
        );
        f.sync("t", vec![(key("t", "a"), declared(5.0))]);
        for k in [key("s", "b"), key("s", "a"), key("t", "a")] {
            f.record(&k, "N", Ok(items("x")), "t".into());
        }
        let ids: Vec<String> = f
            .readings_of("s")
            .into_iter()
            .map(|r| r.extension_id)
            .collect();
        assert_eq!(ids, ["a", "b"]);
        assert_eq!(f.stop_session("s").len(), 2);
        assert!(f.readings_of("s").is_empty());
        assert_eq!(f.readings_of("t").len(), 1);
    }

    #[test]
    fn reads_items_a_band_can_draw() {
        let long = "x".repeat(70);
        let read = read_items(&json!({ "items": [
            { "label": long, "value": "v", "tone": "ok", "href": "https://a.b/c" },
            { "label": "l", "value": "v", "tone": "loud", "href": "" }
        ] }))
        .unwrap();
        assert_eq!(read[0].label.len(), 60);
        assert_eq!(read[0].tone.as_deref(), Some("ok"));
        assert_eq!(read[1].tone, None);
        assert_eq!(read[1].href, None);
        let many: Vec<Value> = (0..20)
            .map(|_| json!({ "label": "a", "value": "b" }))
            .collect();
        assert_eq!(read_items(&json!({ "items": many })).unwrap().len(), 12);
        assert_eq!(read_items(&json!({})), Err("a footer answers with items"));
        assert_eq!(
            read_items(&json!({ "items": [{ "label": 1, "value": "v" }] })),
            Err("every item carries a label and a value")
        );
        for bad in ["javascript:alert(1)", "not a url", "http://", "file:///etc"] {
            assert_eq!(
                read_items(&json!({ "items": [{ "label": "l", "value": "v", "href": bad }] })),
                Err("an item links to http or https, or to nothing"),
                "{bad}"
            );
        }
    }
}
