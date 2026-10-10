//! Checkpoints of an [`Emulator`]: Ghostty's terminal snapshot, unfinished sequence included, and the labels.

use std::fmt::Write;

use libghostty_vt::fmt::{Format, Formatter, FormatterOptions};
use libghostty_vt::render::{CursorVisualStyle, RenderState};
use libghostty_vt::screen::{Cell, CellContentTag, GridRef};
use libghostty_vt::snapshot::Decoder;
use libghostty_vt::style::Style;
use libghostty_vt::terminal::{Mode, ModeKind, Point, PointCoordinate, Terminal};

use crate::emulator::{Emulator, MAX_CONTINUATION};
use crate::Result;

/// The terminal a checkpoint reads.
type Term = Terminal<'static, 'static>;

/// Why no checkpoint was cut here, as a fixed phrase a caller can count.
pub type Uncut = &'static str;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    /// Ghostty's snapshot of the terminal.
    pub snapshot: Vec<u8>,
    pub title: String,
    pub cwd: String,
    /// CRC-32 and length of the fingerprint of the terminal it was cut from.
    pub digest: (u32, u32),
}

/// Ghostty's modes, with whether each is ANSI.
const MODES: &[(u16, bool)] = &[
    (2, true),
    (4, true),
    (12, true),
    (20, true),
    (1, false),
    (3, false),
    (4, false),
    (5, false),
    (6, false),
    (7, false),
    (8, false),
    (9, false),
    (12, false),
    (25, false),
    (40, false),
    (45, false),
    (47, false),
    (66, false),
    (67, false),
    (69, false),
    (1000, false),
    (1002, false),
    (1003, false),
    (1004, false),
    (1005, false),
    (1006, false),
    (1007, false),
    (1015, false),
    (1016, false),
    (1035, false),
    (1036, false),
    (1039, false),
    (1045, false),
    (1047, false),
    (1048, false),
    (1049, false),
    (2004, false),
    (2026, false),
    (2027, false),
    (2031, false),
    (2048, false),
];

impl Emulator {
    /// Why no checkpoint can be cut here: only inside a sequence longer than one carries.
    pub fn uncuttable(&self) -> Option<Uncut> {
        match self.term.continuation_buf(&mut []) {
            Ok(_) | Err(libghostty_vt::Error::OutOfSpace { .. }) => None,
            Err(_) => Some("sequence too long to carry"),
        }
    }

    /// Cuts a checkpoint and carries on from its decode, page memory and all, as a recovery would.
    pub fn checkpoint(&mut self) -> std::result::Result<Checkpoint, Uncut> {
        let (cp, mut rebuilt) = self.cut()?;
        rebuilt.carry_counters(self);
        *self = rebuilt;
        Ok(cp)
    }

    /// Cuts a checkpoint that passed its restore check, with its decode, leaving this terminal as it is.
    pub fn cut(&mut self) -> std::result::Result<(Checkpoint, Emulator), Uncut> {
        if let Some(why) = self.uncuttable() {
            return Err(why);
        }
        let mut snapshot = Vec::new();
        self.term
            .encode_snapshot(&mut snapshot)
            .map_err(|_| "snapshot failed")?;
        let mut cp = Checkpoint {
            snapshot,
            title: self.title.clone(),
            cwd: self.cwd.clone(),
            digest: (0, 0),
        };
        let rebuilt = Emulator::restore(&cp).map_err(|_| "rebuild failed")?;
        let fp = rebuilt.digests();
        if fp != self.digests() {
            return Err("restore check");
        }
        cp.digest = fp.check();
        Ok((cp, rebuilt))
    }

    /// Decodes a checkpoint's terminal; [`Checkpoint::matches`] says whether it is the one cut.
    pub fn restore(cp: &Checkpoint) -> Result<Emulator> {
        let mut decoder = Decoder::new_buf(&cp.snapshot)?;
        decoder
            .set_max_continuation_bytes(MAX_CONTINUATION)?
            .set_retain_continuation(true)?;
        let term = decoder.decode()?;
        let continuation = term.continuation_alloc(None)?;
        let mut em = Emulator::wrap(term)?;
        em.scan.reseed(continuation.as_deref().unwrap_or_default());
        em.title.clone_from(&cp.title);
        em.cwd.clone_from(&cp.cwd);
        Ok(em)
    }

    /// What a restore must reproduce that Ghostty shows, cell by cell, with the open sequence and labels.
    pub fn fingerprint(&self) -> String {
        let mut fp = String::new();
        self.write_fingerprint(&mut fp);
        fp
    }

    /// The fingerprint's three digests, hashed as it is written rather than held: on deep
    /// history it runs to hundreds of bytes a cell.
    fn digests(&self) -> Digests {
        let mut d = Digests::default();
        self.write_fingerprint(&mut d);
        d
    }

    /// Writes [`Emulator::fingerprint`] to `fp`.
    fn write_fingerprint(&self, fp: &mut impl Write) {
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
        let mut cells = Cells::default();
        let _ = writeln!(fp, "history {history}");
        for y in 0..history + usize::from(self.rows) {
            let y = u32::try_from(y).unwrap_or(u32::MAX);
            let at = |x: u16| Point::Screen(PointCoordinate { x, y });
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
                let _ = fp.write_char(' ');
                cells.write(fp, t, at(x));
                let _ = fp.write_char('\n');
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
        let _ = writeln!(fp, "shape {:?}", cursor_shape(t));
        for &(value, ansi) in MODES {
            let kind = if ansi { ModeKind::Ansi } else { ModeKind::Dec };
            let _ = write!(
                fp,
                "{value}{}={:?} ",
                if ansi { "a" } else { "" },
                t.mode(Mode::new(value, kind))
            );
        }
        let pending = t.continuation_alloc(None);
        let _ = writeln!(
            fp,
            "\ncontinuation {:?}",
            pending.as_ref().map(|b| b.as_deref().unwrap_or_default())
        );
        let _ = writeln!(fp, "labels {:?} {:?}", self.title, self.cwd);
    }

    /// FNV-1a of [`Emulator::fingerprint`], stable across runs and platforms for one Ghostty build.
    pub fn state_digest(&self) -> u64 {
        self.digests().fnv
    }
}

impl Checkpoint {
    /// The restore check: whether `rebuilt` is the terminal this was cut from.
    pub fn matches(&self, rebuilt: &Emulator) -> bool {
        rebuilt.digests().check() == self.digest
    }

    /// Bytes for storage. [`Checkpoint::decode`] reads them back.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(24 + self.snapshot.len());
        out.extend_from_slice(&self.digest.0.to_le_bytes());
        out.extend_from_slice(&self.digest.1.to_le_bytes());
        put_bytes(&mut out, self.title.as_bytes());
        put_bytes(&mut out, self.cwd.as_bytes());
        put_bytes(&mut out, &self.snapshot);
        out
    }

    /// `None` for anything [`Checkpoint::encode`] could not have written.
    pub fn decode(bytes: &[u8]) -> Option<Checkpoint> {
        let mut r = Reader(bytes);
        let digest = (r.u32()?, r.u32()?);
        let title = String::from_utf8(r.bytes()?.to_vec()).ok()?;
        let cwd = String::from_utf8(r.bytes()?.to_vec()).ok()?;
        let snapshot = r.bytes()?.to_vec();
        if !r.0.is_empty() {
            return None;
        }
        Some(Checkpoint {
            snapshot,
            title,
            cwd,
            digest,
        })
    }
}

fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    let len = u32::try_from(b.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&len.to_le_bytes());
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
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = usize::try_from(self.u32()?).ok()?;
        self.take(n)
    }
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

#[cfg(test)]
fn fnv1a64(bytes: &[u8]) -> u64 {
    fnv1a64_from(FNV_OFFSET, bytes)
}

fn fnv1a64_from(h: u64, bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(h, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// CRC-32, length and FNV-1a of a fingerprint, taken as it is written.
#[derive(Clone)]
struct Digests {
    crc: crc32fast::Hasher,
    len: usize,
    fnv: u64,
}

impl Default for Digests {
    fn default() -> Self {
        Digests {
            crc: crc32fast::Hasher::new(),
            len: 0,
            fnv: FNV_OFFSET,
        }
    }
}

impl PartialEq for Digests {
    fn eq(&self, other: &Self) -> bool {
        (self.check(), self.fnv) == (other.check(), other.fnv)
    }
}

impl Digests {
    /// What a checkpoint keeps: CRC-32 and length.
    fn check(&self) -> (u32, u32) {
        let len = u32::try_from(self.len).unwrap_or(u32::MAX);
        (self.crc.clone().finalize(), len)
    }
}

impl Write for Digests {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.crc.update(s.as_bytes());
        self.len += s.len();
        self.fnv = fnv1a64_from(self.fnv, s.as_bytes());
        Ok(())
    }
}

/// Writes cells for the fingerprint, reusing what the last cell and style wrote: most cells
/// repeat their neighbour's (blanks) or style, and formatting dominates the walk.
#[derive(Default)]
struct Cells {
    /// The last cell whose text is all in its value and style, and what it wrote.
    last: Option<(Cell, Style)>,
    text: String,
    /// The last style and its text.
    style: Option<(Style, String)>,
    chars: Vec<char>,
    uri: Vec<u8>,
}

impl Cells {
    /// One cell, every attribute.
    fn write(&mut self, fp: &mut impl Write, t: &Term, at: Point) {
        let Ok(g) = t.grid_ref(at) else {
            let _ = fp.write_str("unreadable");
            return;
        };
        let Ok(c) = g.cell() else {
            let _ = fp.write_str("unreadable");
            return;
        };
        let tag = c.content_tag();
        let link = c.has_hyperlink().unwrap_or(false);
        let style = g.style();
        // Graphemes beyond the first codepoint and links live outside the cell's value.
        let key = match style {
            Ok(s) if !link && !matches!(tag, Ok(CellContentTag::CodepointGrapheme)) => Some((c, s)),
            _ => None,
        };
        if key.is_some() && key == self.last {
            let _ = fp.write_str(&self.text);
            return;
        }
        self.text.clear();
        let out = &mut self.text;
        let _ = write!(out, "{tag:?} ");
        let chars = graphemes(&g, &mut self.chars);
        let _ = write!(out, "{chars:?} {:?} ", c.wide());
        match (style, &self.style) {
            (Ok(s), Some((cached, text))) if s == *cached => out.push_str(text),
            (Ok(s), _) => {
                let start = out.len();
                let _ = write!(out, "{style:?}");
                self.style = Some((s, out[start..].to_owned()));
            }
            (Err(_), _) => {
                let _ = write!(out, "{style:?}");
            }
        }
        let _ = write!(
            out,
            " prot {:?} sem {:?}",
            c.is_protected(),
            c.semantic_content()
        );
        if matches!(tag, Ok(CellContentTag::BgColorPalette)) {
            let _ = write!(out, " bg {:?}", c.bg_color_palette());
        }
        if matches!(tag, Ok(CellContentTag::BgColorRgb)) {
            let _ = write!(out, " bg {:?}", c.bg_color_rgb());
        }
        if link {
            let _ = write!(out, " link {:?}", hyperlink(&g, &mut self.uri));
        }
        let _ = fp.write_str(&self.text);
        self.last = key;
    }
}

fn graphemes<'b>(g: &GridRef<'_>, buf: &'b mut Vec<char>) -> &'b [char] {
    buf.resize(buf.len().max(16), '\0');
    loop {
        match g.graphemes(buf) {
            Ok(n) => return &buf[..n],
            Err(libghostty_vt::Error::OutOfSpace { required }) if required > buf.len() => {
                buf.resize(required, '\0')
            }
            Err(_) => return &[],
        }
    }
}

fn hyperlink<'b>(g: &GridRef<'_>, buf: &'b mut Vec<u8>) -> Option<&'b [u8]> {
    buf.resize(buf.len().max(256), 0);
    loop {
        match g.hyperlink_uri(buf) {
            Ok(n) => return Some(&buf[..n]),
            Err(libghostty_vt::Error::OutOfSpace { required }) if required > buf.len() => {
                buf.resize(required, 0)
            }
            Err(_) => return None,
        }
    }
}

fn cursor_shape(t: &Term) -> Option<(CursorVisualStyle, bool)> {
    let mut rs = RenderState::new().ok()?;
    let snap = rs.update(t).ok()?;
    Some((
        snap.cursor_visual_style().ok()?,
        snap.cursor_blinking().ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fingerprint as it was first written, one `String` a cell: what the stored digests hash.
    fn reference(em: &Emulator) -> String {
        let this = em;
        let mut fp = String::new();
        let t = &this.term;
        let _ = writeln!(
            fp,
            "size {}x{} screen {:?}",
            this.cols,
            this.rows,
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
        for y in 0..history + usize::from(this.rows) {
            let y = u32::try_from(y).unwrap_or(u32::MAX);
            let at = |x: u16| Point::Screen(PointCoordinate { x, y });
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
            for x in 0..this.cols {
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
        let _ = writeln!(fp, "shape {:?}", cursor_shape(t));
        for &(value, ansi) in MODES {
            let kind = if ansi { ModeKind::Ansi } else { ModeKind::Dec };
            let _ = write!(
                fp,
                "{value}{}={:?} ",
                if ansi { "a" } else { "" },
                t.mode(Mode::new(value, kind))
            );
        }
        let pending = t.continuation_alloc(None);
        let _ = writeln!(
            fp,
            "\ncontinuation {:?}",
            pending.as_ref().map(|b| b.as_deref().unwrap_or_default())
        );
        let _ = writeln!(fp, "labels {:?} {:?}", this.title, this.cwd);
        fp
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
            ref_graphemes(&g),
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
            let _ = write!(s, " link {:?}", ref_hyperlink(&g));
        }
        s
    }

    fn ref_graphemes(g: &GridRef<'_>) -> Vec<char> {
        let mut buf = vec!['\0'; 16];
        loop {
            match g.graphemes(&mut buf) {
                Ok(n) => {
                    buf.truncate(n);
                    return buf;
                }
                Err(libghostty_vt::Error::OutOfSpace { required }) if required > buf.len() => {
                    buf.resize(required, '\0')
                }
                Err(_) => return Vec::new(),
            }
        }
    }

    fn ref_hyperlink(g: &GridRef<'_>) -> Option<Vec<u8>> {
        let mut buf = vec![0; 256];
        loop {
            match g.hyperlink_uri(&mut buf) {
                Ok(n) => {
                    buf.truncate(n);
                    return Some(buf);
                }
                Err(libghostty_vt::Error::OutOfSpace { required }) if required > buf.len() => {
                    buf.resize(required, 0)
                }
                Err(_) => return None,
            }
        }
    }

    /// The fingerprint and its digests are those of the first implementation, byte for byte,
    /// over every kind of cell, so digests kept in checkpoints still match.
    #[test]
    fn the_fingerprint_is_unchanged() {
        let mut em = Emulator::with_scrollback(30, 5, 1 << 20).unwrap();
        let mut fx = Vec::new();
        em.feed(
            "\x1b]133;A\x07$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07\x1b[1;3;4:3;58;5;9;38;2;1;2;3mstyled\x1b[m \
             plain \x1b]8;;https://x.test/\x07link\x1b]8;;\x07\r\n日本 e\u{301} 👩\u{200d}💻 \x1b[44m\x1b[K\r\n\
             \x1b[48;2;9;8;7m\x1b[2K\x1b[m\x1b[1\"qguarded\x1b[0\"q"
                .as_bytes(),
            &mut fx,
        );
        for i in 0..40 {
            em.feed(
                format!("line {i} \x1b[3{}mcolour\x1b[m\r\n", i % 8).as_bytes(),
                &mut fx,
            );
        }
        em.feed(
            b"\x1b[?1049halt\x1b[?1049l\x1b[2;3H\x1b[7minverse\x1b[?2004h\x1b[3",
            &mut fx,
        );
        let want = reference(&em);
        for kind in [
            "link Some(",
            "CodepointGrapheme",
            "prot Ok(true)",
            " bg Ok(",
            "row 40 ",
        ] {
            assert!(want.contains(kind), "covers {kind}");
        }
        assert_eq!(em.fingerprint(), want);
        let d = em.digests();
        let len = u32::try_from(want.len()).unwrap();
        assert_eq!(d.check(), (crc32fast::hash(want.as_bytes()), len));
        assert_eq!(em.state_digest(), fnv1a64(want.as_bytes()));
    }

    #[test]
    fn fnv1a64_matches_the_reference_values() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
    }

    /// The digest follows the state, not how the bytes that made it were cut.
    #[test]
    fn the_state_digest_follows_the_state() {
        let mut fx = Vec::new();
        let mut a = Emulator::new(40, 6).unwrap();
        a.feed(b"\x1b[1;31mred\x1b[0m line\r\nnext \xc3\xa9", &mut fx);
        let mut b = Emulator::new(40, 6).unwrap();
        for chunk in [
            &b"\x1b[1;3"[..],
            b"1mred\x1b[0m li",
            b"ne\r\nnext \xc3",
            b"\xa9",
        ] {
            b.feed(chunk, &mut fx);
        }
        assert_eq!(a.state_digest(), b.state_digest());
        assert_eq!(a.state_digest(), a.state_digest(), "stable");
        b.feed(b"\x1b[2;3H", &mut fx);
        assert_ne!(a.state_digest(), b.state_digest(), "the cursor moved");
        let blank = Emulator::new(40, 6).unwrap();
        assert_ne!(a.state_digest(), blank.state_digest());
        b.feed(b"\x1b[3", &mut fx);
        let mut c = Emulator::new(40, 6).unwrap();
        c.feed(
            b"\x1b[1;31mred\x1b[0m line\r\nnext \xc3\xa9\x1b[2;3H",
            &mut fx,
        );
        assert_ne!(b.state_digest(), c.state_digest(), "inside a sequence");
    }

    #[test]
    fn malformed_checkpoints_are_refused() {
        let mut em = Emulator::new(20, 4).unwrap();
        em.feed(b"hello\x1b]2;ti", &mut Vec::new());
        let cp = em.checkpoint().unwrap();
        let bytes = cp.encode();
        assert_eq!(Checkpoint::decode(&bytes), Some(cp.clone()));
        for n in 0..bytes.len() {
            assert!(Checkpoint::decode(&bytes[..n]).is_none(), "{n}");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(Checkpoint::decode(&longer).is_none());
        let mut torn = cp.clone();
        torn.snapshot.truncate(torn.snapshot.len() / 2);
        assert!(Emulator::restore(&torn).is_err());
    }
}
