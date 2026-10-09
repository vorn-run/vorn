//! One session's grid: the render state, the row cache and the revisions a
//! delta is cut from (Terminal State Protocol §8).
//!
//! A render update reads libghostty-vt's render state, re-encodes the rows
//! it marks dirty and compares each, byte for byte, with the cached row for
//! the same line. A row that is the same is not changed, so a scroll costs
//! only the new lines: the cache is shifted by the lines counted into
//! history first, exactly as a client shifts its mirror. Memory is one
//! viewport whatever the number of clients.
//!
//! Each update that changes anything raises `rev` by one and stamps what it
//! changed with it: rows, `TermState` fields, palette entries. A short log
//! of `(rev, scrolled)` lets a delta from any revision still in it be cut
//! from the stamps alone; a client further behind gets a snapshot, which is
//! one viewport.

use std::collections::VecDeque;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

use libghostty_vt::render::{CellIterator, CursorVisualStyle, Dirty, RenderState, RowIterator};
use libghostty_vt::screen::{CellContentTag, CellWide, RowSemanticPrompt, Screen as Which};
use libghostty_vt::style::RgbColor;
use libghostty_vt::terminal::{Mode, Point, PointCoordinate, Terminal};
use vorn_screen::Emulator;
use vorn_term_proto::row::{RowWriter, ROW_FMT};
use vorn_term_proto::screen::{
    flags, row_flags, Color, ColorsDelta, CursorState, CursorStyle, Delta, MouseMode, Row, Screen,
    Snapshot, TermDelta, TermState,
};
use vorn_term_proto::Cursor;

use crate::lines::Lines;
use crate::tables::Tables;
use crate::Error;

type Term = Terminal<'static, 'static>;

/// Scroll log entries kept. A client more than this many scrolling frames
/// behind gets a snapshot.
pub const LOG_LEN: usize = 64;

/// A row as the cache holds it, with the revision it last changed at.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Cached {
    line: u64,
    flags: u8,
    cells: Vec<u8>,
    rev: u64,
}

impl Cached {
    /// What a client's mirror holds in a row its scroll left blank.
    fn blank(&mut self) {
        self.line = 0;
        self.flags = 0;
        self.cells.clear();
    }
}

/// The revision each part of `TermState` last changed at.
#[derive(Debug, Default, Clone)]
struct Revs {
    size: u64,
    screen: u64,
    cursor: u64,
    fg: u64,
    bg: u64,
    cursor_color: u64,
    mouse: u64,
    flags: u64,
    title: u64,
    cwd: u64,
    sb_epoch: u64,
    history_lines: u64,
    top_line: u64,
}

/// What an attachment holds: the frame it was last sent and its table
/// marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Held {
    pub state_gen: u64,
    pub table_gen: u32,
    pub rev: u64,
    pub style_mark: u32,
    pub link_mark: u32,
}

/// One session's grid: what every attachment's frames are cut from.
/// Lives on the thread that owns the terminal, like the terminal itself.
pub struct Grid {
    state_gen: u64,
    rev: u64,
    rendered: bool,
    resume: Cursor,
    rs: RenderState<'static>,
    rows_it: RowIterator<'static>,
    cells_it: CellIterator<'static>,
    pub(crate) tables: Tables,
    rows: Vec<Cached>,
    log: VecDeque<(u64, u32)>,
    /// Deltas can be cut from this revision on.
    log_floor: u64,
    term: TermState,
    revs: Revs,
    palette: [RgbColor; 256],
    palette_revs: Box<[u64; 256]>,
    pub(crate) lines: Lines,
    /// Re-encode every row at the next update: the render state's dirty
    /// bits cannot be trusted (another reader consumed them, the terminal
    /// was swapped, the tables were compacted).
    full: bool,
    /// The emulator's count of codepoints that may have joined a cell, at
    /// the last update: Ghostty marks no row dirty for those.
    joiners: u64,
    /// The emulator's count of scrollback clears and resets at the last
    /// update. History can be cleared and refilled between two counts of
    /// scrolled lines, so a clear starts a new epoch whatever the count.
    clears: Option<u64>,
    writer: RowWriter,
    columns: Columns,
    /// A never-written cell as Ghostty stores it. Most of a row is cells
    /// like it, and one that compares equal is known to be empty, narrow
    /// and unstyled without asking more.
    blank: Option<libghostty_vt::screen::Cell>,
    scratch: Vec<u8>,
    text: String,
    uri: Vec<u8>,
}

impl std::fmt::Debug for Grid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Grid")
            .field("state_gen", &self.state_gen)
            .field("rev", &self.rev)
            .field("table_gen", &self.tables.gen())
            .field("rows", &self.rows.len())
            .finish()
    }
}

/// A fresh, non-zero `state_gen`: random, so a vornd restart never repeats
/// one a client holds.
fn fresh_state_gen() -> u64 {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(COUNT.fetch_add(1, Ordering::Relaxed));
    if let Ok(t) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        h.write_u128(t.as_nanos());
    }
    h.finish() | 1
}

impl Grid {
    /// A grid for a terminal built from scratch, under a new `state_gen`.
    pub fn new() -> Result<Grid, Error> {
        Ok(Grid {
            state_gen: fresh_state_gen(),
            rev: 0,
            rendered: false,
            resume: Cursor::default(),
            rs: RenderState::new()?,
            rows_it: RowIterator::new()?,
            cells_it: CellIterator::new()?,
            tables: Tables::default(),
            rows: Vec::new(),
            log: VecDeque::with_capacity(LOG_LEN),
            log_floor: 0,
            term: TermState::default(),
            revs: Revs::default(),
            palette: [RgbColor::default(); 256],
            palette_revs: Box::new([0; 256]),
            lines: Lines::default(),
            full: true,
            joiners: 0,
            clears: None,
            writer: RowWriter::new(),
            columns: Columns::default(),
            blank: blank_cell(),
            scratch: Vec::new(),
            text: String::new(),
            uri: Vec::new(),
        })
    }

    pub fn state_gen(&self) -> u64 {
        self.state_gen
    }

    pub fn rev(&self) -> u64 {
        self.rev
    }

    pub fn table_gen(&self) -> u32 {
        self.tables.gen()
    }

    /// Whether an update has run: there is a frame to send.
    pub fn rendered(&self) -> bool {
        self.rendered
    }

    pub fn term(&self) -> &TermState {
        &self.term
    }

    pub fn resume(&self) -> Cursor {
        self.resume
    }

    /// The oldest revision a delta can still be cut from.
    pub fn log_floor(&self) -> u64 {
        self.log_floor
    }

    pub fn lines(&self) -> &Lines {
        &self.lines
    }

    /// Something other than this grid read the terminal's dirty state, or
    /// the terminal was replaced: the next update re-encodes every row. It
    /// costs time, never bytes, since unchanged rows still compare equal.
    pub fn distrust_dirty(&mut self) {
        self.full = true;
        // A fresh render state starts fully dirty, with no memory of a
        // terminal that may be gone.
        if let Ok(rs) = RenderState::new() {
            self.rs = rs;
        }
    }

    /// Counts the lines scrolled into history since the last count, and
    /// starts a new epoch if the scrollback was cleared since. What it
    /// finds is reported by the next frame.
    pub fn settle(&mut self, em: &Emulator) {
        let t = em.terminal();
        self.lines.settle(t);
        let clears = em.history_clears();
        if self.clears.is_some_and(|c| c != clears) {
            self.lines.new_epoch(t);
        }
        self.clears = Some(clears);
    }

    /// One render update from the terminal, stamped with `resume`: the
    /// cursor after the last record applied. Answers whether anything
    /// changed, which raised `rev`.
    pub fn update(&mut self, em: &Emulator, resume: Cursor) -> Result<bool, Error> {
        let t = em.terminal();
        let next = self.rev + 1;
        let mut changed = false;
        let screen = match t.active_screen()? {
            Which::Primary => Screen::Primary,
            Which::Alternate => Screen::Alternate,
        };
        let size = (t.cols()?, t.rows()?);

        self.settle(em);
        if self.rendered && size != (self.term.cols, self.term.rows) {
            // The primary screen reflows, so line numbers stop meaning
            // anything; every row is resent at the new size.
            self.lines.new_epoch(t);
        }
        if self.rows.len() != usize::from(size.1) {
            self.rows.resize_with(usize::from(size.1), Cached::default);
            self.full = true;
        }
        let moved = self.lines.take();
        let mut scrolled = 0u32;
        if moved.new_epoch || screen != self.term.screen || !self.rendered {
            // No shift is known that the client would make too: resend all.
            self.full = true;
        } else if moved.scrolled > 0 && screen == Screen::Primary {
            scrolled = u32::try_from(moved.scrolled).unwrap_or(u32::MAX);
            let k = (scrolled as usize).min(self.rows.len());
            self.rows.rotate_left(k);
            let n = self.rows.len();
            for r in &mut self.rows[n - k..] {
                r.blank();
            }
        }
        if self.tables.over_limit() {
            self.tables.compact();
            self.full = true;
        }
        let joiners = em.joiners_printed();
        if joiners != self.joiners {
            // A combining mark or joiner may have changed a cell on a row
            // Ghostty did not mark dirty, which the render state therefore
            // did not copy: start it again so it copies every row.
            self.joiners = joiners;
            self.distrust_dirty();
        }

        let top_line = self.lines.top_line();
        let Grid {
            rs,
            rows_it,
            cells_it,
            tables,
            rows,
            full,
            writer,
            columns,
            blank,
            scratch,
            text,
            uri,
            ..
        } = self;
        let snap = rs.update(t)?;
        let all = *full || snap.dirty()? == Dirty::Full;
        {
            let mut it = rows_it.update(&snap)?;
            let mut y: u16 = 0;
            while let Some(row) = it.next() {
                let Some(cached) = rows.get_mut(usize::from(y)) else {
                    break;
                };
                let line = match screen {
                    Screen::Primary => top_line + u64::from(y),
                    Screen::Alternate => 0,
                };
                if all || row.dirty()? || cached.line != line {
                    let raw = row.raw_row()?;
                    let flags = row_flags_of(raw)?;
                    let link_row = raw.has_hyperlink()?;
                    let graphemes = raw.has_grapheme_cluster()?;
                    scratch.clear();
                    let mut cells = cells_it.update(row)?;
                    let mut x: u16 = 0;
                    // Neighbouring cells mostly share a style: Ghostty's id
                    // for it (page-local, so per row) and the wire id.
                    let mut last_style = None;
                    while let Some(cell) = cells.next() {
                        let rc = cell.raw_cell()?;
                        if Some(rc) == *blank {
                            columns.blank(writer, scratch);
                            x += 1;
                            continue;
                        }
                        // Printable ASCII on a row with no clusters is one
                        // narrow codepoint: the common case, read with the
                        // fewest calls.
                        let cp = rc.codepoint()?;
                        if !graphemes && !link_row && (0x20..0x7f).contains(&cp) {
                            let style = if rc.has_styling()? {
                                let gid = rc.style_id()?;
                                match last_style {
                                    Some((g, None, id)) if g == gid => id,
                                    _ => {
                                        let id = tables.style(&cell.style()?, None);
                                        last_style = Some((gid, None, id));
                                        id
                                    }
                                }
                            } else {
                                0
                            };
                            columns.ascii(writer, scratch, style, cp as u8);
                            x += 1;
                            continue;
                        }
                        let wide = rc.wide()?;
                        if wide == CellWide::SpacerTail {
                            columns.cell(writer, scratch, wide, 0, 0, "");
                            x += 1;
                            continue;
                        }
                        let tag = rc.content_tag()?;
                        text.clear();
                        if wide != CellWide::SpacerHead {
                            match tag {
                                // One codepoint: no need to ask for the
                                // cluster, the common case by far.
                                CellContentTag::Codepoint => {
                                    if let Some(c) = char::from_u32(rc.codepoint()?) {
                                        if c != '\0' {
                                            text.push(c);
                                        }
                                    }
                                }
                                CellContentTag::CodepointGrapheme => cell.graphemes_utf8(text)?,
                                _ => {}
                            }
                        }
                        let bg = bg_of(rc, tag)?;
                        let style = if rc.has_styling()? || bg.is_some() {
                            let gid = rc.style_id()?;
                            match last_style {
                                Some((g, b, id)) if g == gid && b == bg => id,
                                _ => {
                                    let id = tables.style(&cell.style()?, bg);
                                    last_style = Some((gid, bg, id));
                                    id
                                }
                            }
                        } else {
                            0
                        };
                        let link = if link_row && rc.has_hyperlink()? {
                            let at = Point::Viewport(PointCoordinate { x, y: u32::from(y) });
                            match read_uri(t, at, uri) {
                                Some(u) => tables.link(u),
                                None => 0,
                            }
                        } else {
                            0
                        };
                        columns.cell(writer, scratch, wide, style, link, text);
                        x += 1;
                    }
                    columns.finish(writer, scratch);
                    if cached.cells != *scratch || cached.flags != flags || cached.line != line {
                        std::mem::swap(&mut cached.cells, scratch);
                        cached.flags = flags;
                        cached.line = line;
                        cached.rev = next;
                        changed = true;
                    }
                    row.set_dirty(false)?;
                }
                y += 1;
            }
        }
        let cursor = cursor_of(&snap)?;
        let colors = snap.colors()?;
        let password = snap.cursor_password_input()?;
        snap.set_dirty(Dirty::Clean)?;
        *full = false;

        // TermState, field by field.
        let mouse = mouse_of(t)?;
        let mut flags = 0;
        for (mode, bit) in [
            (Mode::BRACKETED_PASTE, flags::BRACKETED_PASTE),
            (Mode::FOCUS_EVENT, flags::FOCUS_EVENTS),
            (Mode::SYNC_OUTPUT, flags::SYNC_OUTPUT_ACTIVE),
            (Mode::REVERSE_COLORS, flags::REVERSE_VIDEO),
        ] {
            if t.mode(mode)? {
                flags |= bit;
            }
        }
        if password {
            flags |= flags::PASSWORD_INPUT;
        }
        let (sb_epoch, history_lines) = (self.lines.sb_epoch(), self.lines.history_lines());
        let term = &mut self.term;
        let revs = &mut self.revs;
        changed |= set(&mut term.cols, size.0, &mut revs.size, next);
        changed |= set(&mut term.rows, size.1, &mut revs.size, next);
        changed |= set(&mut term.screen, screen, &mut revs.screen, next);
        changed |= set(&mut term.cursor, cursor, &mut revs.cursor, next);
        changed |= set(&mut term.mouse, mouse, &mut revs.mouse, next);
        changed |= set(&mut term.flags, flags, &mut revs.flags, next);
        if term.title != em.title() {
            term.title = em.title().to_owned();
            revs.title = next;
            changed = true;
        }
        if term.cwd != em.cwd() {
            term.cwd = em.cwd().to_owned();
            revs.cwd = next;
            changed = true;
        }
        changed |= set(&mut term.sb_epoch, sb_epoch, &mut revs.sb_epoch, next);
        changed |= set(
            &mut term.history_lines,
            history_lines,
            &mut revs.history_lines,
            next,
        );
        changed |= set(&mut term.top_line, top_line, &mut revs.top_line, next);
        let rgb = |c: RgbColor| (c.r, c.g, c.b);
        changed |= set(
            &mut term.colors.fg,
            Some(rgb(colors.foreground)),
            &mut revs.fg,
            next,
        );
        changed |= set(
            &mut term.colors.bg,
            Some(rgb(colors.background)),
            &mut revs.bg,
            next,
        );
        changed |= set(
            &mut term.colors.cursor,
            colors.cursor.map(rgb),
            &mut revs.cursor_color,
            next,
        );
        let mut palette_changed = false;
        for (i, c) in colors.palette.iter().enumerate() {
            if !self.rendered || self.palette[i] != *c {
                self.palette[i] = *c;
                self.palette_revs[i] = next;
                palette_changed = true;
            }
        }
        changed |= palette_changed;
        if palette_changed {
            term.colors.palette = self
                .palette
                .iter()
                .enumerate()
                .map(|(i, c)| (i as u8, rgb(*c)))
                .collect();
        }

        self.resume = resume;
        self.rendered = true;
        if changed {
            self.rev = next;
            if scrolled > 0 {
                if self.log.len() == LOG_LEN {
                    if let Some((r, _)) = self.log.pop_front() {
                        self.log_floor = r;
                    }
                }
                self.log.push_back((next, scrolled));
            }
        }
        Ok(changed)
    }

    /// The whole grid, with complete tables and `history` above it.
    pub fn snapshot(&self, history: Vec<Row>) -> Snapshot {
        Snapshot {
            state_gen: self.state_gen,
            rev: self.rev,
            resume: self.resume,
            table_gen: self.tables.gen(),
            row_fmt: ROW_FMT,
            term: self.term.clone(),
            styles: self.tables.styles().to_vec(),
            links: self.tables.links().to_vec(),
            rows: self.rows_since(None),
            history,
        }
    }

    /// The delta from what a client holds to this grid, or `None` when one
    /// cannot be cut and the client needs a snapshot: another `state_gen`
    /// or `table_gen`, a revision the log no longer reaches, or marks past
    /// the tables.
    pub fn delta(&self, held: &Held) -> Option<Delta> {
        if held.state_gen != self.state_gen
            || held.table_gen != self.tables.gen()
            || held.rev < self.log_floor
            || held.rev > self.rev
            || held.style_mark as usize > self.tables.styles().len()
            || held.link_mark as usize > self.tables.links().len()
        {
            return None;
        }
        let base = held.rev;
        // A client applies a delta's size, screen and epoch before its
        // scroll, while this grid shifted, resized and switched in the order
        // they happened. When the viewport was reshaped since the base the
        // two orders can disagree, so such a delta carries every row and no
        // scroll: the client's history misses those lines, which it can
        // fetch, and its viewport is exact.
        let reshaped =
            self.revs.size > base || self.revs.screen > base || self.revs.sb_epoch > base;
        let scrolled: u64 = if reshaped {
            0
        } else {
            self.log
                .iter()
                .filter(|(r, _)| *r > base)
                .map(|(_, s)| u64::from(*s))
                .sum()
        };
        Some(Delta {
            state_gen: self.state_gen,
            table_gen: self.tables.gen(),
            base_rev: base,
            rev: self.rev,
            resume: self.resume,
            scrolled: u32::try_from(scrolled).unwrap_or(u32::MAX),
            term: self.term_since(base),
            styles: self.tables.styles()[held.style_mark as usize..].to_vec(),
            links: self.tables.links()[held.link_mark as usize..].to_vec(),
            rows: self.rows_since((!reshaped).then_some(base)),
        })
    }

    /// What a client holds once it has applied a frame cut now.
    pub fn held(&self) -> Held {
        Held {
            state_gen: self.state_gen,
            table_gen: self.tables.gen(),
            rev: self.rev,
            style_mark: self.tables.styles().len() as u32,
            link_mark: self.tables.links().len() as u32,
        }
    }

    fn rows_since(&self, base: Option<u64>) -> Vec<Row> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| base.is_none_or(|b| r.rev > b))
            .map(|(y, r)| Row {
                y: y as u16,
                line: r.line,
                flags: r.flags,
                cells: r.cells.clone(),
            })
            .collect()
    }

    fn term_since(&self, base: u64) -> Option<TermDelta> {
        let t = &self.term;
        let r = &self.revs;
        let pick = |rev: u64| rev > base;
        let palette: Vec<(u8, (u8, u8, u8))> = self
            .palette_revs
            .iter()
            .enumerate()
            .filter(|(_, rev)| pick(**rev))
            .map(|(i, _)| {
                let c = self.palette[i];
                (i as u8, (c.r, c.g, c.b))
            })
            .collect();
        let colors = ColorsDelta {
            fg: t.colors.fg.filter(|_| pick(r.fg)),
            bg: t.colors.bg.filter(|_| pick(r.bg)),
            cursor: t.colors.cursor.filter(|_| pick(r.cursor_color)),
            palette,
        };
        let d = TermDelta {
            size: pick(r.size).then_some((t.cols, t.rows)),
            screen: pick(r.screen).then_some(t.screen),
            cursor: pick(r.cursor).then_some(t.cursor),
            colors: (colors != ColorsDelta::default()).then_some(colors),
            mouse: pick(r.mouse).then_some(t.mouse),
            flags: pick(r.flags).then_some(t.flags),
            title: pick(r.title).then(|| t.title.clone()),
            cwd: pick(r.cwd).then(|| t.cwd.clone()),
            sb_epoch: pick(r.sb_epoch).then_some(t.sb_epoch),
            history_lines: pick(r.history_lines).then_some(t.history_lines),
            top_line: pick(r.top_line).then_some(t.top_line),
        };
        (d != TermDelta::default()).then_some(d)
    }
}

/// Writes a row's cells so that each stays in its column. The row encoding
/// implies a spacer after a wide cell, so a wide cell is sent as wide only
/// when its spacer tail follows it in the terminal; one that lost its
/// spacer (Ghostty can leave one at the last column, or after a resize)
/// goes as a narrow cell, and a spacer with no wide cell before it as an
/// empty one. Either way every later cell lands where the terminal has it.
#[derive(Debug, Default)]
pub(crate) struct Columns {
    /// A wide cell waiting to see whether its spacer follows.
    held: Option<(u32, u32)>,
    held_text: String,
    /// Empty cells in the default style not written yet: written when a
    /// cell follows them, dropped at the end of the row, where the encoding
    /// leaves them out anyway.
    blanks: u16,
}

impl Columns {
    pub(crate) fn cell(
        &mut self,
        w: &mut RowWriter,
        out: &mut Vec<u8>,
        kind: CellWide,
        style: u32,
        link: u32,
        text: &str,
    ) {
        if kind == CellWide::SpacerTail && self.held.is_some() {
            if let Some((s, l)) = self.held.take() {
                w.cell(out, s, l, &self.held_text, true);
            }
            return;
        }
        if kind == CellWide::SpacerTail {
            // A spacer with no wide cell before it: an empty cell.
            self.blank(w, out);
            return;
        }
        self.flush(w, out);
        if kind == CellWide::Wide {
            self.held = Some((style, link));
            self.held_text.clear();
            self.held_text.push_str(text);
        } else {
            w.cell(out, style, link, text, false);
        }
    }

    /// A printable ASCII cell with no link.
    pub(crate) fn ascii(&mut self, w: &mut RowWriter, out: &mut Vec<u8>, style: u32, byte: u8) {
        if self.held.is_some() || self.blanks > 0 {
            self.flush(w, out);
        }
        w.ascii(out, style, 0, byte);
    }

    /// An empty cell in the default style with no link.
    pub(crate) fn blank(&mut self, w: &mut RowWriter, out: &mut Vec<u8>) {
        if let Some((s, l)) = self.held.take() {
            w.cell(out, s, l, &self.held_text, false);
        }
        self.blanks += 1;
    }

    pub(crate) fn finish(&mut self, w: &mut RowWriter, out: &mut Vec<u8>) {
        if let Some((s, l)) = self.held.take() {
            self.write_blanks(w, out);
            w.cell(out, s, l, &self.held_text, false);
        }
        self.blanks = 0;
        w.finish(out);
    }

    fn flush(&mut self, w: &mut RowWriter, out: &mut Vec<u8>) {
        if let Some((s, l)) = self.held.take() {
            w.cell(out, s, l, &self.held_text, false);
        }
        self.write_blanks(w, out);
    }

    fn write_blanks(&mut self, w: &mut RowWriter, out: &mut Vec<u8>) {
        for _ in 0..std::mem::take(&mut self.blanks) {
            w.cell(out, 0, 0, "", false);
        }
    }
}

/// A cell nothing was written to, as Ghostty stores one, if it is what it
/// should be: empty, narrow, unstyled, unlinked.
fn blank_cell() -> Option<libghostty_vt::screen::Cell> {
    let t = Terminal::new(1, 1).ok()?;
    let c = t
        .grid_ref(Point::Active(PointCoordinate { x: 0, y: 0 }))
        .ok()?
        .cell()
        .ok()?;
    let plain = c.content_tag().ok()? == CellContentTag::Codepoint
        && c.codepoint().ok()? == 0
        && c.wide().ok()? == CellWide::Narrow
        && !c.has_styling().ok()?
        && !c.has_hyperlink().ok()?;
    plain.then_some(c)
}

/// Sets `field` to `v`, stamping `rev` with `next` when it changed.
fn set<T: PartialEq>(field: &mut T, v: T, rev: &mut u64, next: u64) -> bool {
    if *field == v {
        return false;
    }
    *field = v;
    *rev = next;
    true
}

fn row_flags_of(raw: libghostty_vt::screen::Row) -> Result<u8, Error> {
    let mut f = 0;
    if raw.is_wrapped()? {
        f |= row_flags::WRAPPED;
    }
    if raw.is_wrap_continuation()? {
        f |= row_flags::WRAP_CONTINUATION;
    }
    match raw.semantic_prompt()? {
        RowSemanticPrompt::Prompt => f |= row_flags::PROMPT,
        RowSemanticPrompt::Continuation => f |= row_flags::PROMPT_CONTINUATION,
        RowSemanticPrompt::None => {}
    }
    Ok(f)
}

/// The background of a cell that holds only a background colour, which
/// Ghostty keeps in the cell rather than in its style.
pub(crate) fn bg_only(c: libghostty_vt::screen::Cell) -> Result<Option<Color>, Error> {
    bg_of(c, c.content_tag()?)
}

fn bg_of(c: libghostty_vt::screen::Cell, tag: CellContentTag) -> Result<Option<Color>, Error> {
    Ok(match tag {
        CellContentTag::BgColorPalette => Some(Color::Palette(c.bg_color_palette()?.0)),
        CellContentTag::BgColorRgb => {
            let rgb = c.bg_color_rgb()?;
            Some(Color::Rgb(rgb.r, rgb.g, rgb.b))
        }
        _ => None,
    })
}

/// The URI of the hyperlink at `at`, read through a grid reference: the
/// render state does not carry it, so it is read only on rows flagged as
/// having one (TP §3).
pub(crate) fn read_uri<'a>(t: &Term, at: Point, buf: &'a mut Vec<u8>) -> Option<&'a str> {
    let g = t.grid_ref(at).ok()?;
    if buf.len() < 256 {
        buf.resize(256, 0);
    }
    let n = loop {
        match g.hyperlink_uri(buf) {
            Ok(n) => break n,
            Err(libghostty_vt::Error::OutOfSpace { required }) if required > buf.len() => {
                buf.resize(required, 0)
            }
            Err(_) => return None,
        }
    };
    let uri = std::str::from_utf8(buf.get(..n)?).ok()?;
    (!uri.is_empty()).then_some(uri)
}

fn cursor_of(snap: &libghostty_vt::render::Snapshot<'_, '_>) -> Result<CursorState, Error> {
    let at = snap.cursor_viewport()?;
    Ok(CursorState {
        x: at.map_or(0, |c| c.x),
        y: at.map_or(0, |c| c.y),
        visible: at.is_some() && snap.cursor_visible()?,
        blinking: snap.cursor_blinking()?,
        style: match snap.cursor_visual_style()? {
            CursorVisualStyle::Block => CursorStyle::Block,
            CursorVisualStyle::BlockHollow => CursorStyle::BlockHollow,
            CursorVisualStyle::Bar => CursorStyle::Bar,
            CursorVisualStyle::Underline => CursorStyle::Underline,
            _ => CursorStyle::Unknown,
        },
        wide_tail: at.is_some_and(|c| c.at_wide_tail),
    })
}

fn mouse_of(t: &Term) -> Result<MouseMode, Error> {
    Ok(if t.mode(Mode::ANY_MOUSE)? {
        MouseMode::Any
    } else if t.mode(Mode::BUTTON_MOUSE)? {
        MouseMode::Button
    } else if t.mode(Mode::NORMAL_MOUSE)? {
        MouseMode::Normal
    } else if t.mode(Mode::X10_MOUSE)? {
        MouseMode::X10
    } else {
        MouseMode::None
    })
}
