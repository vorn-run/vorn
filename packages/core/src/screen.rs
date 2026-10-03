//! The screen model `terminal-screen.ts` keeps in a headless xterm, kept in a
//! libghostty-vt terminal instead.
//!
//! Spike code for the Ghostty-vs-JS comparison. Differences from the xterm
//! model that matter for the comparison: the parse is synchronous, so there is
//! no queue to bound and no drain to wait for, and the serialized screen comes
//! from Ghostty's own VT formatter rather than `@xterm/addon-serialize`.

use libghostty_vt::fmt::{Format, Formatter, FormatterOptions};
use libghostty_vt::terminal::{Options, Terminal};
use memchr::memchr;
use napi::bindgen_prelude::Buffer;
use napi_derive::napi;

/// How much of a title or a cwd is kept, in UTF-16 units as the JS model counts.
const MAX_LABEL_UNITS: usize = 512;
/// How much of one OSC payload is held while it is scanned. Enough for a full
/// label even percent-encoded; the rest of an oversized payload is skipped.
const MAX_OSC_CAPTURE: usize = 16 * 1024;
/// Vorn's own shell integration reports the cwd on this private OSC.
const OSC_PRIVATE: u32 = 5522;

#[napi(object)]
pub struct ScreenSnapshot {
    pub screen: String,
    pub cols: u32,
    pub rows: u32,
    pub title: String,
    pub cwd: String,
}

#[napi]
pub struct Screen {
    /// `None` once freed: the Ghostty terminal's memory is invisible to V8, so
    /// it is released when the session ends rather than whenever V8 collects.
    term: Option<Terminal<'static, 'static>>,
    cols: u32,
    rows: u32,
    title: String,
    cwd: String,
    osc: OscScanner,
}

#[napi]
impl Screen {
    #[napi(constructor)]
    pub fn new(cols: u32, rows: u32) -> napi::Result<Self> {
        let term = Terminal::new(Options {
            cols: dimension(cols)?,
            rows: dimension(rows)?,
            // Same as the xterm model: the screen, not history.
            max_scrollback: 0,
        })
        .map_err(crate::to_napi)?;
        Ok(Self {
            term: Some(term),
            cols,
            rows,
            title: String::new(),
            cwd: String::new(),
            osc: OscScanner::default(),
        })
    }

    /// One flush of output, as the string node-pty produced. Returns the cwd an
    /// OSC 5522 in it moved to, for the server to record, as the JS model's
    /// handler reports it; OSC 7 updates the model's cwd without reporting.
    #[napi(catch_unwind)]
    pub fn feed(&mut self, data: String) -> Option<String> {
        self.write(data.as_bytes())
    }

    /// The same, from bytes, for a caller that never decoded them.
    #[napi(catch_unwind)]
    pub fn feed_bytes(&mut self, data: Buffer) -> Option<String> {
        self.write(&data)
    }

    /// Title and cwd from a checkpoint: neither is an escape sequence, so a
    /// restored screen does not rebuild them from its bytes.
    #[napi(catch_unwind)]
    pub fn restore_labels(&mut self, title: Option<String>, cwd: Option<String>) {
        if let Some(t) = title.filter(|t| !t.is_empty()) {
            self.title = clip_units(&t);
        }
        if let Some(c) = cwd.filter(|c| !c.is_empty()) {
            self.cwd = clip_units(&c);
        }
    }

    #[napi(catch_unwind)]
    pub fn resize(&mut self, cols: u32, rows: u32) -> napi::Result<()> {
        let (c, r) = (dimension(cols)?, dimension(rows)?);
        if let Some(term) = self.term.as_mut() {
            term.resize(c, r, 1, 1).map_err(crate::to_napi)?;
        }
        self.cols = cols;
        self.rows = rows;
        Ok(())
    }

    #[napi(catch_unwind)]
    pub fn serialize(&self) -> napi::Result<ScreenSnapshot> {
        let term = self
            .term
            .as_ref()
            .ok_or_else(|| napi::Error::from_reason("screen was freed"))?;
        let opts = FormatterOptions::new()
            .with_format(Format::Vt)
            .with_modes(true)
            .with_scrolling_region(true)
            .with_cursor(true)
            .with_style(true)
            .with_hyperlink(true)
            .with_charsets(true);
        let mut f = Formatter::new(term, opts).map_err(crate::to_napi)?;
        let bytes = f.format_alloc(None).map_err(crate::to_napi)?;
        Ok(ScreenSnapshot {
            screen: String::from_utf8_lossy(&bytes).into_owned(),
            cols: self.cols,
            rows: self.rows,
            title: self.title.clone(),
            cwd: self.cwd.clone(),
        })
    }

    /// The last OSC 0/2 title.
    #[napi(getter)]
    pub fn title(&self) -> String {
        self.title.clone()
    }

    /// The last cwd from OSC 7 or OSC 5522, whichever came last.
    #[napi(getter)]
    pub fn cwd(&self) -> String {
        self.cwd.clone()
    }

    /// Release the terminal now. Every later call is a no-op or an error.
    #[napi]
    pub fn free(&mut self) {
        self.term = None;
    }
}

impl Screen {
    fn write(&mut self, bytes: &[u8]) -> Option<String> {
        let term = self.term.as_mut()?;
        term.vt_write(bytes);
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
        reported
    }
}

fn dimension(n: u32) -> napi::Result<u16> {
    u16::try_from(n)
        .ok()
        .filter(|&n| n > 0)
        .ok_or_else(|| napi::Error::from_reason(format!("terminal dimension out of range: {n}")))
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
}

/// Throughput of the libghostty-vt parse alone, without napi, fed per flush as
/// the server feeds it: `VORN_TRANSCRIPTS=<dir> cargo test --release
/// screen_throughput -- --ignored --nocapture`
#[cfg(test)]
mod throughput {
    use super::*;
    use std::time::Instant;

    #[test]
    #[ignore]
    fn screen_throughput() {
        let dir = std::env::var("VORN_TRANSCRIPTS").expect("VORN_TRANSCRIPTS");
        for name in ["agent", "spinner", "bulk"] {
            let raw = std::fs::read_to_string(format!("{dir}/{name}.json")).unwrap();
            let chunks: Vec<String> = serde_json::from_str(&raw).unwrap();
            let flushes: Vec<String> = chunks.chunks(100).map(|c| c.concat()).collect();
            let bytes: usize = chunks.iter().map(String::len).sum();
            let mut best = f64::MAX;
            for _ in 0..20 {
                // The terminal directly: `Screen::new` returns a napi error type,
                // which a test binary has no Node to link against.
                let mut term = Terminal::new(Options {
                    cols: 200,
                    rows: 50,
                    max_scrollback: 0,
                })
                .unwrap();
                let t = Instant::now();
                for f in &flushes {
                    term.vt_write(f.as_bytes());
                }
                best = best.min(t.elapsed().as_secs_f64());
            }
            println!(
                "{name}: {:.2} ms/MB ({:.0} MB/s)",
                best * 1000.0 / (bytes as f64 / 1048576.0),
                bytes as f64 / 1048576.0 / best
            );
        }
    }
}
