//! What vornd reads beside Ghostty by exact byte strings: OSC 5522 cwds and scrollback clears.

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
    /// ED 3, DECSED 3 and RIS seen.
    pub(crate) clears: u64,
}

impl Scan {
    /// Scans a feed, calling `private` with each OSC 5522 payload.
    pub(crate) fn feed(&mut self, bytes: &[u8], mut private: impl FnMut(&[u8])) {
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

    /// Rebuilds the state from Ghostty's continuation, the bytes since its parser left ground.
    pub(crate) fn reseed(&mut self, continuation: &[u8]) {
        let clears = self.clears;
        *self = Scan::default();
        self.feed(continuation, |_| {});
        self.clears = clears;
    }
}

/// Appends payload bytes without the C0 controls Ghostty ignores in an OSC, up to the cap.
fn extend(payload: &mut Vec<u8>, bytes: &[u8]) {
    let room = MAX_OSC_CAPTURE.saturating_sub(payload.len());
    payload.extend(bytes.iter().filter(|&&b| b >= 0x20).take(room));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every OSC 5522 payload and the clear count, fed in `chunks`.
    fn scan(chunks: &[&[u8]]) -> (Vec<String>, u64) {
        let mut s = Scan::default();
        let mut seen = Vec::new();
        for c in chunks {
            s.feed(c, |p| seen.push(String::from_utf8_lossy(p).into_owned()));
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
    fn reseeding_from_a_continuation_resumes_mid_sequence() {
        let mut seen = Vec::new();
        let mut live = Scan::default();
        live.feed(b"text \x1b]5522;cwd;/ha", |p| seen.push(p.to_vec()));
        let mut back = Scan::default();
        back.reseed(b"\x1b]5522;cwd;/ha");
        for s in [&mut live, &mut back] {
            s.feed(b"lf\x07", |p| seen.push(p.to_vec()));
        }
        assert_eq!(seen, [b"cwd;/half".to_vec(), b"cwd;/half".to_vec()]);
        let mut cleared = Scan::default();
        cleared.reseed(b"\x1b[?3");
        cleared.feed(b"J", |_| {});
        assert_eq!(cleared.clears, 1);
    }
}
