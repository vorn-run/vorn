//! A light XML tree, read as the connector's `xml.ts` reads one: an unclosed
//! element ends at its parent's end tag, declarations are skipped unread,
//! and prefixes a feed uses without declaring them get their usual namespace.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::text::{decode_entities, js_trim};

pub mod ns {
    pub const RSS1: &str = "http://purl.org/rss/1.0/";
    pub const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
    pub const CONTENT: &str = "http://purl.org/rss/1.0/modules/content/";
    pub const DC: &str = "http://purl.org/dc/elements/1.1/";
    pub const ATOM: &str = "http://www.w3.org/2005/Atom";
    pub const MEDIA: &str = "http://search.yahoo.com/mrss/";
    pub const XML: &str = "http://www.w3.org/XML/1998/namespace";
}

/// Where a prefix a feed uses without declaring it conventionally points.
fn conventional(prefix: &str) -> Option<&'static str> {
    Some(match prefix {
        "rdf" => ns::RDF,
        "content" => ns::CONTENT,
        "dc" => ns::DC,
        "atom" => ns::ATOM,
        "media" => ns::MEDIA,
        "xml" => ns::XML,
        _ => return None,
    })
}

#[derive(Debug, Clone)]
pub struct Attr {
    pub ns: String,
    pub local: String,
    pub value: String,
}

#[derive(Debug)]
pub enum Node {
    El(usize),
    Text { text: String, cdata: bool },
}

#[derive(Debug)]
pub struct Element {
    /// The name as written, prefix included.
    pub name: String,
    pub ns: String,
    pub local: String,
    pub attrs: Vec<Attr>,
    pub children: Vec<Node>,
    /// What relative references inside resolve against: the nearest xml:base, else the document's address.
    pub base: String,
    /// Byte offsets of the content between the start and end tags in the source.
    start: usize,
    end: usize,
    /// Ended by the document running out rather than by a tag.
    pub truncated: bool,
}

/// A parsed document: its elements in an arena, the root first.
#[derive(Debug)]
pub struct Doc<'s> {
    source: &'s str,
    els: Vec<Element>,
}

/// One element of a [`Doc`].
#[derive(Debug, Clone, Copy)]
pub struct El<'d, 's> {
    doc: &'d Doc<'s>,
    i: usize,
}

static START_TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"^<([A-Za-z_][A-Za-z0-9_.:-]*)((?:\s+[^\s=/>]+(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s"'>]+))?)*)\s*(/?)>"#,
    )
    .expect("a valid pattern")
});
static END_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^</\s*([^\s<>]+)\s*>").expect("a valid pattern"));
pub(crate) static ATTRIBUTE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"([^\s=/>]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+)))?"#)
        .expect("a valid pattern")
});

/// An address resolved against a base, or as written when either is not a URL.
pub fn resolve_url(href: &str, base: &str) -> String {
    let trimmed = js_trim(href);
    if trimmed.is_empty() {
        return String::new();
    }
    parse_url(trimmed, base).unwrap_or_else(|| trimmed.to_owned())
}

/// `new URL(value, base || undefined).href`.
pub fn parse_url(value: &str, base: &str) -> Option<String> {
    let parsed = if base.is_empty() {
        url::Url::parse(value)
    } else {
        url::Url::parse(base).ok()?.join(value)
    };
    parsed.ok().map(String::from)
}

/// An xml:base resolved against the base around it, which stays when the value is not a URL.
fn rebase(value: &str, parent: &str) -> String {
    parse_url(js_trim(value), parent).unwrap_or_else(|| parent.to_owned())
}

fn qualify(name: &str, scope: &HashMap<String, String>, attribute: bool) -> (String, String) {
    match name.split_once(':') {
        None => {
            let ns = if attribute {
                String::new()
            } else {
                scope.get("").cloned().unwrap_or_default()
            };
            (ns, name.to_owned())
        }
        Some((prefix, local)) => {
            let ns = scope
                .get(prefix)
                .cloned()
                .or_else(|| conventional(prefix).map(str::to_owned))
                .unwrap_or_else(|| format!("urn:undeclared:{prefix}"));
            (ns, local.to_owned())
        }
    }
}

fn after(source: &str, close: &str, from: usize) -> usize {
    source
        .get(from..)
        .and_then(|rest| rest.find(close))
        .map_or(source.len(), |at| from + at + close.len())
}

/// Past a `<!…>` declaration, a DOCTYPE's internal subset included; nothing in it is read.
fn skip_declaration(source: &str, from: usize) -> usize {
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    let mut quote = 0u8;
    let mut j = from + 2;
    while j < bytes.len() {
        let c = bytes[j];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if source[j..].starts_with("<!--") {
            j = after(source, "-->", j + 4) - 1;
        } else if c == b'"' || c == b'\'' {
            quote = c;
        } else if c == b'[' {
            depth += 1;
        } else if c == b']' {
            depth = depth.saturating_sub(1);
        } else if c == b'>' && depth == 0 {
            return j + 1;
        }
        j += 1;
    }
    source.len()
}

pub(crate) fn attributes(raw: &str) -> Vec<(String, String)> {
    ATTRIBUTE
        .captures_iter(raw)
        .map(|m| {
            let value = m
                .get(2)
                .or_else(|| m.get(3))
                .or_else(|| m.get(4))
                .map_or("", |v| v.as_str());
            (m[1].to_owned(), decode_entities(value))
        })
        .collect()
}

impl<'s> Doc<'s> {
    /// The root element as a light tree, or none when there is no element.
    pub fn parse(source: &'s str, base_url: &str) -> Option<Doc<'s>> {
        let mut els: Vec<Element> = Vec::new();
        let mut stack: Vec<(usize, HashMap<String, String>)> = Vec::new();
        let mut i = 0;
        let n = source.len();
        let add_text = |els: &mut Vec<Element>, stack: &[(usize, _)], text: String, cdata: bool| {
            if let Some((top, _)) = stack.last() {
                els[*top].children.push(Node::Text { text, cdata });
            }
        };
        while i < n {
            let lt = source[i..].find('<').map(|at| i + at);
            let stop = lt.unwrap_or(n);
            if stop > i {
                add_text(&mut els, &stack, decode_entities(&source[i..stop]), false);
            }
            let Some(lt) = lt else { break };
            i = lt;
            let rest = &source[i..];
            if rest.starts_with("<!--") {
                i = after(source, "-->", i + 4);
            } else if rest.starts_with("<![CDATA[") {
                let end = source[i + 9..].find("]]>").map(|at| i + 9 + at);
                add_text(
                    &mut els,
                    &stack,
                    source[i + 9..end.unwrap_or(n)].to_owned(),
                    true,
                );
                i = end.map_or(n, |e| e + 3);
            } else if rest.starts_with("<?") {
                i = after(source, "?>", i + 2);
            } else if rest.starts_with("<!") {
                i = skip_declaration(source, i);
            } else if rest.starts_with("</") {
                let Some(m) = END_TAG.captures(rest) else {
                    add_text(&mut els, &stack, "<".to_owned(), false);
                    i += 1;
                    continue;
                };
                i += m[0].len();
                let lower = m[1].to_lowercase();
                if let Some(k) = stack
                    .iter()
                    .rposition(|(e, _)| els[*e].name.to_lowercase() == lower)
                {
                    for (open, _) in stack.drain(k..) {
                        els[open].end = lt;
                    }
                }
            } else {
                let Some(m) = START_TAG.captures(rest) else {
                    add_text(&mut els, &stack, "<".to_owned(), false);
                    i += 1;
                    continue;
                };
                i += m[0].len();
                if !els.is_empty() && stack.is_empty() {
                    continue;
                }
                let raw = attributes(m.get(2).map_or("", |a| a.as_str()));
                let is_xmlns = |name: &str| name == "xmlns" || name.starts_with("xmlns:");
                let mut scope = stack.last().map(|(_, s)| s.clone()).unwrap_or_default();
                for (name, value) in raw.iter().filter(|(name, _)| is_xmlns(name)) {
                    let prefix = if name == "xmlns" { "" } else { &name[6..] };
                    scope.insert(prefix.to_owned(), value.clone());
                }
                let attrs: Vec<Attr> = raw
                    .into_iter()
                    .filter(|(name, _)| !is_xmlns(name))
                    .map(|(name, value)| {
                        let (ns, local) = qualify(&name, &scope, true);
                        Attr { ns, local, value }
                    })
                    .collect();
                let parent = stack.last().map(|(p, _)| *p);
                let parent_base = parent.map_or(base_url, |p| els[p].base.as_str());
                let base = attrs
                    .iter()
                    .find(|a| a.ns == ns::XML && a.local == "base")
                    .map_or_else(|| parent_base.to_owned(), |b| rebase(&b.value, parent_base));
                let name = m[1].to_owned();
                let (ns, local) = qualify(&name, &scope, false);
                let index = els.len();
                els.push(Element {
                    name,
                    ns,
                    local,
                    attrs,
                    children: Vec::new(),
                    base,
                    start: i,
                    end: i,
                    truncated: false,
                });
                if let Some(p) = parent {
                    els[p].children.push(Node::El(index));
                }
                if &m[3] != "/" {
                    stack.push((index, scope));
                }
            }
        }
        for (open, _) in stack {
            els[open].end = n;
            els[open].truncated = true;
        }
        (!els.is_empty()).then_some(Doc { source, els })
    }

    pub fn root(&self) -> El<'_, 's> {
        El { doc: self, i: 0 }
    }
}

impl<'d, 's> El<'d, 's> {
    fn el(&self) -> &'d Element {
        &self.doc.els[self.i]
    }

    pub fn name(&self) -> &'d str {
        &self.el().name
    }

    pub fn ns(&self) -> &'d str {
        &self.el().ns
    }

    pub fn local(&self) -> &'d str {
        &self.el().local
    }

    pub fn base(&self) -> &'d str {
        &self.el().base
    }

    pub fn truncated(&self) -> bool {
        self.el().truncated
    }

    /// Its child elements, in order.
    pub fn children(self) -> impl Iterator<Item = El<'d, 's>> {
        let doc = self.doc;
        self.el().children.iter().filter_map(move |c| match c {
            Node::El(i) => Some(El { doc, i: *i }),
            Node::Text { .. } => None,
        })
    }

    pub fn elements(self, local: &'d str, ns: &'d str) -> impl Iterator<Item = El<'d, 's>> {
        self.children()
            .filter(move |c| c.local() == local && c.ns() == ns)
    }

    pub fn child(self, local: &str, ns: &str) -> Option<El<'d, 's>> {
        self.children().find(|c| c.local() == local && c.ns() == ns)
    }

    pub fn attr(&self, local: &str, ns: &str) -> &'d str {
        self.el()
            .attrs
            .iter()
            .find(|a| a.local == local && a.ns == ns)
            .map_or("", |a| a.value.as_str())
    }

    pub fn inner_xml(&self) -> &'s str {
        let e = self.el();
        js_trim(&self.doc.source[e.start..e.end.max(e.start)])
    }

    /// Its text: CDATA as written (decoded once more when it holds only escaped markup), markup inside kept as written.
    pub fn text(&self) -> String {
        let e = self.el();
        if e.children.iter().any(|c| matches!(c, Node::El(_))) {
            return self.inner_xml().to_owned();
        }
        let mut out = String::new();
        for c in &e.children {
            if let Node::Text { text, cdata } = c {
                if *cdata && text.contains("&lt;") && !text.contains('<') {
                    out.push_str(&decode_entities(text));
                } else {
                    out.push_str(text);
                }
            }
        }
        js_trim(&out).to_owned()
    }
}

/// `textOf` for an element that may be missing.
pub fn text_of(el: Option<El<'_, '_>>) -> String {
    el.map(|e| e.text()).unwrap_or_default()
}

/// `attr` for an element that may be missing.
pub fn attr_of<'d>(el: Option<El<'d, '_>>, local: &str, ns: &str) -> &'d str {
    el.map_or("", |e| e.attr(local, ns))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespaces_bases_and_unclosed_elements() {
        let src = r#"<?xml version="1.0"?><!DOCTYPE x [<!ENTITY a "b">]><feed xmlns="http://www.w3.org/2005/Atom" xml:base="https://e.com/a/"><title>T &amp; <![CDATA[<b>]]></title><entry><link href="x"/><dc:subject>s</dc:subject></entry><entry><p>unclosed</feed>"#;
        let doc = Doc::parse(src, "https://fallback/").expect("a root");
        let root = doc.root();
        assert_eq!((root.local(), root.ns()), ("feed", ns::ATOM));
        assert_eq!(root.base(), "https://e.com/a/");
        assert_eq!(text_of(root.child("title", ns::ATOM)), "T & <b>");
        let entries: Vec<_> = root.elements("entry", ns::ATOM).collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0].child("subject", ns::DC).map(|s| s.text()),
            Some("s".into())
        );
        assert!(!entries[1].truncated());
        assert_eq!(resolve_url("x", entries[0].base()), "https://e.com/a/x");
    }

    #[test]
    fn an_unended_document_truncates_what_is_open() {
        let doc = Doc::parse("<rss><channel><item><title>a", "").expect("a root");
        assert!(doc.root().truncated());
    }
}
