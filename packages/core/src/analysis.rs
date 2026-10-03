//! Output analysis: what `appendOutput`, `stripAnsi` and `status-parser.ts` do
//! per raw PTY chunk, in one pass over the bytes.
//!
//! Spike code for the Ghostty-vs-JS comparison. It keeps the JS path's outputs
//! (a ring of stripped lines, bracketed-paste status, the status patterns on the
//! last lines) so the bench compares the same job, with differences that make it
//! closer to a terminal rather than further:
//!
//! - parser state outlives the chunk, so an escape sequence split across two
//!   reads is still stripped;
//! - a carriage return moves a cursor that also outlives the chunk, and erase in
//!   line (`CSI K`) erases, so a status line redrawn in place stays one line
//!   instead of growing by every frame until the next newline;
//! - the status patterns run on the last five lines, not on the last five lines
//!   of a 2000-character window. Every pattern matches within one line, so a
//!   completed line is checked at most once, and only the line in progress is
//!   checked on every chunk.

use std::collections::VecDeque;
use std::sync::OnceLock;

use memchr::{memchr3, memmem};
use napi_derive::napi;
use regex::Regex;

const MAX_OUTPUT_LINES: usize = 1000;
/// An OSC/DCS/APC payload longer than this is taken as unterminated and
/// dropped, so one stray `ESC ]` cannot swallow the rest of a session's output.
const MAX_STRING_BYTES: usize = 4096;
/// `analyzeOutput` reads the last five lines: four completed and the current one.
const RECENT_COMPLETED: usize = 4;

/// Returned by [`Analyzer::append`].
pub const STATUS_NONE: u32 = 0;
pub const STATUS_RUNNING: u32 = 1;
pub const STATUS_WAITING: u32 = 2;
pub const STATUS_ERROR: u32 = 3;

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    Esc,
    Csi,
    /// OSC, DCS, SOS, PM, APC: a payload that ends at BEL or ST.
    Str,
    StrEsc,
    /// `ESC (` / `ESC )`: intermediates, then one final byte.
    Charset,
}

fn error_patterns() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i:^error:)|^Error:|FATAL|panic:|Traceback|command not found|ENOENT|EACCES")
            .unwrap()
    })
}

fn waiting_patterns() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[$❯>?]\s*$|(?i:\(y/n\)|Enter .*:|waiting for input)").unwrap())
}

struct Line {
    text: String,
    /// Whether an error pattern matches, worked out the first time the line is
    /// among the recent ones when a chunk ends. Lines that scroll past inside
    /// one chunk are never checked, as in the JS path.
    error: Option<bool>,
}

/// One session's analysis state.
#[napi]
pub struct Analyzer {
    state: State,
    /// The first CSI parameter and whether the sequence is private (`CSI ?`).
    csi_param: u32,
    csi_private: bool,
    /// Bytes of the current string payload, for [`MAX_STRING_BYTES`].
    str_len: usize,
    /// The line being written, its length in characters, and the cursor
    /// column. Text is appended in place while the cursor sits at the end, which
    /// is nearly always; only an overwrite after `\r` goes through characters.
    line: String,
    line_chars: usize,
    col: usize,
    /// A `\r` not yet resolved: CRLF ends the line, anything else overwrites.
    pending_cr: bool,
    lines: VecDeque<Line>,
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[napi]
impl Analyzer {
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            state: State::Ground,
            csi_param: 0,
            csi_private: false,
            str_len: 0,
            line: String::with_capacity(256),
            line_chars: 0,
            col: 0,
            pending_cr: false,
            lines: VecDeque::with_capacity(64),
        }
    }

    /// Feed one raw chunk. `analyze` is false for hook-driven sessions, which
    /// only take status from bracketed paste. Returns one of the `STATUS_*`
    /// codes: none, running, waiting, error.
    #[napi(catch_unwind)]
    pub fn append(&mut self, data: String, analyze: bool) -> u32 {
        self.append_str(&data, analyze)
    }

    /// The last `lines` completed lines, oldest first; all of them when omitted
    /// or zero, as `getOutput` reads it.
    #[napi(catch_unwind)]
    pub fn output(&self, lines: Option<u32>) -> Vec<String> {
        let n = lines
            .filter(|&n| n > 0)
            .map_or(self.lines.len(), |n| (n as usize).min(self.lines.len()));
        self.lines
            .iter()
            .skip(self.lines.len() - n)
            .map(|l| l.text.clone())
            .collect()
    }

    /// Release the line ring now; the analyzer starts empty if fed again.
    #[napi]
    pub fn free(&mut self) {
        *self = Self::new();
        self.lines.shrink_to_fit();
        self.line.shrink_to_fit();
    }
}

impl Analyzer {
    pub fn append_str(&mut self, data: &str, analyze: bool) -> u32 {
        let bytes = data.as_bytes();
        // Before the feed, which only needs the bytes; the scan is SIMD either way.
        let on = memmem::rfind(bytes, b"\x1b[?2004h");
        let off = memmem::rfind(bytes, b"\x1b[?2004l");
        self.feed(data);
        if on.is_some() || off.is_some() {
            return match (on, off) {
                (Some(a), Some(b)) if a > b => STATUS_WAITING,
                (Some(_), None) => STATUS_WAITING,
                _ => STATUS_RUNNING,
            };
        }
        if !analyze {
            return STATUS_NONE;
        }
        self.status()
    }

    pub fn feed(&mut self, data: &str) {
        let bytes = data.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if self.state == State::Ground {
                // A run of plain text up to the next byte that means something.
                let end = memchr3(0x1b, b'\r', b'\n', &bytes[i..]).map_or(bytes.len(), |n| i + n);
                if end > i {
                    // Escape, CR and LF are ASCII, so `end` is a char boundary.
                    self.text_run(&data[i..end]);
                    i = end;
                    continue;
                }
                match bytes[i] {
                    0x1b => self.state = State::Esc,
                    b'\r' => self.carriage_return(),
                    _ => self.line_feed(),
                }
                i += 1;
                continue;
            }
            let b = bytes[i];
            match (self.state, b) {
                // A non-ASCII byte cannot be part of an escape sequence. Leaving
                // it for the ground state keeps `i` on a char boundary, which the
                // text slice above relies on (and which would panic otherwise).
                (State::Str, _) if b < 0x80 || self.str_len < MAX_STRING_BYTES => {}
                (_, 0x80..) => {
                    self.state = State::Ground;
                    continue;
                }
                // CAN and SUB cancel any sequence.
                (_, 0x18 | 0x1a) => {
                    self.state = State::Ground;
                    i += 1;
                    continue;
                }
                // CR and LF inside CSI or ESC still act, as a terminal does.
                (State::Esc | State::Csi | State::Charset, b'\r') => {
                    self.carriage_return();
                    i += 1;
                    continue;
                }
                (State::Esc | State::Csi | State::Charset, b'\n') => {
                    self.line_feed();
                    i += 1;
                    continue;
                }
                _ => {}
            }
            i += 1;
            self.escape_byte(b);
        }
    }

    fn escape_byte(&mut self, b: u8) {
        match self.state {
            State::Ground => unreachable!(),
            State::Esc => {
                self.state = match b {
                    b'[' => {
                        self.csi_param = 0;
                        self.csi_private = false;
                        State::Csi
                    }
                    b']' | b'P' | b'X' | b'^' | b'_' => {
                        self.str_len = 0;
                        State::Str
                    }
                    b'(' | b')' => State::Charset,
                    0x1b => State::Esc,
                    _ => State::Ground,
                }
            }
            State::Csi => match b {
                b'0'..=b'9' => {
                    self.csi_param = self
                        .csi_param
                        .saturating_mul(10)
                        .saturating_add(u32::from(b - b'0'))
                }
                b'?' | b'<' | b'=' | b'>' => self.csi_private = true,
                // Later parameters and intermediates: nothing here reads them.
                0x20..=0x3f => {}
                0x40..=0x7e => {
                    self.state = State::Ground;
                    if b == b'K' && !self.csi_private {
                        self.erase_in_line(self.csi_param);
                    }
                }
                _ => self.state = State::Ground,
            },
            State::Str => match b {
                0x07 => self.state = State::Ground,
                0x1b => self.state = State::StrEsc,
                _ => {
                    self.str_len += 1;
                    if self.str_len > MAX_STRING_BYTES {
                        self.state = State::Ground;
                    }
                }
            },
            State::StrEsc => {
                if b == b'\\' {
                    self.state = State::Ground;
                } else {
                    // ESC inside a string aborts it and starts a new escape.
                    self.state = State::Esc;
                    self.escape_byte(b);
                }
            }
            State::Charset => {
                if !(0x20..=0x2f).contains(&b) {
                    self.state = State::Ground;
                }
            }
        }
    }

    fn resolve_cr(&mut self) {
        if self.pending_cr {
            self.pending_cr = false;
            self.col = 0;
        }
    }

    fn text_run(&mut self, run: &str) {
        self.resolve_cr();
        let n = if run.is_ascii() {
            run.len()
        } else {
            run.chars().count()
        };
        if self.col == self.line_chars {
            self.line.push_str(run);
            self.line_chars += n;
            self.col = self.line_chars;
            return;
        }
        let mut cells: Vec<char> = self.line.chars().collect();
        for ch in run.chars() {
            if self.col < cells.len() {
                cells[self.col] = ch;
            } else {
                cells.push(ch);
            }
            self.col += 1;
        }
        self.line_chars = cells.len();
        self.line = cells.into_iter().collect();
    }

    fn carriage_return(&mut self) {
        // Resolved by whatever follows: CRLF ends the line with its text intact.
        self.pending_cr = true;
    }

    fn line_feed(&mut self) {
        self.pending_cr = false;
        let next = if self.lines.len() == MAX_OUTPUT_LINES {
            // Reuse the evicted line's allocation for the next one.
            let mut old = self.lines.pop_front().map(|l| l.text).unwrap_or_default();
            old.clear();
            old
        } else {
            String::with_capacity(self.line.len().max(64))
        };
        let text = std::mem::replace(&mut self.line, next);
        self.line_chars = 0;
        self.col = 0;
        self.lines.push_back(Line { text, error: None });
    }

    fn erase_in_line(&mut self, mode: u32) {
        self.resolve_cr();
        match mode {
            0 if self.col < self.line_chars => {
                let at = self
                    .line
                    .char_indices()
                    .nth(self.col)
                    .map_or(self.line.len(), |(i, _)| i);
                self.line.truncate(at);
                self.line_chars = self.col;
            }
            2 => {
                self.line.clear();
                self.line.extend(std::iter::repeat_n(' ', self.col));
                self.line_chars = self.col;
            }
            _ => {}
        }
    }

    /// `analyzeOutput` on the last five lines, the line in progress included.
    fn status(&mut self) -> u32 {
        let mut error = false;
        for l in self.lines.iter_mut().rev().take(RECENT_COMPLETED) {
            error |= *l
                .error
                .get_or_insert_with(|| error_patterns().is_match(&l.text));
        }
        let current = self.line.as_str();
        let recent = self.lines.iter().rev().take(RECENT_COMPLETED);
        if error || error_patterns().is_match(current) {
            return STATUS_ERROR;
        }
        // The last line with something on it, as `trimEnd().split('\n').pop()` finds it.
        let last = std::iter::once(current)
            .chain(recent.map(|l| l.text.as_str()))
            .map(str::trim_end)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        if waiting_patterns().is_match(last) {
            return STATUS_WAITING;
        }
        STATUS_RUNNING
    }
}

/// Every chunk through one analyzer in a single call: the same work as
/// [`Analyzer::append`] per chunk without a napi crossing per chunk, so the
/// bench can show what the boundary costs. Returns the last status code.
#[napi(catch_unwind)]
pub fn analyze_batch(chunks: Vec<String>, analyze: bool) -> u32 {
    let mut a = Analyzer::new();
    let mut last = STATUS_NONE;
    for c in &chunks {
        last = a.append_str(c, analyze);
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(a: &Analyzer) -> Vec<String> {
        a.output(None)
    }

    #[test]
    fn strips_escapes_and_splits_lines() {
        let mut a = Analyzer::new();
        a.feed("\x1b[1m\x1b[38;5;114mhello\x1b[0m\r\n\x1b]0;title\x07world\n");
        assert_eq!(lines(&a), ["hello", "world"]);
    }

    #[test]
    fn carriage_return_overwrites_across_chunks() {
        let mut a = Analyzer::new();
        a.feed("Working");
        a.feed("\rWo");
        a.feed("rk\r\n");
        assert_eq!(lines(&a), ["Working"]);
        a.feed("abc\rXX\n");
        assert_eq!(lines(&a)[1], "XXc");
    }

    #[test]
    fn erase_in_line_after_carriage_return() {
        let mut a = Analyzer::new();
        a.feed("⠋ Thinking… (1s)");
        a.feed("\r\x1b[2K⠙ Done\r\n");
        assert_eq!(lines(&a), ["⠙ Done"]);
    }

    #[test]
    fn escape_split_across_chunks() {
        let mut a = Analyzer::new();
        a.feed("a\x1b[3");
        a.feed("8;5;1mb\n");
        assert_eq!(lines(&a), ["ab"]);
    }

    #[test]
    fn non_ascii_after_escape_does_not_panic() {
        let mut a = Analyzer::new();
        a.feed("\x1bé\n");
        a.feed("\x1b[1é\n");
        a.feed("x\x1b");
        a.feed("ü\n");
        a.feed("\x1b]0;t\x1bé\n");
        assert_eq!(lines(&a), ["é", "é", "xü", "é"]);
    }

    #[test]
    fn arbitrary_input_never_panics() {
        // A small deterministic fuzz: every mix of escapes, controls and multibyte text.
        let pieces = [
            "\x1b", "[", "]", "1", ";", "?", "K", "\r", "\n", "\x07", "\\", "é", "😀", "a", "\x18",
            "P",
        ];
        let mut seed = 0x9e3779b97f4a7c15u64;
        for _ in 0..2000 {
            let mut a = Analyzer::new();
            for _ in 0..8 {
                let mut chunk = String::new();
                for _ in 0..12 {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    chunk.push_str(pieces[(seed % pieces.len() as u64) as usize]);
                }
                a.append_str(&chunk, true);
            }
        }
        let mut a = Analyzer::new();
        a.feed("\x1b[99999999999999K");
    }

    #[test]
    fn unterminated_string_gives_up() {
        let mut a = Analyzer::new();
        a.feed("\x1b]0;never ends\n");
        a.feed(&"x".repeat(MAX_STRING_BYTES));
        a.feed("\nvisible\n");
        assert_eq!(lines(&a).last().map(String::as_str), Some("visible"));
    }

    #[test]
    fn controls_inside_csi_still_act() {
        // The newline acts; the `b` after it is the CSI's final byte, as on a terminal.
        let mut a = Analyzer::new();
        a.feed("a\x1b[\nb\nc\n");
        assert_eq!(lines(&a), ["a", "", "c"]);
    }

    #[test]
    fn output_zero_means_all() {
        let mut a = Analyzer::new();
        a.feed("1\n2\n");
        assert_eq!(a.output(Some(0)), ["1", "2"]);
        assert_eq!(a.output(Some(1)), ["2"]);
    }

    #[test]
    fn status() {
        let mut a = Analyzer::new();
        assert_eq!(a.append_str("doing things\n", true), STATUS_RUNNING);
        assert_eq!(a.append_str("Continue? (y/n) ", true), STATUS_WAITING);
        assert_eq!(a.append_str("\nerror: boom\n", true), STATUS_ERROR);
        assert_eq!(a.append_str("1\n2\n3\n4\n$ ", true), STATUS_WAITING);
        assert_eq!(a.append_str("\x1b[?2004h", true), STATUS_WAITING);
        assert_eq!(
            a.append_str("\x1b[?2004h x \x1b[?2004l", false),
            STATUS_RUNNING
        );
        assert_eq!(a.append_str("plain", false), STATUS_NONE);
    }
}

/// Throughput of the analysis alone, without napi: `VORN_TRANSCRIPTS=<dir of
/// <name>.json chunk arrays> cargo test --release analysis_throughput -- --ignored --nocapture`
#[cfg(test)]
mod throughput {
    use super::*;
    use std::time::Instant;

    #[test]
    #[ignore]
    fn analysis_throughput() {
        let dir = std::env::var("VORN_TRANSCRIPTS").expect("VORN_TRANSCRIPTS");
        for name in ["agent", "spinner", "bulk"] {
            let raw = std::fs::read_to_string(format!("{dir}/{name}.json")).unwrap();
            let chunks: Vec<String> = serde_json::from_str(&raw).unwrap();
            let bytes: usize = chunks.iter().map(String::len).sum();
            let mut best = f64::MAX;
            for _ in 0..20 {
                let t = Instant::now();
                let mut a = Analyzer::new();
                for c in &chunks {
                    std::hint::black_box(a.append_str(c, true));
                }
                best = best.min(t.elapsed().as_secs_f64());
            }
            println!(
                "{name}: {:.2} ms/MB over {} chunks",
                best * 1000.0 / (bytes as f64 / 1048576.0),
                chunks.len()
            );
        }
    }
}
