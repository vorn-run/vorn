//! RSS 2.0, RSS 1.0, Atom and JSON Feed read into one shape, as the
//! connector's `feed.ts` reads them.

use std::sync::LazyLock;

use regex::Regex;
use sha2::{Digest, Sha256};
use vorn_extension_sdk::{json, Value};

use crate::dates::{iso_ms, parse_date};
use crate::text::{cut, escape_html, inline_text, js_trim, js_trim_start, one_line, plain_text, tidy, SUMMARY_LENGTH};
use crate::xml::{attr_of, ns, resolve_url, text_of, Doc, El};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Rss2,
    Rss1,
    Atom,
    Json,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::Rss2 => "rss2",
            Format::Rss1 => "rss1",
            Format::Atom => "atom",
            Format::Json => "json",
        }
    }

    /// The type a page's link names a feed of this format with.
    pub fn mime(self) -> &'static str {
        match self {
            Format::Rss2 | Format::Rss1 => "application/rss+xml",
            Format::Atom => "application/atom+xml",
            Format::Json => "application/feed+json",
        }
    }
}

/// One item, whichever format it came from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub url: String,
    pub author: String,
    /// ISO 8601 in UTC, or "" when the feed gives none.
    pub published_at: String,
    pub updated_at: String,
    /// Plain text, at most 2000 characters.
    pub summary: String,
    pub html: String,
    pub categories: Vec<String>,
    pub feed_title: String,
    pub feed_url: String,
}

impl Item {
    /// When it happened: published, else updated.
    pub fn time(&self) -> &str {
        if self.published_at.is_empty() {
            &self.updated_at
        } else {
            &self.published_at
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id, "title": self.title, "url": self.url, "author": self.author,
            "publishedAt": self.published_at, "updatedAt": self.updated_at,
            "summary": self.summary, "html": self.html, "categories": self.categories,
            "feedTitle": self.feed_title, "feedUrl": self.feed_url,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    pub title: String,
    /// Where the feed was read from, after redirects.
    pub url: String,
    pub site_url: String,
    pub format: Format,
}

impl Info {
    pub fn to_json(&self) -> Value {
        json!({ "title": self.title, "url": self.url, "siteUrl": self.site_url, "format": self.format.name() })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub feed: Info,
    pub items: Vec<Item>,
}

pub struct Source<'a> {
    /// The address as configured, which every item carries.
    pub feed_url: &'a str,
    /// The address after redirects, which relative links resolve against.
    pub final_url: &'a str,
}

pub fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A stable id for an item that has no id and no link.
pub fn hash_id(title: &str, published_at: &str) -> String {
    format!("sha256:{}", &sha256_hex(&format!("{title}\n{published_at}"))[..16])
}

static AUTHOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\S+@\S+\s*\((.+)\)$").expect("a valid pattern"));
static HTTP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^https?://").expect("a valid pattern"));

/// The name in `lawyer@boyer.net (Lawyer Boyer)`, else the value as written.
pub fn author_name(value: &str) -> String {
    let text = inline_text(value);
    AUTHOR
        .captures(&text)
        .map_or_else(|| text.clone(), |m| js_trim(&m[1]).to_owned())
}

fn unique(values: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for v in values.iter().map(|v| one_line(v)).filter(|v| !v.is_empty()) {
        if !out.contains(&v) {
            out.push(v);
        }
    }
    out
}

fn finish(mut draft: Item, feed_title: &str, feed_url: &str) -> Item {
    if draft.id.is_empty() {
        draft.id = if draft.url.is_empty() {
            hash_id(&draft.title, &draft.published_at)
        } else {
            draft.url.clone()
        };
    }
    draft.summary = cut(&draft.summary, SUMMARY_LENGTH);
    draft.categories = unique(std::mem::take(&mut draft.categories));
    draft.feed_title = feed_title.to_owned();
    draft.feed_url = feed_url.to_owned();
    draft
}

fn texts(el: El<'_, '_>, local: &str, ns: &str) -> Vec<String> {
    el.children()
        .filter(|c| c.local() == local && c.ns() == ns)
        .map(|c| c.text())
        .collect()
}

/// An Atom `link rel="alternate"` (no rel counts), `text/html` first, resolved.
fn alternate(el: El<'_, '_>, ns: &str) -> String {
    let links: Vec<_> = el
        .children()
        .filter(|c| c.local() == "link" && c.ns() == ns)
        .filter(|link| {
            let rel = js_trim(link.attr("rel", "")).to_lowercase();
            let rel = if rel.is_empty() { "alternate".to_owned() } else { rel };
            rel == "alternate" || rel == "http://www.iana.org/assignments/relation/alternate"
        })
        .collect();
    let best = links
        .iter()
        .find(|l| l.attr("type", "").to_lowercase().starts_with("text/html"))
        .or(links.first());
    best.map_or_else(String::new, |b| resolve_url(b.attr("href", ""), b.base()))
}

fn link_text(el: Option<El<'_, '_>>) -> String {
    el.map_or_else(String::new, |e| resolve_url(&one_line(&e.text()), e.base()))
}

fn or_else(first: String, next: impl FnOnce() -> String) -> String {
    if first.is_empty() {
        next()
    } else {
        first
    }
}

fn rss_item(item: El<'_, '_>, core: &str) -> Item {
    let guid = item.child("guid", core);
    let guid_text = one_line(&text_of(guid));
    let permalink = match guid {
        Some(g)
            if js_trim(g.attr("isPermaLink", "")).to_lowercase() != "false"
                && HTTP.is_match(&guid_text) =>
        {
            resolve_url(&guid_text, g.base())
        }
        _ => String::new(),
    };
    let description = text_of(item.child("description", core));
    let group = item.child("group", ns::MEDIA);
    let media = text_of(
        item.child("description", ns::MEDIA)
            .or_else(|| group.and_then(|g| g.child("description", ns::MEDIA))),
    );
    let encoded = text_of(item.child("encoded", ns::CONTENT));
    let summary_source = [&description, &media, &encoded]
        .into_iter()
        .find(|s| !s.is_empty())
        .cloned()
        .unwrap_or_default();
    let mut categories = texts(item, "category", core);
    categories.extend(texts(item, "subject", ns::DC));
    Item {
        id: guid_text,
        title: or_else(inline_text(&text_of(item.child("title", core))), || {
            inline_text(&text_of(
                item.child("title", ns::MEDIA)
                    .or_else(|| group.and_then(|g| g.child("title", ns::MEDIA))),
            ))
        }),
        url: or_else(link_text(item.child("link", core)), || {
            or_else(permalink, || alternate(item, ns::ATOM))
        }),
        author: or_else(author_name(&text_of(item.child("author", core))), || {
            inline_text(&text_of(item.child("creator", ns::DC)))
        }),
        published_at: or_else(parse_date(&text_of(item.child("pubDate", core))), || {
            parse_date(&text_of(item.child("date", ns::DC)))
        }),
        updated_at: parse_date(&text_of(item.child("updated", ns::ATOM))),
        summary: plain_text(&summary_source),
        html: if encoded.is_empty() { description } else { encoded },
        categories: categories.iter().map(|c| inline_text(c)).collect(),
        ..Item::default()
    }
}

fn rss2(root: El<'_, '_>, source: &Source<'_>) -> Parsed {
    let core = root.ns();
    let channel = root.child("channel", core);
    let title = inline_text(&text_of(channel.and_then(|c| c.child("title", core))));
    // RSS 0.9x put items beside the channel rather than inside it.
    let items = channel
        .into_iter()
        .flat_map(|c| c.elements("item", core))
        .chain(root.elements("item", core));
    let site_url = or_else(link_text(channel.and_then(|c| c.child("link", core))), || {
        channel.map_or_else(String::new, |c| alternate(c, ns::ATOM))
    });
    let items = items
        .filter(|i| !i.truncated())
        .map(|i| finish(rss_item(i, core), &title, source.feed_url))
        .collect();
    Parsed {
        feed: Info { title, url: source.final_url.to_owned(), site_url, format: Format::Rss2 },
        items,
    }
}

fn rss1(root: El<'_, '_>, source: &Source<'_>) -> Parsed {
    let channel = root.children().find(|c| c.local() == "channel");
    let core = channel.map_or(ns::RSS1, |c| c.ns());
    let title = inline_text(&text_of(channel.and_then(|c| c.child("title", core))));
    let items = root
        .elements("item", core)
        .filter(|i| !i.truncated())
        .map(|item| {
            let description = text_of(
                item.child("description", core)
                    .or_else(|| item.child("description", ns::DC)),
            );
            let encoded = text_of(item.child("encoded", ns::CONTENT));
            let draft = Item {
                id: js_trim(item.attr("about", ns::RDF)).to_owned(),
                title: inline_text(&text_of(item.child("title", core))),
                url: link_text(item.child("link", core)),
                author: inline_text(&text_of(item.child("creator", ns::DC))),
                published_at: parse_date(&text_of(item.child("date", ns::DC))),
                updated_at: String::new(),
                summary: plain_text(if description.is_empty() { &encoded } else { &description }),
                html: if encoded.is_empty() { description } else { encoded },
                categories: texts(item, "subject", ns::DC).iter().map(|c| inline_text(c)).collect(),
                ..Item::default()
            };
            finish(draft, &title, source.feed_url)
        })
        .collect();
    let site_url = link_text(channel.and_then(|c| c.child("link", core)));
    Parsed {
        feed: Info { title, url: source.final_url.to_owned(), site_url, format: Format::Rss1 },
        items,
    }
}

/// An Atom text construct as text and as HTML, by its `type`.
fn atom_text(el: Option<El<'_, '_>>) -> (String, String) {
    let Some(el) = el.filter(|e| e.attr("src", "").is_empty()) else {
        return (String::new(), String::new());
    };
    let kind = js_trim(el.attr("type", "")).to_lowercase();
    let kind = if kind.is_empty() { "text" } else { kind.as_str() };
    if kind == "xhtml" || kind == "application/xhtml+xml" {
        let markup = el
            .children()
            .next()
            .map_or_else(|| el.text(), |div| div.inner_xml().to_owned());
        return (plain_text(&markup), markup);
    }
    let value = el.text();
    if kind == "html" || kind == "text/html" {
        return (plain_text(&value), value);
    }
    (tidy(&value), escape_html(&value))
}

fn atom_authors(el: El<'_, '_>, core: &str) -> String {
    el.elements("author", core)
        .map(|a| one_line(&text_of(a.child("name", core))))
        .filter(|n| !n.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

fn atom(root: El<'_, '_>, source: &Source<'_>) -> Parsed {
    let core = root.ns();
    let title = atom_text(root.child("title", core)).0.replace('\n', " ");
    let feed_author = atom_authors(root, core);
    let items = root
        .elements("entry", core)
        .filter(|e| !e.truncated())
        .map(|entry| {
            let (summary_text, summary_html) = atom_text(entry.child("summary", core));
            let (content_text, content_html) = atom_text(entry.child("content", core));
            let updated = parse_date(&text_of(entry.child("updated", core)));
            let draft = Item {
                id: one_line(&text_of(entry.child("id", core))),
                title: atom_text(entry.child("title", core)).0.replace('\n', " "),
                url: alternate(entry, core),
                author: or_else(atom_authors(entry, core), || feed_author.clone()),
                published_at: or_else(parse_date(&text_of(entry.child("published", core))), || {
                    updated.clone()
                }),
                updated_at: updated,
                summary: or_else(summary_text, || content_text),
                html: or_else(content_html, || summary_html),
                categories: entry
                    .elements("category", core)
                    .map(|c| attr_of(Some(c), "term", "").to_owned())
                    .collect(),
                ..Item::default()
            };
            finish(draft, &title, source.feed_url)
        })
        .collect();
    Parsed {
        feed: Info {
            title,
            url: source.final_url.to_owned(),
            site_url: alternate(root, core),
            format: Format::Atom,
        },
        items,
    }
}

fn str_of(v: &Value) -> &str {
    v.as_str().map_or("", js_trim)
}

/// 1.1's `authors`, else 1.0's singular `author`.
fn json_authors(value: &Value) -> String {
    let list: Vec<&Value> = match value["authors"].as_array() {
        Some(a) => a.iter().collect(),
        None => vec![&value["author"]],
    };
    list.into_iter()
        .filter(|a| a.is_object())
        .map(|a| one_line(str_of(&a["name"])))
        .filter(|n| !n.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

fn json_feed(body: &str, source: &Source<'_>) -> Result<Parsed, String> {
    let data: Value = vorn_extension_sdk::serde_json::from_str(body)
        .map_err(|e| format!("unparseable: {e}"))?;
    let title = one_line(str_of(&data["title"]));
    let feed_author = json_authors(&data);
    let items = data["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|i| i.is_object())
        .map(|item| {
            let html = str_of(&item["content_html"]).to_owned();
            let content_text = str_of(&item["content_text"]);
            let id = match &item["id"] {
                Value::String(s) => js_trim(s).to_owned(),
                // `String(n)` writes a whole float without its `.0`.
                Value::Number(n) => match n.as_f64() {
                    Some(f) if f.fract() == 0.0 && f.abs() < 1e21 => format!("{f:.0}"),
                    _ => n.to_string(),
                },
                _ => String::new(),
            };
            let link = or_else(str_of(&item["url"]).to_owned(), || {
                str_of(&item["external_url"]).to_owned()
            });
            let draft = Item {
                id,
                title: one_line(str_of(&item["title"])),
                url: resolve_url(&link, source.final_url),
                author: or_else(json_authors(item), || feed_author.clone()),
                published_at: parse_date(str_of(&item["date_published"])),
                updated_at: parse_date(str_of(&item["date_modified"])),
                summary: or_else(tidy(str_of(&item["summary"])), || {
                    or_else(tidy(content_text), || plain_text(&html))
                }),
                html: or_else(html.clone(), || {
                    if content_text.is_empty() { String::new() } else { escape_html(content_text) }
                }),
                categories: item["tags"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|t| t.as_str().map(str::to_owned))
                    .collect(),
                ..Item::default()
            };
            finish(draft, &title, source.feed_url)
        })
        .collect();
    Ok(Parsed {
        feed: Info {
            title,
            url: source.final_url.to_owned(),
            site_url: resolve_url(str_of(&data["home_page_url"]), source.final_url),
            format: Format::Json,
        },
        items,
    })
}

/// Whether a JSON body declares itself a JSON Feed, by a `version` or an `items` list.
pub fn is_json_feed(body: &str) -> bool {
    vorn_extension_sdk::serde_json::from_str::<Value>(body)
        .is_ok_and(|d| d.is_object() && (d["version"].is_string() || d["items"].is_array()))
}

static FIRST_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[^\s>]{1,15}").expect("a valid pattern"));

/// Any of the four formats as one shape, or the reason the body is not a feed.
pub fn parse_feed(body: &str, source: &Source<'_>) -> Result<Parsed, String> {
    let text = js_trim_start(body.strip_prefix('\u{feff}').unwrap_or(body));
    if text.starts_with('{') {
        return json_feed(text, source);
    }
    let doc = Doc::parse(text, source.final_url).ok_or("unparseable: no root element")?;
    let root = doc.root();
    match root.local().to_lowercase().as_str() {
        "rss" => Ok(rss2(root, source)),
        "rdf" => Ok(rss1(root, source)),
        "feed" => Ok(atom(root, source)),
        "html" => Err(format!(
            "not a feed (the body starts with \"{}\"); try find feeds",
            FIRST_WORD.find(text).map_or("", |m| m.as_str())
        )),
        _ => Err(format!("unparseable: root <{}> is not rss, RDF or feed", root.name())),
    }
}

/// Newest first, dateless last, otherwise in feed order.
pub fn newest_first(items: &mut [Item]) {
    items.sort_by(|a, b| {
        let (at, bt) = (a.time(), b.time());
        match (at.is_empty(), bt.is_empty()) {
            _ if at == bt => std::cmp::Ordering::Equal,
            (true, _) => std::cmp::Ordering::Greater,
            (_, true) => std::cmp::Ordering::Less,
            _ => bt.cmp(at),
        }
    });
}

/// Items dated within the last `hours`; dateless ones are left out.
pub fn within_hours(items: Vec<Item>, hours: f64, now_ms: i64) -> Vec<Item> {
    #[allow(clippy::cast_precision_loss)]
    let floor = now_ms as f64 - hours * 3_600_000.0;
    items
        .into_iter()
        .filter(|i| {
            #[allow(clippy::cast_precision_loss)]
            iso_ms(i.time()).is_some_and(|t| t as f64 >= floor)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: Source<'static> = Source { feed_url: "https://f/x", final_url: "https://f/x" };

    #[test]
    fn rss2_items_take_their_fallbacks() {
        let body = r#"<rss><channel><title>T</title><link>https://site/</link>
            <item><title>A</title><guid>https://site/a</guid><pubDate>Fri, 21 Jul 2023 09:04 EDT</pubDate>
              <author>x@y.z (Ann)</author><category>c</category><category> c </category></item>
            <item><description>&lt;p&gt;Only &amp;amp; text&lt;/p&gt;</description></item>
          </channel></rss>"#;
        let feed = parse_feed(body, &SOURCE).expect("a feed");
        assert_eq!(feed.feed.site_url, "https://site/");
        let a = &feed.items[0];
        assert_eq!((a.url.as_str(), a.author.as_str()), ("https://site/a", "Ann"));
        assert_eq!(a.categories, ["c"]);
        let b = &feed.items[1];
        assert_eq!(b.summary, "Only & text");
        assert!(b.id.starts_with("sha256:"), "{}", b.id);
    }

    #[test]
    fn a_page_is_not_a_feed() {
        let err = parse_feed("<html><body>", &SOURCE).expect_err("not a feed");
        assert_eq!(err, "not a feed (the body starts with \"<html\"); try find feeds");
    }

    #[test]
    fn dateless_items_sort_last() {
        let item = |t: &str| Item { published_at: t.into(), ..Item::default() };
        let mut items = vec![item(""), item("2001"), item("2003"), item("2002")];
        newest_first(&mut items);
        let order: Vec<&str> = items.iter().map(Item::time).collect();
        assert_eq!(order, ["2003", "2002", "2001", ""]);
    }
}
