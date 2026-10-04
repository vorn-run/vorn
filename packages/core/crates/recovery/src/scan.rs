//! Where a checkpoint may be cut: a record boundary with the VT parser in its
//! ground state and no UTF-8 sequence open.
//!
//! A small copy of the state machine libghostty-vt's parser runs (the DEC
//! VT500 parser as Ghostty tables it, plus its UTF-8 decoder in ground), fed
//! the same bytes, so the engine knows the parser's state without asking
//! Ghostty, which does not say. Only the state is tracked: no parameters, no
//! payloads.
//!
//! Two of Ghostty's choices matter here and are copied: in ground every byte
//! goes through the UTF-8 decoder, so 0x80..=0x9F are not C1 controls there;
//! and in an OSC string every byte from 0x20 up is payload, so a C1 ST (0x9C)
//! does not end an OSC, while BEL and `ESC \` do.

/// The parser's state, as far as a checkpoint cares.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum VtState {
    #[default]
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
    /// SOS, PM and APC strings, which Ghostty keeps in one state.
    SosPmApcString,
}

/// Feeds bytes through the parser's state machine, across calls.
#[derive(Debug, Default, Clone)]
pub struct Scanner {
    state: VtState,
    /// Continuation bytes the open UTF-8 sequence still needs (ground only).
    utf8_need: u8,
    /// The valid range of the next continuation byte (Unicode table 3-7).
    utf8_next: (u8, u8),
}

impl Scanner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> VtState {
        self.state
    }

    /// Whether a UTF-8 sequence is open: its lead byte was seen, not all of
    /// its continuation bytes.
    pub fn utf8_open(&self) -> bool {
        self.utf8_need > 0
    }

    /// A checkpoint may be cut here: ground, and no UTF-8 sequence open.
    pub fn is_safe(&self) -> bool {
        self.state == VtState::Ground && self.utf8_need == 0
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.byte(b);
        }
    }

    fn byte(&mut self, b: u8) {
        if self.state == VtState::Ground {
            self.ground(b);
        } else {
            self.state = next(self.state, b);
        }
    }

    /// Ghostty's UTF-8 decoder, which only runs in ground. A byte that breaks
    /// an open sequence ends it (a replacement character) and is then decoded
    /// again on its own, as the decoder's caller does.
    fn ground(&mut self, b: u8) {
        if self.utf8_need > 0 {
            let (lo, hi) = self.utf8_next;
            if (lo..=hi).contains(&b) {
                self.utf8_need -= 1;
                self.utf8_next = (0x80, 0xbf);
                return;
            }
            self.utf8_need = 0;
        }
        match b {
            0x1b => self.state = VtState::Escape,
            0x00..=0x7f => {}
            0xc2..=0xdf => self.open(1, 0x80, 0xbf),
            0xe0 => self.open(2, 0xa0, 0xbf),
            0xed => self.open(2, 0x80, 0x9f),
            0xe1..=0xef => self.open(2, 0x80, 0xbf),
            0xf0 => self.open(3, 0x90, 0xbf),
            0xf4 => self.open(3, 0x80, 0x8f),
            0xf1..=0xf3 => self.open(3, 0x80, 0xbf),
            // A stray continuation byte or one no sequence starts with: one
            // replacement character, consumed.
            _ => {}
        }
    }

    fn open(&mut self, need: u8, lo: u8, hi: u8) {
        self.utf8_need = need;
        self.utf8_next = (lo, hi);
    }
}

/// One transition of Ghostty's parse table (`parse_table.zig`) outside
/// ground: first the transitions from anywhere, then each state's own.
fn next(s: VtState, b: u8) -> VtState {
    use VtState::*;
    // From anywhere, except where a state's own table overrides it: an OSC
    // string takes 0x20..=0xff as payload.
    let osc_payload = s == OscString && b >= 0x20;
    if !osc_payload {
        match b {
            0x18 | 0x1a | 0x80..=0x8f | 0x91..=0x97 | 0x99 | 0x9a | 0x9c => return Ground,
            0x1b => return Escape,
            0x98 | 0x9e | 0x9f => return SosPmApcString,
            0x9b => return CsiEntry,
            0x90 => return DcsEntry,
            0x9d => return OscString,
            _ => {}
        }
    }
    match s {
        Ground => Ground,
        Escape => match b {
            0x20..=0x2f => EscapeIntermediate,
            b'X' | b'^' | b'_' => SosPmApcString,
            b'P' => DcsEntry,
            b'[' => CsiEntry,
            b']' => OscString,
            0x30..=0x7e => Ground,
            _ => Escape,
        },
        EscapeIntermediate => match b {
            0x30..=0x7e => Ground,
            _ => EscapeIntermediate,
        },
        CsiEntry => match b {
            0x40..=0x7e => Ground,
            b':' => CsiIgnore,
            0x20..=0x2f => CsiIntermediate,
            0x30..=0x39 | b';' | 0x3c..=0x3f => CsiParam,
            _ => CsiEntry,
        },
        CsiParam => match b {
            0x40..=0x7e => Ground,
            0x3c..=0x3f => CsiIgnore,
            0x20..=0x2f => CsiIntermediate,
            _ => CsiParam,
        },
        CsiIntermediate => match b {
            0x40..=0x7e => Ground,
            0x30..=0x3f => CsiIgnore,
            _ => CsiIntermediate,
        },
        CsiIgnore => match b {
            0x40..=0x7e => Ground,
            _ => CsiIgnore,
        },
        DcsEntry => match b {
            0x20..=0x2f => DcsIntermediate,
            b':' => DcsIgnore,
            0x30..=0x39 | b';' | 0x3c..=0x3f => DcsParam,
            0x40..=0x7e => DcsPassthrough,
            _ => DcsEntry,
        },
        DcsParam => match b {
            b':' | 0x3c..=0x3f => DcsIgnore,
            0x20..=0x2f => DcsIntermediate,
            0x40..=0x7e => DcsPassthrough,
            _ => DcsParam,
        },
        DcsIntermediate => match b {
            0x30..=0x3f => DcsIgnore,
            0x40..=0x7e => DcsPassthrough,
            _ => DcsIntermediate,
        },
        DcsPassthrough => DcsPassthrough,
        DcsIgnore => DcsIgnore,
        OscString => match b {
            0x07 => Ground,
            _ => OscString,
        },
        SosPmApcString => SosPmApcString,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn after(chunks: &[&[u8]]) -> Vec<bool> {
        let mut s = Scanner::new();
        chunks
            .iter()
            .map(|c| {
                s.feed(c);
                s.is_safe()
            })
            .collect()
    }

    #[test]
    fn plain_text_and_whole_sequences_are_safe() {
        assert_eq!(
            after(&[b"hello", b"\x1b[1;31mred\x1b[0m", b"\x1b]2;t\x07", b"\x1b7"]),
            [true; 4]
        );
    }

    #[test]
    fn a_csi_split_anywhere_is_not() {
        let seq = b"\x1b[?1049h";
        for cut in 1..seq.len() {
            let (a, b) = seq.split_at(cut);
            assert_eq!(after(&[a, b]), [false, true], "cut at {cut}");
        }
    }

    #[test]
    fn osc_ends_on_bel_or_st_and_not_on_c1_st() {
        assert_eq!(
            after(&[b"\x1b]0;title", b"\x1b", b"\\"]),
            [false, false, true]
        );
        // 0x9c is payload in an OSC string, as Ghostty tables it.
        assert_eq!(after(&[b"\x1b]2;a\x9c", b"\x07"]), [false, true]);
        // UTF-8 in a title does not open a sequence the scanner tracks.
        assert_eq!(after(&[b"\x1b]2;\xe6\x97", b"\xa5\x07"]), [false, true]);
    }

    #[test]
    fn strings_and_aborts() {
        // DCS, APC, PM and SOS run until ST.
        assert_eq!(after(&[b"\x1bP$qm", b"\x1b\\"]), [false, true]);
        assert_eq!(after(&[b"\x1b_Gabc", b"\x1b\\"]), [false, true]);
        assert_eq!(after(&[b"\x1b^pm\x1b\\", b"\x1bXsos"]), [true, false]);
        // CAN and SUB abort anything.
        assert_eq!(after(&[b"\x1b[12;", b"\x18"]), [false, true]);
        assert_eq!(after(&[b"\x1b]2;never", b"\x1a"]), [false, true]);
        assert_eq!(after(&[b"\x1bP1;2", b"\x18"]), [false, true]);
        // ESC inside a CSI starts a new escape.
        assert_eq!(after(&[b"\x1b[1\x1b", b"c"]), [false, true]);
    }

    #[test]
    fn split_utf8_is_not_safe() {
        // 日 is e6 97 a5; 😀 is f0 9f 98 80.
        assert_eq!(after(&[b"\xe6", b"\x97", b"\xa5"]), [false, false, true]);
        assert_eq!(after(&[b"\xf0\x9f\x98", b"\x80"]), [false, true]);
        assert_eq!(after(&[b"\xc3", b"\xa9x"]), [false, true]);
    }

    #[test]
    fn broken_utf8_closes_its_sequence() {
        // A byte outside the continuation range ends the sequence and counts
        // on its own: here ESC, which then opens an escape.
        let mut s = Scanner::new();
        s.feed(b"\xe6\x1b");
        assert_eq!(s.state(), VtState::Escape);
        assert!(!s.utf8_open());
        // Overlong and surrogate leads are refused at the second byte.
        assert_eq!(after(&[b"\xe0\x80"]), [true]);
        assert_eq!(after(&[b"\xed\xa0"]), [true]);
        assert_eq!(after(&[b"\xc0", b"\x80", b"\xff"]), [true, true, true]);
    }

    #[test]
    fn c1_bytes_are_text_in_ground_but_controls_in_a_sequence() {
        // 0x9b in ground is a stray byte, not a CSI.
        assert_eq!(after(&[b"\x9b"]), [true]);
        // In a CSI, 0x9c (ST) returns to ground and 0x9d opens an OSC.
        assert_eq!(after(&[b"\x1b[1\x9c"]), [true]);
        assert_eq!(after(&[b"\x1b[1\x9d"]), [false]);
    }
}
