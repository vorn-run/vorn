//! Artifacts in the browser pane: what vornd keeps of one (its versions and
//! the comments on them), what a page declares about itself (its manifest),
//! and the rules for both: comments, doc edits and the design canvas.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A published deliverable, as `artifact:get` returns it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub latest_version: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub version: u32,
    /// `agent` or `user`.
    pub author: String,
    #[serde(default)]
    pub answers_batch_id: Option<String>,
    #[serde(default)]
    pub created_at: String,
}

/// Where a comment points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Anchor {
    Quote {
        quote: String,
        prefix: String,
        suffix: String,
    },
    Point {
        artboard: String,
        x: f64,
        y: f64,
        element: String,
    },
    Edit {
        before: String,
        after: String,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: String,
    pub version: u32,
    #[serde(default)]
    pub anchor: Option<Anchor>,
    pub body: String,
    /// `draft` or `sent`.
    pub state: String,
    #[serde(default)]
    pub batch_id: Option<String>,
    #[serde(default)]
    pub sent_at: Option<String>,
}

impl Comment {
    pub fn is_draft(&self) -> bool {
        self.state == "draft"
    }

    /// `Target`: what the rail quotes for it.
    pub fn target(&self) -> String {
        match &self.anchor {
            Some(Anchor::Quote { quote, .. }) => quote.clone(),
            Some(Anchor::Edit { before, after }) => format!("{before} \u{2192} {after}"),
            Some(Anchor::Point {
                artboard, element, ..
            }) => format!("{artboard} \u{b7} {element}"),
            None => "The whole version".to_owned(),
        }
    }
}

/// `artifact:get`'s answer: the artifact with its versions and comments.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct State {
    pub artifact: Artifact,
    #[serde(default)]
    pub versions: Vec<Version>,
    #[serde(default)]
    pub comments: Vec<Comment>,
    #[serde(default)]
    pub queued: bool,
}

impl State {
    /// The comments not yet sent, on `version`.
    pub fn drafts(&self, version: u32) -> Vec<&Comment> {
        self.comments
            .iter()
            .filter(|c| c.is_draft() && c.version == version)
            .collect()
    }

    pub fn version(&self, n: u32) -> Option<&Version> {
        self.versions.iter().find(|v| v.version == n)
    }

    /// The comments `version` answers, and the version they were on.
    pub fn answered(&self, version: u32) -> (Vec<&Comment>, Option<u32>) {
        let Some(batch) = self
            .version(version)
            .and_then(|v| v.answers_batch_id.as_deref())
        else {
            return (Vec::new(), None);
        };
        let answered: Vec<&Comment> = self
            .comments
            .iter()
            .filter(|c| c.batch_id.as_deref() == Some(batch))
            .collect();
        let on = answered.iter().map(|c| c.version).max();
        (answered, on)
    }
}

/// `POPOVER_WIDTH`.
pub const POPOVER_WIDTH: f32 = 260.0;

/// Under the selection, or over it when there is no room.
pub fn place_popover(rect: (f32, f32, f32, f32), area: (f32, f32)) -> (f32, f32) {
    let (x, y, _, h) = rect;
    let x = x.min(area.0 - POPOVER_WIDTH - 8.0).max(8.0);
    let below = y + h + 8.0;
    let y = if below + 150.0 > area.1 {
        (y - 158.0).max(8.0)
    } else {
        below
    };
    (x, y)
}

/// The comments sent last, together.
pub fn latest_sent_batch(comments: &[Comment]) -> Vec<&Comment> {
    let mut latest: Option<&Comment> = None;
    for c in comments {
        if c.state == "sent"
            && c.batch_id.is_some()
            && latest.is_none_or(|l| {
                c.sent_at.as_deref().unwrap_or("") > l.sent_at.as_deref().unwrap_or("")
            })
        {
            latest = Some(c);
        }
    }
    let Some(batch) = latest.and_then(|l| l.batch_id.as_deref()) else {
        return Vec::new();
    };
    comments
        .iter()
        .filter(|c| c.batch_id.as_deref() == Some(batch))
        .collect()
}

/// One anchored comment to highlight on the page.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Mark {
    pub id: String,
    pub quote: String,
    pub prefix: String,
    pub suffix: String,
    /// `draft`, `sent` or `focus`.
    pub state: &'static str,
}

/// Sent first, so a draft on the same words paints over it.
pub fn marks_for(drafts: &[&Comment], sent: &[&Comment], focus: Option<&str>) -> Vec<Mark> {
    sent.iter()
        .chain(drafts)
        .filter_map(|c| {
            let Some(Anchor::Quote {
                quote,
                prefix,
                suffix,
            }) = &c.anchor
            else {
                return None;
            };
            let state = if Some(c.id.as_str()) == focus {
                "focus"
            } else if c.is_draft() {
                "draft"
            } else {
                "sent"
            };
            Some(Mark {
                id: c.id.clone(),
                quote: quote.clone(),
                prefix: prefix.clone(),
                suffix: suffix.clone(),
                state,
            })
        })
        .collect()
}

/// One paragraph the person changed: what it said, and what it says now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocEdit {
    pub before: String,
    pub after: String,
}

/// `paragraphs`: blocks split on blank lines (`/\n\s*\n/`), trimmed.
pub fn paragraphs(md: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for line in md.split('\n') {
        if line.trim().is_empty() {
            push_para(&mut out, &mut cur);
            continue;
        }
        if !cur.is_empty() {
            cur.push('\n');
        }
        cur.push_str(line);
    }
    push_para(&mut out, &mut cur);
    out
}

fn push_para(out: &mut Vec<String>, cur: &mut String) {
    let p = cur.trim();
    if !p.is_empty() {
        out.push(p.to_owned());
    }
    cur.clear();
}

/// Markup-blind: a paragraph re-emphasised is not a paragraph rewritten.
fn para_key(p: &str) -> String {
    let stripped: String = p
        .chars()
        .filter(|c| !matches!(c, '*' | '_' | '`' | '#' | '>' | '~' | '\\' | '-'))
        .collect();
    stripped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The edited doc as the new version's body, and each run
/// of changed paragraphs as one edit, by a longest common subsequence over
/// the paragraphs' keys.
pub fn merge_doc_edit(original: &str, edited: &str) -> (String, Vec<DocEdit>) {
    let a = paragraphs(original);
    let b = paragraphs(edited);
    let ka: Vec<String> = a.iter().map(|p| para_key(p)).collect();
    let kb: Vec<String> = b.iter().map(|p| para_key(p)).collect();
    let mut lcs = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if ka[i] == kb[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut out: Vec<&str> = Vec::new();
    let mut edits = Vec::new();
    let (mut removed, mut added): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
    let flush = |removed: &mut Vec<&str>, added: &mut Vec<&str>, edits: &mut Vec<DocEdit>| {
        if !removed.is_empty() || !added.is_empty() {
            edits.push(DocEdit {
                before: removed.join("\n\n"),
                after: added.join("\n\n"),
            });
        }
        removed.clear();
        added.clear();
    };
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && ka[i] == kb[j] {
            flush(&mut removed, &mut added, &mut edits);
            out.push(&a[i]);
            i += 1;
            j += 1;
        } else if j < b.len() && (i == a.len() || lcs[i][j + 1] >= lcs[i + 1][j]) {
            added.push(&b[j]);
            out.push(&b[j]);
            j += 1;
        } else {
            removed.push(&a[i]);
            i += 1;
        }
    }
    flush(&mut removed, &mut added, &mut edits);
    (format!("{}\n", out.join("\n\n")), edits)
}

/// A design's canvas: its zoom limits and spacing.
pub const MIN_ZOOM: f64 = 0.25;
pub const MAX_ZOOM: f64 = 1.0;
pub const CANVAS_PAD: f64 = 24.0;
pub const LABEL_HEIGHT: f64 = 20.0;

/// One frame of a design.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Artboard {
    pub id: String,
    pub label: String,
    pub width: u32,
    pub height: u32,
}

/// An artboard where the canvas draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub left: f64,
    pub top: f64,
}

/// Side by side at `zoom`, and the canvas's size.
pub fn place_artboards(boards: &[Artboard], zoom: f64) -> (Vec<Placed>, f64, f64) {
    let mut left = CANVAS_PAD;
    let mut tallest = 0.0f64;
    let placed = boards
        .iter()
        .map(|b| {
            let at = Placed {
                left,
                top: CANVAS_PAD + LABEL_HEIGHT,
            };
            left += js_round(f64::from(b.width) * zoom) + CANVAS_PAD;
            tallest = tallest.max(js_round(f64::from(b.height) * zoom));
            at
        })
        .collect();
    (placed, left, CANVAS_PAD * 2.0 + LABEL_HEIGHT + tallest)
}

/// `Math.round`: halves go up, also below zero.
fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// `clampZoom`.
pub fn clamp_zoom(zoom: f64) -> f64 {
    (js_round(zoom * 100.0) / 100.0).clamp(MIN_ZOOM, MAX_ZOOM)
}

/// The largest zoom at which every artboard is in view.
pub fn fit_zoom(boards: &[Artboard], area: (f64, f64)) -> f64 {
    if boards.is_empty() || area.0 <= 0.0 || area.1 <= 0.0 {
        return MAX_ZOOM;
    }
    let widths: f64 = boards.iter().map(|b| f64::from(b.width)).sum();
    let tallest = boards.iter().map(|b| b.height).max().map_or(1.0, f64::from);
    let by_width = (area.0 - CANVAS_PAD * (boards.len() as f64 + 1.0)) / widths;
    let by_height = (area.1 - CANVAS_PAD * 2.0 - LABEL_HEIGHT) / tallest;
    clamp_zoom((by_width.min(by_height) * 100.0).floor() / 100.0)
}

/// The page told which artboard it is, in its hash.
pub fn artboard_url(url: &str, id: &str) -> String {
    let base = url.split_once('#').map_or(url, |(b, _)| b);
    format!("{base}#artboard={}", encode_uri_component(id))
}

/// `encodeURIComponent`.
fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// One declared input of a design.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Tweak {
    Number {
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        default: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        step: Option<f64>,
    },
    Boolean {
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        default: bool,
    },
    Color {
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        default: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        options: Option<Vec<String>>,
    },
    Select {
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        default: String,
        options: Vec<String>,
    },
}

impl Tweak {
    pub fn label(&self) -> Option<&str> {
        match self {
            Tweak::Number { label, .. }
            | Tweak::Boolean { label, .. }
            | Tweak::Color { label, .. }
            | Tweak::Select { label, .. } => label.as_deref(),
        }
    }

    pub fn default_value(&self) -> Value {
        match self {
            Tweak::Number { default, .. } => Value::from(*default),
            Tweak::Boolean { default, .. } => Value::Bool(*default),
            Tweak::Color { default, .. } | Tweak::Select { default, .. } => {
                Value::String(default.clone())
            }
        }
    }
}

/// What a page says it is, from its `<script id="artifact">` block.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Manifest {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// In the order the page declared them.
    #[serde(skip_serializing_if = "Vec::is_empty", serialize_with = "ordered")]
    pub tweaks: Vec<(String, Tweak)>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub artboards: Vec<Artboard>,
}

fn ordered<S: serde::Serializer>(t: &[(String, Tweak)], s: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap;
    let mut m = s.serialize_map(Some(t.len()))?;
    for (k, v) in t {
        m.serialize_entry(k, v)?;
    }
    m.end()
}

const MANIFEST_BUDGET: usize = 16_000;
const MAX_TWEAKS: usize = 24;
const MAX_ARTBOARDS: usize = 8;

/// `parseManifest`: the page's declaration, validated, or `None` for an
/// ordinary page.
pub fn parse_manifest(text: &str) -> Option<Manifest> {
    if text.is_empty() || text.encode_utf16().count() > MANIFEST_BUDGET {
        return None;
    }
    let Value::Object(m) = serde_json::from_str::<Value>(text).ok()? else {
        return None;
    };
    let kind = m.get("kind").and_then(Value::as_str)?;
    if !matches!(kind, "page" | "doc" | "design") {
        return None;
    }
    let title = m
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| cut(t, 120));
    let mut tweaks = Vec::new();
    if let Some(Value::Object(declared)) = m.get("tweaks") {
        for (key, value) in declared {
            if tweaks.len() >= MAX_TWEAKS {
                break;
            }
            if !is_name(key, 40, false) {
                continue;
            }
            if let Some(t) = parse_tweak(value) {
                tweaks.push((key.clone(), t));
            }
        }
    }
    let mut artboards: Vec<Artboard> = Vec::new();
    if kind == "design" {
        if let Some(Value::Array(raw)) = m.get("artboards") {
            for b in raw {
                if artboards.len() >= MAX_ARTBOARDS {
                    break;
                }
                if let Some(b) = parse_artboard(b) {
                    if !artboards.iter().any(|x| x.id == b.id) {
                        artboards.push(b);
                    }
                }
            }
        }
    }
    Some(Manifest {
        kind: kind.to_owned(),
        title,
        tweaks,
        artboards,
    })
}

/// `String.prototype.slice(0, n)`, by characters.
fn cut(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// `/^[a-zA-Z_][\w]{0,39}$/`, and with `dash` the artboard id's `[\w-]`.
fn is_name(s: &str, max: usize, dash: bool) -> bool {
    let b = s.as_bytes();
    b.first()
        .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
        && b.len() <= max
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || *c == b'_' || (dash && *c == b'-'))
}

fn parse_tweak(raw: &Value) -> Option<Tweak> {
    let t = raw.as_object()?;
    let label = t
        .get("label")
        .and_then(Value::as_str)
        .map(|l| cut(l, 40))
        .filter(|l| !l.is_empty());
    let finite = |k: &str| t.get(k).and_then(Value::as_f64).filter(|v| v.is_finite());
    let ty = t.get("type").and_then(Value::as_str)?;
    match (ty, t.get("default")?) {
        ("number", Value::Number(_)) => Some(Tweak::Number {
            label,
            default: finite("default")?,
            unit: t.get("unit").and_then(Value::as_str).map(|u| cut(u, 8)),
            min: finite("min"),
            max: finite("max"),
            step: finite("step"),
        }),
        ("boolean", Value::Bool(b)) => Some(Tweak::Boolean { label, default: *b }),
        ("color", Value::String(d)) => Some(Tweak::Color {
            label,
            default: cut(d, 40),
            options: strings(t),
        }),
        ("select", Value::String(d)) => {
            let options = strings(t).filter(|o| !o.is_empty())?;
            let fallback = cut(d, 40);
            let default = if options.contains(&fallback) {
                fallback
            } else {
                options[0].clone()
            };
            Some(Tweak::Select {
                label,
                default,
                options,
            })
        }
        _ => None,
    }
}

fn strings(t: &Map<String, Value>) -> Option<Vec<String>> {
    let all = t.get("options")?.as_array()?;
    all.iter()
        .map(|o| o.as_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()
        .map(|v| v.into_iter().take(12).map(|o| cut(&o, 40)).collect())
}

fn parse_artboard(raw: &Value) -> Option<Artboard> {
    let b = raw.as_object()?;
    let id = b.get("id").and_then(Value::as_str)?;
    if !is_name(id, 40, true) {
        return None;
    }
    let size = |k: &str| {
        b.get(k)
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite() && (120.0..=4096.0).contains(v))
            .map(|v| js_round(v) as u32)
    };
    let label = b
        .get("label")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map_or_else(|| id.to_owned(), |l| cut(l, 40));
    Some(Artboard {
        id: id.to_owned(),
        label,
        width: size("width")?,
        height: size("height")?,
    })
}

/// Per declared tweak, what was set here, else what the page
/// holds, else the default; a value counts only of the declared type, finite,
/// and still among the options.
pub fn merge_tweaks(
    declared: &[(String, Tweak)],
    stored: &Map<String, Value>,
    live: Option<&Map<String, Value>>,
) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, decl) in declared {
        let usable = |v: Option<&Value>| -> Option<Value> {
            let v = v?;
            let options = match decl {
                Tweak::Number { .. } => {
                    return v.as_f64().filter(|n| n.is_finite()).map(|_| v.clone())
                }
                Tweak::Boolean { .. } => return v.is_boolean().then(|| v.clone()),
                Tweak::Color { options, .. } => options.as_deref(),
                Tweak::Select { options, .. } => Some(options.as_slice()),
            };
            let s = v.as_str()?;
            options
                .is_none_or(|o| o.iter().any(|x| x == s))
                .then(|| v.clone())
        };
        let value = usable(stored.get(key))
            .or_else(|| usable(live.and_then(|l| l.get(key))))
            .unwrap_or_else(|| decl.default_value());
        out.insert(key.clone(), value);
    }
    out
}

/// A colour tweak's `#rgb` or `#rrggbb` as `0xrrggbb`, for its swatch.
pub fn hex_color(css: &str) -> Option<u32> {
    let hex = css
        .trim()
        .strip_prefix('#')
        .filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()))?;
    let full = match hex.len() {
        3 => hex.chars().flat_map(|c| [c, c]).collect(),
        6 => hex.to_owned(),
        _ => return None,
    };
    u32::from_str_radix(&full, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swatches_read_short_and_long_hex() {
        assert_eq!(hex_color("#fa0"), Some(0xffaa00));
        assert_eq!(hex_color(" #1A2b3C "), Some(0x1a2b3c));
        assert_eq!(hex_color("red"), None);
        assert_eq!(hex_color("#12345"), None);
        assert_eq!(hex_color("#ggg"), None);
    }

    fn comment(id: &str, state: &str, batch: Option<&str>, sent: Option<&str>) -> Comment {
        Comment {
            id: id.into(),
            version: 1,
            anchor: Some(Anchor::Quote {
                quote: id.into(),
                prefix: String::new(),
                suffix: String::new(),
            }),
            body: "b".into(),
            state: state.into(),
            batch_id: batch.map(Into::into),
            sent_at: sent.map(Into::into),
        }
    }

    #[test]
    fn the_latest_batch_is_the_one_sent_last() {
        let all = vec![
            comment("a", "sent", Some("b1"), Some("2024-01-01")),
            comment("b", "sent", Some("b2"), Some("2024-02-01")),
            comment("c", "sent", Some("b2"), Some("2024-02-01")),
            comment("d", "draft", None, None),
        ];
        let ids: Vec<&str> = latest_sent_batch(&all)
            .iter()
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(ids, ["b", "c"]);
        let sent = latest_sent_batch(&all);
        let marks = marks_for(&[&all[3]], &sent, Some("c"));
        let states: Vec<&str> = marks.iter().map(|m| m.state).collect();
        assert_eq!(states, ["sent", "focus", "draft"]);
    }

    #[test]
    fn a_popover_flips_above_when_there_is_no_room_below() {
        assert_eq!(
            place_popover((10.0, 10.0, 50.0, 20.0), (800.0, 600.0)),
            (10.0, 38.0)
        );
        assert_eq!(
            place_popover((700.0, 500.0, 50.0, 20.0), (800.0, 600.0)),
            (532.0, 342.0)
        );
    }

    #[test]
    fn each_changed_run_of_paragraphs_is_one_edit() {
        let (body, edits) = merge_doc_edit("A\n\nB\n\nC\n", "A\n\n**B**\n\nC2\n\nD");
        assert_eq!(body, "A\n\nB\n\nC2\n\nD\n");
        assert_eq!(
            edits,
            [DocEdit {
                before: "C".into(),
                after: "C2\n\nD".into()
            }]
        );
    }

    #[test]
    fn the_canvas_fits_every_artboard() {
        let boards = vec![
            Artboard {
                id: "a".into(),
                label: "A".into(),
                width: 1440,
                height: 900,
            },
            Artboard {
                id: "b".into(),
                label: "B".into(),
                width: 390,
                height: 844,
            },
        ];
        assert_eq!(fit_zoom(&boards, (1000.0, 700.0)), 0.5);
        let (placed, w, h) = place_artboards(&boards, 0.5);
        assert_eq!(placed[1].left, 24.0 + 720.0 + 24.0);
        assert_eq!(
            (w, h),
            (24.0 + 720.0 + 24.0 + 195.0 + 24.0, 48.0 + 20.0 + 450.0)
        );
        assert_eq!(
            artboard_url("http://x/a#old", "m b"),
            "http://x/a#artboard=m%20b"
        );
    }

    #[test]
    fn manifests_keep_only_what_is_valid() {
        let m = parse_manifest(
            r##"{"kind":"design","title":"  Hero  ","tweaks":{"zeta":{"type":"boolean","default":true},"1bad":{"type":"boolean","default":true},"accent":{"type":"select","default":"x","options":["a","b"]},"none":{"type":"select","default":"a","options":[]}},"artboards":[{"id":"m","width":390,"height":844},{"id":"m","width":500,"height":500},{"id":"tiny","width":10,"height":10}]}"##,
        )
        .expect("a manifest");
        assert_eq!(m.title.as_deref(), Some("Hero"));
        let names: Vec<&str> = m.tweaks.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["zeta", "accent"]);
        assert_eq!(m.tweaks[1].1.default_value(), "a");
        assert_eq!(m.artboards.len(), 1);
        assert_eq!(m.artboards[0].label, "m");
        assert!(parse_manifest(r#"{"kind":"video"}"#).is_none());
        assert!(parse_manifest("nope").is_none());
    }

    #[test]
    fn tweaks_merge_yours_over_the_page_over_defaults() {
        let m = parse_manifest(
            r#"{"kind":"design","tweaks":{"dark":{"type":"boolean","default":false},"plan":{"type":"select","default":"a","options":["a","b"]},"pad":{"type":"number","default":8}}}"#,
        )
        .expect("a manifest");
        let stored = serde_json::json!({"plan": "gone", "pad": 12, "old": 1});
        let live = serde_json::json!({"dark": true, "plan": "b", "pad": 4});
        let merged = merge_tweaks(
            &m.tweaks,
            stored.as_object().expect("object"),
            live.as_object(),
        );
        assert_eq!(
            Value::Object(merged),
            serde_json::json!({"dark": true, "plan": "b", "pad": 12})
        );
        let none = merge_tweaks(&m.tweaks, &Map::new(), None);
        assert_eq!(none["plan"], "a");
        assert_eq!(none["dark"], false);
    }
}
