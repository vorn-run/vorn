//! The new-item poll: per-feed state in a cursor, and the items newer than
//! it, as the connector's `poll.ts` keeps them.

use std::collections::{BTreeMap, HashSet};

use vorn_extension_sdk::{json, Value};

use crate::dates::iso_ms;
use crate::feed::{sha256_hex, Item};

/// Ids remembered per feed; arXiv alone stamps hundreds of items with one date.
pub const SEEN_CAP: usize = 1000;

/// What the cursor keeps for one feed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FeedState {
    /// The newest date seen, publishedAt else updatedAt.
    pub t: String,
    pub etag: Option<String>,
    pub lm: Option<String>,
    /// Hashed ids of the items at `t` and of every dateless item still in the feed.
    pub seen: Vec<String>,
}

pub fn seen_key(id: &str) -> String {
    sha256_hex(id)[..12].to_owned()
}

/// Per-feed state from a cursor; anything unreadable starts afresh.
pub fn read_cursor(raw: Option<&str>) -> BTreeMap<String, FeedState> {
    let mut states = BTreeMap::new();
    let Some(parsed) = raw.and_then(|r| vorn_extension_sdk::serde_json::from_str::<Value>(r).ok()) else {
        return states;
    };
    if parsed["v"] != json!(1) {
        return states;
    }
    let Some(feeds) = parsed["feeds"].as_object() else { return states };
    for (url, state) in feeds {
        if !state.is_object() {
            continue;
        }
        let text = |k: &str| state[k].as_str().map(str::to_owned);
        states.insert(
            url.clone(),
            FeedState {
                t: text("t").unwrap_or_default(),
                etag: text("etag"),
                lm: text("lm"),
                seen: state["seen"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|s| s.as_str().map(str::to_owned))
                    .collect(),
            },
        );
    }
    states
}

/// The cursor as JSON, feeds in the order they were polled.
pub fn write_cursor(states: &[(String, FeedState)]) -> String {
    let feeds: vorn_extension_sdk::serde_json::Map<String, Value> = states
        .iter()
        .map(|(url, s)| {
            let mut o = vorn_extension_sdk::serde_json::Map::new();
            o.insert("t".into(), json!(s.t));
            if let Some(e) = &s.etag {
                o.insert("etag".into(), json!(e));
            }
            if let Some(l) = &s.lm {
                o.insert("lm".into(), json!(l));
            }
            o.insert("seen".into(), json!(s.seen));
            (url.clone(), Value::Object(o))
        })
        .collect();
    json!({ "v": 1, "feeds": feeds }).to_string()
}

/// The items of one feed to fire and the state to keep: newer than `t`, or on `t` with an unseen id; dateless ones once.
pub fn select_new(items: &[Item], previous: Option<&FeedState>, floor_ms: f64) -> (Vec<Item>, FeedState) {
    let seen: HashSet<&str> = previous.map(|p| p.seen.iter().map(String::as_str).collect()).unwrap_or_default();
    let t = previous.map_or("", |p| p.t.as_str());
    let mut fired = HashSet::new();
    let mut fire = Vec::new();
    let mut newest = t.to_owned();
    for item in items {
        let at = item.time();
        let key = seen_key(&item.id);
        if at > newest.as_str() {
            at.clone_into(&mut newest);
        }
        let fresh = match previous {
            #[allow(clippy::cast_precision_loss)]
            None => !at.is_empty() && iso_ms(at).is_some_and(|ms| ms as f64 >= floor_ms),
            Some(_) if at.is_empty() => !seen.contains(key.as_str()),
            Some(_) => at > t || (at == t && !seen.contains(key.as_str())),
        };
        if fresh && fired.insert(key) {
            fire.push(item.clone());
        }
    }
    let carried = if newest == t { previous.map(|p| p.seen.as_slice()).unwrap_or_default() } else { &[] };
    let mut kept: Vec<String> = Vec::new();
    let keys = items
        .iter()
        .filter(|i| i.time().is_empty() || i.time() == newest)
        .map(|i| seen_key(&i.id))
        .chain(carried.iter().cloned());
    for key in keys {
        if !kept.contains(&key) {
            kept.push(key);
        }
    }
    kept.truncate(SEEN_CAP);
    (fire, FeedState { t: newest, seen: kept, ..FeedState::default() })
}

/// Oldest first, dateless last.
pub fn oldest_first(items: &mut [Item]) {
    items.sort_by(|a, b| {
        let (at, bt) = (a.time(), b.time());
        match (at.is_empty(), bt.is_empty()) {
            _ if at == bt => std::cmp::Ordering::Equal,
            (true, _) => std::cmp::Ordering::Greater,
            (_, true) => std::cmp::Ordering::Less,
            _ => at.cmp(bt),
        }
    });
}

/// An item in the shape the connector's trigger fires it.
pub fn to_connector_item(item: &Item) -> Value {
    let mut o = vorn_extension_sdk::serde_json::Map::new();
    let at = item.time();
    o.insert("externalId".into(), json!(format!("{} {}", item.feed_url, item.id)));
    let title = [&item.title, &item.url].into_iter().find(|s| !s.is_empty());
    o.insert("title".into(), json!(title.map_or("Untitled item", String::as_str)));
    for (key, value) in [("url", &item.url), ("description", &item.summary), ("assignee", &item.author)] {
        if !value.is_empty() {
            o.insert(key.into(), json!(value));
        }
    }
    o.insert("labels".into(), json!(item.categories));
    if !at.is_empty() {
        o.insert("updatedAt".into(), json!(at));
    }
    o.insert("data".into(), item.to_json());
    Value::Object(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, t: &str) -> Item {
        Item { id: id.into(), published_at: t.into(), ..Item::default() }
    }

    #[test]
    fn a_second_poll_fires_only_what_is_newer() {
        let first = [item("a", "2024-01-01T00:00:00.000Z"), item("b", "")];
        let (fire, state) = select_new(&first, None, 0.0);
        assert_eq!(fire.len(), 1, "the dateless item waits for a known feed");
        let second = [item("c", "2024-01-02T00:00:00.000Z"), item("a", "2024-01-01T00:00:00.000Z"), item("b", "")];
        let (fire, state) = select_new(&second, Some(&state), 0.0);
        assert_eq!(fire.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["c"]);
        assert_eq!(state.t, "2024-01-02T00:00:00.000Z");
        let cursor = write_cursor(&[("u".into(), state.clone())]);
        assert_eq!(read_cursor(Some(&cursor)).get("u"), Some(&state));
    }
}
