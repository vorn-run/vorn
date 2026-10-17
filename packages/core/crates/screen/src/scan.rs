//! What vornd reads beside Ghostty by exact byte strings: OSC 5522 reports, OSC 133 marks and
//! scrollback clears.

use std::sync::LazyLock;

use memchr::memmem::Finder;

/// Starts vorn's own reports: `OSC 5522 ; cwd ; <path> ST` and the like.
const PRIVATE: &[u8] = b"\x1b]5522;";
/// Starts a shell's prompt and command mark: `OSC 133 ; A ST` and the like.
const MARK: &[u8] = b"\x1b]133;";
/// ED 3, DECSED 3 and RIS, as programs write them.
const CLEARS: [&[u8]; 3] = [b"\x1b[3J", b"\x1b[?3J", b"\x1bc"];
/// The longest pattern, which bounds the bytes held between feeds.
const LONGEST: usize = PRIVATE.len();
/// Built once: on a feed of a few hundred bytes building a searcher costs more than searching.
static FINDERS: LazyLock<([Finder<'static>; 3], Finder<'static>)> =
    LazyLock::new(|| (CLEARS.map(Finder::new), Finder::new(b"\x1b]")));
/// How much of one OSC payload is kept; the rest is dropped.
const MAX_OSC_CAPTURE: usize = 16 * 1024;

/// Which OSC a payload came in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Osc {
    /// OSC 5522, vorn's own.
    Private,
    /// OSC 133, the shell's prompt and command marks.
    Mark,
}

impl Osc {
    fn pattern(self) -> &'static [u8] {
        match self {
            Osc::Private => PRIVATE,
            Osc::Mark => MARK,
        }
    }
}

/// What the bytes from an `ESC` are.
enum Seen {
    Clear,
    /// The start of an OSC whose payload follows.
    Osc(Osc),
    /// They end before they can be told apart from a pattern.
    Prefix,
    Other,
}

fn seen(s: &[u8]) -> Seen {
    match s {
        [_, b'c', ..] | [_, b'[', b'3', b'J', ..] | [_, b'[', b'?', b'3', b'J', ..] => Seen::Clear,
        [_, b']', b'5', b'5', b'2', b'2', b';', ..] => Seen::Osc(Osc::Private),
        [_, b']', b'1', b'3', b'3', b';', ..] => Seen::Osc(Osc::Mark),
        _ if s.len() < LONGEST
            && CLEARS
                .iter()
                .chain([&PRIVATE, &MARK])
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
    /// An OSC payload the stream is inside.
    osc: Option<(Osc, Vec<u8>)>,
    /// ED 3, DECSED 3 and RIS seen.
    pub(crate) clears: u64,
    /// OSC 133 marks seen.
    pub(crate) marks: u64,
}

impl Scan {
    /// Scans a feed, calling `report` with each OSC 5522 and OSC 133 payload.
    pub(crate) fn feed(&mut self, bytes: &[u8], mut report: impl FnMut(Osc, &[u8])) {
        let mut from = 0;
        if let Some((kind, payload)) = self.osc.take() {
            match self.payload(bytes, 0, kind, payload, &mut report) {
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
                Seen::Osc(kind) => {
                    let begin = from + kind.pattern().len() - held;
                    match self.payload(bytes, begin, kind, Vec::new(), &mut report) {
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
        let (clears, osc_start) = &*FINDERS;
        for finder in clears {
            self.clears += finder.find_iter(rest).count() as u64;
        }
        let mut at = from;
        while let Some(n) = osc_start.find(&bytes[at..]) {
            let start = at + n;
            let Seen::Osc(kind) = seen(&bytes[start..]) else {
                at = start + 2;
                continue;
            };
            match self.payload(
                bytes,
                start + kind.pattern().len(),
                kind,
                Vec::new(),
                &mut report,
            ) {
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

    /// Reports the OSC payload from `begin` and returns its end, or holds it if the feed ends first.
    fn payload(
        &mut self,
        bytes: &[u8],
        begin: usize,
        kind: Osc,
        mut payload: Vec<u8>,
        report: &mut impl FnMut(Osc, &[u8]),
    ) -> Option<usize> {
        let rest = &bytes[begin..];
        // Ghostty dispatches an OSC at BEL or any ESC and aborts it at CAN or SUB.
        let Some(len) = rest
            .iter()
            .position(|&b| matches!(b, 0x07 | 0x1b | 0x18 | 0x1a))
        else {
            extend(&mut payload, rest);
            self.osc = Some((kind, payload));
            return None;
        };
        extend(&mut payload, &rest[..len]);
        if matches!(rest[len], 0x07 | 0x1b) {
            self.marks += u64::from(kind == Osc::Mark);
            report(kind, &payload);
        }
        Some(begin + len)
    }

    /// Rebuilds the state from Ghostty's continuation, the bytes since its parser left ground.
    pub(crate) fn reseed(&mut self, continuation: &[u8]) {
        let (clears, marks) = (self.clears, self.marks);
        *self = Scan::default();
        self.feed(continuation, |_, _| {});
        (self.clears, self.marks) = (clears, marks);
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

    /// Every OSC payload (OSC 133 ones marked `133:`) and the clear count, fed in `chunks`.
    fn scan(chunks: &[&[u8]]) -> (Vec<String>, u64) {
        let mut s = Scan::default();
        let mut seen = Vec::new();
        for c in chunks {
            s.feed(c, |kind, p| {
                let p = String::from_utf8_lossy(p);
                seen.push(match kind {
                    Osc::Private => p.into_owned(),
                    Osc::Mark => format!("133:{p}"),
                });
            });
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
    fn reads_prompt_and_command_marks_beside_private_reports() {
        let (seen, _) = split_everywhere(
            b"\x1b]133;D;1\x07\x1b]5522;cwd;/a\x07\x1b]133;A\x07$ \x1b]2;t\x07\x1b]133;C;cmdline_url=ls\x1b\\\x1b]13;x\x07",
        );
        assert_eq!(seen, ["133:D;1", "cwd;/a", "133:A", "133:C;cmdline_url=ls"]);
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
        live.feed(b"text \x1b]5522;cwd;/ha", |_, p| seen.push(p.to_vec()));
        let mut back = Scan::default();
        back.reseed(b"\x1b]5522;cwd;/ha");
        for s in [&mut live, &mut back] {
            s.feed(b"lf\x07", |_, p| seen.push(p.to_vec()));
        }
        assert_eq!(seen, [b"cwd;/half".to_vec(), b"cwd;/half".to_vec()]);
        let mut cleared = Scan::default();
        cleared.reseed(b"\x1b[?3");
        cleared.feed(b"J", |_, _| {});
        assert_eq!(cleared.clears, 1);
    }
}
