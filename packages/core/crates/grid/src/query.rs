//! What a grid client asks of the terminal beyond its frames (Terminal State
//! Protocol §7 and §9): history rows by absolute line, a word, line or
//! command-output selection at a point, the text between two points, and a
//! search of the scrollback. Selections are per client, so the terminal's
//! own installed selection is never used.
//!
//! All of it reads the terminal through grid references, which libghostty-vt
//! documents as too slow for a render loop and fine for a request. Search
//! narrows the rows first with one formatter pass over the whole screen, so
//! grid references are read only for rows that match.

use libghostty_vt::fmt::{Format, Formatter, FormatterOptions};
use libghostty_vt::screen::{CellWide, GridRef, RowSemanticPrompt, Screen as Which};
use libghostty_vt::selection::{FormatOptions, SelectLineOptions, SelectWordOptions, Selection};
use libghostty_vt::terminal::{Point, PointCoordinate, PointSpace, Terminal};
use vorn_term_proto::msg::{CopyFormat, GridPoint, Hit, SelectKind};
use vorn_term_proto::row::RowWriter;
use vorn_term_proto::screen::{row_flags, Row};

use crate::grid::{bg_only, read_uri, Columns};
use crate::lines::Lines;
use crate::tables::Tables;

type Term = Terminal<'static, 'static>;

/// The most history rows one request returns.
pub const MAX_FETCH: u16 = 1000;
/// The most hits one search returns.
pub const MAX_HITS: usize = 10_000;

/// Where absolute lines are on the active screen right now.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Frame {
    primary: bool,
    /// The absolute line of screen row 0 (the oldest history row).
    oldest: u64,
    /// Rows on the active screen, history included.
    total: u64,
    pub(crate) sb_epoch: u32,
}

impl Frame {
    pub(crate) fn of(t: &Term, lines: &Lines) -> Frame {
        let primary = matches!(t.active_screen(), Ok(Which::Primary));
        Frame {
            primary,
            oldest: if primary { lines.oldest_line() } else { 0 },
            total: t.total_rows().map_or(0, |n| n as u64),
            sb_epoch: lines.sb_epoch(),
        }
    }

    /// The screen row of a point, if it is on the screen and in this epoch.
    /// On the alternate screen a point's line is its viewport row.
    fn row_of(&self, p: &GridPoint) -> Option<u32> {
        if self.primary && p.sb_epoch != self.sb_epoch {
            return None;
        }
        let y = p.line.checked_sub(self.oldest)?;
        if y >= self.total {
            return None;
        }
        u32::try_from(y).ok()
    }

    fn point_at(&self, y: u32, x: u16) -> GridPoint {
        GridPoint {
            line: self.oldest + u64::from(y),
            col: x,
            sb_epoch: self.sb_epoch,
        }
    }
}

/// History rows from `from_line`, at most `count` and [`MAX_FETCH`], encoded
/// against `tables`. Lines past the oldest one held are skipped; the reply
/// says where history starts. Empty on the alternate screen, whose screen
/// coordinates do not reach the primary screen's history.
pub(crate) fn history(
    t: &Term,
    frame: &Frame,
    tables: &mut Tables,
    from_line: u64,
    count: u16,
) -> Vec<Row> {
    if !frame.primary {
        return Vec::new();
    }
    let from = from_line.max(frame.oldest);
    let end = from_line
        .saturating_add(u64::from(count.min(MAX_FETCH)))
        .min(frame.oldest + frame.total);
    let cols = t.cols().unwrap_or(0);
    let mut cx = RowCx::default();
    (from..end)
        .filter_map(|line| {
            let y = u32::try_from(line - frame.oldest).ok()?;
            let (flags, cells) = cx.encode(t, y, cols, tables)?;
            Some(Row {
                y: 0,
                line,
                flags,
                cells,
            })
        })
        .collect()
}

/// Reused buffers for reading rows through grid references.
#[derive(Default)]
struct RowCx {
    writer: RowWriter,
    columns: Columns,
    chars: Vec<char>,
    text: String,
    uri: Vec<u8>,
}

impl RowCx {
    /// Screen row `y` in `row_fmt` 1, with its flags.
    fn encode(
        &mut self,
        t: &Term,
        y: u32,
        cols: u16,
        tables: &mut Tables,
    ) -> Option<(u8, Vec<u8>)> {
        let mut out = Vec::new();
        let mut flags = 0;
        for x in 0..cols {
            let at = Point::Screen(PointCoordinate { x, y });
            let g = t.grid_ref(at).ok()?;
            if x == 0 {
                let row = g.row().ok()?;
                if row.is_wrapped().ok()? {
                    flags |= row_flags::WRAPPED;
                }
                if row.is_wrap_continuation().ok()? {
                    flags |= row_flags::WRAP_CONTINUATION;
                }
                match row.semantic_prompt().ok()? {
                    RowSemanticPrompt::Prompt => flags |= row_flags::PROMPT,
                    RowSemanticPrompt::Continuation => flags |= row_flags::PROMPT_CONTINUATION,
                    RowSemanticPrompt::None => {}
                }
            }
            let cell = g.cell().ok()?;
            let wide = cell.wide().ok()?;
            if wide == CellWide::SpacerTail {
                self.columns
                    .cell(&mut self.writer, &mut out, wide, 0, 0, "");
                continue;
            }
            self.text.clear();
            if wide != CellWide::SpacerHead {
                cluster(&g, &mut self.chars, &mut self.text);
            }
            let bg = bg_only(cell).ok()?;
            let style = if cell.has_styling().ok()? || bg.is_some() {
                tables.style(&g.style().ok()?, bg)
            } else {
                0
            };
            let link = if cell.has_hyperlink().ok()? {
                read_uri(t, at, &mut self.uri).map_or(0, |u| tables.link(u))
            } else {
                0
            };
            self.columns
                .cell(&mut self.writer, &mut out, wide, style, link, &self.text);
        }
        self.columns.finish(&mut self.writer, &mut out);
        Some((flags, out))
    }

    /// Screen row `y` as text with the column each byte starts in: what a
    /// search matches against.
    fn text(&mut self, t: &Term, y: u32, cols: u16) -> (String, Vec<u16>) {
        let mut s = String::new();
        let mut at = Vec::new();
        for x in 0..cols {
            let Ok(g) = t.grid_ref(Point::Screen(PointCoordinate { x, y })) else {
                break;
            };
            let wide = g.cell().and_then(|c| c.wide()).unwrap_or(CellWide::Narrow);
            if wide == CellWide::SpacerTail {
                continue;
            }
            self.text.clear();
            if wide != CellWide::SpacerHead {
                cluster(&g, &mut self.chars, &mut self.text);
            }
            if self.text.is_empty() {
                self.text.push(' ');
            }
            s.push_str(&self.text);
            at.resize(s.len(), x);
        }
        (s, at)
    }
}

/// A cell's grapheme cluster, appended to `out`.
fn cluster(g: &GridRef<'_>, chars: &mut Vec<char>, out: &mut String) {
    if chars.len() < 8 {
        chars.resize(8, '\0');
    }
    let n = loop {
        match g.graphemes(chars) {
            Ok(n) => break n,
            Err(libghostty_vt::Error::OutOfSpace { required }) if required > chars.len() => {
                chars.resize(required, '\0')
            }
            Err(_) => return,
        }
    };
    out.extend(&chars[..n.min(chars.len())]);
}

fn ref_at<'t>(t: &'t Term, frame: &Frame, p: &GridPoint) -> Option<GridRef<'t>> {
    let y = frame.row_of(p)?;
    let x = p.col.min(t.cols().ok()?.saturating_sub(1));
    t.grid_ref(Point::Screen(PointCoordinate { x, y })).ok()
}

fn point_of(t: &Term, frame: &Frame, g: &GridRef<'_>) -> Option<GridPoint> {
    let p = t.point_from_grid_ref(g, PointSpace::Screen).ok()??;
    Some(frame.point_at(p.y, p.x))
}

/// The word, line or command output at `at`, as two inclusive points.
pub(crate) fn select_at(
    t: &Term,
    frame: &Frame,
    at: &GridPoint,
    kind: SelectKind,
) -> Option<(GridPoint, GridPoint)> {
    let g = ref_at(t, frame, at)?;
    let sel = match kind {
        SelectKind::Word => t.select_word(SelectWordOptions::new(g)),
        SelectKind::Line => t.select_line(SelectLineOptions::new(g)),
        SelectKind::Output => t.select_output(g),
    }
    .ok()??;
    Some((
        point_of(t, frame, &sel.start())?,
        point_of(t, frame, &sel.end())?,
    ))
}

/// The text from `from` to `to`, both inclusive, in `format`: soft wraps
/// joined and trailing blanks trimmed, as a copy wants it.
pub(crate) fn copy(
    t: &Term,
    frame: &Frame,
    from: &GridPoint,
    to: &GridPoint,
    rect: bool,
    format: CopyFormat,
) -> Option<String> {
    let sel = Selection::new(ref_at(t, frame, from)?, ref_at(t, frame, to)?, rect);
    let opts = FormatOptions::new()
        .with_emit_format(match format {
            CopyFormat::Plain => Format::Plain,
            CopyFormat::Vt => Format::Vt,
            CopyFormat::Html => Format::Html,
        })
        .with_unwrap(true)
        .with_trim(true)
        .with_selection(&sel);
    let bytes = t.format_selection_alloc(None, opts).ok()??;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// What a search looks for.
pub(crate) enum Needle {
    Text { query: String, case: bool },
    Regex(regex::Regex),
}

impl Needle {
    /// `None` for an empty query or a pattern that does not compile.
    pub(crate) fn new(query: &str, regex: bool, case: bool) -> Option<Needle> {
        if query.is_empty() {
            return None;
        }
        if regex {
            regex::RegexBuilder::new(query)
                .case_insensitive(!case)
                .size_limit(1 << 20)
                .build()
                .ok()
                .map(Needle::Regex)
        } else {
            Some(Needle::Text {
                query: if case {
                    query.to_owned()
                } else {
                    query.to_lowercase()
                },
                case,
            })
        }
    }

    /// Byte ranges of the matches in `hay`.
    fn find(&self, hay: &str, out: &mut Vec<(usize, usize)>) {
        match self {
            Needle::Regex(re) => out.extend(
                re.find_iter(hay)
                    .filter(|m| !m.is_empty())
                    .map(|m| (m.start(), m.end())),
            ),
            Needle::Text { query, case } => {
                // Lowercasing can change byte lengths; match on the lowered
                // text only where it kept them, else fall back to exact.
                let lowered;
                let hay = if *case {
                    hay
                } else {
                    lowered = hay.to_lowercase();
                    if lowered.len() == hay.len() {
                        &lowered
                    } else {
                        hay
                    }
                };
                let mut at = 0;
                while let Some(i) = hay[at..].find(query.as_str()) {
                    out.push((at + i, at + i + query.len()));
                    at += i + query.len().max(1);
                }
            }
        }
    }
}

/// Every match of `needle` on the screen from `from_line` on, row by row,
/// oldest first, at most [`MAX_HITS`]. Matches do not span rows.
pub(crate) fn search(t: &Term, frame: &Frame, needle: &Needle, from_line: Option<u64>) -> Vec<Hit> {
    let opts = FormatterOptions::new().with_format(Format::Plain);
    let Ok(plain) = Formatter::new(t, opts).and_then(|mut f| f.format_alloc(None)) else {
        return Vec::new();
    };
    let plain = String::from_utf8_lossy(&plain);
    let cols = t.cols().unwrap_or(0);
    let first = from_line
        .and_then(|l| l.checked_sub(frame.oldest))
        .unwrap_or(0);
    let mut cx = RowCx::default();
    let mut found = Vec::new();
    let mut hits = Vec::new();
    for (y, line) in plain.split('\n').enumerate() {
        let Ok(y) = u32::try_from(y) else { break };
        if u64::from(y) < first {
            continue;
        }
        found.clear();
        needle.find(line, &mut found);
        if found.is_empty() {
            continue;
        }
        // Columns come from the cells themselves, so wide characters and
        // combining marks land where they are drawn.
        let (text, cols_at) = cx.text(t, y, cols);
        found.clear();
        needle.find(&text, &mut found);
        for &(a, b) in &found {
            let (Some(&x0), Some(&x1)) = (cols_at.get(a), cols_at.get(b.saturating_sub(1))) else {
                continue;
            };
            hits.push(Hit {
                from: frame.point_at(y, x0),
                to: frame.point_at(y, x1),
            });
            if hits.len() == MAX_HITS {
                return hits;
            }
        }
    }
    hits
}
