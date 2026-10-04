//! Seeded VT: record logs that look like what shells and full-screen
//! programs print, cut into records at arbitrary byte boundaries.
//!
//! A [`Generator`] strings together *pieces* (a prompt, a coloured `ls`, a
//! redraw of one row, a kitty keyboard push, a resize...) drawn from the
//! families a [`Mix`] weighs, and cuts the byte stream into Data records of
//! random length wherever the cut falls, so escape sequences and UTF-8
//! characters regularly straddle records. Resize records land between
//! records, sometimes in the middle of a sequence, as they do when sessiond
//! takes a resize between two reads.
//!
//! The same seed and [`Profile`] give the same log, byte for byte, on every
//! platform and version: the generator owns its PRNG ([`crate::Rng`]) and
//! iterates nothing unordered.

use std::fmt::Write as _;

use vorn_term_proto::{Cursor, Entry, Record, RecordHeader, Stream};

use crate::log::{Log, Size};
use crate::rng::Rng;

/// When a generated log ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Until {
    /// At least this many bytes of output.
    Bytes(u64),
    /// This many Resize records (and whatever output falls between them).
    Resizes(u32),
}

/// Relative weights of the piece families. Zero leaves a family out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mix {
    /// Lines of words, wide and combining characters, tabs, long lines that
    /// wrap, backspaces and bells.
    pub text: u32,
    /// Prompts with OSC 133 marks and a command's coloured output.
    pub prompt: u32,
    /// SGR: 16, 256 and truecolor, underline styles and colours, resets.
    pub color: u32,
    /// OSC 0 and 2 titles, ended by BEL or ST.
    pub title: u32,
    /// OSC 7 and Vorn's OSC 5522 `cwd;`.
    pub cwd: u32,
    /// OSC 8 hyperlinks around text.
    pub hyperlink: u32,
    /// `\r`-driven progress bars with EL.
    pub progress: u32,
    /// Full-screen drawing: CUP, ED/EL, ICH/DCH/ECH, line drawing charsets.
    pub redraw: u32,
    /// Entering and leaving the alternate screen (1049, 1047, 47).
    pub alt_screen: u32,
    /// DECSTBM scrolling regions and scrolling inside them.
    pub scroll_region: u32,
    /// DECSC/DECRC and SCOSC/SCORC.
    pub save_restore: u32,
    /// DEC private and ANSI modes: DECCKM, bracketed paste, mouse, focus,
    /// cursor visibility, autowrap, insert, sync output, keypad.
    pub modes: u32,
    /// Origin mode (DECOM). Separate because a checkpoint does not restore
    /// it faithfully (see `tests/checkpoint.rs`).
    pub origin: u32,
    /// Left and right margins (DECLRMM + DECSLRM).
    pub margins: u32,
    /// Kitty keyboard flags: push, pop, set.
    pub kitty: u32,
    /// DECSCUSR cursor shapes.
    pub cursor_shape: u32,
    /// Queries a terminal would answer (DA1, DA2, DSR, DECRQM, XTVERSION,
    /// kitty flags, OSC 11, DECRQSS). Replay never writes the answers.
    pub query: u32,
    /// DCS, APC, PM and SOS strings a terminal parses and drops.
    pub strings: u32,
    /// CSI, OSC, DCS and APC sequences cut off part way, the rest never sent.
    pub unterminated: u32,
    /// Sequences aborted by CAN or SUB.
    pub aborted: u32,
    /// RIS, a full reset.
    pub reset: u32,
    /// Resize records.
    pub resize: u32,
}

impl Mix {
    /// Nothing at all; start here and add families.
    pub const NONE: Mix = Mix {
        text: 0,
        prompt: 0,
        color: 0,
        title: 0,
        cwd: 0,
        hyperlink: 0,
        progress: 0,
        redraw: 0,
        alt_screen: 0,
        scroll_region: 0,
        save_restore: 0,
        modes: 0,
        origin: 0,
        margins: 0,
        kitty: 0,
        cursor_shape: 0,
        query: 0,
        strings: 0,
        unterminated: 0,
        aborted: 0,
        reset: 0,
        resize: 0,
    };

    /// A shell: prompts, output, titles, cwds, links, progress bars.
    pub const SHELL: Mix = Mix {
        text: 30,
        prompt: 10,
        color: 15,
        title: 4,
        cwd: 4,
        hyperlink: 3,
        progress: 4,
        query: 1,
        ..Mix::NONE
    };

    /// A full-screen program: the alternate screen, regions, modes, kitty
    /// keyboard flags and redraws.
    pub const FULL_SCREEN: Mix = Mix {
        text: 10,
        color: 10,
        redraw: 25,
        alt_screen: 3,
        scroll_region: 6,
        save_restore: 4,
        modes: 6,
        kitty: 4,
        cursor_shape: 2,
        title: 1,
        query: 1,
        ..Mix::NONE
    };

    /// Everything, edge cases included.
    pub const EVERYTHING: Mix = Mix {
        text: 25,
        prompt: 6,
        color: 12,
        title: 3,
        cwd: 3,
        hyperlink: 2,
        progress: 2,
        redraw: 15,
        alt_screen: 2,
        scroll_region: 4,
        save_restore: 3,
        modes: 4,
        origin: 1,
        margins: 1,
        kitty: 3,
        cursor_shape: 1,
        query: 2,
        strings: 2,
        unterminated: 2,
        aborted: 2,
        reset: 1,
        resize: 2,
    };

    /// The families a checkpoint cut with `Screen::serialize` restores
    /// exactly, as far as the seeded tests find: no alternate screen, saved
    /// cursors, origin mode, margins, kitty keyboard flags, cursor shapes,
    /// cut-off sequences (which turn into arbitrary escapes such as DECSC),
    /// resizes (a soft wrap comes back as a hard line break) or full-screen
    /// redraws (blank cells keep styles the formatter does not write). Each
    /// exception is a named case in `tests/checkpoint.rs`.
    pub const ROUND_TRIP: Mix = Mix {
        redraw: 0,
        alt_screen: 0,
        save_restore: 0,
        origin: 0,
        margins: 0,
        kitty: 0,
        cursor_shape: 0,
        unterminated: 0,
        resize: 0,
        ..Mix::EVERYTHING
    };

    fn weights(&self) -> [(Family, u32); 22] {
        use Family::*;
        [
            (Text, self.text),
            (Prompt, self.prompt),
            (Color, self.color),
            (Title, self.title),
            (Cwd, self.cwd),
            (Hyperlink, self.hyperlink),
            (Progress, self.progress),
            (Redraw, self.redraw),
            (AltScreen, self.alt_screen),
            (ScrollRegion, self.scroll_region),
            (SaveRestore, self.save_restore),
            (Modes, self.modes),
            (Origin, self.origin),
            (Margins, self.margins),
            (Kitty, self.kitty),
            (CursorShape, self.cursor_shape),
            (Query, self.query),
            (Strings, self.strings),
            (Unterminated, self.unterminated),
            (Aborted, self.aborted),
            (Reset, self.reset),
            (Resize, self.resize),
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Text,
    Prompt,
    Color,
    Title,
    Cwd,
    Hyperlink,
    Progress,
    Redraw,
    AltScreen,
    ScrollRegion,
    SaveRestore,
    Modes,
    Origin,
    Margins,
    Kitty,
    CursorShape,
    Query,
    Strings,
    Unterminated,
    Aborted,
    Reset,
    Resize,
}

/// What to generate: the starting size, when to stop, how records are cut
/// and which pieces to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile {
    pub size: Size,
    pub until: Until,
    /// The longest Data record. Records are 1 to this many bytes, half of
    /// them 16 or fewer so splits inside sequences are common.
    pub max_record: usize,
    pub mix: Mix,
    /// Resizes pick a size between these, inclusive.
    pub min_size: Size,
    pub max_size: Size,
    /// Background colours in SGR. A checkpoint loses rows that are blank
    /// but for a background colour at the bottom of the screen (see
    /// `tests/checkpoint.rs`), so the checkpoint tests turn them off.
    pub backgrounds: bool,
}

impl Profile {
    /// Every family, 64 KiB, starting at 80x24.
    pub fn mixed() -> Self {
        Self {
            size: Size::new(80, 24),
            until: Until::Bytes(64 << 10),
            max_record: 4096,
            mix: Mix::EVERYTHING,
            min_size: Size::new(2, 2),
            max_size: Size::new(220, 70),
            backgrounds: true,
        }
    }

    /// What a checkpoint restores exactly: [`Mix::ROUND_TRIP`] without
    /// background colours.
    pub fn round_trip() -> Self {
        Self {
            mix: Mix::ROUND_TRIP,
            backgrounds: false,
            ..Self::mixed()
        }
    }

    pub fn shell() -> Self {
        Self {
            mix: Mix::SHELL,
            ..Self::mixed()
        }
    }

    pub fn full_screen() -> Self {
        Self {
            mix: Mix::FULL_SCREEN,
            ..Self::mixed()
        }
    }

    /// `n` resizes interleaved with full-screen and shell output: about two
    /// pieces of output between resizes.
    pub fn resize_storm(n: u32) -> Self {
        Self {
            until: Until::Resizes(n),
            max_record: 512,
            mix: Mix {
                resize: 30,
                ..Mix::EVERYTHING
            },
            ..Self::mixed()
        }
    }

    /// The same profile, stopping after `n` bytes of output.
    pub fn bytes(self, n: u64) -> Self {
        Self {
            until: Until::Bytes(n),
            ..self
        }
    }

    pub fn mix(self, mix: Mix) -> Self {
        Self { mix, ..self }
    }

    pub fn size(self, size: Size) -> Self {
        Self { size, ..self }
    }
}

/// Generates one session's records; an iterator, so 50 MB need not be held
/// at once.
#[derive(Debug, Clone)]
pub struct Generator {
    rng: Rng,
    profile: Profile,
    total_weight: u64,
    /// The size the program believes it has, for coordinates.
    size: Size,
    /// Output not yet cut into a record.
    pending: Vec<u8>,
    /// Records cut and waiting to be returned, oldest first (at most a few).
    ready: std::collections::VecDeque<Record>,
    next_cut: usize,
    next: Cursor,
    bytes: u64,
    resizes: u32,
    done: bool,
}

impl Generator {
    pub fn new(seed: u64, profile: Profile) -> Self {
        let total_weight = profile
            .mix
            .weights()
            .iter()
            .map(|&(_, w)| u64::from(w))
            .sum();
        let mut g = Self {
            rng: Rng::new(seed),
            profile,
            total_weight,
            size: profile.size,
            pending: Vec::new(),
            ready: Default::default(),
            next_cut: 0,
            next: Cursor::start(0),
            bytes: 0,
            resizes: 0,
            done: false,
        };
        g.next_cut = g.cut_len();
        g
    }

    /// The whole log at once.
    pub fn log(seed: u64, profile: Profile) -> Log {
        let entries = Generator::new(seed, profile).collect();
        Log {
            size: profile.size,
            entries,
        }
    }

    /// `n` independent sessions from one seed: session `i` is the same
    /// whatever `n` is.
    pub fn sessions(seed: u64, n: usize, profile: Profile) -> Vec<Log> {
        (0..n as u64)
            .map(|i| Self::log(Rng::derive(seed, i).next_u64(), profile))
            .collect()
    }

    fn cut_len(&mut self) -> usize {
        let max = self.profile.max_record.max(1) as u64;
        let len = if self.rng.chance(1, 2) {
            self.rng.range(1, max.min(16))
        } else {
            self.rng.range(1, max)
        };
        len as usize
    }

    fn finished(&self) -> bool {
        match self.profile.until {
            Until::Bytes(n) => self.bytes + self.pending.len() as u64 >= n,
            Until::Resizes(n) => self.resizes >= n,
        }
    }

    /// Generate pieces until a record is ready or the log is done.
    fn fill(&mut self) {
        while self.ready.is_empty() && !self.done {
            if self.finished() || self.total_weight == 0 {
                self.flush_all();
                self.done = true;
                return;
            }
            match self.family() {
                Family::Resize => self.resize(),
                f => {
                    let piece = self.piece(f);
                    self.pending.extend_from_slice(&piece);
                    self.cut();
                }
            }
        }
    }

    fn family(&mut self) -> Family {
        let mut at = self.rng.below(self.total_weight);
        for (f, w) in self.profile.mix.weights() {
            let w = u64::from(w);
            if at < w {
                return f;
            }
            at -= w;
        }
        unreachable!("the weights add up to total_weight")
    }

    /// Cut every full record out of `pending`.
    fn cut(&mut self) {
        while self.pending.len() >= self.next_cut {
            let rest = self.pending.split_off(self.next_cut);
            let bytes = std::mem::replace(&mut self.pending, rest);
            self.emit_data(bytes);
            self.next_cut = self.cut_len();
        }
    }

    fn flush_all(&mut self) {
        if !self.pending.is_empty() {
            let bytes = std::mem::take(&mut self.pending);
            self.emit_data(bytes);
        }
    }

    fn emit_data(&mut self, bytes: Vec<u8>) {
        self.bytes += bytes.len() as u64;
        self.ready.push_back(Record::Data {
            stream: Stream::Pty,
            bytes,
        });
    }

    /// A resize between two reads: whatever of `pending` was read before it
    /// goes first, which may end in the middle of a sequence.
    fn resize(&mut self) {
        let before = self.rng.index(self.pending.len() + 1);
        if before > 0 {
            let rest = self.pending.split_off(before);
            let bytes = std::mem::replace(&mut self.pending, rest);
            self.emit_data(bytes);
        }
        let (lo, hi) = (self.profile.min_size, self.profile.max_size);
        let cols = self
            .rng
            .range(u64::from(lo.cols.max(1)), u64::from(hi.cols.max(lo.cols)));
        let rows = self
            .rng
            .range(u64::from(lo.rows.max(1)), u64::from(hi.rows.max(lo.rows)));
        // Both are within u16 bounds by construction.
        let size = Size::new(cols as u16, rows as u16);
        self.size = size;
        self.resizes += 1;
        self.ready.push_back(Record::Resize {
            cols: size.cols,
            rows: size.rows,
            px_w: 0,
            px_h: 0,
            req: None,
        });
    }

    fn piece(&mut self, f: Family) -> Vec<u8> {
        let mut s = String::new();
        match f {
            Family::Text => self.text(&mut s),
            Family::Prompt => self.prompt(&mut s),
            Family::Color => {
                let sgr = self.sgr();
                s.push_str(&sgr);
                s.push_str(self.rng.pick(WORDS));
            }
            Family::Title => {
                let num = self.rng.pick(&["0", "2"]);
                let title = self.rng.pick(TITLES);
                let st = self.st();
                let _ = write!(s, "\x1b]{num};{title}{st}");
            }
            Family::Cwd => {
                let path = self.rng.pick(PATHS);
                let st = self.st();
                if self.rng.chance(1, 2) {
                    let _ = write!(s, "\x1b]7;file://box{}{st}", path.replace(' ', "%20"));
                } else {
                    let _ = write!(s, "\x1b]5522;cwd;{path}{st}");
                }
            }
            Family::Hyperlink => {
                let st = self.st();
                let id = if self.rng.chance(1, 2) { "id=x1" } else { "" };
                let url = self
                    .rng
                    .pick(&["https://example.com/a", "file:///tmp/x.txt"]);
                let text = self.rng.pick(WORDS);
                let _ = write!(s, "\x1b]8;{id};{url}{st}{text}\x1b]8;;{st}");
            }
            Family::Progress => {
                let width = 1 + self.rng.below(30);
                for pct in [0u64, 25, 50, 75, 100] {
                    let done = (width * pct / 100) as usize;
                    let _ = write!(
                        s,
                        "\r[{}{}] {pct:>3}%\x1b[K",
                        "#".repeat(done),
                        " ".repeat(width as usize - done)
                    );
                }
                s.push_str("\r\n");
            }
            Family::Redraw => self.redraw(&mut s),
            Family::AltScreen => {
                let seq = self.rng.pick(&[
                    "\x1b[?1049h\x1b[H\x1b[2J",
                    "\x1b[?1049l",
                    "\x1b[?1049h",
                    "\x1b[?1047h",
                    "\x1b[?1047l",
                    "\x1b[?47h",
                    "\x1b[?47l",
                ]);
                s.push_str(seq);
            }
            Family::ScrollRegion => self.region(&mut s),
            Family::SaveRestore => {
                s.push_str(self.rng.pick(&["\x1b7", "\x1b8", "\x1b[s", "\x1b[u"]));
            }
            Family::Modes => {
                let mode = self.rng.pick(&[
                    "?1", "?25", "?7", "?12", "?1000", "?1002", "?1003", "?1004", "?1006", "?2004",
                    "?2026", "?1007", "?1036", "?45", "4", "20",
                ]);
                let on = if self.rng.chance(1, 2) { 'h' } else { 'l' };
                let _ = write!(s, "\x1b[{mode}{on}");
                if self.rng.chance(1, 8) {
                    s.push_str(self.rng.pick(&["\x1b=", "\x1b>"]));
                }
            }
            Family::Origin => {
                let on = if self.rng.chance(1, 2) { 'h' } else { 'l' };
                let _ = write!(s, "\x1b[?6{on}");
            }
            Family::Margins => {
                let cols = u64::from(self.size.cols);
                if self.rng.chance(1, 3) {
                    s.push_str("\x1b[?69l");
                } else if cols >= 2 {
                    let left = 1 + self.rng.below(cols / 2);
                    let right = self.rng.range(left + 1, cols.max(left + 1));
                    let _ = write!(s, "\x1b[?69h\x1b[{left};{right}s");
                }
            }
            Family::Kitty => match self.rng.below(4) {
                0 | 1 => {
                    let _ = write!(s, "\x1b[>{}u", self.rng.below(32));
                }
                2 => {
                    if self.rng.chance(1, 2) {
                        s.push_str("\x1b[<u");
                    } else {
                        let _ = write!(s, "\x1b[<{}u", 1 + self.rng.below(3));
                    }
                }
                _ => {
                    let _ = write!(s, "\x1b[={};{}u", self.rng.below(32), 1 + self.rng.below(3));
                }
            },
            Family::CursorShape => {
                let _ = write!(s, "\x1b[{} q", self.rng.below(7));
            }
            Family::Query => s.push_str(self.rng.pick(QUERIES)),
            Family::Strings => {
                let body = self.rng.pick(&[
                    "\x1bP+q544e",
                    "\x1bP1$r0m",
                    "\x1b_hello",
                    "\x1b^pm",
                    "\x1bXsos",
                ]);
                let mut b = body.as_bytes().to_vec();
                // ST as ESC \ or as the one C1 byte 0x9c, which is not UTF-8
                // and so cannot go through the `String` the other pieces use.
                if self.rng.chance(1, 2) {
                    b.extend_from_slice(b"\x1b\\");
                } else {
                    b.push(0x9c);
                }
                return b;
            }
            Family::Unterminated => {
                let whole = self.rng.pick(UNTERMINATED).as_bytes();
                let keep = self.rng.range(1, whole.len() as u64 - 1) as usize;
                return whole[..keep].to_vec();
            }
            Family::Aborted => {
                let whole = self.rng.pick(UNTERMINATED).as_bytes();
                let keep = self.rng.range(1, whole.len() as u64 - 1) as usize;
                let mut b = whole[..keep].to_vec();
                b.push(if self.rng.chance(1, 2) { 0x18 } else { 0x1a });
                return b;
            }
            Family::Reset => s.push_str("\x1bc"),
            Family::Resize => unreachable!("resizes are records, not bytes"),
        }
        s.into_bytes()
    }

    fn st(&mut self) -> &'static str {
        if self.rng.chance(1, 2) {
            "\x07"
        } else {
            "\x1b\\"
        }
    }

    fn sgr(&mut self) -> String {
        let bg = self.profile.backgrounds;
        match self.rng.below(6) {
            0 => format!("\x1b[{}m", self.rng.pick(SGR)),
            1 => format!("\x1b[38;5;{}m", self.rng.below(256)),
            2 if bg => format!("\x1b[48;5;{}m", self.rng.below(256)),
            3 => format!(
                "\x1b[38;2;{};{};{}m",
                self.rng.below(256),
                self.rng.below(256),
                self.rng.below(256)
            ),
            4 => format!("\x1b[{}m", 30 + self.rng.below(8) + 60 * self.rng.below(2)),
            _ => "\x1b[0m".to_owned(),
        }
    }

    fn text(&mut self, s: &mut String) {
        match self.rng.below(8) {
            // A line longer than the screen, so it wraps.
            0 => {
                let n = usize::from(self.size.cols) + self.rng.index(40);
                for i in 0..n {
                    s.push(char::from(b'a' + (i % 26) as u8));
                }
                s.push_str("\r\n");
            }
            1 => s.push_str(self.rng.pick(&["\x07", "\x08", "\t", "x\x08y", "\r"])),
            _ => {
                let words = 1 + self.rng.below(8);
                for i in 0..words {
                    if i > 0 {
                        s.push(' ');
                    }
                    s.push_str(self.rng.pick(WORDS));
                }
                if self.rng.chance(2, 3) {
                    s.push_str("\r\n");
                }
            }
        }
    }

    fn prompt(&mut self, s: &mut String) {
        let st = self.st();
        let dir = self.rng.pick(&["~", "~/src/vorn", "/tmp"]);
        let cmd = self.rng.pick(&[
            "ls --color",
            "git status",
            "cargo test",
            "echo hi",
            "cat notes.md",
        ]);
        let _ = write!(
            s,
            "\x1b]133;A{st}\x1b[1;32muser@box\x1b[0m:\x1b[1;34m{dir}\x1b[0m$ \x1b]133;B{st}{cmd}\r\n\x1b]133;C{st}"
        );
        for _ in 0..self.rng.below(6) {
            let sgr = self.sgr();
            let word = self.rng.pick(WORDS);
            let _ = write!(s, "{sgr}{word}\x1b[0m  {}\r\n", self.rng.pick(WORDS));
        }
        let _ = write!(s, "\x1b]133;D;{}{st}", self.rng.below(3));
    }

    fn redraw(&mut self, s: &mut String) {
        let (cols, rows) = (u64::from(self.size.cols), u64::from(self.size.rows));
        match self.rng.below(6) {
            0 | 1 => {
                let _ = write!(
                    s,
                    "\x1b[{};{}H{}",
                    1 + self.rng.below(rows),
                    1 + self.rng.below(cols),
                    self.rng.pick(WORDS)
                );
            }
            2 => s.push_str(self.rng.pick(&[
                "\x1b[K", "\x1b[1K", "\x1b[2K", "\x1b[J", "\x1b[1J", "\x1b[2J",
            ])),
            3 => s.push_str(self.rng.pick(&[
                "\x1b[3@", "\x1b[2P", "\x1b[4X", "\x1b[2A", "\x1b[3C", "\x1b[B", "\x1b[5D",
                "\x1b[3G", "\x1b[2d",
            ])),
            4 => s.push_str("\x1b(0lqqk\r\nx  x\r\nmqqj\x1b(B"),
            _ => {
                // A whole row of a status line.
                let row = 1 + self.rng.below(rows);
                let _ = write!(
                    s,
                    "\x1b[{row};1H\x1b[7m{}\x1b[0m",
                    " ".repeat(cols as usize)
                );
            }
        }
    }

    fn region(&mut self, s: &mut String) {
        let rows = u64::from(self.size.rows);
        match self.rng.below(4) {
            0 => s.push_str("\x1b[r"),
            1 if rows >= 2 => {
                let top = 1 + self.rng.below(rows - 1);
                let bottom = self.rng.range(top + 1, rows);
                let _ = write!(s, "\x1b[{top};{bottom}r");
            }
            _ => s.push_str(self.rng.pick(&[
                "\x1bD", "\x1bM", "\x1b[2S", "\x1b[T", "\x1b[2L", "\x1b[M", "\n\n\n",
            ])),
        }
    }
}

impl Iterator for Generator {
    type Item = Entry;

    fn next(&mut self) -> Option<Entry> {
        self.fill();
        let rec = self.ready.pop_front()?;
        let hdr = RecordHeader {
            epoch: self.next.epoch,
            rseq: self.next.next_rseq,
            start_offset: self.next.next_offset,
        };
        self.next = hdr.after(&rec);
        Some(Entry { hdr, at_ns: 0, rec })
    }
}

const WORDS: &[&str] = &[
    "ls",
    "-la",
    "error:",
    "warning",
    "✓ done",
    "日本語",
    "e\u{301}",
    "👩\u{200d}👩\u{200d}👧",
    "Ünïcödé",
    "→",
    "€42",
    "src/main.rs",
    "x",
    "  ",
    "ok",
    "FAILED",
    "😀",
    "한국어",
    "a\u{308}",
    "▓▒░",
];

const TITLES: &[&str] = &[
    "vim",
    "htop",
    "✳ claude",
    "~/src",
    "build: 3/7",
    "",
    "日本語 title",
];

const PATHS: &[&str] = &[
    "/home/user",
    "/tmp",
    "/srv/my dir",
    "/var/log",
    "C:/Users/x",
];

const SGR: &[&str] = &[
    "1",
    "2",
    "3",
    "4",
    "4:3",
    "5",
    "7",
    "8",
    "9",
    "53",
    "0",
    "22",
    "39",
    "49",
    "58;5;9",
    "58;2;1;2;3",
    "1;4;31",
];

const QUERIES: &[&str] = &[
    "\x1b[c",
    "\x1b[>c",
    "\x1b[6n",
    "\x1b[5n",
    "\x1b[?2004$p",
    "\x1b[?u",
    "\x1b[>q",
    "\x1b]11;?\x07",
    "\x1bP$qm\x1b\\",
    "\x1b[18t",
    "\x1b[14t",
];

/// Whole sequences that the unterminated and aborted families cut short.
const UNTERMINATED: &[&str] = &[
    "\x1b[38;2;10;20;30m",
    "\x1b[?1049h",
    "\x1b[12;40H",
    "\x1b]2;a title that never ends\x07",
    "\x1b]8;;https://example.com\x1b\\",
    "\x1bP+q544e\x1b\\",
    "\x1b_payload\x1b\\",
    "\x1b[>1u",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_family_draws_a_piece() {
        // Each family on its own, so a family that produced nothing or
        // panicked would show here rather than as a skewed mix.
        let all = Mix::EVERYTHING;
        for (family, _) in all.weights() {
            let mut mix = Mix::NONE;
            match family {
                Family::Resize => {
                    mix.resize = 1;
                    mix.text = 1;
                }
                _ => {
                    mix.text = 0;
                    mix = set(mix, family);
                }
            }
            let log = Generator::log(1, Profile::mixed().mix(mix).bytes(512));
            assert!(!log.entries.is_empty(), "{family:?}");
            log.validate().unwrap();
        }
    }

    fn set(mut m: Mix, f: Family) -> Mix {
        let w = match f {
            Family::Text => &mut m.text,
            Family::Prompt => &mut m.prompt,
            Family::Color => &mut m.color,
            Family::Title => &mut m.title,
            Family::Cwd => &mut m.cwd,
            Family::Hyperlink => &mut m.hyperlink,
            Family::Progress => &mut m.progress,
            Family::Redraw => &mut m.redraw,
            Family::AltScreen => &mut m.alt_screen,
            Family::ScrollRegion => &mut m.scroll_region,
            Family::SaveRestore => &mut m.save_restore,
            Family::Modes => &mut m.modes,
            Family::Origin => &mut m.origin,
            Family::Margins => &mut m.margins,
            Family::Kitty => &mut m.kitty,
            Family::CursorShape => &mut m.cursor_shape,
            Family::Query => &mut m.query,
            Family::Strings => &mut m.strings,
            Family::Unterminated => &mut m.unterminated,
            Family::Aborted => &mut m.aborted,
            Family::Reset => &mut m.reset,
            Family::Resize => &mut m.resize,
        };
        *w = 1;
        m
    }

    #[test]
    fn an_empty_mix_ends_at_once() {
        let log = Generator::log(1, Profile::mixed().mix(Mix::NONE));
        assert!(log.entries.is_empty());
    }
}
