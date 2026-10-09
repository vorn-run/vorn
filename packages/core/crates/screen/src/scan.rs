//! What vornd reads beside Ghostty by exact byte strings: OSC 5522 cwds, scrollback clears, joining codepoints.

use std::sync::LazyLock;

use memchr::memmem::Finder;

/// Starts vorn's cwd report: `OSC 5522 ; cwd ; <path> ST`.
const PRIVATE: &[u8] = b"\x1b]5522;";
/// ED 3, DECSED 3 and RIS, as programs write them.
const CLEARS: [&[u8]; 3] = [b"\x1b[3J", b"\x1b[?3J", b"\x1bc"];
/// The longest pattern, which bounds the bytes held between feeds.
const LONGEST: usize = PRIVATE.len();
/// Built once: on a feed of a few hundred bytes building a searcher costs more than searching.
static FINDERS: LazyLock<([Finder<'static>; 3], Finder<'static>)> =
    LazyLock::new(|| (CLEARS.map(Finder::new), Finder::new(PRIVATE)));
/// How much of one OSC 5522 payload is kept; the rest is dropped.
const MAX_OSC_CAPTURE: usize = 16 * 1024;

/// What the bytes from an `ESC` are.
enum Seen {
    Clear,
    /// The start of an OSC 5522, whose payload follows.
    Private,
    /// They end before they can be told apart from a pattern.
    Prefix,
    Other,
}

fn seen(s: &[u8]) -> Seen {
    match s {
        [_, b'c', ..] | [_, b'[', b'3', b'J', ..] | [_, b'[', b'?', b'3', b'J', ..] => Seen::Clear,
        [_, b']', b'5', b'5', b'2', b'2', b';', ..] => Seen::Private,
        _ if s.len() < LONGEST
            && CLEARS
                .iter()
                .chain([&PRIVATE])
                .any(|pat| pat.starts_with(s)) =>
        {
            Seen::Prefix
        }
        _ => Seen::Other,
    }
}

/// The scan's state across feeds.
#[derive(Debug, Default)]
pub(crate) struct Scan {
    /// The last feed's end from its last `ESC`, when the next feed may finish a pattern there.
    tail: Vec<u8>,
    /// An OSC 5522 payload the stream is inside.
    osc: Option<Vec<u8>>,
    /// The start of a UTF-8 sequence the last feed ended inside.
    utf8: Vec<u8>,
    /// ED 3, DECSED 3 and RIS seen.
    pub(crate) clears: u64,
}

impl Scan {
    /// Scans a feed, calling `private` with each OSC 5522 payload and pushing joining codepoints on `joins`.
    pub(crate) fn feed(
        &mut self,
        bytes: &[u8],
        mut private: impl FnMut(&[u8]),
        joins: &mut Vec<u32>,
    ) {
        self.joins(bytes, joins);
        let mut from = 0;
        if let Some(payload) = self.osc.take() {
            match self.payload(bytes, 0, payload, &mut private) {
                Some(end) => from = end,
                None => return,
            }
        }
        if !self.tail.is_empty() {
            let held = self.tail.len();
            let mut window = std::mem::take(&mut self.tail);
            window.extend_from_slice(&bytes[from..bytes.len().min(from + LONGEST)]);
            match seen(&window) {
                Seen::Clear => self.clears += 1,
                Seen::Private => {
                    let begin = from + PRIVATE.len() - held;
                    match self.payload(bytes, begin, Vec::new(), &mut private) {
                        Some(end) => from = end,
                        None => return,
                    }
                }
                Seen::Prefix => {
                    self.tail = window;
                    return;
                }
                Seen::Other => {}
            }
        }
        let rest = &bytes[from..];
        let (clears, private_osc) = &*FINDERS;
        for finder in clears {
            self.clears += finder.find_iter(rest).count() as u64;
        }
        let mut at = from;
        while let Some(n) = private_osc.find(&bytes[at..]) {
            match self.payload(bytes, at + n + PRIVATE.len(), Vec::new(), &mut private) {
                Some(end) => at = end,
                None => return,
            }
        }
        let near = bytes.len().saturating_sub(LONGEST - 1).max(from);
        if let Some(esc) = memchr::memrchr(0x1b, &bytes[near..]).map(|n| near + n) {
            if matches!(seen(&bytes[esc..]), Seen::Prefix) {
                self.tail.extend_from_slice(&bytes[esc..]);
            }
        }
    }

    /// Reports the OSC 5522 payload from `begin` and returns its end, or holds it if the feed ends first.
    fn payload(
        &mut self,
        bytes: &[u8],
        begin: usize,
        mut payload: Vec<u8>,
        private: &mut impl FnMut(&[u8]),
    ) -> Option<usize> {
        let rest = &bytes[begin..];
        // Ghostty dispatches an OSC at BEL or any ESC and aborts it at CAN or SUB.
        let Some(len) = rest
            .iter()
            .position(|&b| matches!(b, 0x07 | 0x1b | 0x18 | 0x1a))
        else {
            extend(&mut payload, rest);
            self.osc = Some(payload);
            return None;
        };
        extend(&mut payload, &rest[..len]);
        if matches!(rest[len], 0x07 | 0x1b) {
            private(&payload);
        }
        Some(begin + len)
    }

    /// Codepoints from U+0300 on, read at their lead bytes; ASCII chunks are skipped whole.
    fn joins(&mut self, bytes: &[u8], joins: &mut Vec<u32>) {
        let mut start = 0;
        if !self.utf8.is_empty() {
            let mut seq = std::mem::take(&mut self.utf8);
            let need = seq_len(seq[0]) - seq.len();
            let take = need.min(bytes.len());
            seq.extend_from_slice(&bytes[..take]);
            if take < need {
                self.utf8 = seq;
                return;
            }
            push_join(&seq, joins);
            start = take;
        }
        let bytes = &bytes[start..];
        for (n, chunk) in bytes.chunks(64).enumerate() {
            if chunk.is_ascii() {
                continue;
            }
            // Eight bytes at a time: a lead byte is one with its top two bits set.
            let words = chunk.chunks_exact(8);
            let rest = words.remainder().len();
            for (w, word) in words.enumerate() {
                let word = u64::from_le_bytes(word.try_into().expect("chunks of eight"));
                let mut leads = word & (word << 1) & 0x8080_8080_8080_8080;
                while leads != 0 {
                    let at = n * 64 + w * 8 + leads.trailing_zeros() as usize / 8;
                    self.lead(bytes, at, joins);
                    leads &= leads - 1;
                }
            }
            for at in n * 64 + chunk.len() - rest..n * 64 + chunk.len() {
                self.lead(bytes, at, joins);
            }
        }
    }

    /// The sequence `bytes[at]` leads, if it can be U+0300 or above.
    fn lead(&mut self, bytes: &[u8], at: usize, joins: &mut Vec<u32>) {
        let b = bytes[at];
        // Box drawing, arrows, braille and the like, U+2100 to U+2BFF, never join.
        if b < 0xcc || (b == 0xe2 && bytes.get(at + 1).is_some_and(|s| (0x84..=0xaf).contains(s))) {
            return;
        }
        let seq = &bytes[at..bytes.len().min(at + seq_len(b))];
        if seq.len() < seq_len(b) {
            self.utf8 = seq.to_vec();
        } else {
            push_join(seq, joins);
        }
    }

    /// Rebuilds the state from Ghostty's continuation, the bytes since its parser left ground.
    pub(crate) fn reseed(&mut self, continuation: &[u8]) {
        let clears = self.clears;
        *self = Scan::default();
        self.feed(continuation, |_| {}, &mut Vec::new());
        self.clears = clears;
    }
}

/// The length of the UTF-8 sequence a lead byte from 0xCC starts.
fn seq_len(lead: u8) -> usize {
    match lead {
        0xe0..=0xef => 3,
        0xf0..=0xff => 4,
        _ => 2,
    }
}

/// Decodes one sequence and keeps its codepoint if it may join.
fn push_join(seq: &[u8], joins: &mut Vec<u32>) {
    let Ok(s) = std::str::from_utf8(seq) else {
        return;
    };
    if let Some(cp) = s.chars().next().map(u32::from) {
        if may_join(cp) && !joins.contains(&cp) {
            joins.push(cp);
        }
    }
}

/// Appends payload bytes without the C0 controls Ghostty ignores in an OSC, up to the cap.
fn extend(payload: &mut Vec<u8>, bytes: &[u8]) {
    let room = MAX_OSC_CAPTURE.saturating_sub(payload.len());
    payload.extend(bytes.iter().filter(|&&b| b >= 0x20).take(room));
}

/// Whether a codepoint may join the cell before it: U+0300 on, but the common wide and symbol blocks.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every OSC 5522 payload and the clear count, fed in `chunks`.
    fn scan(chunks: &[&[u8]]) -> (Vec<String>, u64) {
        let mut s = Scan::default();
        let mut seen = Vec::new();
        for c in chunks {
            s.feed(
                c,
                |p| seen.push(String::from_utf8_lossy(p).into_owned()),
                &mut Vec::new(),
            );
        }
        (seen, s.clears)
    }

    /// The same input split anywhere gives the same result.
    fn split_everywhere(bytes: &[u8]) -> (Vec<String>, u64) {
        let whole = scan(&[bytes]);
        for at in 0..=bytes.len() {
            assert_eq!(scan(&[&bytes[..at], &bytes[at..]]), whole, "split at {at}");
        }
        let ones: Vec<&[u8]> = bytes.chunks(1).collect();
        assert_eq!(scan(&ones), whole, "one byte at a time");
        whole
    }

    #[test]
    fn private_osc_ends_as_ghostty_ends_it() {
        let (seen, _) = split_everywhere(
            b"a\x1b]5522;cwd;/one\x07b\x1b]5522;cwd;/t\nwo\x1b\\\x1b]5522;x\x18\x1b]5522;y\x1b[m",
        );
        assert_eq!(seen, ["cwd;/one", "cwd;/two", "y"]);
        let none = scan(&[b"\x1b]552;x\x07\x1b]55222;x\x07"]).0;
        assert!(none.is_empty(), "{none:?}");
    }

    #[test]
    fn counts_scrollback_clears() {
        let (_, n) =
            split_everywhere(b"\x1b[3J\x1b[?3J\x1b[2J\x1b[13J\x1b[>3J\x1bc\x1b(c\x1b]5522;\x1b[3J");
        assert_eq!(n, 4);
    }

    #[test]
    fn finds_codepoints_that_may_join_across_splits() {
        let mut s = Scan::default();
        let mut joins = Vec::new();
        s.feed(
            "plain ascii \u{2500} \u{e9} \u{3b1}".as_bytes(),
            |_| {},
            &mut joins,
        );
        assert_eq!(joins, [0x3b1]);
        joins.clear();
        s.feed(b"e\xcc", |_| {}, &mut joins);
        s.feed(b"\x81", |_| {}, &mut joins);
        assert_eq!(joins, [0x301]);
    }

    #[test]
    fn reseeding_from_a_continuation_resumes_mid_sequence() {
        let mut seen = Vec::new();
        let mut live = Scan::default();
        live.feed(
            b"text \x1b]5522;cwd;/ha",
            |p| seen.push(p.to_vec()),
            &mut Vec::new(),
        );
        let mut back = Scan::default();
        back.reseed(b"\x1b]5522;cwd;/ha");
        for s in [&mut live, &mut back] {
            s.feed(b"lf\x07", |p| seen.push(p.to_vec()), &mut Vec::new());
        }
        assert_eq!(seen, [b"cwd;/half".to_vec(), b"cwd;/half".to_vec()]);
        let mut cleared = Scan::default();
        cleared.reseed(b"\x1b[?3");
        cleared.feed(b"J", |_| {}, &mut Vec::new());
        assert_eq!(cleared.clears, 1);
    }
}
