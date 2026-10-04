//! A terminal's screen, kept in a libghostty-vt terminal: what the server's
//! history checkpoints serialize, and the title and cwd that travel beside it.
//!
//! Plain Rust with no Node in it, so the napi adapter in `vorn-core`, a future
//! daemon and the native UI can all link it, and its tests and benchmarks run
//! as ordinary binaries.
//!
//! Differences from the headless xterm it replaces that callers see: the parse
//! is synchronous, so there is no queue to bound and no drain to wait for, and
//! the serialized screen comes from Ghostty's own VT formatter rather than
//! `@xterm/addon-serialize`. Each accepted difference is named in
//! `tests/helpers/screen-parity.ts` and checked against the recorded JS
//! reference in `tests/js-reference.test.ts`.

pub mod checkpoint;
pub mod emulator;
mod vtparse;

pub use checkpoint::{Checkpoint, Step, Uncut};
pub use emulator::{ClipboardTarget, Effect, Emulator};

use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use libghostty_vt::fmt::{Format, Formatter, FormatterOptions};
use libghostty_vt::terminal::{Options, Terminal};
use memchr::memchr;

/// How much of a title or a cwd is kept, in UTF-16 units as the JS model counts.
pub const MAX_LABEL_UNITS: usize = 512;
/// How much of one OSC payload is held while it is scanned. Enough for a full
/// label even percent-encoded; the rest of an oversized payload is skipped.
const MAX_OSC_CAPTURE: usize = 16 * 1024;
/// Vorn's own shell integration reports the cwd on this private OSC.
const OSC_PRIVATE: u32 = 5522;

#[derive(Debug)]
pub enum Error {
    /// A width or height of zero, or over what a terminal can hold.
    Dimension(u32),
    Ghostty(libghostty_vt::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Dimension(n) => write!(f, "terminal dimension out of range: {n}"),
            Error::Ghostty(e) => write!(f, "libghostty-vt: {e:?}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<libghostty_vt::Error> for Error {
    fn from(e: libghostty_vt::Error) -> Self {
        Error::Ghostty(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// A screen, and what has to travel beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Escape sequences that reproduce the screen.
    pub screen: String,
    pub cols: u32,
    pub rows: u32,
    pub title: String,
    pub cwd: String,
}

/// What one feed changed that the server acts on.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Fed {
    /// The cwd an OSC 5522 moved to, for the session record. OSC 7 moves the
    /// model's cwd without reporting, as the JS model does.
    pub cwd: Option<String>,
    /// How many BELs rang: a real 0x07 as a terminal acts on it, not one
    /// that ends an OSC title.
    pub bells: u32,
}

pub struct Screen {
    term: Terminal<'static, 'static>,
    /// Counted by Ghostty's bell effect during `vt_write`, read and reset by `feed`.
    bells: Arc<AtomicU32>,
    cols: u32,
    rows: u32,
    title: String,
    cwd: String,
    osc: OscScanner,
}

impl Screen {
    pub fn new(cols: u32, rows: u32) -> Result<Self> {
        // Same as the xterm model: the screen, not history.
        Self::with_scrollback(cols, rows, 0)
    }

    /// A screen that keeps history above it. `max_scrollback` is Ghostty's
    /// limit, which it counts in bytes of page memory (whatever its C header
    /// says) and rounds up to whole pages, so it bounds memory rather than a
    /// number of lines. The recovery harness compares retained scrollback, so
    /// its terminals keep some; the server's model keeps none and uses
    /// [`Screen::new`].
    pub fn with_scrollback(cols: u32, rows: u32, max_scrollback: usize) -> Result<Self> {
        let mut term = Terminal::new(Options {
            cols: dimension(cols)?,
            rows: dimension(rows)?,
            max_scrollback,
        })?;
        let bells = Arc::new(AtomicU32::new(0));
        let rung = Arc::clone(&bells);
        term.on_bell(move |_| {
            rung.fetch_add(1, Ordering::Relaxed);
        })?;
        Ok(Self {
            term,
            bells,
            cols,
            rows,
            title: String::new(),
            cwd: String::new(),
            osc: OscScanner::default(),
        })
    }

    /// One flush of output.
    pub fn feed(&mut self, bytes: &[u8]) -> Fed {
        self.term.vt_write(bytes);
        let bells = self.bells.swap(0, Ordering::Relaxed);
        // Titles and cwds are read off the stream here rather than from Ghostty,
        // which drops a title over 2 KB instead of keeping its start, and so the
        // xterm model's rules apply: last writer wins, OSC 7 is percent-decoded,
        // and a sequence split across flushes still counts.
        let mut reported = None;
        let (title, cwd) = (&mut self.title, &mut self.cwd);
        self.osc.scan(bytes, |num, payload| match num {
            0 | 2 => *title = clip_units(&String::from_utf8_lossy(payload)),
            7 => {
                let raw = String::from_utf8_lossy(payload);
                let path = match raw.strip_prefix("file://") {
                    Some(rest) => rest.find('/').map_or("", |at| &rest[at..]),
                    None => &raw,
                };
                if !path.is_empty() {
                    *cwd = clip_units(&percent_decode(path).unwrap_or_else(|| path.to_owned()));
                }
            }
            OSC_PRIVATE => {
                if let Some(rest) = payload.strip_prefix(b"cwd;") {
                    let next = clip_units(&String::from_utf8_lossy(rest));
                    if !next.is_empty() && is_plausible_path(&next) && next != *cwd {
                        *cwd = next.clone();
                        reported = Some(next);
                    }
                }
            }
            _ => {}
        });
        Fed {
            cwd: reported,
            bells,
        }
    }

    /// Title and cwd from a checkpoint: neither is an escape sequence, so a
    /// restored screen does not rebuild them from its bytes.
    pub fn restore_labels(&mut self, title: Option<&str>, cwd: Option<&str>) {
        if let Some(t) = title.filter(|t| !t.is_empty()) {
            self.title = clip_units(t);
        }
        if let Some(c) = cwd.filter(|c| !c.is_empty()) {
            self.cwd = clip_units(c);
        }
    }

    pub fn resize(&mut self, cols: u32, rows: u32) -> Result<()> {
        let (c, r) = (dimension(cols)?, dimension(rows)?);
        self.term.resize(c, r, 1, 1)?;
        self.cols = cols;
        self.rows = rows;
        Ok(())
    }

    pub fn serialize(&self) -> Result<Snapshot> {
        let opts = FormatterOptions::new()
            .with_format(Format::Vt)
            .with_modes(true)
            .with_scrolling_region(true)
            .with_cursor(true)
            .with_style(true)
            .with_hyperlink(true)
            .with_charsets(true);
        let mut f = Formatter::new(&self.term, opts)?;
        let bytes = f.format_alloc(None)?;
        let mut screen = String::from_utf8_lossy(&bytes).into_owned();
        region_before_cursor(&mut screen);
        Ok(Snapshot {
            screen,
            cols: self.cols,
            rows: self.rows,
            title: self.title.clone(),
            cwd: self.cwd.clone(),
        })
    }

    /// The last OSC 0/2 title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The last cwd from OSC 7 or OSC 5522, whichever came last.
    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    /// The terminal itself, read only: what a state comparison needs that
    /// [`Screen::serialize`] does not report (modes, cursor, kitty keyboard
    /// flags, the active screen, the formatter with other options).
    pub fn terminal(&self) -> &Terminal<'static, 'static> {
        &self.term
    }

    /// Ghostty's own title, for parity checks against the stream scanner.
    pub fn ghostty_title(&self) -> Result<String> {
        Ok(self.term.title()?.to_owned())
    }
}

fn dimension(n: u32) -> Result<u16> {
    u16::try_from(n)
        .ok()
        .filter(|&n| n > 0)
        .ok_or(Error::Dimension(n))
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum OscState {
    #[default]
    Ground,
    Esc,
    /// The number before the first `;`, or `None` once it is not one.
    Num,
    Payload,
    /// ESC inside a payload: `\` ends it, anything else aborts it.
    PayloadEsc,
}

/// Finds OSC sequences in a stream, across calls, as xterm's parser delimits
/// them: `ESC ]`, a number, `;`, a payload, then BEL or `ESC \`. CAN and SUB
/// abort one.
#[derive(Default)]
struct OscScanner {
    state: OscState,
    num: Option<u32>,
    payload: Vec<u8>,
}

impl OscScanner {
    fn scan(&mut self, bytes: &[u8], mut on: impl FnMut(u32, &[u8])) {
        let mut i = 0;
        while i < bytes.len() {
            if self.state == OscState::Ground {
                match memchr(0x1b, &bytes[i..]) {
                    Some(n) => {
                        self.state = OscState::Esc;
                        i += n + 1;
                    }
                    None => return,
                }
                continue;
            }
            let b = bytes[i];
            i += 1;
            if b == 0x18 || b == 0x1a {
                self.state = OscState::Ground;
                continue;
            }
            match self.state {
                OscState::Ground => unreachable!(),
                OscState::Esc => {
                    if b == b']' {
                        self.state = OscState::Num;
                        self.num = Some(0);
                        self.payload.clear();
                    } else if b != 0x1b {
                        self.state = OscState::Ground;
                    }
                }
                OscState::Num => match b {
                    b'0'..=b'9' => {
                        self.num = self
                            .num
                            .and_then(|n| n.checked_mul(10)?.checked_add(u32::from(b - b'0')))
                    }
                    b';' => self.state = OscState::Payload,
                    // No payload at all: nothing to report.
                    0x07 => self.state = OscState::Ground,
                    0x1b => {
                        self.num = None;
                        self.state = OscState::PayloadEsc;
                    }
                    _ => self.num = None,
                },
                OscState::Payload => match b {
                    0x07 => self.finish(&mut on),
                    0x1b => self.state = OscState::PayloadEsc,
                    _ => {
                        if self.num.is_some() && self.payload.len() < MAX_OSC_CAPTURE {
                            self.payload.push(b);
                        }
                    }
                },
                OscState::PayloadEsc => {
                    if b == b'\\' {
                        self.finish(&mut on);
                    } else {
                        // ESC aborts the string and starts a new escape.
                        self.state = OscState::Esc;
                        i -= 1;
                    }
                }
            }
        }
    }

    fn finish(&mut self, on: &mut impl FnMut(u32, &[u8])) {
        self.state = OscState::Ground;
        if let Some(n) = self.num {
            on(n, &self.payload);
        }
        self.payload.clear();
    }
}

/// At most [`MAX_LABEL_UNITS`] UTF-16 units, cut at a character boundary.
fn clip_units(s: &str) -> String {
    let mut units = 0;
    for (at, ch) in s.char_indices() {
        units += ch.len_utf16();
        if units > MAX_LABEL_UNITS {
            return s[..at].to_owned();
        }
    }
    s.to_owned()
}

/// `decodeURIComponent`: `None` where it would throw, so the caller keeps the raw value.
fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = b.get(i + 1..i + 3)?;
            let hex = std::str::from_utf8(hex).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `isPlausiblePath` in terminal-screen.ts.
fn is_plausible_path(p: &str) -> bool {
    let b = p.as_bytes();
    let absolute = p.starts_with('/')
        || (b.len() >= 3
            && b[0].is_ascii_alphabetic()
            && b[1] == b':'
            && matches!(b[2], b'/' | b'\\'));
    absolute && !p.chars().any(|c| (c as u32) < 0x20 || c == '\u{7f}')
}

/// The formatter writes the cursor and then the scrolling region, but setting
/// a region (DECSTBM) homes the cursor, so a screen restored from its output
/// has the cursor at 1;1. Found by the round-trip test (RC-T4). Swap the two
/// when they end the output. With origin mode on, the cursor position would
/// also need to become relative to the region, so that case is left as the
/// formatter wrote it and is a named fixture in `tests/round_trip.rs`.
fn region_before_cursor(screen: &mut String) {
    if screen.contains("\x1b[?6h") {
        return;
    }
    let Some(region_at) = screen.rfind("\x1b[") else {
        return;
    };
    if !is_csi(&screen[region_at..], 'r') {
        return;
    }
    let Some(cursor_at) = screen[..region_at].rfind("\x1b[") else {
        return;
    };
    if cursor_at + csi_len(&screen[cursor_at..]) != region_at
        || !is_csi(&screen[cursor_at..region_at], 'H')
    {
        return;
    }
    let cursor = screen[cursor_at..region_at].to_owned();
    let region = screen[region_at..].to_owned();
    screen.truncate(cursor_at);
    screen.push_str(&region);
    screen.push_str(&cursor);
}

/// `s` is exactly one CSI with numeric parameters ending in `fin`.
fn is_csi(s: &str, fin: char) -> bool {
    s.len() == csi_len(s)
        && s.ends_with(fin)
        && s[2..s.len() - 1]
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b';')
}

/// The length of the CSI at the start of `s`: up to and including its final byte.
fn csi_len(s: &str) -> usize {
    s.bytes()
        .enumerate()
        .skip(2)
        .find(|(_, b)| (0x40..=0x7e).contains(b))
        .map_or(s.len(), |(i, _)| i + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan_all(chunks: &[&[u8]]) -> Vec<(u32, String)> {
        let mut s = OscScanner::default();
        let mut seen = Vec::new();
        for c in chunks {
            s.scan(c, |n, p| {
                seen.push((n, String::from_utf8_lossy(p).into_owned()))
            });
        }
        seen
    }

    #[test]
    fn osc_terminators_and_splits() {
        assert_eq!(
            scan_all(&[b"a\x1b]2;one\x07b\x1b]0;two\x1b\\"]),
            [(2, "one".into()), (0, "two".into())]
        );
        assert_eq!(
            scan_all(&[b"x\x1b]55", b"22;cwd;/tm", b"p\x1b", b"\\"]),
            [(5522, "cwd;/tmp".into())]
        );
        // Aborted by CAN, and by an ESC that starts another sequence.
        assert_eq!(
            scan_all(&[b"\x1b]2;a\x18\x1b]2;b\x1b]2;c\x07"]),
            [(2, "c".into())]
        );
    }

    #[test]
    fn labels() {
        assert_eq!(clip_units(&"é".repeat(600)).chars().count(), 512);
        assert_eq!(clip_units(&"😀".repeat(300)).chars().count(), 256);
        assert_eq!(percent_decode("/a/my%20dir").as_deref(), Some("/a/my dir"));
        assert_eq!(percent_decode("/a/100%"), None);
        assert_eq!(percent_decode("/a/%ff"), None);
        assert!(is_plausible_path("/x"));
        assert!(is_plausible_path("C:\\Users"));
        assert!(!is_plausible_path("x/y"));
        assert!(!is_plausible_path("/x\ny"));
    }

    #[test]
    fn feeds_titles_and_cwds() {
        let mut s = Screen::new(80, 24).unwrap();
        assert_eq!(s.feed(b"\x1b]2;vim\x07"), Fed::default());
        assert_eq!(s.title(), "vim");
        assert_eq!(s.ghostty_title().unwrap(), "vim");
        // OSC 7 moves the model's cwd, percent-decoded, without reporting it.
        assert_eq!(s.feed(b"\x1b]7;file://host/a/my%20dir\x07").cwd, None);
        assert_eq!(s.cwd(), "/a/my dir");
        // OSC 5522 reports, once per move, and only a plausible path.
        assert_eq!(
            s.feed(b"\x1b]5522;cwd;/srv\x07").cwd.as_deref(),
            Some("/srv")
        );
        assert_eq!(s.feed(b"\x1b]5522;cwd;/srv\x07").cwd, None);
        assert_eq!(s.feed(b"\x1b]5522;cwd;relative\x07").cwd, None);
        assert_eq!(s.cwd(), "/srv");
    }

    #[test]
    fn counts_real_bells_only() {
        let mut s = Screen::new(80, 24).unwrap();
        // BEL ending an OSC is a terminator, not a bell.
        assert_eq!(s.feed(b"\x1b]0;\xe2\x9c\xb3 claude\x07working").bells, 0);
        assert_eq!(s.feed(b"done\x07\x07").bells, 2);
        assert_eq!(s.feed(b"quiet").bells, 0);
    }

    #[test]
    fn restores_labels_and_serializes_them() {
        let mut s = Screen::new(10, 3).unwrap();
        s.restore_labels(Some("htop"), Some("/home"));
        s.restore_labels(Some(""), None);
        s.feed(b"hi\r\nthere");
        let snap = s.serialize().unwrap();
        assert_eq!((snap.cols, snap.rows), (10, 3));
        assert_eq!((snap.title.as_str(), snap.cwd.as_str()), ("htop", "/home"));
        assert!(snap.screen.contains("hi"), "{:?}", snap.screen);
        assert!(snap.screen.contains("there"), "{:?}", snap.screen);
    }

    #[test]
    fn resize_follows_and_rejects_nonsense() {
        let mut s = Screen::new(80, 24).unwrap();
        s.resize(120, 40).unwrap();
        let snap = s.serialize().unwrap();
        assert_eq!((snap.cols, snap.rows), (120, 40));
        assert!(matches!(s.resize(0, 10), Err(Error::Dimension(0))));
        assert!(matches!(
            Screen::new(70_000, 10),
            Err(Error::Dimension(70_000))
        ));
    }

    #[test]
    fn never_answers_a_query() {
        // Nothing is wired to on_pty_write, so a device-attributes query is
        // parsed and dropped rather than answered into the PTY's input.
        let mut s = Screen::new(80, 24).unwrap();
        s.feed(b"\x1b[c\x1b[6n\x1b[>c");
        assert!(!s.serialize().unwrap().screen.contains("?62"));
    }
}
