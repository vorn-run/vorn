//! A client's copy of a terminal's screen (Terminal State Protocol §6 and
//! §8): a grid of encoded rows, the style and link tables they refer to, and
//! the revision they show. No VT parser: everything in it is a frame vornd
//! sent, so each byte is parsed once on the desktop.
//!
//! The rules a client must enforce live here, once, for the native app and
//! the test client alike:
//!
//! - a delta applies only on top of the revision it was cut from, in the same
//!   `state_gen` and `table_gen` (the base check);
//! - table definitions arrive in id order past the client's marks, so the
//!   client always holds ids `0..mark`;
//! - no row refers to a style or link the client lacks.
//!
//! Anything that breaks one is a reconnect or a bug, and the answer is the
//! same: drop the frame and ask for a resync. A frame is checked whole before
//! any of it is applied, so a refused frame leaves the mirror as it was.

use vorn_term_proto::row::{self, DecodeError, Run, ROW_FMT};
use vorn_term_proto::screen::{
    ColorsDelta, Delta, LinkDef, Row, Screen, Snapshot, StyleDef, TermDelta, TermState,
};
use vorn_term_proto::Cursor;

/// Why a frame was refused. Each one means the client should resync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// The frame is from another terminal build (a vornd restart, a recovery).
    StateGen,
    /// The tables were compacted while the client was away.
    TableGen,
    /// The delta was cut from a revision this mirror does not hold.
    Base {
        holds: u64,
        base: u64,
    },
    /// A table definition out of order: not the next id past the mark.
    TableGap {
        mark: u32,
        got: u32,
    },
    /// A row refers to a style or link the mirror does not hold.
    UnknownStyle(u32),
    UnknownLink(u32),
    /// A cell layout this build does not read.
    RowFormat(u8),
    BadRow(DecodeError),
    /// A row outside the viewport.
    RowOutOfRange(u16),
}

pub struct Mirror {
    state_gen: u64,
    rev: u64,
    table_gen: u32,
    resume: Cursor,
    term: TermState,
    styles: Vec<StyleDef>,
    links: Vec<LinkDef>,
    rows: Vec<Row>,
    /// Rows that scrolled off the viewport, oldest first: a cache, fetched
    /// again by line when it has gaps.
    history: Vec<Row>,
}

impl Mirror {
    pub fn from_snapshot(snap: Snapshot) -> Result<Self, Refused> {
        if snap.row_fmt != ROW_FMT {
            return Err(Refused::RowFormat(snap.row_fmt));
        }
        let mut m = Mirror {
            state_gen: snap.state_gen,
            rev: snap.rev,
            table_gen: snap.table_gen,
            resume: snap.resume,
            term: snap.term,
            styles: Vec::new(),
            links: Vec::new(),
            rows: Vec::new(),
            history: Vec::new(),
        };
        m.extend_tables(snap.styles, snap.links)?;
        for r in snap.rows.iter().chain(&snap.history) {
            m.check_row(r)?;
        }
        m.rows = blank_rows(m.term.rows);
        for r in snap.rows {
            m.place(r)?;
        }
        m.history = snap.history;
        Ok(m)
    }

    /// Apply a delta, or refuse it whole.
    pub fn apply(&mut self, delta: Delta) -> Result<(), Refused> {
        if delta.state_gen != self.state_gen {
            return Err(Refused::StateGen);
        }
        if delta.table_gen != self.table_gen {
            return Err(Refused::TableGen);
        }
        if delta.base_rev != self.rev {
            return Err(Refused::Base {
                holds: self.rev,
                base: delta.base_rev,
            });
        }
        // Check the whole frame against the state it will produce before
        // touching anything, so a refusal leaves the mirror as it was.
        let style_mark = next_mark(self.style_mark(), delta.styles.iter().map(|s| s.id))?;
        let link_mark = next_mark(self.link_mark(), delta.links.iter().map(|l| l.id))?;
        let height = match delta.term.as_ref().and_then(|t| t.size) {
            Some((_, rows)) => rows,
            None => self.term.rows,
        };
        for r in &delta.rows {
            check_row(r, style_mark, link_mark)?;
            if r.y >= height {
                return Err(Refused::RowOutOfRange(r.y));
            }
        }

        self.extend_tables(delta.styles, delta.links)?;
        let mut new_epoch = false;
        if let Some(t) = delta.term {
            new_epoch = self.update_term(t);
        }
        self.scroll(delta.scrolled, !new_epoch);
        for r in delta.rows {
            self.place(r)?;
        }
        self.rev = delta.rev;
        self.resume = delta.resume;
        Ok(())
    }

    pub fn state_gen(&self) -> u64 {
        self.state_gen
    }

    pub fn rev(&self) -> u64 {
        self.rev
    }

    pub fn resume(&self) -> Cursor {
        self.resume
    }

    pub fn term(&self) -> &TermState {
        &self.term
    }

    /// How many style definitions the mirror holds: ids `0..style_mark()`.
    pub fn style_mark(&self) -> u32 {
        self.styles.len() as u32
    }

    pub fn link_mark(&self) -> u32 {
        self.links.len() as u32
    }

    pub fn style(&self, id: u32) -> Option<&StyleDef> {
        self.styles.get(id as usize)
    }

    pub fn link(&self, id: u32) -> Option<&LinkDef> {
        self.links.get(id as usize)
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn history(&self) -> &[Row] {
        &self.history
    }

    /// The viewport as text, one string per row with trailing blanks
    /// trimmed: what the formatter's plain output shows.
    pub fn text(&self) -> Vec<String> {
        self.rows.iter().map(row_text).collect()
    }

    fn extend_tables(&mut self, styles: Vec<StyleDef>, links: Vec<LinkDef>) -> Result<(), Refused> {
        for s in styles {
            let mark = self.style_mark();
            // A definition the client already holds may be resent; anything
            // past the mark must be the next id.
            if s.id < mark {
                continue;
            }
            if s.id != mark {
                return Err(Refused::TableGap { mark, got: s.id });
            }
            self.styles.push(s);
        }
        for l in links {
            let mark = self.link_mark();
            if l.id < mark {
                continue;
            }
            if l.id != mark {
                return Err(Refused::TableGap { mark, got: l.id });
            }
            self.links.push(l);
        }
        Ok(())
    }

    fn check_row(&self, r: &Row) -> Result<(), Refused> {
        check_row(r, self.style_mark(), self.link_mark())
    }

    /// Apply a term delta. True when the scrollback epoch changed.
    fn update_term(&mut self, t: TermDelta) -> bool {
        let term = &mut self.term;
        if let Some((cols, rows)) = t.size {
            term.cols = cols;
            term.rows = rows;
            // The frame that resizes carries every row of the new size.
            self.rows.resize(rows as usize, Row::default());
            for (y, r) in self.rows.iter_mut().enumerate() {
                r.y = y as u16;
            }
        }
        if let Some(v) = t.screen {
            term.screen = v;
        }
        if let Some(v) = t.cursor {
            term.cursor = v;
        }
        if let Some(v) = t.colors {
            merge_colors(&mut term.colors, v);
        }
        if let Some(v) = t.mouse {
            term.mouse = v;
        }
        if let Some(v) = t.flags {
            term.flags = v;
        }
        if let Some(v) = t.title {
            term.title = v;
        }
        if let Some(v) = t.cwd {
            term.cwd = v;
        }
        let mut new_epoch = false;
        if let Some(v) = t.sb_epoch {
            if v != term.sb_epoch {
                // Line numbers changed meaning; the cache is no longer valid.
                self.history.clear();
                new_epoch = true;
            }
            term.sb_epoch = v;
        }
        if let Some(v) = t.history_lines {
            term.history_lines = v;
        }
        if let Some(v) = t.top_line {
            term.top_line = v;
        }
        new_epoch
    }

    /// Lines pushed into history since the base revision: the top rows move
    /// into the history cache and the rest move up. Rows vornd did not resend
    /// are still right, because a row is resent only when its line changed.
    /// When the same frame starts a new scrollback epoch the rows that left
    /// are dropped: their line numbers belong to the old epoch.
    fn scroll(&mut self, scrolled: u32, keep: bool) {
        if scrolled == 0 || self.term.screen == Screen::Alternate {
            return;
        }
        let n = (scrolled as usize).min(self.rows.len());
        let gone = self.rows.drain(..n);
        if keep {
            self.history.extend(gone);
        } else {
            drop(gone);
        }
        self.rows.extend(blank_rows(n as u16));
        for (y, r) in self.rows.iter_mut().enumerate() {
            r.y = y as u16;
        }
    }

    fn place(&mut self, r: Row) -> Result<(), Refused> {
        let slot = self
            .rows
            .get_mut(r.y as usize)
            .ok_or(Refused::RowOutOfRange(r.y))?;
        *slot = r;
        Ok(())
    }
}

/// The table mark after a frame's definitions, or the gap that refuses it.
/// Mirrors `extend_tables`: resent ids below the mark are skipped.
fn next_mark(mut mark: u32, ids: impl Iterator<Item = u32>) -> Result<u32, Refused> {
    for id in ids {
        if id < mark {
            continue;
        }
        if id != mark {
            return Err(Refused::TableGap { mark, got: id });
        }
        mark += 1;
    }
    Ok(mark)
}

fn check_row(r: &Row, style_mark: u32, link_mark: u32) -> Result<(), Refused> {
    for run in row::decode(&r.cells).map_err(Refused::BadRow)? {
        if run.style >= style_mark {
            return Err(Refused::UnknownStyle(run.style));
        }
        // Link 0 is "no link".
        if run.link != 0 && run.link >= link_mark {
            return Err(Refused::UnknownLink(run.link));
        }
    }
    Ok(())
}

/// A colors delta carries only what changed: absent defaults stay, and
/// palette entries replace the same index.
fn merge_colors(colors: &mut ColorsDelta, d: ColorsDelta) {
    if d.fg.is_some() {
        colors.fg = d.fg;
    }
    if d.bg.is_some() {
        colors.bg = d.bg;
    }
    if d.cursor.is_some() {
        colors.cursor = d.cursor;
    }
    for (i, rgb) in d.palette {
        match colors.palette.iter_mut().find(|(j, _)| *j == i) {
            Some(entry) => entry.1 = rgb,
            None => colors.palette.push((i, rgb)),
        }
    }
}

fn blank_rows(n: u16) -> Vec<Row> {
    (0..n)
        .map(|y| Row {
            y,
            ..Row::default()
        })
        .collect()
}

/// A row's text, every cell's cluster in order, empty cells as spaces and
/// trailing spaces trimmed.
pub fn row_text(r: &Row) -> String {
    let runs: Vec<Run> = row::decode(&r.cells).unwrap_or_default();
    let mut s = String::new();
    for run in runs {
        for cell in run.cells {
            if cell.text.is_empty() {
                s.push(' ');
            } else {
                s.push_str(&cell.text);
            }
        }
    }
    s.truncate(s.trim_end_matches(' ').len());
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use vorn_term_proto::row::Cell;

    fn line(y: u16, style: u32, text: &str) -> Row {
        let cells = text.chars().map(|c| Cell::new(c.to_string())).collect();
        Row {
            y,
            line: u64::from(y),
            flags: 0,
            cells: row::encode(&[Run {
                style,
                link: 0,
                cells,
            }]),
        }
    }

    fn style(id: u32) -> StyleDef {
        StyleDef {
            id,
            ..StyleDef::default()
        }
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            state_gen: 9,
            rev: 1,
            resume: Cursor {
                epoch: 0,
                next_rseq: 4,
                next_offset: 40,
            },
            table_gen: 0,
            row_fmt: ROW_FMT,
            term: TermState {
                cols: 10,
                rows: 3,
                ..TermState::default()
            },
            styles: vec![style(0), style(1)],
            links: vec![LinkDef::default()],
            rows: vec![line(0, 0, "$ ls"), line(1, 1, "a  b")],
            history: Vec::new(),
        }
    }

    fn delta(base: u64, rev: u64) -> Delta {
        Delta {
            state_gen: 9,
            table_gen: 0,
            base_rev: base,
            rev,
            resume: Cursor {
                epoch: 0,
                next_rseq: 5,
                next_offset: 50,
            },
            ..Delta::default()
        }
    }

    #[test]
    fn a_snapshot_shows_its_rows() {
        let m = Mirror::from_snapshot(snapshot()).unwrap();
        assert_eq!(m.text(), vec!["$ ls", "a  b", ""]);
        assert_eq!((m.style_mark(), m.link_mark()), (2, 1));
    }

    #[test]
    fn a_delta_replaces_the_rows_it_carries() {
        let mut m = Mirror::from_snapshot(snapshot()).unwrap();
        let mut d = delta(1, 2);
        d.rows = vec![line(2, 0, "$ ")];
        m.apply(d).unwrap();
        assert_eq!(m.text(), vec!["$ ls", "a  b", "$"]);
        assert_eq!((m.rev(), m.resume().next_rseq), (2, 5));
    }

    #[test]
    fn a_scroll_moves_rows_into_history() {
        let mut m = Mirror::from_snapshot(snapshot()).unwrap();
        let mut d = delta(1, 2);
        d.scrolled = 1;
        d.rows = vec![line(2, 0, "next")];
        m.apply(d).unwrap();
        assert_eq!(m.text(), vec!["a  b", "", "next"]);
        assert_eq!(m.history().len(), 1);
        assert_eq!(row_text(&m.history()[0]), "$ ls");
    }

    #[test]
    fn the_base_check_refuses_a_delta_from_elsewhere() {
        let mut m = Mirror::from_snapshot(snapshot()).unwrap();
        assert_eq!(
            m.apply(delta(5, 6)),
            Err(Refused::Base { holds: 1, base: 5 })
        );
        let mut other = delta(1, 2);
        other.state_gen = 10;
        assert_eq!(m.apply(other), Err(Refused::StateGen));
        let mut compacted = delta(1, 2);
        compacted.table_gen = 1;
        assert_eq!(m.apply(compacted), Err(Refused::TableGen));
        assert_eq!(m.rev(), 1);
    }

    /// TP-T26's client half: definitions past the marks extend the tables in
    /// order, and no row may refer to one the mirror lacks.
    #[test]
    fn tables_grow_in_order_and_rows_never_outrun_them() {
        let mut m = Mirror::from_snapshot(snapshot()).unwrap();

        let mut unknown = delta(1, 2);
        unknown.rows = vec![line(0, 7, "x")];
        assert_eq!(m.apply(unknown), Err(Refused::UnknownStyle(7)));

        let mut gap = delta(1, 2);
        gap.styles = vec![style(3)];
        assert_eq!(m.apply(gap), Err(Refused::TableGap { mark: 2, got: 3 }));

        let mut grown = delta(1, 2);
        grown.styles = (0..500).map(style).collect();
        grown.rows = vec![line(0, 499, "x")];
        m.apply(grown).unwrap();
        assert_eq!(m.style_mark(), 500);
        assert_eq!(m.text()[0], "x");
    }

    #[test]
    fn a_refused_delta_leaves_the_mirror_as_it_was() {
        let mut m = Mirror::from_snapshot(snapshot()).unwrap();
        let mut d = delta(1, 2);
        d.styles = vec![style(2)];
        d.rows = vec![line(0, 0, "changed"), line(9, 0, "off screen")];
        assert_eq!(m.apply(d), Err(Refused::RowOutOfRange(9)));
        assert_eq!(m.style_mark(), 2);
        assert_eq!(m.text()[0], "$ ls");
    }

    #[test]
    fn a_new_scrollback_epoch_drops_the_history_cache() {
        let mut m = Mirror::from_snapshot(snapshot()).unwrap();
        let mut d = delta(1, 2);
        d.scrolled = 2;
        m.apply(d).unwrap();
        assert_eq!(m.history().len(), 2);
        let mut reflow = delta(2, 3);
        reflow.term = Some(TermDelta {
            sb_epoch: Some(1),
            ..TermDelta::default()
        });
        m.apply(reflow).unwrap();
        assert!(m.history().is_empty());
    }

    #[test]
    fn rows_that_scroll_into_a_new_epoch_are_not_cached() {
        let mut m = Mirror::from_snapshot(snapshot()).unwrap();
        let mut d = delta(1, 2);
        d.scrolled = 1;
        m.apply(d).unwrap();
        assert_eq!(m.history().len(), 1);
        let mut reflow = delta(2, 3);
        reflow.scrolled = 1;
        reflow.term = Some(TermDelta {
            sb_epoch: Some(1),
            ..TermDelta::default()
        });
        m.apply(reflow).unwrap();
        assert!(m.history().is_empty());
        assert_eq!(m.text(), vec!["", "", ""]);
    }

    #[test]
    fn a_colors_delta_changes_only_what_it_carries() {
        let mut m = Mirror::from_snapshot(snapshot()).unwrap();
        let mut d = delta(1, 2);
        d.term = Some(TermDelta {
            colors: Some(ColorsDelta {
                fg: Some((1, 2, 3)),
                bg: Some((4, 5, 6)),
                cursor: None,
                palette: vec![(1, (10, 0, 0)), (2, (20, 0, 0))],
            }),
            ..TermDelta::default()
        });
        m.apply(d).unwrap();
        let mut d = delta(2, 3);
        d.term = Some(TermDelta {
            colors: Some(ColorsDelta {
                bg: Some((7, 8, 9)),
                palette: vec![(2, (30, 0, 0))],
                ..ColorsDelta::default()
            }),
            ..TermDelta::default()
        });
        m.apply(d).unwrap();
        let c = &m.term().colors;
        assert_eq!((c.fg, c.bg), (Some((1, 2, 3)), Some((7, 8, 9))));
        assert_eq!(c.palette, vec![(1, (10, 0, 0)), (2, (30, 0, 0))]);
    }

    #[test]
    fn a_refused_delta_after_a_resize_leaves_the_size() {
        let mut m = Mirror::from_snapshot(snapshot()).unwrap();
        let mut d = delta(1, 2);
        d.term = Some(TermDelta {
            size: Some((10, 5)),
            ..TermDelta::default()
        });
        d.rows = vec![line(4, 0, "ok"), line(0, 9, "bad")];
        assert_eq!(m.apply(d), Err(Refused::UnknownStyle(9)));
        assert_eq!((m.term().rows, m.rows().len()), (3, 3));
    }

    #[test]
    fn a_resize_reshapes_the_viewport() {
        let mut m = Mirror::from_snapshot(snapshot()).unwrap();
        let mut d = delta(1, 2);
        d.term = Some(TermDelta {
            size: Some((10, 5)),
            ..TermDelta::default()
        });
        d.rows = vec![line(4, 0, "bottom")];
        m.apply(d).unwrap();
        assert_eq!(m.rows().len(), 5);
        assert_eq!(m.text()[4], "bottom");
    }
}
