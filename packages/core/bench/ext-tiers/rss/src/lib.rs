//! The RSS connector's reading as a Wasm component: Read a feed, Read feeds,
//! Find feeds and the new-item poll, ported from its TypeScript so the two
//! tiers can be measured on the same work. Bench-only; the shipped connector
//! is the program.
//!
//! A component cannot be a connector (that protocol is stdio), so what the
//! program reads from its connection's config arrives here in the args:
//! `feeds`, `lookbackHours` and `userAgent`. Fetches run one after another,
//! where the program keeps six in flight.

pub mod dates;
pub mod discover;
pub mod feed;
pub mod http;
pub mod poll;
pub mod text;
pub mod xml;

use std::sync::LazyLock;

use regex::Regex;
use vorn_extension_sdk::{json, Ctx, Error, Extension, Host, Init, Level, Value};

use crate::dates::{iso_ms, now_ms};
use crate::feed::{is_json_feed, newest_first, within_hours, Item, Parsed};
use crate::http::{
    feed_address, feed_list, fetch_body, js_string, page_accept, parse_fetched, read_feed_at, send,
    unique_feeds, FeedError, Fetched, Validators,
};
use crate::poll::{
    oldest_first, read_cursor, select_new, to_connector_item, write_cursor, FeedState,
};
use crate::text::{js_trim, js_trim_start};

pub const MAX_ITEMS: f64 = 100.0;
pub const DEFAULT_ITEMS: f64 = 20.0;
pub const DEFAULT_LOOKBACK_HOURS: f64 = 24.0;
pub const MAX_LOOKBACK_HOURS: f64 = 8760.0;
pub const DEFAULT_USER_AGENT: &str = "vorn-connector-rss/0.1.3 (+https://vorn.run)";

fn failed(message: impl Into<String>) -> Error {
    Error::new("failed", message)
}

fn blank(value: &Value) -> bool {
    value.is_null() || js_trim(&js_string(value)).is_empty()
}

/// `Number(value)` for what args carry; NaN when it does not read.
fn number(value: &Value) -> f64 {
    match value {
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::String(s) => js_trim(s).parse().unwrap_or(f64::NAN),
        _ => f64::NAN,
    }
}

/// A whole number in range, the fallback when empty, else an error naming the field.
fn whole_number(
    value: &Value,
    name: &str,
    min: f64,
    max: f64,
    fallback: f64,
) -> Result<f64, Error> {
    if blank(value) {
        return Ok(fallback);
    }
    let parsed = number(value);
    if parsed.fract() != 0.0 || !parsed.is_finite() || parsed < min || parsed > max {
        let to = if max.is_infinite() {
            " up".to_owned()
        } else {
            format!(" to {max}")
        };
        return Err(failed(format!(
            "{name} must be a whole number from {min}{to}"
        )));
    }
    Ok(parsed)
}

/// Hours above zero, or none when empty.
fn optional_hours(value: &Value, name: &str) -> Result<Option<f64>, Error> {
    if blank(value) {
        return Ok(None);
    }
    let parsed = number(value);
    if !parsed.is_finite() || parsed <= 0.0 {
        return Err(failed(format!("{name} must be a number of hours above 0")));
    }
    Ok(Some(parsed))
}

fn user_agent(args: &Value) -> String {
    let given = js_trim(args["userAgent"].as_str().unwrap_or(""));
    if given.is_empty() {
        DEFAULT_USER_AGENT
    } else {
        given
    }
    .to_owned()
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn count(n: f64) -> usize {
    n as usize
}

static ALNUM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[\p{L}\p{N}]$").expect("a valid pattern"));

/// A keyword matched as a whole word or phrase, ignoring case.
struct Keyword(Regex);

impl Keyword {
    fn is_word(c: Option<char>) -> bool {
        c.is_some_and(|c| ALNUM.is_match(c.encode_utf8(&mut [0; 4])))
    }

    /// The lookarounds the program's pattern has, which `regex` has not: retry past each match that touches a letter.
    fn matches(&self, text: &str) -> bool {
        let mut from = 0;
        while let Some(m) = self.0.find_at(text, from) {
            let before = text[..m.start()].chars().next_back();
            let after = text[m.end()..].chars().next();
            if !Self::is_word(before) && !Self::is_word(after) {
                return true;
            }
            match text[m.start()..].chars().next() {
                Some(c) => from = m.start() + c.len_utf8(),
                None => return false,
            }
        }
        false
    }
}

fn keyword_filter(value: &Value) -> Vec<Keyword> {
    js_string(value)
        .split(',')
        .map(js_trim)
        .filter(|w| !w.is_empty())
        .filter_map(|w| {
            Regex::new(&format!("(?i){}", regex::escape(w)))
                .ok()
                .map(Keyword)
        })
        .collect()
}

fn passes(keywords: &[Keyword], item: &Item) -> bool {
    keywords.is_empty()
        || keywords
            .iter()
            .any(|k| k.matches(&item.title) || k.matches(&item.summary))
}

/// One copy per address, the newest; an item with no address is keyed by its feed and id.
fn dedupe_by_url(items: Vec<Item>) -> Vec<Item> {
    let mut kept: Vec<(String, Item)> = Vec::new();
    for item in items {
        let key = if item.url.is_empty() {
            format!("{} {}", item.feed_url, item.id)
        } else {
            item.url.clone()
        };
        match kept.iter_mut().find(|(k, _)| *k == key) {
            Some((_, held)) if item.time() > held.time() => *held = item,
            Some(_) => {}
            None => kept.push((key, item)),
        }
    }
    kept.into_iter().map(|(_, i)| i).collect()
}

fn items_json(items: &[Item]) -> Value {
    Value::Array(items.iter().map(Item::to_json).collect())
}

fn read_feed(host: &mut dyn Host, args: &Value) -> Result<Value, Error> {
    let limit = whole_number(&args["limit"], "limit", 1.0, MAX_ITEMS, DEFAULT_ITEMS)?;
    let hours = optional_hours(&args["sinceHours"], "sinceHours")?;
    let Parsed { feed, items } = read_feed_at(host, &js_string(&args["url"]), &user_agent(args))
        .map_err(|e| failed(e.message()))?;
    let mut recent = match hours {
        Some(h) => within_hours(items, h, now_ms()),
        None => items,
    };
    newest_first(&mut recent);
    recent.truncate(count(limit));
    Ok(json!({ "feed": feed.to_json(), "items": items_json(&recent), "count": recent.len() }))
}

fn read_feeds(host: &mut dyn Host, args: &Value) -> Result<Value, Error> {
    let per_feed = whole_number(&args["perFeed"], "perFeed", 1.0, MAX_ITEMS, DEFAULT_ITEMS)?;
    let min_ok = whole_number(&args["minFeedsOk"], "minFeedsOk", 0.0, f64::INFINITY, 1.0)?;
    let hours = optional_hours(&args["sinceHours"], "sinceHours")?;
    let keywords = keyword_filter(&args["keywords"]);
    let given = feed_list(&args["urls"]);
    let raws = if given.is_empty() {
        feed_list(&args["feeds"])
    } else {
        given
    };
    if raws.is_empty() {
        return Err(failed(
            "Give feed URLs in urls, or add them to the connection's feeds",
        ));
    }
    let agent = user_agent(args);
    let now = now_ms();
    let (urls, mut failures) = unique_feeds(&raws);
    let total = urls.len() + failures.len();
    let mut picked = Vec::new();
    for url in &urls {
        match read_feed_at(host, url, &agent) {
            Ok(parsed) => {
                let recent = match hours {
                    Some(h) => within_hours(parsed.items, h, now),
                    None => parsed.items,
                };
                let mut matching: Vec<Item> = recent
                    .into_iter()
                    .filter(|i| passes(&keywords, i))
                    .collect();
                newest_first(&mut matching);
                matching.truncate(count(per_feed));
                picked.extend(matching);
            }
            Err(e) => failures.push(e),
        }
    }
    let feeds_ok = total - failures.len();
    #[allow(clippy::cast_precision_loss)]
    if (feeds_ok as f64) < min_ok {
        let reasons: Vec<String> = failures.iter().map(FeedError::message).collect();
        return Err(failed(format!(
            "{feeds_ok} of {total} feeds answered, fewer than minFeedsOk {min_ok}: {}",
            reasons.join("; ")
        )));
    }
    let mut items = dedupe_by_url(picked);
    newest_first(&mut items);
    let failed_json: Vec<Value> = failures.iter().map(FeedError::to_json).collect();
    Ok(
        json!({ "items": items_json(&items), "count": items.len(), "feedsOk": feeds_ok, "feedsFailed": failed_json }),
    )
}

/// The body as a feed when it is one; a JSON body has to say so with a version or an items list.
fn as_feed(url: &str, got: &Fetched) -> Option<Parsed> {
    if js_trim_start(&got.body).starts_with('{') && !is_json_feed(js_trim(&got.body)) {
        return None;
    }
    parse_fetched(url, got).ok()
}

fn find_feeds(host: &mut dyn Host, args: &Value) -> Result<Value, Error> {
    let url = feed_address(&js_string(&args["url"])).map_err(|e| failed(e.message()))?;
    let got = fetch_body(host, &url, &user_agent(args), &page_accept())
        .map_err(|e| failed(e.message()))?;
    if let Some(parsed) = as_feed(&url, &got) {
        let f = parsed.feed;
        let found = json!({ "url": url, "title": f.title, "type": f.format.mime(), "format": f.format.name() });
        return Ok(json!({ "feeds": [found], "count": 1, "isFeed": true }));
    }
    let feeds: Vec<Value> = discover::discover_feeds(&got.body, &got.url)
        .iter()
        .map(|d| d.to_json())
        .collect();
    Ok(json!({ "count": feeds.len(), "feeds": feeds, "isFeed": false }))
}

/// One poll of every feed in `feeds`, each conditional on what `cursor` holds for it.
fn poll_feeds(host: &mut dyn Host, args: &Value) -> Result<Value, Error> {
    let raws = feed_list(&args["feeds"]);
    if raws.is_empty() {
        return Err(failed("Add feed URLs to the connection's feeds"));
    }
    let hours = whole_number(
        &args["lookbackHours"],
        "lookbackHours",
        1.0,
        MAX_LOOKBACK_HOURS,
        DEFAULT_LOOKBACK_HOURS,
    )?;
    let agent = user_agent(args);
    let previous = read_cursor(args["cursor"].as_str());
    #[allow(clippy::cast_precision_loss)]
    let floor = match args["since"].as_str().and_then(iso_ms) {
        Some(ms) => ms as f64,
        None => now_ms() as f64 - hours * 3_600_000.0,
    };
    let (urls, mut failures) = unique_feeds(&raws);
    let total = urls.len() + failures.len();
    let mut next: Vec<(String, FeedState)> = Vec::new();
    let mut fire = Vec::new();
    for url in &urls {
        let before = previous.get(url);
        let validators = Validators {
            etag: before.and_then(|b| b.etag.as_deref()),
            last_modified: before.and_then(|b| b.lm.as_deref()),
        };
        let polled = send(host, url, &agent, http::FEED_ACCEPT, &validators).and_then(|got| {
            got.map(|g| parse_fetched(url, &g).map(|p| (p, g)))
                .transpose()
        });
        match polled {
            Ok(None) => next.extend(before.map(|b| (url.clone(), b.clone()))),
            Ok(Some((parsed, got))) => {
                let (fired, mut state) = select_new(&parsed.items, before, floor);
                state.etag = Some(got.etag).filter(|e| !e.is_empty());
                state.lm = Some(got.last_modified).filter(|l| !l.is_empty());
                next.push((url.clone(), state));
                fire.extend(fired);
            }
            Err(e) => {
                next.extend(before.map(|b| (url.clone(), b.clone())));
                failures.push(e);
            }
        }
    }
    for f in &failures {
        host.log(Level::Warn, &format!("rss: {}: {}", f.url, f.reason));
    }
    if failures.len() == total {
        let reasons: Vec<String> = failures.iter().map(FeedError::message).collect();
        return Err(failed(format!("Every feed failed: {}", reasons.join("; "))));
    }
    oldest_first(&mut fire);
    let items: Vec<Value> = fire.iter().map(to_connector_item).collect();
    Ok(json!({ "items": items, "nextCursor": write_cursor(&next), "hasMore": false }))
}

pub struct Rss;

impl Extension for Rss {
    fn init(_: &Init, _: &mut dyn Host) -> Result<Self, Error> {
        Ok(Rss)
    }

    fn handle(
        &mut self,
        host: &mut dyn Host,
        method: &str,
        args: Value,
        _: &Ctx,
    ) -> Result<Value, Error> {
        match method {
            "readFeed" => read_feed(host, &args),
            "readFeeds" => read_feeds(host, &args),
            "findFeeds" => find_feeds(host, &args),
            "poll" => poll_feeds(host, &args),
            _ => Err(Error::unknown(method)),
        }
    }
}

vorn_extension_sdk::export!(Rss);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_match_whole_words_only() {
        let k = keyword_filter(&json!("AI, c++"));
        let item = |title: &str| Item {
            title: title.into(),
            ..Item::default()
        };
        assert!(passes(&k, &item("New AI agents")));
        assert!(!passes(&k, &item("she said so")));
        assert!(passes(&k, &item("Learn C++ today")));
        assert!(
            passes(&k, &item("said AI")),
            "a later match is tried after an inner one"
        );
    }

    #[test]
    fn whole_numbers_name_their_range() {
        assert_eq!(
            whole_number(&json!(""), "limit", 1.0, 100.0, 20.0),
            Ok(20.0)
        );
        assert_eq!(
            whole_number(&json!("7"), "limit", 1.0, 100.0, 20.0),
            Ok(7.0)
        );
        let err = whole_number(&json!(1.5), "minFeedsOk", 0.0, f64::INFINITY, 1.0)
            .expect_err("not whole");
        assert_eq!(err.message, "minFeedsOk must be a whole number from 0 up");
    }
}
