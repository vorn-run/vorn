//! What a bytes client is given to start from: Terminal State Protocol §7
//! VtSnapshot, cut inside the session actor and stamped with its cursor.
//!
//! The terminal is drawn as VT by Ghostty's formatter, so xterm.js (or any
//! other parser) fed it shows the same screen, then applies the Bytes after
//! [`VtSnapshot::resume`]. Like a checkpoint, a snapshot is cut only at a
//! record boundary where the parser is in its ground state with no UTF-8
//! sequence open, because the formatter cannot describe a half-parsed
//! sequence (TP §14); a request that arrives inside one waits for the next
//! boundary, at most [`HOLD`].

use std::time::Duration;

use libghostty_vt::fmt::{Format, Formatter, FormatterOptions};
use vorn_screen::Emulator;
use vorn_term_proto::Cursor;

/// How long a snapshot request may wait for a record boundary where one can
/// be cut. Past it the snapshot is cut where the stream is, as a stream stuck
/// inside an unterminated sequence would otherwise never get one.
pub const HOLD: Duration = Duration::from_secs(1);

/// A session's screen as VT, and where it ends in the record log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VtSnapshot {
    /// The first record and byte the snapshot does not include: the client
    /// applies Bytes from here.
    pub resume: Cursor,
    pub cols: u16,
    pub rows: u16,
    /// Escape sequences that draw the screen and its scrollback into a blank
    /// terminal of `cols` x `rows`.
    pub vt: Vec<u8>,
    pub title: String,
    pub cwd: String,
    /// Cut where the parser was not in its ground state, after [`HOLD`].
    pub forced: bool,
}

/// Cuts a snapshot of `em` at `resume`.
///
/// Every formatter option the protocol lists is on except the palette:
/// Ghostty writes all 256 entries, its defaults included, which would
/// replace the client's own theme. A palette a program set is therefore not
/// carried; the client keeps its colours.
pub(crate) fn cut(em: &Emulator, resume: Cursor, forced: bool) -> VtSnapshot {
    let t = em.terminal();
    let format = |o: FormatterOptions<'_, '_>| {
        Formatter::new(t, o)
            .and_then(|mut f| f.format_alloc(None))
            .map(|b| b.to_vec())
            .unwrap_or_default()
    };
    // The cells alone, and everything: the difference is the state written
    // after them (modes, region, tab stops, cursor...).
    let cells = || {
        FormatterOptions::new()
            .with_format(Format::Vt)
            .with_style(true)
            .with_hyperlink(true)
    };
    // Soft-wrapped rows are joined, so the client wraps them itself and
    // reflows them as the terminal did on a later resize.
    let content = format(cells().with_unwrap(true));
    let full = format(
        cells()
            .with_unwrap(true)
            .with_modes(true)
            .with_scrolling_region(true)
            .with_tabstops(true)
            .with_pwd(true)
            .with_keyboard(true)
            .with_cursor(true)
            .with_protection(true)
            .with_kitty_keyboard(true)
            .with_charsets(true),
    );
    let drawn = 1 + count(&format(cells()), b"\r\n");
    let rows = t.scrollback_rows().unwrap_or(0) + usize::from(em.rows());
    let mut vt = pad_rows(full, &content, rows.saturating_sub(drawn));
    put_cursor_last(&mut vt, t.cursor_x().ok(), t.cursor_y().ok());
    VtSnapshot {
        resume,
        cols: em.cols(),
        rows: em.rows(),
        vt,
        title: em.title().to_owned(),
        cwd: em.cwd().to_owned(),
        forced,
    }
}

/// The formatter leaves out the blank rows after the last one written, so
/// a client fed only what it writes ends with fewer rows than the terminal
/// has, and every row of its screen one or more lines too high. Line feeds
/// after the cells, before the state written after them (a scrolling region
/// would hold the feeds inside it), put the client's last row where the
/// terminal's is.
fn pad_rows(full: Vec<u8>, content: &[u8], blank: usize) -> Vec<u8> {
    if blank == 0 {
        return full;
    }
    let at = match find(&full, content) {
        Some(i) => i + content.len(),
        None => full.len(),
    };
    let mut out = Vec::with_capacity(full.len() + 2 * blank);
    out.extend_from_slice(&full[..at]);
    for _ in 0..blank {
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(&full[at..]);
    out
}

/// The formatter places the cursor and then writes the scrolling region and
/// the tab stops, and both of those move the cursor (DECSTBM homes it, tab
/// stops are set by moving to each column). So the cursor is placed again,
/// last. With origin mode on a CUP is relative to the region, which the
/// formatter's own placement already accounts for, so it is left alone.
fn put_cursor_last(vt: &mut Vec<u8>, x: Option<u16>, y: Option<u16>) {
    let (Some(x), Some(y)) = (x, y) else {
        return;
    };
    if contains(vt, b"\x1b[?6h") {
        return;
    }
    vt.extend_from_slice(format!("\x1b[{};{}H", u32::from(y) + 1, u32::from(x) + 1).as_bytes());
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

fn count(hay: &[u8], needle: &[u8]) -> usize {
    hay.windows(needle.len()).filter(|w| *w == needle).count()
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fed(cols: u32, rows: u32, bytes: &[u8]) -> Emulator {
        let mut em = Emulator::with_scrollback(cols, rows, 1 << 20).unwrap();
        em.feed(bytes, &mut Vec::new());
        em
    }

    /// The text a terminal shows, scrollback included.
    fn text(em: &Emulator) -> String {
        let opts = FormatterOptions::new().with_format(Format::Plain);
        let b = Formatter::new(em.terminal(), opts)
            .and_then(|mut f| f.format_alloc(None))
            .unwrap();
        String::from_utf8(b.to_vec()).unwrap()
    }

    #[test]
    fn blank_rows_under_the_last_line_keep_the_screen_where_it_was() {
        for tail in [&b""[..], b"\r\n", b"\r\n\r\n\r\n", b"\x1b[2;3r\r\n"] {
            let src = fed(10, 4, &[&b"1\r\n2\r\n3\r\n4\r\n5\r\n6"[..], tail].concat());
            let s = cut(&src, Cursor::start(0), false);
            let mut copy = fed(10, 4, &s.vt);
            assert_eq!(text(&copy), text(&src), "{tail:?}");
            let t = (src.terminal(), copy.terminal());
            assert_eq!(
                t.0.scrollback_rows().unwrap(),
                t.1.scrollback_rows().unwrap()
            );
            assert_eq!(t.0.cursor_y().unwrap(), t.1.cursor_y().unwrap(), "{tail:?}");
            // The next line lands on the same row.
            let mut src = src;
            src.feed(b"\r\nnext", &mut Vec::new());
            copy.feed(b"\r\nnext", &mut Vec::new());
            assert_eq!(text(&copy), text(&src), "{tail:?}");
        }
    }

    #[test]
    fn a_blank_terminal_fed_the_snapshot_shows_the_same_screen() {
        let src = fed(
            20,
            4,
            b"\x1b[31mred\x1b[0m\r\nline2\r\nline3\r\nline4\r\nline5\r\n\xe4\xb8\xad\xe6\x96\x87 \xe2\x82\xac\x1b[2;5r\x1b[3;7H",
        );
        let mut src = src;
        src.set_default_colors(Some(([1, 2, 3], [4, 5, 6])))
            .unwrap();
        let s = cut(&src, Cursor::start(0), false);
        assert!(!contains(&s.vt, b"\x1b]4;"), "no palette");
        assert!(!contains(&s.vt, b"\x1b]1"), "no default colours");
        let copy = fed(u32::from(s.cols), u32::from(s.rows), &s.vt);
        assert_eq!(text(&copy), text(&src));
        let t = (src.terminal(), copy.terminal());
        assert_eq!(t.0.cursor_x().unwrap(), t.1.cursor_x().unwrap());
        assert_eq!(t.0.cursor_y().unwrap(), t.1.cursor_y().unwrap());
    }
}
