//! An analyzer's state as bytes, so a session engine can put it in a
//! checkpoint and carry on from it after a restart exactly where it was:
//! the same line ring, the line in progress and the escape sequence it is in
//! the middle of.
//!
//! The layout is private to this crate and versioned by its first byte; a
//! reader that meets another version refuses it, and the caller falls back
//! to replaying the output that made it.

use std::collections::VecDeque;

use crate::{Analyzer, Line, State};

const VERSION: u8 = 1;

impl Analyzer {
    /// Everything the analyzer would need to carry on, as bytes.
    pub fn save(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64 + self.line.len() + self.lines.len() * 48);
        out.push(VERSION);
        out.push(state_byte(self.state));
        out.extend_from_slice(&self.csi_param.to_le_bytes());
        out.push(u8::from(self.csi_private));
        out.extend_from_slice(&(self.str_len as u64).to_le_bytes());
        put_str(&mut out, &self.line);
        out.extend_from_slice(&(self.line_chars as u64).to_le_bytes());
        out.extend_from_slice(&(self.col as u64).to_le_bytes());
        out.push(u8::from(self.pending_cr));
        out.extend_from_slice(&(self.lines.len() as u32).to_le_bytes());
        for l in &self.lines {
            put_str(&mut out, &l.text);
            out.push(match l.error {
                None => 0,
                Some(false) => 1,
                Some(true) => 2,
            });
        }
        out
    }

    /// The analyzer [`Analyzer::save`] wrote, or `None` for bytes it could
    /// not have written.
    pub fn restore(bytes: &[u8]) -> Option<Analyzer> {
        let mut r = Reader(bytes);
        if r.u8()? != VERSION {
            return None;
        }
        let state = state_of(r.u8()?)?;
        let csi_param = r.u32()?;
        let csi_private = r.bool()?;
        let str_len = usize::try_from(r.u64()?).ok()?;
        let line = r.string()?;
        let line_chars = usize::try_from(r.u64()?).ok()?;
        let col = usize::try_from(r.u64()?).ok()?;
        let pending_cr = r.bool()?;
        let n = r.u32()? as usize;
        // Each line takes at least five bytes, which bounds a forged count.
        if n > r.0.len() / 5 {
            return None;
        }
        let mut lines = VecDeque::with_capacity(n.max(64));
        for _ in 0..n {
            let text = r.string()?;
            let error = match r.u8()? {
                0 => None,
                1 => Some(false),
                2 => Some(true),
                _ => return None,
            };
            lines.push_back(Line { text, error });
        }
        if !r.0.is_empty() || line_chars != line.chars().count() || col > line_chars {
            return None;
        }
        Some(Analyzer {
            state,
            csi_param,
            csi_private,
            str_len,
            line,
            line_chars,
            col,
            pending_cr,
            lines,
        })
    }
}

fn state_byte(s: State) -> u8 {
    match s {
        State::Ground => 0,
        State::Esc => 1,
        State::Csi => 2,
        State::Str => 3,
        State::StrEsc => 4,
        State::Charset => 5,
    }
}

fn state_of(b: u8) -> Option<State> {
    Some(match b {
        0 => State::Ground,
        1 => State::Esc,
        2 => State::Csi,
        3 => State::Str,
        4 => State::StrEsc,
        5 => State::Charset,
        _ => return None,
    })
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn bool(&mut self) -> Option<bool> {
        match self.u8()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn string(&mut self) -> Option<String> {
        let n = self.u32()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fed in two halves with a save and restore between them, the analyzer
    /// ends exactly as one fed the whole stream at once, including an
    /// escape sequence and a line cut in the middle.
    #[test]
    fn a_restored_analyzer_carries_on_where_it_was() {
        let stream = "one\r\nError: two\r\n\x1b[31mred\x1b[0m thr\rTHREE\r\n$ \x1b]0;title\x07four\x1b[2K\rfive (y/n) ";
        for cut in 0..=stream.len() {
            if !stream.is_char_boundary(cut) {
                continue;
            }
            let (a, b) = stream.split_at(cut);
            let mut whole = Analyzer::new();
            let s1 = whole.append_str(a, true);
            let s2 = whole.append_str(b, true);
            let mut first = Analyzer::new();
            assert_eq!(first.append_str(a, true), s1);
            let mut back = Analyzer::restore(&first.save()).expect("restores");
            assert_eq!(back.append_str(b, true), s2, "cut at {cut}");
            assert_eq!(back.save(), whole.save(), "cut at {cut}");
        }
    }

    #[test]
    fn bytes_it_did_not_write_are_refused() {
        let mut a = Analyzer::new();
        a.append_str("hello\r\nworld", true);
        let bytes = a.save();
        for n in 0..bytes.len() {
            assert!(Analyzer::restore(&bytes[..n]).is_none(), "{n}");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(Analyzer::restore(&longer).is_none());
        let mut other = bytes;
        other[0] = VERSION + 1;
        assert!(Analyzer::restore(&other).is_none());
    }
}
