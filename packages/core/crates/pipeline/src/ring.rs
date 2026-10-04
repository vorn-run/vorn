//! The bytes a terminal emitted, kept as they were emitted, up to a cap.
//!
//! What a client attaching to a live session is given first, so its emulator
//! draws what is on screen before live output starts. The same rules as the
//! JavaScript buffer it replaces (`terminal-scrollback.ts`): trimmed from the
//! front at a line boundary, so a client is never handed half an escape
//! sequence, and compacted only once it has run a quarter past its cap.
//!
//! Counted in bytes, where the JavaScript buffer counts UTF-16 units. For the
//! ASCII nearly all terminal output is they are the same; a run of CJK keeps a
//! third as many characters here, for the same memory.

use memchr::memchr;

/// How much to keep per terminal.
pub const MAX_BYTES: usize = 256 * 1024;
/// How far past the cap the buffer may run before it is trimmed.
const SLACK: usize = MAX_BYTES / 4;

#[derive(Default)]
pub struct Scrollback {
    bytes: Vec<u8>,
}

impl Scrollback {
    pub fn append(&mut self, data: &[u8]) {
        self.bytes.extend_from_slice(data);
        if self.bytes.len() > MAX_BYTES + SLACK {
            self.trim();
        }
    }

    /// Replace what is held, as recovery does from a checkpoint.
    pub fn seed(&mut self, data: &[u8]) {
        self.bytes.clear();
        self.bytes.extend_from_slice(data);
        self.trim();
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// What a client is given, at most `MAX_BYTES`.
    pub fn read(&mut self) -> String {
        self.trim();
        // Every cut is at a newline or a character boundary, and everything
        // appended came from a `String`, so this is whole UTF-8. Lossy rather
        // than unchecked all the same: a wrong guess here is a replacement
        // character, not undefined behaviour.
        String::from_utf8_lossy(&self.bytes).into_owned()
    }

    /// Cut the front at the first newline past the point that leaves `MAX_BYTES`,
    /// or at that point itself when no newline follows -- moved forward to the
    /// next character boundary, since bytes can split a character where UTF-16
    /// units cannot.
    fn trim(&mut self) {
        if self.bytes.len() <= MAX_BYTES {
            return;
        }
        let cut = self.bytes.len() - MAX_BYTES;
        let start = match memchr(b'\n', &self.bytes[cut..]) {
            Some(at) => cut + at + 1,
            None => {
                let mut at = cut;
                while at < self.bytes.len() && is_continuation(self.bytes[at]) {
                    at += 1;
                }
                at
            }
        };
        self.bytes.drain(..start);
    }
}

fn is_continuation(byte: u8) -> bool {
    byte & 0xc0 == 0x80
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rule as `terminal-scrollback.ts` states it, on the joined text.
    fn reference(data: &[u8]) -> Vec<u8> {
        if data.len() <= MAX_BYTES {
            return data.to_vec();
        }
        let cut = data.len() - MAX_BYTES;
        match memchr(b'\n', &data[cut..]) {
            Some(at) => data[cut + at + 1..].to_vec(),
            None => data[cut..].to_vec(),
        }
    }

    #[test]
    fn keeps_everything_under_the_cap() {
        let mut ring = Scrollback::default();
        ring.append(b"\x1b[31mred\x1b[0m\r\n");
        ring.append(b"more");
        assert_eq!(ring.read(), "\x1b[31mred\x1b[0m\r\nmore");
    }

    #[test]
    fn trims_at_the_first_newline_past_the_cut() {
        let mut ring = Scrollback::default();
        let mut all = Vec::new();
        for i in 0..40_000 {
            let line = format!("line {i}\n");
            all.extend_from_slice(line.as_bytes());
        }
        ring.seed(&all);
        let read = ring.read();
        assert_eq!(read.as_bytes(), reference(&all));
        assert!(read.starts_with("line "));
        assert!(read.len() <= MAX_BYTES);
    }

    #[test]
    fn stays_bounded_while_appending() {
        let mut ring = Scrollback::default();
        for _ in 0..100 {
            ring.append(&[b'x'; 64 * 1024]);
            assert!(ring.len() <= MAX_BYTES + SLACK);
        }
        assert_eq!(ring.read().len(), MAX_BYTES);
    }

    #[test]
    fn never_cuts_inside_a_character() {
        let mut ring = Scrollback::default();
        // Three bytes each and no newline: most cut points land mid-character.
        for _ in 0..3 {
            ring.append("日本語".repeat(40_000).as_bytes());
            let read = ring.read();
            assert!(read.chars().all(|c| "日本語".contains(c)));
            assert!(read.len() <= MAX_BYTES);
        }
    }

    #[test]
    fn reads_empty_when_nothing_was_written() {
        assert_eq!(Scrollback::default().read(), "");
    }
}
