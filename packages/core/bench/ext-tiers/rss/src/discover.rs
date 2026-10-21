//! Feed autodiscovery from a page's `<link rel="alternate">` tags, as the
//! connector's `discover.ts` does it.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use vorn_extension_sdk::{json, Value};

use crate::feed::Format;
use crate::text::js_trim;
use crate::xml::{attributes, resolve_url};

#[derive(Debug, Clone, PartialEq)]
pub struct Discovered {
    pub url: String,
    pub title: String,
    pub kind: String,
    pub format: Format,
}

impl Discovered {
    pub fn to_json(&self) -> Value {
        json!({ "url": self.url, "title": self.title, "type": self.kind, "format": self.format.name() })
    }
}

/// Autodiscovery types and the format each one names; RSS 1.0 and 2.0 share one.
fn feed_type(kind: &str) -> Option<Format> {
    Some(match kind {
        "application/rss+xml" => Format::Rss2,
        "application/atom+xml" => Format::Atom,
        "application/feed+json" | "application/json" => Format::Json,
        _ => return None,
    })
}

static COMMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<!--[\s\S]*?-->").expect("a valid pattern"));
static BASE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<base\b([^>]*)>").expect("a valid pattern"));
static LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<link\b([^>]*)>").expect("a valid pattern"));

/// First value per lowercased name, decoded and trimmed.
fn attributes_of(tag: &str) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    for (name, value) in attributes(tag) {
        let name = name.to_lowercase();
        if !found.iter().any(|(n, _)| *n == name) {
            found.push((name, js_trim(&value).to_owned()));
        }
    }
    found
}

fn get<'a>(attrs: &'a [(String, String)], name: &str) -> &'a str {
    attrs
        .iter()
        .find(|(n, _)| n == name)
        .map_or("", |(_, v)| v.as_str())
}

/// The feeds a page declares, absolute and in document order; plain JSON only without a feed+json link.
pub fn discover_feeds(html: &str, page_url: &str) -> Vec<Discovered> {
    let page = COMMENT.replace_all(html, "");
    let base_attrs = attributes_of(BASE.captures(&page).map_or("", |m| m.get(1).map_or("", |g| g.as_str())));
    let base = resolve_url(get(&base_attrs, "href"), page_url);
    let base = if base.is_empty() { page_url.to_owned() } else { base };
    let mut links = Vec::new();
    for m in LINK.captures_iter(&page) {
        let attrs = attributes_of(&m[1]);
        let rel = get(&attrs, "rel").to_lowercase();
        let kind = js_trim(get(&attrs, "type").split(';').next().unwrap_or("")).to_lowercase();
        let href = get(&attrs, "href");
        let Some(format) = feed_type(&kind) else { continue };
        if !rel.split_whitespace().any(|r| r == "alternate") || href.is_empty() {
            continue;
        }
        links.push(Discovered {
            url: resolve_url(href, &base),
            title: get(&attrs, "title").to_owned(),
            kind,
            format,
        });
    }
    let preferred = links.iter().any(|l| l.kind == "application/feed+json");
    let mut seen = HashSet::new();
    links.retain(|l| !(preferred && l.kind == "application/json") && seen.insert(l.url.clone()));
    links
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_alternates_against_the_base() {
        let html = r#"<base href="/b/"><!-- <link rel="alternate" type="application/rss+xml" href="no"> -->
            <link rel="alternate" type="application/json" href="j">
            <link rel="Alternate home" type="application/feed+json; charset=utf-8" href="f" title="F">
            <link rel="alternate" type="application/feed+json" href="f">
            <link rel="stylesheet" type="application/rss+xml" href="s">"#;
        let found = discover_feeds(html, "https://e.com/page");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].url, "https://e.com/b/f");
        assert_eq!((found[0].title.as_str(), found[0].format), ("F", Format::Json));
    }
}
