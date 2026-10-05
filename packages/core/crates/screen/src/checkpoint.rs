//! Checkpoints of an [`Emulator`]: the VT, and the resizes between it, that
//! rebuild the terminal exactly from blank.
//!
//! Ghostty's own VT formatter writes what a screen looks like, which is not
//! enough to carry on from: it prints empty cells as spaces, drops soft
//! wraps, a pending wrap, a saved cursor, background-only cells on blank
//! rows and the screen that is not showing, and the next bytes then land
//! differently. So the screen is drawn here cell by cell from Ghostty's grid,
//! with the state only [`crate::emulator::Tracker`] knows rebuilt by the same
//! sequences that made it, and the formatter is used only for the few pieces
//! it does exactly (margins, tab stops, the open hyperlink, protection).
//!
//! A checkpoint is only handed out after it passed its own restore check: a
//! terminal rebuilt from it has the same [`Emulator::fingerprint`] as the one
//! it was cut from. The fingerprint's digest travels with it, so the check
//! can be repeated by whoever restores it, with whatever Ghostty they link.

use std::fmt::Write as _;

use libghostty_vt::fmt::{Format, Formatter, FormatterOptions};
use libghostty_vt::render::{CursorVisualStyle, RenderState};
use libghostty_vt::screen::{
    CellContentTag, CellSemanticContent, CellWide, GridRef, RowSemanticPrompt,
};
use libghostty_vt::style::{RgbColor, Style, StyleColor, Underline};
use libghostty_vt::terminal::{Mode, ModeKind, Point, PointCoordinate, Terminal};

/// The terminal a checkpoint reads.
type Term = Terminal<'static, 'static>;

use crate::emulator::{Charset, Cs, Emulator, Kitty, Protect, Redraw, Saved, Sem, Which};
use crate::{Error, Result};

/// Why no checkpoint was cut at this point. Each reason is a fixed phrase so
/// a caller can count them.
pub type Uncut = &'static str;

/// One step of a rebuild.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Vt(Vec<u8>),
    Resize { cols: u16, rows: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    /// The size the rebuild starts at, before any resize step.
    pub start: (u16, u16),
    /// The history limit of the terminal it was cut from, which the rebuild
    /// keeps too.
    pub scrollback: u64,
    pub steps: Vec<Step>,
    pub cols: u16,
    pub rows: u16,
    pub title: String,
    pub cwd: String,
    /// CRC-32 and length of the fingerprint of the terminal it was cut from.
    pub digest: (u32, u32),
}

/// Ghostty's modes, with whether each is ANSI and its initial value.
const MODES: &[(u16, bool, bool)] = &[
    (2, true, false),
    (4, true, false),
    (12, true, true),
    (20, true, false),
    (1, false, false),
    (3, false, false),
    (4, false, false),
    (5, false, false),
    (6, false, false),
    (7, false, true),
    (8, false, false),
    (9, false, false),
    (12, false, false),
    (25, false, true),
    (40, false, false),
    (45, false, false),
    (47, false, false),
    (66, false, false),
    (67, false, false),
    (69, false, false),
    (1000, false, false),
    (1002, false, false),
    (1003, false, false),
    (1004, false, false),
    (1005, false, false),
    (1006, false, false),
    (1007, false, true),
    (1015, false, false),
    (1016, false, false),
    (1035, false, true),
    (1036, false, true),
    (1039, false, false),
    (1045, false, false),
    (1047, false, false),
    (1048, false, false),
    (1049, false, false),
    (2004, false, false),
    (2026, false, false),
    (2027, false, false),
    (2031, false, false),
    (2048, false, false),
];

/// Modes a rebuild sets elsewhere, or cannot set without side effects:
/// 132-column mode resizes, the screen modes switch screens, 1048 saves the
/// cursor, origin and margins are placed around the cursor and margins, and
/// 2027 is set before any text is drawn.
fn set_elsewhere(value: u16, ansi: bool) -> bool {
    !ansi && matches!(value, 3 | 6 | 47 | 69 | 1047 | 1048 | 1049 | 2027)
}

fn mode(value: u16, ansi: bool) -> Mode {
    Mode::new(value, if ansi { ModeKind::Ansi } else { ModeKind::Dec })
}

impl Emulator {
    /// Cuts a checkpoint here, or says why it cannot, and carries on from
    /// the terminal rebuilt from it.
    ///
    /// Carrying on from the rebuild is what makes recovery exact. Ghostty
    /// keeps row flags in page memory the screen no longer shows (a scroll
    /// reuses a row with its wrap flags, a resize can grow back into rows it
    /// let go of), and a later resize can reflow by them. No checkpoint can
    /// read that memory, but a terminal that carries on from the rebuild has
    /// the same memory as any other rebuild of the same checkpoint, so a
    /// session recovered from it and one that never died stay the same.
    pub fn checkpoint(&mut self) -> std::result::Result<Checkpoint, Uncut> {
        let (cp, mut rebuilt) = self.cut()?;
        // Counters of the output, not of the VT that rebuilt it.
        rebuilt.parsed = self.parsed;
        rebuilt.history_clears = self.history_clears;
        *self = rebuilt;
        Ok(cp)
    }

    /// Cuts a checkpoint and returns it with the terminal rebuilt from it,
    /// leaving this one as it is.
    pub fn cut(&mut self) -> std::result::Result<(Checkpoint, Emulator), Uncut> {
        if let Some(why) = self.uncuttable() {
            return Err(why);
        }
        let cp = self.build()?;
        let rebuilt = Emulator::restore(&cp).map_err(|_| "rebuild failed")?;
        if rebuilt.fingerprint() != self.fingerprint() {
            return Err("restore check");
        }
        Ok((cp, rebuilt))
    }

    /// Rebuilds a terminal from a checkpoint. Whether it is the terminal the
    /// checkpoint was cut from is [`Checkpoint::matches`]'s question.
    pub fn restore(cp: &Checkpoint) -> Result<Emulator> {
        let scrollback = usize::try_from(cp.scrollback).map_err(|_| Error::Dimension(0))?;
        let mut em =
            Emulator::with_scrollback(u32::from(cp.start.0), u32::from(cp.start.1), scrollback)?;
        let mut discard = Vec::new();
        for step in &cp.steps {
            match step {
                Step::Vt(bytes) => em.feed(bytes, &mut discard),
                Step::Resize { cols, rows } => {
                    em.resize(u32::from(*cols), u32::from(*rows), &mut discard)?
                }
            }
            discard.clear();
        }
        if (em.cols, em.rows) != (cp.cols, cp.rows) {
            return Err(Error::Dimension(u32::from(cp.cols)));
        }
        em.title.clone_from(&cp.title);
        em.cwd.clone_from(&cp.cwd);
        // What rebuilt it was checkpoint VT, not output.
        em.parsed = 0;
        Ok(em)
    }

    fn build(&mut self) -> std::result::Result<Checkpoint, Uncut> {
        let mut steps = Vec::new();
        let mut vt = Vec::new();
        if self.track.redraw != Redraw::No {
            // Only a prompt start sets it. On the blank screen this marks the
            // first row as a prompt, and OSC 133;C at column 0 unmarks it.
            vt.extend_from_slice(match self.track.redraw {
                Redraw::Last => b"\x1b]133;A;redraw=last\x1b\\".as_slice(),
                _ => b"\x1b]133;A;redraw=1\x1b\\",
            });
            vt.extend_from_slice(b"\x1b]133;C\x1b\\");
        }
        palette(self, &mut vt)?;
        let grapheme = self
            .term
            .mode(Mode::GRAPHEME_CLUSTER)
            .map_err(|_| "modes unreadable")?;
        let (start, entry) = match self.track.inactive.clone() {
            None => {
                if grapheme {
                    vt.extend_from_slice(b"\x1b[?2027h");
                }
                ((self.cols, self.rows), Charset::default())
            }
            Some(cap) => {
                let part = cap.part.clone()?;
                if cap.grapheme {
                    vt.extend_from_slice(b"\x1b[?2027h");
                }
                if self.track.active == Which::Primary {
                    // The alternate screen is drawn first, on a fresh one.
                    vt.extend_from_slice(b"\x1b[?47h");
                }
                vt.extend_from_slice(&part);
                vt.extend_from_slice(&cap.op);
                let mut wrap = true;
                for &(cols, rows, w) in &cap.resizes {
                    if w != wrap {
                        vt.extend_from_slice(if w { b"\x1b[?7h" } else { b"\x1b[?7l" });
                        wrap = w;
                    }
                    steps.push(Step::Vt(std::mem::take(&mut vt)));
                    steps.push(Step::Resize { cols, rows });
                }
                if !wrap {
                    vt.extend_from_slice(b"\x1b[?7h");
                }
                if cap.grapheme != grapheme {
                    vt.extend_from_slice(if grapheme {
                        b"\x1b[?2027h"
                    } else {
                        b"\x1b[?2027l"
                    });
                }
                ((cap.cols, cap.rows), cap.entry_charset)
            }
        };
        let prev = self.previous_char()?;
        let part = Part::new(self, true, entry)?.draw(self, prev)?;
        vt.extend_from_slice(&part);
        steps.push(Step::Vt(vt));
        let fp = self.fingerprint();
        Ok(Checkpoint {
            start,
            scrollback: self.scrollback as u64,
            steps,
            cols: self.cols,
            rows: self.rows,
            title: self.title.clone(),
            cwd: self.cwd.clone(),
            digest: digest(&fp),
        })
    }

    /// What REP would repeat: the newest printed codepoint Ghostty kept.
    fn previous_char(&mut self) -> std::result::Result<Option<u32>, Uncut> {
        let grapheme = self
            .term
            .mode(Mode::GRAPHEME_CLUSTER)
            .map_err(|_| "modes unreadable")?;
        let recent: Vec<u32> = self.parser.recent.iter().collect();
        for cp in recent {
            if cp <= 0xff {
                return Ok(Some(cp));
            }
            if grapheme {
                return Err("previous character unknown");
            }
            if self.counted(cp) {
                return Ok(Some(cp));
            }
        }
        if self.parser.recent.is_full() {
            Err("previous character unknown")
        } else {
            Ok(None)
        }
    }

    /// Everything a rebuild must reproduce, as text: Ghostty's full VT
    /// output, every cell and row attribute of the showing screen, what the
    /// emulator follows beside Ghostty, and the session labels.
    pub fn fingerprint(&self) -> String {
        let mut fp = String::new();
        let t = &self.term;
        let _ = writeln!(
            fp,
            "size {}x{} screen {:?}",
            self.cols,
            self.rows,
            t.active_screen()
        );
        let opts = FormatterOptions::new()
            .with_format(Format::Vt)
            .with_palette(true)
            .with_modes(true)
            .with_scrolling_region(true)
            .with_tabstops(true)
            .with_pwd(true)
            .with_keyboard(true)
            .with_cursor(true)
            .with_style(true)
            .with_hyperlink(true)
            .with_protection(true)
            .with_kitty_keyboard(true)
            .with_charsets(true);
        match Formatter::new(t, opts).and_then(|mut f| f.format_alloc(None)) {
            Ok(bytes) => {
                let _ = writeln!(fp, "vt {:?}", String::from_utf8_lossy(&bytes));
            }
            Err(e) => {
                let _ = writeln!(fp, "vt error {e:?}");
            }
        }
        let history = t.scrollback_rows().unwrap_or(0);
        let _ = writeln!(fp, "history {history}");
        for y in 0..history + usize::from(self.rows) {
            let at = |x: u16| Point::Screen(PointCoordinate { x, y: y as u32 });
            let row = t.grid_ref(at(0)).and_then(|g| g.row());
            let _ = write!(fp, "row {y} ");
            match row {
                Ok(r) => {
                    let _ = writeln!(
                        fp,
                        "wrap {:?} cont {:?} prompt {:?} kitty {:?}",
                        r.is_wrapped(),
                        r.is_wrap_continuation(),
                        r.semantic_prompt(),
                        r.has_kitty_virtual_placeholder()
                    );
                }
                Err(e) => {
                    let _ = writeln!(fp, "error {e:?}");
                }
            }
            for x in 0..self.cols {
                let _ = writeln!(fp, " {}", describe_cell(t, at(x)));
            }
        }
        let _ = writeln!(
            fp,
            "cursor {:?} {:?} pending {:?} style {:?} kitty {:?}",
            t.cursor_x(),
            t.cursor_y(),
            t.is_cursor_pending_wrap(),
            t.cursor_style(),
            t.kitty_keyboard_flags().map(|f| f.bits()),
        );
        let _ = writeln!(
            fp,
            "title {:?} pwd {:?} colors {:?} {:?} {:?}",
            t.title(),
            t.pwd(),
            t.fg_color(),
            t.bg_color(),
            t.cursor_color()
        );
        let _ = writeln!(fp, "shape {:?}", cursor_shape(self));
        for &(value, ansi, _) in MODES {
            let _ = write!(
                fp,
                "{value}{}={:?} ",
                if ansi { "a" } else { "" },
                t.mode(mode(value, ansi))
            );
        }
        let tr = &self.track;
        let _ = writeln!(
            fp,
            "\nactive {:?} screens {:?} taint {:?} redraw {:?}",
            tr.active, tr.screens, tr.taint, tr.redraw
        );
        let _ = writeln!(fp, "inactive {:?}", tr.inactive);
        let _ = writeln!(fp, "labels {:?} {:?}", self.title, self.cwd);
        fp
    }
}

impl Checkpoint {
    /// The restore check: whether `rebuilt` is the terminal this was cut from.
    pub fn matches(&self, rebuilt: &Emulator) -> bool {
        digest(&rebuilt.fingerprint()) == self.digest
    }

    /// Bytes for storage. [`Checkpoint::decode`] reads them back.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for n in [self.start.0, self.start.1, self.cols, self.rows] {
            out.extend_from_slice(&n.to_le_bytes());
        }
        out.extend_from_slice(&self.scrollback.to_le_bytes());
        out.extend_from_slice(&self.digest.0.to_le_bytes());
        out.extend_from_slice(&self.digest.1.to_le_bytes());
        put_bytes(&mut out, self.title.as_bytes());
        put_bytes(&mut out, self.cwd.as_bytes());
        out.extend_from_slice(&(self.steps.len() as u32).to_le_bytes());
        for step in &self.steps {
            match step {
                Step::Vt(bytes) => {
                    out.push(0);
                    put_bytes(&mut out, bytes);
                }
                Step::Resize { cols, rows } => {
                    out.push(1);
                    out.extend_from_slice(&cols.to_le_bytes());
                    out.extend_from_slice(&rows.to_le_bytes());
                }
            }
        }
        out
    }

    /// `None` for anything [`Checkpoint::encode`] could not have written.
    pub fn decode(bytes: &[u8]) -> Option<Checkpoint> {
        let mut r = Reader(bytes);
        let start = (r.u16()?, r.u16()?);
        let (cols, rows) = (r.u16()?, r.u16()?);
        let scrollback = r.u64()?;
        let digest = (r.u32()?, r.u32()?);
        let title = String::from_utf8(r.bytes()?.to_vec()).ok()?;
        let cwd = String::from_utf8(r.bytes()?.to_vec()).ok()?;
        let n = r.u32()?;
        let mut steps = Vec::new();
        for _ in 0..n {
            steps.push(match r.u8()? {
                0 => Step::Vt(r.bytes()?.to_vec()),
                1 => Step::Resize {
                    cols: r.u16()?,
                    rows: r.u16()?,
                },
                _ => return None,
            });
        }
        if !r.0.is_empty() || [start.0, start.1, cols, rows].contains(&0) {
            return None;
        }
        Some(Checkpoint {
            start,
            scrollback,
            steps,
            cols,
            rows,
            title,
            cwd,
            digest,
        })
    }
}

fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u32).to_le_bytes());
    out.extend_from_slice(b);
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.u32()? as usize;
        self.take(n)
    }
}

fn digest(fp: &str) -> (u32, u32) {
    (crc32fast::hash(fp.as_bytes()), fp.len() as u32)
}

fn active(x: u16, y: u16) -> Point {
    Point::Active(PointCoordinate { x, y: u32::from(y) })
}

/// One cell, every attribute, for the fingerprint.
fn describe_cell(t: &Term, at: Point) -> String {
    let Ok(g) = t.grid_ref(at) else {
        return "unreadable".into();
    };
    let Ok(c) = g.cell() else {
        return "unreadable".into();
    };
    let mut s = format!(
        "{:?} {:?} {:?} {:?} prot {:?} sem {:?}",
        c.content_tag(),
        graphemes(&g),
        c.wide(),
        g.style(),
        c.is_protected(),
        c.semantic_content()
    );
    if matches!(c.content_tag(), Ok(CellContentTag::BgColorPalette)) {
        let _ = write!(s, " bg {:?}", c.bg_color_palette());
    }
    if matches!(c.content_tag(), Ok(CellContentTag::BgColorRgb)) {
        let _ = write!(s, " bg {:?}", c.bg_color_rgb());
    }
    if c.has_hyperlink().unwrap_or(false) {
        let _ = write!(s, " link {:?}", hyperlink(&g));
    }
    s
}

fn graphemes(g: &GridRef<'_>) -> Vec<char> {
    let mut buf = vec!['\0'; 16];
    loop {
        match g.graphemes(&mut buf) {
            Ok(n) => {
                buf.truncate(n);
                return buf;
            }
            Err(libghostty_vt::error::Error::OutOfSpace { required }) if required > buf.len() => {
                buf.resize(required, '\0')
            }
            Err(_) => return Vec::new(),
        }
    }
}

fn hyperlink(g: &GridRef<'_>) -> Option<Vec<u8>> {
    let mut buf = vec![0; 256];
    loop {
        match g.hyperlink_uri(&mut buf) {
            Ok(n) => {
                buf.truncate(n);
                return Some(buf);
            }
            Err(libghostty_vt::error::Error::OutOfSpace { required }) if required > buf.len() => {
                buf.resize(required, 0)
            }
            Err(_) => return None,
        }
    }
}

fn cursor_shape(em: &Emulator) -> Option<(CursorVisualStyle, bool)> {
    let mut rs = RenderState::new().ok()?;
    let snap = rs.update(&em.term).ok()?;
    Some((
        snap.cursor_visual_style().ok()?,
        snap.cursor_blinking().ok()?,
    ))
}

/// OSC 4 for each palette entry that differs from the default, and OSC
/// 10/11/12 for dynamic colors that were set.
fn palette(em: &Emulator, out: &mut Vec<u8>) -> std::result::Result<(), Uncut> {
    let t = &em.term;
    let (now, def) = (
        t.color_palette().map_err(|_| "palette unreadable")?,
        t.default_color_palette()
            .map_err(|_| "palette unreadable")?,
    );
    for i in 0..=255u8 {
        let idx = libghostty_vt::style::PaletteIndex(i);
        let c = now.get(idx);
        if c != def.get(idx) {
            write_bytes(out, format_args!("\x1b]4;{i};{}\x1b\\", rgb_spec(c)));
        }
    }
    let dynamic = [
        (10, t.fg_color(), t.default_fg_color()),
        (11, t.bg_color(), t.default_bg_color()),
        (12, t.cursor_color(), t.default_cursor_color()),
    ];
    for (n, now, def) in dynamic {
        let now = now.map_err(|_| "colors unreadable")?;
        if now != def.map_err(|_| "colors unreadable")? {
            match now {
                Some(c) => write_bytes(out, format_args!("\x1b]{n};{}\x1b\\", rgb_spec(c))),
                None => write_bytes(out, format_args!("\x1b]1{n}\x1b\\")),
            }
        }
    }
    Ok(())
}

fn rgb_spec(c: RgbColor) -> String {
    format!("rgb:{:02x}/{:02x}/{:02x}", c.r, c.g, c.b)
}

fn write_bytes(out: &mut Vec<u8>, args: std::fmt::Arguments<'_>) {
    use std::io::Write as _;
    let _ = out.write_fmt(args);
}

/// The SGR that sets exactly `s`.
fn sgr(out: &mut Vec<u8>, s: &Style) {
    let mut p = String::from("\x1b[0");
    if s.bold {
        p.push_str(";1");
    }
    if s.faint {
        p.push_str(";2");
    }
    if s.italic {
        p.push_str(";3");
    }
    match s.underline {
        Underline::None => {}
        Underline::Single => p.push_str(";4"),
        Underline::Double => p.push_str(";4:2"),
        Underline::Curly => p.push_str(";4:3"),
        Underline::Dotted => p.push_str(";4:4"),
        Underline::Dashed => p.push_str(";4:5"),
        _ => p.push_str(";4"),
    }
    if s.blink {
        p.push_str(";5");
    }
    if s.inverse {
        p.push_str(";7");
    }
    if s.invisible {
        p.push_str(";8");
    }
    if s.strikethrough {
        p.push_str(";9");
    }
    if s.overline {
        p.push_str(";53");
    }
    for (base, c) in [(38, s.fg_color), (48, s.bg_color), (58, s.underline_color)] {
        match c {
            StyleColor::None => {}
            StyleColor::Palette(i) => {
                let _ = write!(p, ";{base};5;{}", i.0);
            }
            StyleColor::Rgb(c) => {
                let _ = write!(p, ";{base};2;{};{};{}", c.r, c.g, c.b);
            }
        }
    }
    p.push('m');
    out.extend_from_slice(p.as_bytes());
}

fn cup(out: &mut Vec<u8>, x: u16, y: u16) {
    write_bytes(
        out,
        format_args!("\x1b[{};{}H", u32::from(y) + 1, u32::from(x) + 1),
    );
}

/// The pieces of Ghostty's formatter output that are taken as they are.
#[derive(Default)]
struct Trailer {
    hyperlink: Option<Vec<u8>>,
    protected: bool,
    top_bottom: Option<(Vec<u8>, u16)>,
    left_right: Option<(Vec<u8>, u16)>,
    tabs: Vec<u8>,
    keyboard: Option<Vec<u8>>,
    pwd: Option<Vec<u8>>,
}

/// Runs the formatter with and without its trailing state and splits the
/// difference into sequences.
fn trailer(em: &Emulator) -> std::result::Result<Trailer, Uncut> {
    let run = |extras: bool| {
        let opts = FormatterOptions::new()
            .with_format(Format::Vt)
            .with_hyperlink(extras)
            .with_protection(extras)
            .with_scrolling_region(extras)
            .with_tabstops(extras)
            .with_keyboard(extras)
            .with_pwd(extras);
        Formatter::new(&em.term, opts).and_then(|mut f| f.format_alloc(None))
    };
    let (plain, full) = (
        run(false).map_err(|_| "formatter failed")?,
        run(true).map_err(|_| "formatter failed")?,
    );
    let tail = full.strip_prefix(&plain[..]).ok_or("formatter trailer")?;
    let mut tr = Trailer::default();
    let mut rest = tail;
    while !rest.is_empty() {
        let n = seq_len(rest).ok_or("formatter trailer")?;
        let (seq, after) = rest.split_at(n);
        rest = after;
        if seq.starts_with(b"\x1b]8;") {
            // An OSC 8 that closes the link is the same as none.
            if !seq.starts_with(b"\x1b]8;;\x1b") && !seq.starts_with(b"\x1b]8;;\x07") {
                tr.hyperlink = Some(seq.to_vec());
            }
        } else if seq.starts_with(b"\x1b]7;") {
            tr.pwd = Some(seq.to_vec());
        } else if seq == b"\x1b[1\"q" {
            tr.protected = true;
        } else if seq == b"\x1b[0\"q" {
        } else if seq == b"\x1b[3g" || seq == b"\x1bH" || is_csi_with(seq, b'G') {
            tr.tabs.extend_from_slice(seq);
        } else if seq.starts_with(b"\x1b[>4;") && seq.ends_with(b"m") {
            tr.keyboard = Some(seq.to_vec());
        } else if is_csi_with(seq, b'r') {
            let first = csi_first(seq).ok_or("formatter trailer")?;
            tr.top_bottom = Some((seq.to_vec(), first));
        } else if is_csi_with(seq, b's') {
            let first = csi_first(seq).ok_or("formatter trailer")?;
            tr.left_right = Some((seq.to_vec(), first));
        } else {
            return Err("formatter trailer");
        }
    }
    Ok(tr)
}

/// The length of the escape sequence at the start of `s`.
fn seq_len(s: &[u8]) -> Option<usize> {
    if s.first() != Some(&0x1b) {
        return None;
    }
    match s.get(1)? {
        b'[' => s
            .iter()
            .skip(2)
            .position(|b| (0x40..=0x7e).contains(b))
            .map(|i| i + 3),
        b']' => {
            for i in 2..s.len() {
                if s[i] == 0x07 {
                    return Some(i + 1);
                }
                if s[i] == 0x1b && s.get(i + 1) == Some(&b'\\') {
                    return Some(i + 2);
                }
            }
            None
        }
        _ => Some(2),
    }
}

fn is_csi_with(seq: &[u8], fin: u8) -> bool {
    seq.starts_with(b"\x1b[")
        && seq.last() == Some(&fin)
        && seq[2..seq.len() - 1]
            .iter()
            .all(|b| b.is_ascii_digit() || *b == b';')
}

/// The first parameter of a numeric CSI, 1-based as written.
fn csi_first(seq: &[u8]) -> Option<u16> {
    let body = &seq[2..seq.len() - 1];
    let first = body.split(|&b| b == b';').next()?;
    if first.is_empty() {
        return Some(1);
    }
    std::str::from_utf8(first).ok()?.parse().ok()
}

/// Draws the showing screen as a screen that is about to stop showing: what
/// belongs to it rather than to the terminal.
pub(crate) fn screen_part(em: &Emulator) -> std::result::Result<Vec<u8>, Uncut> {
    Part::new(em, false, Charset::default())?.draw(em, None)
}

/// The pen of a rebuild while it draws.
struct Pen {
    out: Vec<u8>,
    style: Option<Style>,
    link: Option<Vec<u8>>,
    protected: Option<bool>,
    sem: Option<Sem>,
    /// Whether any OSC 133 was written, so rows' marks need setting after.
    marked: bool,
    printed: bool,
}

impl Pen {
    fn style(&mut self, s: &Style) {
        if self.style.as_ref() != Some(s) {
            sgr(&mut self.out, s);
            self.style = Some(*s);
        }
    }

    fn link(&mut self, uri: Option<Vec<u8>>) {
        if self.link != uri {
            self.out.extend_from_slice(b"\x1b]8;;");
            if let Some(u) = &uri {
                self.out.extend_from_slice(u);
            }
            self.out.extend_from_slice(b"\x1b\\");
            self.link = uri;
        }
    }

    fn protect(&mut self, on: bool) {
        if self.protected != Some(on) {
            self.out
                .extend_from_slice(if on { b"\x1b[1\"q" } else { b"\x1b[0\"q" });
            self.protected = Some(on);
        }
    }

    fn sem(&mut self, s: Sem) {
        if self.sem != Some(s) {
            self.out.extend_from_slice(match s {
                Sem::Output => b"\x1b]133;C\x1b\\".as_slice(),
                Sem::Input => b"\x1b]133;B\x1b\\",
                Sem::Prompt => b"\x1b]133;P;k=i\x1b\\",
            });
            self.sem = Some(s);
            self.marked = true;
        }
    }

    fn neutral(&mut self) {
        self.style(&Style::default());
        self.link(None);
        self.protect(false);
    }

    fn text(&mut self, cps: &[char]) {
        let mut buf = [0; 4];
        for c in cps {
            self.out
                .extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
        self.printed = true;
    }
}

/// A cell as the rebuild sees it.
enum Kind {
    /// Never written, or erased with the default background.
    Blank,
    /// Erased with a background color.
    Bg(Style),
    Text(Cell),
    /// The right half of a wide character.
    Tail,
    /// A wide character that did not fit at the end of a wrapped row.
    Head,
}

struct Cell {
    cps: Vec<char>,
    style: Style,
    link: Option<Vec<u8>>,
    protected: bool,
    sem: Sem,
    wide: bool,
}

fn read_cell(t: &Term, x: u16, y: u16) -> std::result::Result<Kind, Uncut> {
    read_at(t, active(x, y))
}

fn read_at(t: &Term, at: Point) -> std::result::Result<Kind, Uncut> {
    const BAD: Uncut = "cell unreadable";
    let g = t.grid_ref(at).map_err(|_| BAD)?;
    let c = g.cell().map_err(|_| BAD)?;
    let wide = c.wide().map_err(|_| BAD)?;
    let protected = c.is_protected().map_err(|_| BAD)?;
    let sem = match c.semantic_content().map_err(|_| BAD)? {
        CellSemanticContent::Output => Sem::Output,
        CellSemanticContent::Input => Sem::Input,
        CellSemanticContent::Prompt => Sem::Prompt,
    };
    let link = if c.has_hyperlink().map_err(|_| BAD)? {
        Some(hyperlink(&g).ok_or(BAD)?)
    } else {
        None
    };
    match wide {
        CellWide::SpacerTail => return Ok(Kind::Tail),
        CellWide::SpacerHead => return Ok(Kind::Head),
        _ => {}
    }
    if !c.has_text().map_err(|_| BAD)? {
        let plain = !protected && link.is_none() && sem == Sem::Output && wide == CellWide::Narrow;
        return match c.content_tag().map_err(|_| BAD)? {
            CellContentTag::Codepoint if plain && !c.has_styling().map_err(|_| BAD)? => {
                Ok(Kind::Blank)
            }
            CellContentTag::BgColorPalette if plain => Ok(Kind::Bg(Style {
                bg_color: StyleColor::Palette(c.bg_color_palette().map_err(|_| BAD)?),
                ..Style::default()
            })),
            CellContentTag::BgColorRgb if plain => Ok(Kind::Bg(Style {
                bg_color: StyleColor::Rgb(c.bg_color_rgb().map_err(|_| BAD)?),
                ..Style::default()
            })),
            _ => Err("styled empty cell"),
        };
    }
    Ok(Kind::Text(Cell {
        cps: graphemes(&g),
        style: g.style().map_err(|_| BAD)?,
        link,
        protected,
        sem,
        wide: wide == CellWide::Wide,
    }))
}

struct RowInfo {
    wrap: bool,
    cont: bool,
    prompt: RowSemanticPrompt,
}

/// One screen's drawing, with the trailing state it needs.
struct Part {
    global: bool,
    cols: u16,
    rows: u16,
    /// History rows above the screen, which `rows_info` starts with.
    history: usize,
    /// Whether the terminal keeps history at all.
    keeps_history: bool,
    rows_info: Vec<RowInfo>,
    trailer: Trailer,
    /// The character sets the screen starts with in the rebuild.
    entry: Charset,
}

impl Part {
    fn new(em: &Emulator, global: bool, entry: Charset) -> std::result::Result<Part, Uncut> {
        let t = &em.term;
        let history = t.scrollback_rows().map_err(|_| "rows unreadable")?;
        let total = history + usize::from(em.rows);
        let mut rows_info = Vec::with_capacity(total);
        for y in 0..total {
            let at = Point::Screen(PointCoordinate { x: 0, y: y as u32 });
            let r = t
                .grid_ref(at)
                .and_then(|g| g.row())
                .map_err(|_| "row unreadable")?;
            let bad = |_| "row unreadable";
            if r.has_kitty_virtual_placeholder().map_err(bad)? {
                return Err("kitty placeholder");
            }
            rows_info.push(RowInfo {
                wrap: r.is_wrapped().map_err(bad)?,
                cont: r.is_wrap_continuation().map_err(bad)?,
                prompt: r.semantic_prompt().map_err(bad)?,
            });
        }
        Ok(Part {
            global,
            cols: em.cols,
            rows: em.rows,
            history,
            keeps_history: em.scrollback > 0,
            rows_info,
            trailer: trailer(em)?,
            entry,
        })
    }

    fn draw(self, em: &Emulator, prev: Option<u32>) -> std::result::Result<Vec<u8>, Uncut> {
        let t = &em.term;
        let model = *em.track.screen();
        let mut pen = Pen {
            out: Vec::new(),
            style: None,
            link: None,
            protected: None,
            sem: None,
            marked: false,
            printed: false,
        };
        // Whatever a screen switch copied onto the cursor, start neutral. The
        // semantic state is set without counting as a mark: on a blank
        // screen it changes no row.
        pen.neutral();
        pen.out.extend_from_slice(b"\x1b]133;C\x1b\\");
        pen.sem = Some(Sem::Output);
        let mut charset = self.entry;
        if let Some(sc) = model.saved {
            charset = self.saved_cursor(&mut pen, &sc, charset)?;
        }
        // Text is drawn through a slot that prints as it is.
        let slot = (0..4u8)
            .find(|&s| matches!(charset.g[usize::from(s)], Cs::Utf8 | Cs::Ascii))
            .ok_or("no plain character set")?;
        if charset.single_shift.is_some() {
            return Err("single shift at a rebuild");
        }
        invoke_gl(&mut pen.out, slot);
        charset.gl = slot;
        kitty(&mut pen.out, &model.kitty);
        self.content(t, &mut pen)?;
        self.marks(&mut pen);
        let pending = t
            .is_cursor_pending_wrap()
            .map_err(|_| "cursor unreadable")?;
        let (x, y) = (
            t.cursor_x().map_err(|_| "cursor unreadable")?,
            t.cursor_y().map_err(|_| "cursor unreadable")?,
        );
        let origin = t.mode(Mode::ORIGIN).map_err(|_| "modes unreadable")?;
        let lr_mode = t
            .mode(Mode::LEFT_RIGHT_MARGIN)
            .map_err(|_| "modes unreadable")?;
        if self.global {
            if !pending {
                previous_char(t, &mut pen, prev, x, y, self.cols, self.rows)?;
            }
            if lr_mode {
                pen.out.extend_from_slice(b"\x1b[?69h");
            }
            if let Some((seq, _)) = &self.trailer.top_bottom {
                pen.out.extend_from_slice(seq);
            }
            if let Some((seq, _)) = &self.trailer.left_right {
                pen.out.extend_from_slice(seq);
            }
            pen.out.extend_from_slice(&self.trailer.tabs);
        }
        if pending {
            if origin || lr_mode {
                return Err("pending wrap inside margins");
            }
            let at = match read_cell(t, x, y)? {
                Kind::Tail if x > 0 => x - 1,
                _ => x,
            };
            let base = reprint(t, &mut pen, at, y)?;
            if self.global && prev != Some(base) {
                return Err("pending wrap after another character");
            }
        }
        cursor_shape_seq(em, &mut pen.out)?;
        if self.global {
            for &(value, ansi, default) in MODES {
                if set_elsewhere(value, ansi) {
                    continue;
                }
                let on = t.mode(mode(value, ansi)).map_err(|_| "modes unreadable")?;
                if on != default {
                    let q = if ansi { "" } else { "?" };
                    write_bytes(
                        &mut pen.out,
                        format_args!("\x1b[{q}{value}{}", if on { 'h' } else { 'l' }),
                    );
                }
            }
            if origin {
                pen.out.extend_from_slice(b"\x1b[?6h");
            }
        }
        if !pending {
            let (mut cx, mut cy) = (x, y);
            if origin {
                let top = self.trailer.top_bottom.as_ref().map_or(1, |r| r.1);
                let left = self.trailer.left_right.as_ref().map_or(1, |r| r.1);
                cy = cy
                    .checked_sub(top.saturating_sub(1))
                    .ok_or("cursor outside the origin region")?;
                cx = cx
                    .checked_sub(left.saturating_sub(1))
                    .ok_or("cursor outside the origin region")?;
            }
            cup(&mut pen.out, cx, cy);
        }
        self.semantic(&mut pen, &model, x, y, pending, origin)?;
        let style = t.cursor_style().map_err(|_| "cursor unreadable")?;
        pen.style = None;
        pen.style(&style);
        if let Some(link) = &self.trailer.hyperlink {
            pen.out.extend_from_slice(link);
        } else if pen.link.is_some() {
            pen.link(None);
        }
        if self.trailer.protected != model.protected {
            return Err("protection model");
        }
        match model.protect {
            Protect::Off => {}
            Protect::Iso => pen.out.extend_from_slice(b"\x1bV\x1bW"),
            Protect::Dec => pen.out.extend_from_slice(b"\x1b[1\"q\x1b[0\"q"),
        }
        if model.protected {
            pen.out.extend_from_slice(match model.protect {
                Protect::Iso => b"\x1bV".as_slice(),
                _ => b"\x1b[1\"q",
            });
        }
        charsets(&mut pen.out, charset, &model.charset)?;
        if self.global {
            if let Some(seq) = &self.trailer.keyboard {
                pen.out.extend_from_slice(seq);
            }
            if let Some(seq) = &self.trailer.pwd {
                pen.out.extend_from_slice(seq);
            }
            let title = t.title().map_err(|_| "title unreadable")?;
            if !title.is_empty() {
                write_bytes(&mut pen.out, format_args!("\x1b]2;{title}\x1b\\"));
            }
        }
        if prev.is_none() && self.global && pen.printed {
            return Err("nothing printed yet");
        }
        Ok(pen.out)
    }

    /// Sets a saved cursor with the sequences that saved it, starting from
    /// `charset`; returns the character sets left behind.
    fn saved_cursor(
        &self,
        pen: &mut Pen,
        sc: &Saved,
        from: Charset,
    ) -> std::result::Result<Charset, Uncut> {
        let initial = Saved {
            x: 0,
            y: 0,
            style: Style::default(),
            protected: false,
            pending_wrap: false,
            origin: false,
            charset: Charset::default(),
        };
        if *sc == initial {
            return Ok(from);
        }
        if sc.charset.single_shift.is_some() {
            return Err("saved single shift");
        }
        charsets(&mut pen.out, from, &sc.charset)?;
        pen.style = None;
        pen.style(&sc.style);
        if sc.protected {
            pen.out.extend_from_slice(b"\x1b[1\"q");
        }
        if sc.origin {
            pen.out.extend_from_slice(b"\x1b[?6h");
        }
        cup(&mut pen.out, sc.x, sc.y);
        if sc.pending_wrap {
            if sc.x + 1 != self.cols {
                return Err("saved pending wrap");
            }
            pen.out.push(b'x');
            pen.printed = true;
        }
        pen.out.extend_from_slice(b"\x1b7");
        if sc.pending_wrap {
            pen.out.extend_from_slice(b"\x1b[0m");
            cup(&mut pen.out, sc.x, sc.y);
            pen.out.extend_from_slice(b"\x1b[X");
        }
        if sc.origin {
            pen.out.extend_from_slice(b"\x1b[?6l");
        }
        if sc.protected {
            pen.out.extend_from_slice(b"\x1b[0\"q");
        }
        pen.style = None;
        pen.protected = Some(false);
        pen.neutral();
        Ok(sc.charset)
    }

    /// Draws every row, history first, with the rows' wrap flags.
    ///
    /// Ghostty keeps a row's wrap flags when a scroll reuses it, so without
    /// history the flags need not pair up the way wrapping makes them: a row
    /// can continue one that does not wrap, and the last row can wrap into
    /// the first. They still pair up around the screen as a ring, since a
    /// scroll moves the top row to the bottom whole. So the screen is drawn
    /// as a scroll does it: every row at the bottom, starting from a row of
    /// its own that ends up reused for the last one, and a row that continues
    /// one that does not wrap is linked to it beforehand, then unwrapped
    /// while it is the bottom row, which has no row below to unlink.
    fn content(&self, t: &Term, pen: &mut Pen) -> std::result::Result<(), Uncut> {
        let (cols, rows) = (self.cols, self.rows);
        let info = &self.rows_info;
        let total = info.len();
        // A row's mark can only be set while the row is on the screen.
        if info[..self.history]
            .iter()
            .any(|r| r.prompt != RowSemanticPrompt::None)
        {
            return Err("prompt marks in history");
        }
        let ring = !self.keeps_history;
        // Rows that wrap only to link the row after them.
        let mut unwrap = vec![false; total];
        if ring {
            for (k, (row, un)) in info.iter().zip(unwrap.iter_mut()).enumerate() {
                let next = &info[(k + 1) % total];
                if row.wrap && !next.cont {
                    return Err("wrap flags");
                }
                *un = !row.wrap && next.cont;
            }
            // The rows are drawn on the rows they end up on, so the link is
            // made on a blank screen; drawing replaces what it prints.
            for (k, _) in unwrap[..total - 1].iter().enumerate().filter(|(_, &u)| u) {
                cup(&mut pen.out, cols - 1, k as u16);
                pen.neutral();
                pen.text(&['x', 'x']);
            }
            cup(&mut pen.out, 0, rows - 1);
            if info[0].cont {
                // The row before the first: it wraps into it, and its row
                // is reused for the last one.
                pen.style(&Style::default());
                pen.text(&vec![' '; usize::from(cols)]);
            }
        } else {
            for k in 1..total {
                if info[k - 1].wrap != info[k].cont {
                    return Err("wrap flags");
                }
            }
            if info[total - 1].wrap {
                return Err("last row wraps");
            }
            if info[0].cont {
                // The row it continues is gone; it cannot be drawn without
                // landing in history.
                return Err("history starts mid-line");
            }
            cup(&mut pen.out, 0, 0);
        }
        // Empty runs a wrap passes through are drawn as spaces, so that the
        // wrap happens, and emptied once everything is drawn, by sequences
        // that leave wraps alone (ECH and EL 0 would undo them).
        let mut fixes = Vec::new();
        // Whether the row before ended in a wide character's spacer, so the
        // wide character itself wraps.
        let mut after_head = false;
        for (sy, row) in info.iter().enumerate() {
            let at = |x: u16| Point::Screen(PointCoordinate { x, y: sy as u32 });
            if ring || sy > 0 {
                let wraps_in = if sy == 0 { row.cont } else { info[sy - 1].wrap };
                if !wraps_in {
                    pen.style(&Style::default());
                    pen.out.extend_from_slice(b"\r\n");
                } else if !after_head {
                    // A blank wrap: the pen's background would color the
                    // new row a scroll brings in.
                    pen.neutral();
                    pen.text(&[' ']);
                    pen.out.push(b'\r');
                }
            }
            // A scroll brings in the row a wrap lands on.
            let scrolls = ring || sy >= usize::from(rows);
            let mut unwrap_here = unwrap[sy];
            if unwrap_here && !after_head {
                pen.style(&Style::default());
                pen.out.extend_from_slice(b"\x1b[X");
                unwrap_here = false;
            }
            let in_history = sy < self.history;
            // Where the row sits once everything is drawn.
            let final_y = sy.checked_sub(self.history).map(|y| y as u16);
            let mut x = 0u16;
            let mut skip = 0u16;
            let mut head = false;
            while x < cols {
                if unwrap_here && x > 0 {
                    // The wide character that wrapped is drawn; the cell
                    // after it is not yet.
                    pen.style(&Style::default());
                    pen.out.extend_from_slice(b"\x1b[X");
                    unwrap_here = false;
                }
                let style = match read_at(t, at(x))? {
                    Kind::Blank => Style::default(),
                    Kind::Bg(s) => s,
                    Kind::Tail => return Err("spacer without a wide character"),
                    Kind::Head => {
                        let below = Point::Screen(PointCoordinate {
                            x: 0,
                            y: sy as u32 + 1,
                        });
                        let next_wide = sy + 1 < total
                            && matches!(read_at(t, below)?, Kind::Text(Cell { wide: true, .. }));
                        if x + 1 != cols || !row.wrap || !next_wide {
                            return Err("stray spacer head");
                        }
                        // Printing the wide character at the last column
                        // leaves this head and wraps.
                        flush_skip(pen, &mut skip);
                        head = true;
                        x += 1;
                        continue;
                    }
                    Kind::Text(cell) => {
                        flush_skip(pen, &mut skip);
                        if cell.wide && x + 1 >= cols {
                            return Err("wide character at the edge");
                        }
                        if in_history && cell.sem != Sem::Output {
                            return Err("prompt marks in history");
                        }
                        if after_head && scrolls && cell.style.bg_color != StyleColor::None {
                            return Err("colored wrap at the bottom");
                        }
                        paint(pen, &cell);
                        x += if cell.wide { 2 } else { 1 };
                        if cell.wide && !matches!(read_at(t, at(x - 1))?, Kind::Tail) {
                            return Err("wide character without spacer");
                        }
                        continue;
                    }
                };
                let mut n = 1;
                while x + n < cols {
                    let same = match read_at(t, at(x + n))? {
                        Kind::Blank => style == Style::default(),
                        Kind::Bg(s) => s == style,
                        _ => false,
                    };
                    if !same {
                        break;
                    }
                    n += 1;
                }
                let leading = x == 0 && row.cont;
                let trailing = x + n == cols && row.wrap;
                if leading || trailing {
                    let y = final_y.ok_or("empty cells wrap in history")?;
                    flush_skip(pen, &mut skip);
                    pen.neutral();
                    pen.text(&vec![' '; usize::from(n)]);
                    fixes.push(if leading {
                        Fix::EraseLeft {
                            x: x + n - 1,
                            y,
                            style,
                        }
                    } else {
                        Fix::InsertBlanks { x, y, n, style }
                    });
                } else if style == Style::default() {
                    skip += n;
                } else {
                    flush_skip(pen, &mut skip);
                    pen.link(None);
                    pen.protect(false);
                    pen.style(&style);
                    csi_n(&mut pen.out, n, b'X');
                    if x + n < cols {
                        csi_n(&mut pen.out, n, b'C');
                    }
                }
                x += n;
            }
            if unwrap_here {
                return Err("wrap flags");
            }
            after_head = head;
        }
        for fix in fixes {
            let (x, y, style) = match fix {
                Fix::EraseLeft { x, y, style } | Fix::InsertBlanks { x, y, style, .. } => {
                    (x, y, style)
                }
            };
            cup(&mut pen.out, x, y);
            pen.link(None);
            pen.protect(false);
            pen.style(&style);
            match fix {
                Fix::EraseLeft { .. } => pen.out.extend_from_slice(b"\x1b[1K"),
                Fix::InsertBlanks { n, .. } => csi_n(&mut pen.out, n, b'@'),
            }
        }
        pen.neutral();
        Ok(())
    }

    /// Sets each row's OSC 133 mark, which drawing may have disturbed.
    fn marks(&self, pen: &mut Pen) {
        let screen = &self.rows_info[self.history..];
        let any = screen.iter().any(|r| r.prompt != RowSemanticPrompt::None);
        if !any && !pen.marked {
            return;
        }
        for (y, r) in screen.iter().enumerate() {
            cup(&mut pen.out, 0, y as u16);
            let (seq, sem) = match r.prompt {
                RowSemanticPrompt::None => (b"\x1b]133;C\x1b\\".as_slice(), Sem::Output),
                RowSemanticPrompt::Prompt => (b"\x1b]133;P;k=i\x1b\\".as_slice(), Sem::Prompt),
                RowSemanticPrompt::Continuation => {
                    (b"\x1b]133;P;k=c\x1b\\".as_slice(), Sem::Prompt)
                }
            };
            pen.out.extend_from_slice(seq);
            pen.sem = Some(sem);
        }
    }

    /// Leaves the cursor writing what the model says it writes, without
    /// disturbing a row's mark.
    fn semantic(
        &self,
        pen: &mut Pen,
        model: &crate::emulator::ScreenModel,
        x: u16,
        y: u16,
        pending: bool,
        origin: bool,
    ) -> std::result::Result<(), Uncut> {
        let s = model.semantic;
        let seq: &[u8] = match (s.content, s.clear_eol) {
            (Sem::Input, false) => b"\x1b]133;B\x1b\\",
            (Sem::Input, true) => b"\x1b]133;I\x1b\\",
            (Sem::Prompt, _) => match self.rows_info[self.history + usize::from(y)].prompt {
                RowSemanticPrompt::Prompt => b"\x1b]133;P;k=i\x1b\\",
                RowSemanticPrompt::Continuation => b"\x1b]133;P;k=c\x1b\\",
                RowSemanticPrompt::None => return Err("prompt on an unmarked row"),
            },
            (Sem::Output, _) => {
                if pen.sem == Some(Sem::Output) {
                    return Ok(());
                }
                let marked =
                    self.rows_info[self.history + usize::from(y)].prompt != RowSemanticPrompt::None;
                if x == 0 && marked {
                    // OSC 133;C at column 0 would clear the row's mark.
                    if pending || origin || self.cols < 2 {
                        return Err("output cursor on a marked row");
                    }
                    cup(&mut pen.out, 1, y);
                    pen.out.extend_from_slice(b"\x1b]133;C\x1b\\");
                    cup(&mut pen.out, x, y);
                    return Ok(());
                }
                b"\x1b]133;C\x1b\\"
            }
        };
        pen.out.extend_from_slice(seq);
        Ok(())
    }
}

/// A run of empty cells drawn as spaces, emptied after drawing.
#[derive(Clone, Copy)]
enum Fix {
    /// EL 1 at the run's last cell: the run starts the row.
    EraseLeft { x: u16, y: u16, style: Style },
    /// ICH at the run's first cell: the run ends the row.
    InsertBlanks {
        x: u16,
        y: u16,
        n: u16,
        style: Style,
    },
}

/// Moves over the empty cells passed so far.
fn flush_skip(pen: &mut Pen, skip: &mut u16) {
    if *skip > 0 {
        csi_n(&mut pen.out, *skip, b'C');
        *skip = 0;
    }
}

fn csi_n(out: &mut Vec<u8>, n: u16, fin: u8) {
    write_bytes(out, format_args!("\x1b[{n}{}", fin as char));
}

fn paint(pen: &mut Pen, cell: &Cell) {
    pen.link(cell.link.clone());
    pen.protect(cell.protected);
    pen.sem(cell.sem);
    pen.style(&cell.style);
    pen.text(&cell.cps);
}

/// Prints the cell at (`x`, `y`) again, as it is; returns its base codepoint.
fn reprint(t: &Term, pen: &mut Pen, x: u16, y: u16) -> std::result::Result<u32, Uncut> {
    match read_cell(t, x, y)? {
        Kind::Text(cell) => {
            cup(&mut pen.out, x, y);
            paint(pen, &cell);
            Ok(cell.cps.first().map_or(0, |c| *c as u32))
        }
        _ => Err("pending wrap on an empty cell"),
    }
}

/// Makes `prev` the last character printed: by printing a cell that holds
/// it again, or by printing it on an empty cell and erasing it.
fn previous_char(
    t: &Term,
    pen: &mut Pen,
    prev: Option<u32>,
    cx: u16,
    cy: u16,
    cols: u16,
    rows: u16,
) -> std::result::Result<(), Uncut> {
    let Some(cp) = prev else {
        return Ok(());
    };
    let ch = char::from_u32(cp).ok_or("previous character invalid")?;
    // Prefer the cell left of the cursor, which usually holds it.
    let mut order = Vec::with_capacity(usize::from(cols) * usize::from(rows));
    if cx > 0 {
        order.push((cx - 1, cy));
    }
    for y in 0..rows {
        for x in 0..cols {
            order.push((x, y));
        }
    }
    let mut blank = None;
    for &(x, y) in &order {
        match read_cell(t, x, y)? {
            Kind::Text(cell) if cell.cps == [ch] => {
                reprint(t, pen, x, y)?;
                return Ok(());
            }
            Kind::Blank if blank.is_none() => blank = Some((x, y)),
            _ => {}
        }
    }
    let (x, y) = blank.ok_or("previous character not on screen")?;
    let wide = !matches!(cp, 0..=0xff) && {
        // A wide character needs the blank cell after it too.
        x + 1 < cols && matches!(read_cell(t, x + 1, y)?, Kind::Blank)
    };
    cup(&mut pen.out, x, y);
    pen.neutral();
    pen.text(&[ch]);
    cup(&mut pen.out, x, y);
    csi_n(&mut pen.out, if wide { 2 } else { 1 }, b'X');
    Ok(())
}

fn invoke_gl(out: &mut Vec<u8>, slot: u8) {
    out.extend_from_slice(match slot {
        0 => b"\x0f".as_slice(),
        1 => b"\x0e",
        2 => b"\x1bn",
        _ => b"\x1bo",
    });
}

/// Moves the character sets from `from` to `to`.
fn charsets(out: &mut Vec<u8>, from: Charset, to: &Charset) -> std::result::Result<(), Uncut> {
    for slot in 0..4 {
        let (a, b) = (from.g[slot], to.g[slot]);
        if a != b {
            let fin = b
                .final_byte()
                .ok_or("character set cannot return to UTF-8")?;
            out.extend_from_slice(&[0x1b, b"()*+"[slot], fin]);
        }
    }
    if from.gl != to.gl {
        invoke_gl(out, to.gl);
    }
    if from.gr != to.gr {
        out.extend_from_slice(match to.gr {
            1 => b"\x1b~".as_slice(),
            2 => b"\x1b}",
            3 => b"\x1b|",
            _ => return Err("GR invokes G0"),
        });
    }
    if from.single_shift != to.single_shift {
        match to.single_shift {
            Some(2) => out.extend_from_slice(b"\x1bN"),
            Some(3) => out.extend_from_slice(b"\x1bO"),
            _ => return Err("single shift"),
        }
    }
    Ok(())
}

/// Pushes the Kitty keyboard stack from empty: every slot in ring order,
/// ending with the cursor where it was.
fn kitty(out: &mut Vec<u8>, k: &Kitty) {
    if *k == Kitty::default() {
        return;
    }
    for i in 1..=(8 + usize::from(k.idx)) {
        write_bytes(out, format_args!("\x1b[>{}u", k.flags[i % 8]));
    }
}

fn cursor_shape_seq(em: &Emulator, out: &mut Vec<u8>) -> std::result::Result<(), Uncut> {
    let (shape, _) = cursor_shape(em).ok_or("cursor shape unreadable")?;
    match shape {
        CursorVisualStyle::Bar => out.extend_from_slice(b"\x1b[6 q"),
        CursorVisualStyle::Underline => out.extend_from_slice(b"\x1b[4 q"),
        _ => {}
    }
    Ok(())
}
