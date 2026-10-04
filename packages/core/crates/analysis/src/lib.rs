//! Output analysis: what `appendOutput`, `stripAnsi` and `status-parser.ts` do
//! per raw PTY chunk, in one pass over the bytes.
//!
//! Plain Rust with no Node in it; `vorn-core` wraps it for the server. It keeps the JS path's outputs
//! (a ring of stripped lines, bracketed-paste status, the status patterns on the
//! last lines) so the bench compares the same job, with differences that make it
//! closer to a terminal rather than further:
//!
//! - parser state outlives the chunk, so an escape sequence split across two
//!   reads is still stripped;
//! - a carriage return moves a cursor that also outlives the chunk, and erase in
//!   line (`CSI K`) erases, so a status line redrawn in place stays one line
//!   instead of growing by every frame until the next newline;
//! - the status patterns run line by line on the last five lines, so a
//!   completed line is checked at most once and only the line in progress is
//!   checked on every chunk. They still only see what falls in the JS path's
//!   2000-character window: a line, or the part of one, further back than that
//!   is not matched even when it is among the last five.

use std::collections::VecDeque;
use std::sync::OnceLock;

use memchr::{memchr3, memmem};
use regex::Regex;

const MAX_OUTPUT_LINES: usize = 1000;
/// An OSC/DCS/APC payload longer than this is taken as unterminated and
/// dropped, so one stray `ESC ]` cannot swallow the rest of a session's output.
const MAX_STRING_BYTES: usize = 4096;
/// `analyzeOutput` reads the last five lines: four completed and the current one.
const RECENT_COMPLETED: usize = 4;
/// How far back `analyzeOutput` looks for the last non-blank line: its buffer
/// holds the last 2000 characters of stripped output.
const WAITING_WINDOW_CHARS: usize = 2000;

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

impl Analyzer {
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

    /// The last `lines` completed lines, oldest first; all of them when omitted
    /// or zero, as `getOutput` reads it.
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

    /// The line in progress, stripped, as the JS path keeps it between chunks.
    pub fn partial(&self) -> String {
        self.line.clone()
    }

    /// Release the line ring now; the analyzer starts empty if fed again.
    pub fn free(&mut self) {
        *self = Self::new();
        self.lines.shrink_to_fit();
        self.line.shrink_to_fit();
    }
}

impl Analyzer {
    /// Feed one raw chunk. `analyze` is false for hook-driven sessions, which
    /// only take status from bracketed paste. Returns one of the `STATUS_*`
    /// codes: none, running, waiting, error.
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
                    // An overlong string can hit its limit mid-character: skip
                    // the rest of that character so `i` is on a boundary again.
                    while i < bytes.len() && bytes[i] & 0xc0 == 0x80 {
                        i += 1;
                    }
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

    /// `analyzeOutput`: errors on the last five lines, the line in progress
    /// included; the waiting prompt anywhere in the last 2000 characters.
    fn status(&mut self) -> u32 {
        // Only the window `analyzeOutput` keeps, 2000 characters of the stream
        // shared across its last five lines: a long stream with no newline
        // would otherwise be rescanned in full on every chunk.
        let mut left = WAITING_WINDOW_CHARS;
        let current = tail(&self.line, left);
        let mut error = error_patterns().is_match(current);
        left = left.saturating_sub(current.chars().count());
        for l in self.lines.iter_mut().rev().take(RECENT_COMPLETED) {
            if left == 0 {
                break;
            }
            // The newline that ends this line is inside the window too.
            left -= 1;
            let t = tail(&l.text, left);
            if t.len() == l.text.len() {
                // Whole in the window: the verdict can be kept for the next chunk.
                left -= t.chars().count();
                error |= *l
                    .error
                    .get_or_insert_with(|| error_patterns().is_match(&l.text));
            } else {
                error |= error_patterns().is_match(t);
                left = 0;
            }
        }
        if error {
            return STATUS_ERROR;
        }
        // The last line with something on it, as `trimEnd().split('\n').pop()`
        // finds it in the JS path's 2000-character buffer: blank lines after a
        // prompt don't hide it.
        let mut budget = WAITING_WINDOW_CHARS;
        let mut last = "";
        let mut completed = self.lines.iter().rev();
        let mut l = current;
        loop {
            let trimmed = l.trim_end();
            if !trimmed.is_empty() {
                last = trimmed;
                break;
            }
            // The line and the newline after it.
            budget = budget.saturating_sub(l.chars().count() + 1);
            match completed.next() {
                // Each earlier line only as far as the window still reaches.
                Some(next) if budget > 0 => l = tail(&next.text, budget),
                _ => break,
            }
        }
        if waiting_patterns().is_match(last) {
            return STATUS_WAITING;
        }
        STATUS_RUNNING
    }
}

/// The last `n` characters of `s`, found from the end so the cost is `n`, not `s`.
fn tail(s: &str, n: usize) -> &str {
    match n.checked_sub(1).and_then(|k| s.char_indices().rev().nth(k)) {
        Some((i, _)) => &s[i..],
        None if n == 0 => "",
        None => s,
    }
}

/// Every chunk through one analyzer, returning the last status code.
pub fn analyze_batch<S: AsRef<str>>(chunks: &[S], analyze: bool) -> u32 {
    let mut a = Analyzer::new();
    let mut last = STATUS_NONE;
    for c in chunks {
        last = a.append_str(c.as_ref(), analyze);
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

    #[test]
    fn overlong_multibyte_string_is_dropped_on_a_boundary() {
        // An OSC whose payload hits the limit mid-character must not leave the
        // scanner inside that character.
        for pad in 0..3 {
            let mut a = Analyzer::new();
            let osc = format!("\x1b]0;{}{}", "x".repeat(pad), "€".repeat(MAX_STRING_BYTES));
            a.append_str(&osc, true);
            a.append_str("after\n", true);
            // The rest of the payload is text now, as after any dropped string.
            assert!(a.output(None).last().unwrap().ends_with("after"));
        }
    }

    #[test]
    fn tail_counts_chars() {
        assert_eq!(tail("héllo", 3), "llo");
        assert_eq!(tail("hé", 5), "hé");
        assert_eq!(tail("hé", 0), "");
        assert_eq!(tail("€€€", 2), "€€");
    }

    #[test]
    fn error_window_is_shared_like_js() {
        // An error more than 2000 characters back is out of the JS window,
        // even when it is within the last five lines.
        let mut a = Analyzer::new();
        assert_eq!(a.append_str("error: boom\n", true), STATUS_ERROR);
        let long = "x".repeat(2100);
        assert_eq!(a.append_str(&format!("{long}\n"), true), STATUS_RUNNING);
    }

    #[test]
    fn waiting_window_is_shared_like_js() {
        // The prompt is 1500 blank characters and a newline back, so only
        // the last 499 characters of its line are in the window: no prompt.
        let mut a = Analyzer::new();
        let line = format!("Continue? (y/n){}\n", " ".repeat(1000));
        let pad = format!("{}\n", " ".repeat(1499));
        a.append_str(&line, true);
        assert_eq!(a.append_str(&pad, true), STATUS_RUNNING);
    }

    #[test]
    fn prompt_found_past_blank_lines() {
        // `analyzeOutput` trims its whole buffer, so blank lines after a
        // prompt still leave the prompt as the last line.
        let mut a = Analyzer::new();
        assert_eq!(
            a.append_str(
                "$ 






",
                true
            ),
            STATUS_WAITING
        );
        // Beyond the 2000-character window it is gone, as in JS.
        let blank = format!("{}\n", " ".repeat(100)).repeat(25);
        assert_eq!(a.append_str(&blank, true), STATUS_RUNNING);
    }
}
