//! A byte-for-byte model of Ghostty's VT parser: its state machine
//! (`parse_table.zig`), its UTF-8 decoder (`UTF8Decoder.zig`), and the
//! dispatches the terminal acts on.
//!
//! It runs beside the real terminal over the same bytes, for two reasons.
//! A checkpoint may only be cut where Ghostty's parser is in its ground state
//! with no UTF-8 sequence open, and Ghostty does not say where it is. And a
//! few sequences change state the terminal cannot be asked about afterwards
//! (a saved cursor, the Kitty keyboard stack, the screen being left), so the
//! caller has to see them as they pass. It never interprets the stream
//! itself: it only says which sequence ended at which byte.
//!
//! The model follows Ghostty's tables rather than the vt100.net diagram it
//! descends from, where the two differ: C1 bytes inside an OSC string are
//! payload, a C1 control executed outside the ground state is an `ESC`
//! dispatch, and in the ground state bytes are UTF-8, never C1 controls.

/// Ghostty's limits: a CSI with more parameters is dropped, and
/// intermediates past the fourth are discarded.
pub(crate) const MAX_PARAMS: usize = 24;
const MAX_INTERMEDIATE: usize = 4;
/// How much of one OSC payload is kept for the caller. Ghostty keeps more;
/// the caller only reads short ones (titles, cwds, notifications, OSC 133).
const MAX_OSC: usize = 16 * 1024;
/// How many recently printed codepoints are remembered for REP's model.
const RECENT: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    Escape,
    EscapeIntermediate,
    CsiEntry,
    CsiParam,
    CsiIntermediate,
    CsiIgnore,
    DcsEntry,
    DcsParam,
    DcsIntermediate,
    DcsPassthrough,
    DcsIgnore,
    OscString,
    SosPmApcString,
}

/// What a transition does besides changing state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    None,
    Execute,
    Collect,
    Param,
    EscDispatch,
    CsiDispatch,
    OscPut,
}

/// A sequence Ghostty dispatched, reported at the byte that ended it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Event<'a> {
    /// A C0 control the terminal executes that a caller tracks: LF, VT, FF
    /// (which index the cursor) and SO, SI (which shift the character set).
    Execute(u8),
    /// An escape sequence, or a C1 control executed as one.
    Esc { inter: &'a [u8], fin: u8 },
    Csi {
        inter: &'a [u8],
        params: &'a [u16],
        fin: u8,
    },
    /// An OSC string ended, by any byte: Ghostty dispatches on every exit.
    Osc(&'a [u8]),
    /// An APC, SOS or PM string started.
    ApcStart,
    Dcs {
        inter: &'a [u8],
        params: &'a [u16],
        fin: u8,
    },
}

/// Whether a printed codepoint may join the cell before it (a combining
/// mark, a joiner, a variation selector, an emoji modifier) instead of
/// taking a cell of its own: a cheap filter before asking Ghostty's width
/// tables. Conservative: everything from U+0300 on except the blocks
/// terminal programs print most (box drawing, braille spinners, symbols,
/// CJK, Hangul syllables, emoji pictographs, private use), none of which
/// holds a zero-width character.
pub(crate) fn may_join(cp: u32) -> bool {
    if cp < 0x300 {
        return false;
    }
    let plain = matches!(cp,
        0x2010..=0x2027
            | 0x2030..=0x205e
            | 0x2070..=0x20cf
            | 0x2100..=0x2bff
            | 0x2e00..=0x2e7f
            | 0x3000..=0x3029
            | 0x3030..=0x3098
            | 0x309b..=0x9fff
            | 0xac00..=0xd7a3
            | 0xe000..=0xf8ff
            | 0xff00..=0xffef
            | 0x1f000..=0x1f3fa
            | 0x1f400..=0x1faff
            | 0x20000..=0x3ffff);
    !plain
}

/// The last few codepoints printed, newest last, for the caller's model of
/// what REP repeats.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Recent {
    cps: [u32; RECENT],
    len: u8,
}

impl Recent {
    fn push(&mut self, cp: u32) {
        if usize::from(self.len) == RECENT {
            self.cps.copy_within(1.., 0);
            self.cps[RECENT - 1] = cp;
        } else {
            self.cps[usize::from(self.len)] = cp;
            self.len += 1;
        }
    }

    /// Newest first.
    pub(crate) fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.cps[..usize::from(self.len)].iter().rev().copied()
    }

    /// Whether more were printed than are remembered.
    pub(crate) fn is_full(&self) -> bool {
        usize::from(self.len) == RECENT
    }

    pub(crate) fn clear(&mut self) {
        self.len = 0;
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Parser {
    state: State,
    utf8_state: u8,
    utf8_acc: u32,
    params: [u16; MAX_PARAMS],
    n_params: usize,
    acc: u16,
    acc_digits: u8,
    inter: [u8; MAX_INTERMEDIATE],
    n_inter: usize,
    osc: Vec<u8>,
    /// Codepoints printed since the caller last cleared it.
    pub(crate) recent: Recent,
    /// How many codepoints have been printed in all, so a caller can tell
    /// whether anything was printed between two points.
    pub(crate) printed: u64,
    /// The highest codepoint printed since the caller last reset it.
    pub(crate) printed_max: u32,
    /// Distinct codepoints printed since the caller last took them that may
    /// have joined the cell before them rather than taking one of their own
    /// (see [`may_join`]), at most [`JOIN_CANDIDATES`]...
    pub(crate) join_candidates: Vec<u32>,
    /// ...and whether more were printed than that.
    pub(crate) join_overflow: bool,
}

/// How many distinct join candidates are kept between two takes.
pub(crate) const JOIN_CANDIDATES: usize = 16;

impl Default for Parser {
    fn default() -> Self {
        Self {
            state: State::Ground,
            utf8_state: UTF8_ACCEPT,
            utf8_acc: 0,
            params: [0; MAX_PARAMS],
            n_params: 0,
            acc: 0,
            acc_digits: 0,
            inter: [0; MAX_INTERMEDIATE],
            n_inter: 0,
            osc: Vec::new(),
            recent: Recent::default(),
            printed: 0,
            printed_max: 0,
            join_candidates: Vec::new(),
            join_overflow: false,
        }
    }
}

/// What a step reported: the bytes it consumed, and the event the last of
/// them ended, if any.
pub(crate) struct Step<'a> {
    pub consumed: usize,
    pub event: Option<Event<'a>>,
}

impl Parser {
    /// In the ground state with no UTF-8 sequence open: the only place a
    /// checkpoint may be cut.
    pub fn at_ground(&self) -> bool {
        self.state == State::Ground && self.utf8_state == UTF8_ACCEPT
    }

    /// Consumes `bytes` up to and including the first byte that ends an
    /// event, or all of them.
    pub fn step<'s>(&'s mut self, bytes: &[u8]) -> Step<'s> {
        let mut i = 0;
        while i < bytes.len() {
            if self.state == State::Ground {
                match self.ground(&bytes[i..]) {
                    Ground::Ran(n) => i += n,
                    Ground::Execute(n, c) => {
                        return Step {
                            consumed: i + n,
                            event: Some(Event::Execute(c)),
                        }
                    }
                }
                continue;
            }
            let c = bytes[i];
            i += 1;
            if let Some(kind) = self.next_non_ground(c) {
                return Step {
                    consumed: i,
                    event: Some(self.event(kind, c)),
                };
            }
        }
        Step {
            consumed: bytes.len(),
            event: None,
        }
    }

    /// Runs the ground state over `bytes` until it leaves it or a tracked
    /// C0 control executes.
    fn ground(&mut self, bytes: &[u8]) -> Ground {
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if self.utf8_state == UTF8_ACCEPT && c < 0x80 {
                // ASCII: printable from 0x10, as Ghostty prints every
                // codepoint above 0x0F that is not ESC.
                i += 1;
                match c {
                    0x1b => {
                        self.enter_escape();
                        return Ground::Ran(i);
                    }
                    0x0a..=0x0c | 0x0e | 0x0f => return Ground::Execute(i, c),
                    0x00..=0x0f => {}
                    _ => self.print(u32::from(c)),
                }
                continue;
            }
            // Ghostty's decoder: a byte that breaks an open sequence emits a
            // replacement character and is then decoded again by itself.
            let (cp, consumed) = utf8_next(&mut self.utf8_state, &mut self.utf8_acc, c);
            if let Some(cp) = cp {
                self.print(cp);
            }
            if consumed {
                i += 1;
            }
        }
        Ground::Ran(i)
    }

    fn print(&mut self, cp: u32) {
        self.recent.push(cp);
        self.printed += 1;
        self.printed_max = self.printed_max.max(cp);
        if may_join(cp) && !self.join_candidates.contains(&cp) {
            if self.join_candidates.len() < JOIN_CANDIDATES {
                self.join_candidates.push(cp);
            } else {
                self.join_overflow = true;
            }
        }
    }

    fn enter_escape(&mut self) {
        self.state = State::Escape;
        self.clear();
    }

    fn clear(&mut self) {
        self.n_params = 0;
        self.acc = 0;
        self.acc_digits = 0;
        self.n_inter = 0;
    }

    /// One byte outside the ground state. Returns what it dispatched.
    fn next_non_ground(&mut self, c: u8) -> Option<Dispatch> {
        let (next, action) = transition(self.state, c);
        let mut dispatch = None;
        // The exit action. dcs_unhook and apc_end change nothing a caller
        // tracks.
        if next != self.state && self.state == State::OscString {
            dispatch = Some(Dispatch::Osc);
        }
        match action {
            Action::None => {}
            Action::Execute => {
                if c > 0x7f {
                    dispatch = Some(Dispatch::C1);
                } else if matches!(c, 0x0a..=0x0c | 0x0e | 0x0f) {
                    dispatch = Some(Dispatch::Execute);
                }
            }
            Action::Collect => {
                if self.n_inter < MAX_INTERMEDIATE {
                    self.inter[self.n_inter] = c;
                    self.n_inter += 1;
                }
            }
            Action::Param => {
                if c == b';' || c == b':' {
                    if self.n_params < MAX_PARAMS {
                        self.params[self.n_params] = self.acc;
                        self.n_params += 1;
                        self.acc = 0;
                        self.acc_digits = 0;
                    }
                } else {
                    self.acc = self
                        .acc
                        .saturating_mul(10)
                        .saturating_add(u16::from(c - b'0'));
                    self.acc_digits = self.acc_digits.wrapping_add(1);
                }
            }
            Action::OscPut => {
                if self.osc.len() < MAX_OSC {
                    self.osc.push(c);
                }
            }
            Action::EscDispatch => dispatch = Some(Dispatch::Esc),
            Action::CsiDispatch => {
                if self.n_params < MAX_PARAMS {
                    self.finish_params();
                    dispatch = Some(Dispatch::Csi);
                }
            }
        }
        if next != self.state {
            // Entry action.
            match next {
                State::Escape | State::CsiEntry | State::DcsEntry => self.clear(),
                State::OscString => self.osc.clear(),
                State::DcsPassthrough => {
                    if self.n_params < MAX_PARAMS {
                        self.finish_params();
                        // The exit dispatch (an OSC ended by this byte)
                        // cannot coincide: OSC strings never enter DCS.
                        dispatch = Some(Dispatch::Dcs);
                    }
                }
                State::SosPmApcString => dispatch = Some(Dispatch::Apc),
                _ => {}
            }
        }
        // The OSC that a byte ended is reported in place of what that byte
        // starts: the only starts it could coincide with are ESC (escape) and
        // C1 openers, which report nothing at their first byte.
        self.state = next;
        dispatch
    }

    fn finish_params(&mut self) {
        if self.acc_digits > 0 {
            self.params[self.n_params] = self.acc;
            self.n_params += 1;
        }
    }

    fn event(&self, kind: Dispatch, c: u8) -> Event<'_> {
        match kind {
            Dispatch::Execute => Event::Execute(c),
            Dispatch::C1 => Event::Esc {
                inter: &[],
                fin: c - 0x40,
            },
            Dispatch::Esc => Event::Esc {
                inter: &self.inter[..self.n_inter],
                fin: c,
            },
            Dispatch::Csi => Event::Csi {
                inter: &self.inter[..self.n_inter],
                params: &self.params[..self.n_params],
                fin: c,
            },
            Dispatch::Dcs => Event::Dcs {
                inter: &self.inter[..self.n_inter],
                params: &self.params[..self.n_params],
                fin: c,
            },
            Dispatch::Osc => Event::Osc(&self.osc),
            Dispatch::Apc => Event::ApcStart,
        }
    }
}

enum Ground {
    /// Consumed this many bytes without an event.
    Ran(usize),
    /// Consumed this many bytes, the last a tracked C0 control.
    Execute(usize, u8),
}

#[derive(Debug, Clone, Copy)]
enum Dispatch {
    Execute,
    C1,
    Esc,
    Csi,
    Dcs,
    Osc,
    Apc,
}

/// Ghostty's `parse_table.zig`: the next state and action for a byte
/// outside the ground state.
fn transition(state: State, c: u8) -> (State, Action) {
    use Action as A;
    use State as S;
    // Anywhere transitions, which every state's own entries override only
    // where Ghostty's table does (OSC strings take 0x20..=0xFF as payload).
    let anywhere = match c {
        0x18 | 0x1a => Some((S::Ground, A::Execute)),
        0x80..=0x8f | 0x91..=0x97 | 0x99 | 0x9a => Some((S::Ground, A::Execute)),
        0x9c => Some((S::Ground, A::None)),
        0x1b => Some((S::Escape, A::None)),
        0x98 | 0x9e | 0x9f => Some((S::SosPmApcString, A::None)),
        0x9b => Some((S::CsiEntry, A::None)),
        0x90 => Some((S::DcsEntry, A::None)),
        0x9d => Some((S::OscString, A::None)),
        _ => None,
    };
    let c0 = matches!(c, 0x00..=0x17 | 0x19 | 0x1c..=0x1f);
    let own = match state {
        S::Ground => unreachable!("the ground state is decoded as UTF-8"),
        S::EscapeIntermediate => match c {
            _ if c0 => Some((state, A::Execute)),
            0x20..=0x2f => Some((state, A::Collect)),
            0x7f => Some((state, A::None)),
            0x30..=0x7e => Some((S::Ground, A::EscDispatch)),
            _ => None,
        },
        S::SosPmApcString => match c {
            _ if c0 => Some((state, A::None)),
            0x20..=0x7f => Some((state, A::None)),
            _ => None,
        },
        S::Escape => match c {
            _ if c0 => Some((state, A::Execute)),
            0x7f => Some((state, A::None)),
            0x30..=0x4f | 0x51..=0x57 | 0x59 | 0x5a | 0x5c | 0x60..=0x7e => {
                Some((S::Ground, A::EscDispatch))
            }
            0x20..=0x2f => Some((S::EscapeIntermediate, A::Collect)),
            0x58 | 0x5e | 0x5f => Some((S::SosPmApcString, A::None)),
            0x50 => Some((S::DcsEntry, A::None)),
            0x5b => Some((S::CsiEntry, A::None)),
            0x5d => Some((S::OscString, A::None)),
            _ => None,
        },
        S::DcsEntry => match c {
            _ if c0 => Some((state, A::None)),
            0x7f => Some((state, A::None)),
            0x20..=0x2f => Some((S::DcsIntermediate, A::Collect)),
            0x3a => Some((S::DcsIgnore, A::None)),
            0x30..=0x39 | 0x3b => Some((S::DcsParam, A::Param)),
            0x3c..=0x3f => Some((S::DcsParam, A::Collect)),
            0x40..=0x7e => Some((S::DcsPassthrough, A::None)),
            _ => None,
        },
        S::DcsIntermediate => match c {
            _ if c0 => Some((state, A::None)),
            0x20..=0x2f => Some((state, A::Collect)),
            0x7f => Some((state, A::None)),
            0x30..=0x3f => Some((S::DcsIgnore, A::None)),
            0x40..=0x7e => Some((S::DcsPassthrough, A::None)),
            _ => None,
        },
        S::DcsIgnore => match c {
            _ if c0 => Some((state, A::None)),
            _ => None,
        },
        S::DcsParam => match c {
            _ if c0 => Some((state, A::None)),
            0x30..=0x39 | 0x3b => Some((state, A::Param)),
            0x7f => Some((state, A::None)),
            0x3a | 0x3c..=0x3f => Some((S::DcsIgnore, A::None)),
            0x20..=0x2f => Some((S::DcsIntermediate, A::Collect)),
            0x40..=0x7e => Some((S::DcsPassthrough, A::None)),
            _ => None,
        },
        S::DcsPassthrough => match c {
            _ if c0 => Some((state, A::None)),
            0x20..=0x7e => Some((state, A::None)),
            0x7f => Some((state, A::None)),
            _ => None,
        },
        S::CsiParam => match c {
            _ if c0 => Some((state, A::Execute)),
            0x30..=0x3b => Some((state, A::Param)),
            0x7f => Some((state, A::None)),
            0x40..=0x7e => Some((S::Ground, A::CsiDispatch)),
            0x3c..=0x3f => Some((S::CsiIgnore, A::None)),
            0x20..=0x2f => Some((S::CsiIntermediate, A::Collect)),
            _ => None,
        },
        S::CsiIgnore => match c {
            _ if c0 => Some((state, A::Execute)),
            0x20..=0x3f | 0x7f => Some((state, A::None)),
            0x40..=0x7e => Some((S::Ground, A::None)),
            _ => None,
        },
        S::CsiIntermediate => match c {
            _ if c0 => Some((state, A::Execute)),
            0x20..=0x2f => Some((state, A::Collect)),
            0x7f => Some((state, A::None)),
            0x40..=0x7e => Some((S::Ground, A::CsiDispatch)),
            0x30..=0x3f => Some((S::CsiIgnore, A::None)),
            _ => None,
        },
        S::CsiEntry => match c {
            _ if c0 => Some((state, A::Execute)),
            0x7f => Some((state, A::None)),
            0x40..=0x7e => Some((S::Ground, A::CsiDispatch)),
            0x3a => Some((S::CsiIgnore, A::None)),
            0x20..=0x2f => Some((S::CsiIntermediate, A::Collect)),
            0x30..=0x39 | 0x3b => Some((S::CsiParam, A::Param)),
            0x3c..=0x3f => Some((S::CsiParam, A::Collect)),
            _ => None,
        },
        S::OscString => match c {
            0x07 => Some((S::Ground, A::None)),
            _ if c0 => Some((state, A::None)),
            0x20..=0xff => Some((state, A::OscPut)),
            _ => None,
        },
    };
    own.or(anywhere).unwrap_or((state, A::None))
}

const UTF8_ACCEPT: u8 = 0;
const UTF8_REJECT: u8 = 12;

#[rustfmt::skip]
const UTF8_CLASSES: [u8; 256] = [
   0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,  0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
   0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,  0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
   0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,  0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
   0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,  0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
   1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,  9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,
   7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,  7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,
   8,8,2,2,2,2,2,2,2,2,2,2,2,2,2,2,  2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,
  10,3,3,3,3,3,3,3,3,3,3,3,3,4,3,3, 11,6,6,6,5,8,8,8,8,8,8,8,8,8,8,8,
];

#[rustfmt::skip]
const UTF8_TRANSITIONS: [u8; 108] = [
   0,12,24,36,60,96,84,12,12,12,48,72, 12,12,12,12,12,12,12,12,12,12,12,12,
  12, 0,12,12,12,12,12, 0,12, 0,12,12, 12,24,12,12,12,12,12,24,12,24,12,12,
  12,12,12,12,12,12,12,24,12,12,12,12, 12,24,12,12,12,12,12,12,12,24,12,12,
  12,12,12,12,12,12,12,36,12,36,12,12, 12,36,12,12,12,12,12,36,12,36,12,12,
  12,36,12,12,12,12,12,12,12,12,12,12,
];

/// One byte through Ghostty's decoder: the codepoint it completed (a
/// replacement character for an ill-formed sequence), and whether the byte
/// was consumed.
fn utf8_next(state: &mut u8, acc: &mut u32, byte: u8) -> (Option<u32>, bool) {
    let class = UTF8_CLASSES[usize::from(byte)];
    let initial = *state;
    *acc = if initial != UTF8_ACCEPT {
        (*acc << 6) | u32::from(byte & 0x3f)
    } else {
        (0xff_u32 >> class) & u32::from(byte)
    };
    *state = UTF8_TRANSITIONS[usize::from(initial + class)];
    match *state {
        UTF8_ACCEPT => (Some(std::mem::take(acc)), true),
        UTF8_REJECT => {
            *acc = 0;
            *state = UTF8_ACCEPT;
            (Some(0xfffd), initial == UTF8_ACCEPT)
        }
        _ => (None, true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every event in `bytes`, fed in chunks of `chunk`, with whether the
    /// parser was at ground after each.
    fn events(bytes: &[u8], chunk: usize) -> Vec<String> {
        let mut p = Parser::default();
        let mut out = Vec::new();
        for part in bytes.chunks(chunk) {
            let mut at = 0;
            while at < part.len() {
                let step = p.step(&part[at..]);
                at += step.consumed;
                if let Some(e) = step.event {
                    out.push(format!("{e:?}"));
                }
            }
        }
        out.push(format!("ground={}", p.at_ground()));
        out
    }

    #[test]
    fn sequences_and_ground() {
        let got = events(b"a\x1b[?1049h\x1b7\x1b]133;A\x07\nx\x1b[>1u", 64);
        assert_eq!(
            got,
            [
                r#"Csi { inter: [63], params: [1049], fin: 104 }"#,
                r#"Esc { inter: [], fin: 55 }"#,
                r#"Osc([49, 51, 51, 59, 65])"#,
                r#"Execute(10)"#,
                r#"Csi { inter: [62], params: [1], fin: 117 }"#,
                "ground=true",
            ]
        );
        // Split anywhere, the same events come out.
        let whole = events(b"\x1b]2;t\x1b\\\x1b(0q\x0e\xe2\x94\x80\x1b[1;2r", 64);
        for n in 1..8 {
            assert_eq!(
                events(b"\x1b]2;t\x1b\\\x1b(0q\x0e\xe2\x94\x80\x1b[1;2r", n),
                whole
            );
        }
    }

    #[test]
    fn open_sequences_are_not_ground() {
        for open in [
            &b"\x1b"[..],
            b"\x1b[1",
            b"\x1b]2;x",
            b"\x1bP1",
            b"\xe2\x94",
            b"\x1b_G",
        ] {
            let got = events(open, 64);
            assert_eq!(got.last().unwrap(), "ground=false", "{open:?}");
        }
        // A broken UTF-8 sequence resolves at the next byte.
        assert_eq!(events(b"\xe2x", 64).last().unwrap(), "ground=true");
        // CAN aborts a CSI; C1 ST ends a DCS but is payload in an OSC.
        assert_eq!(events(b"\x1b[1\x18", 64).last().unwrap(), "ground=true");
        assert_eq!(events(b"\x1bP1q\x9c", 64).last().unwrap(), "ground=true");
        assert_eq!(events(b"\x1b]2;\x9c", 64).last().unwrap(), "ground=false");
    }

    #[test]
    fn c1_controls_dispatch_as_escapes_outside_ground() {
        // 0x85 is NEL inside a CSI, and UTF-8 continuation garbage in ground.
        let got = events(b"\x1b[\x85", 64);
        assert_eq!(got[0], r#"Esc { inter: [], fin: 69 }"#);
        assert_eq!(events(b"\x85", 64), ["ground=true"]);
    }

    #[test]
    fn too_many_params_drop_the_csi() {
        let mut s = b"\x1b[".to_vec();
        for _ in 0..MAX_PARAMS {
            s.extend_from_slice(b"1;");
        }
        s.extend_from_slice(b"1h");
        assert_eq!(events(&s, 64), ["ground=true"]);
    }

    #[test]
    fn remembers_what_was_printed() {
        let mut p = Parser::default();
        let mut bytes: &[u8] = "ab✓\x1b[mé".as_bytes();
        while !bytes.is_empty() {
            let n = p.step(bytes).consumed;
            bytes = &bytes[n..];
        }
        let got: Vec<u32> = p.recent.iter().collect();
        assert_eq!(got, ['é' as u32, '✓' as u32, 'b' as u32, 'a' as u32]);
        assert_eq!(p.printed, 4);
    }
}
