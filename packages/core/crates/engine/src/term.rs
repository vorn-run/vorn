//! One session's terminal and what reads the output beside it: the
//! emulator, the output analyzer and the state that ties them to a point in
//! the stream. A checkpoint blob is all of it, as bytes, and a terminal left
//! idle is kept as nothing more ([`Packed`]).
//!
//! Blob layout, format [`FORMAT`]: the fidelity byte, the last agent status
//! (u32), the UTF-8 bytes a record split for the analyzer (a length byte and
//! up to three bytes), then the screen checkpoint and the analyzer's state,
//! each as a u32 length and its bytes. All integers little-endian.

use vorn_analysis::Analyzer;
use vorn_screen::{Checkpoint, Counters, Effect, Emulator, Uncut};

/// The checkpoint format this engine writes and the only one it reads. A
/// checkpoint in any other format is not a restore base.
pub const FORMAT: u16 = 2;

/// Whether a session's terminal is what a vornd that never died would have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fidelity {
    Exact,
    /// Rebuilt from something less than a valid restore base, or across
    /// lost output: the screen may be incomplete.
    Approximate,
}

/// Why a checkpoint is not a restore base. Each is a fixed phrase so the
/// debug report can show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejected {
    /// A format this engine does not read.
    Format,
    /// The blob's CRC does not match.
    Crc,
    /// The blob is not one this format could have written.
    Decode,
    /// The screen checkpoint would not rebuild a terminal.
    Rebuild,
    /// The rebuilt terminal is not the one the checkpoint was cut from.
    RestoreCheck,
}

impl Rejected {
    pub fn as_str(self) -> &'static str {
        match self {
            Rejected::Format => "unreadable format",
            Rejected::Crc => "bad CRC",
            Rejected::Decode => "undecodable blob",
            Rejected::Rebuild => "rebuild failed",
            Rejected::RestoreCheck => "restore check failed",
        }
    }
}

pub(crate) struct Term {
    pub(crate) em: Emulator,
    analyzer: Analyzer,
    /// The start of a UTF-8 sequence the last record ended inside, which
    /// the analyzer, reading text, gets with the next record.
    carry: Vec<u8>,
    /// The agent status last reported, so only changes are.
    pub(crate) status: u32,
    pub(crate) fidelity: Fidelity,
    /// The default colours queries are answered with, kept across a reset.
    colors: Option<([u8; 3], [u8; 3])>,
}

impl Term {
    pub(crate) fn fresh(cols: u16, rows: u16, scrollback: usize) -> vorn_screen::Result<Term> {
        Ok(Term {
            em: Emulator::with_scrollback(u32::from(cols), u32::from(rows), scrollback)?,
            analyzer: Analyzer::new(),
            carry: Vec::new(),
            status: vorn_analysis::STATUS_NONE,
            fidelity: Fidelity::Exact,
            colors: None,
        })
    }

    /// Sets the default colours, which OSC 10 and 11 queries are answered
    /// with. Not part of a checkpoint: each vornd sets its own.
    pub(crate) fn set_colors(&mut self, colors: Option<([u8; 3], [u8; 3])>) {
        self.colors = colors;
        if colors.is_some() {
            // A terminal that refuses them answers no colour query, as before.
            let _ = self.em.set_default_colors(colors);
        }
    }

    /// Feeds output to both readers. Returns the agent status when it
    /// changed.
    pub(crate) fn feed(
        &mut self,
        bytes: &[u8],
        analyze: bool,
        effects: &mut Vec<Effect>,
    ) -> Option<u32> {
        self.em.feed(bytes, effects);
        let text = self.decode(bytes);
        let status = self.analyzer.append_str(&text, analyze);
        if status == vorn_analysis::STATUS_NONE || status == self.status {
            return None;
        }
        self.status = status;
        Some(status)
    }

    /// The text the analyzer reads: `bytes` after whatever the last record
    /// left open, with invalid sequences replaced as a lossy decode would,
    /// and an incomplete one at the end kept for the next record.
    fn decode(&mut self, bytes: &[u8]) -> String {
        let joined;
        let mut rest: &[u8] = if self.carry.is_empty() {
            bytes
        } else {
            joined = [std::mem::take(&mut self.carry).as_slice(), bytes].concat();
            &joined
        };
        let mut out = String::with_capacity(rest.len());
        loop {
            match std::str::from_utf8(rest) {
                Ok(s) => {
                    out.push_str(s);
                    return out;
                }
                Err(e) => {
                    let (good, bad) = rest.split_at(e.valid_up_to());
                    // Valid by the error's own account.
                    out.push_str(std::str::from_utf8(good).unwrap_or_default());
                    match e.error_len() {
                        Some(n) => {
                            out.push(char::REPLACEMENT_CHARACTER);
                            rest = &bad[n..];
                        }
                        None => {
                            self.carry = bad.to_vec();
                            return out;
                        }
                    }
                }
            }
        }
    }

    /// The terminal cleared at its own size: what is left after output was
    /// lost. The analyzer keeps its lines, which are text already read.
    pub(crate) fn reset(&mut self) -> vorn_screen::Result<()> {
        let (cols, rows) = (self.em.cols(), self.em.rows());
        let scrollback = self.em.scrollback_limit();
        self.em = Emulator::with_scrollback(u32::from(cols), u32::from(rows), scrollback)?;
        self.carry.clear();
        self.fidelity = Fidelity::Approximate;
        self.set_colors(self.colors);
        Ok(())
    }

    /// Cuts a checkpoint here, carrying on from the terminal rebuilt from
    /// it (see [`Emulator::checkpoint`]), and returns the blob.
    pub(crate) fn save(&mut self) -> Result<Vec<u8>, Uncut> {
        // Cut without them, so the restore check compares like with like:
        // a rebuild has Ghostty's defaults until its vornd sets its own.
        if self.colors.is_some() {
            let _ = self.em.set_default_colors(None);
        }
        let cut = self.em.checkpoint();
        self.set_colors(self.colors);
        let screen = cut?.encode();
        let analysis = self.analyzer.save();
        let mut out = Vec::with_capacity(16 + screen.len() + analysis.len());
        out.push(match self.fidelity {
            Fidelity::Exact => 0,
            Fidelity::Approximate => 1,
        });
        out.extend_from_slice(&self.status.to_le_bytes());
        // At most three bytes: a longer run was decoded already.
        out.push(self.carry.len() as u8);
        out.extend_from_slice(&self.carry);
        for part in [&screen, &analysis] {
            out.extend_from_slice(&(part.len() as u32).to_le_bytes());
            out.extend_from_slice(part);
        }
        Ok(out)
    }

    /// The terminal a blob of format [`FORMAT`] holds, after the restore
    /// check. A blob whose screen rebuilds but fails the check comes back
    /// as `Err((RestoreCheck, Some(term)))`, the best a session without a
    /// valid base has.
    pub(crate) fn load(blob: &[u8]) -> Result<Term, (Rejected, Option<Box<Term>>)> {
        let (term, cp) = Term::unpack(blob).map_err(|why| (why, None))?;
        if cp.matches(&term.em) {
            Ok(term)
        } else {
            Err((Rejected::RestoreCheck, Some(Box::new(term))))
        }
    }

    /// The terminal a blob holds, with its screen checkpoint, unchecked.
    fn unpack(blob: &[u8]) -> Result<(Term, Checkpoint), Rejected> {
        let parts = split(blob).ok_or(Rejected::Decode)?;
        let cp = Checkpoint::decode(parts.screen).ok_or(Rejected::Decode)?;
        let analyzer = Analyzer::restore(parts.analysis).ok_or(Rejected::Decode)?;
        let em = Emulator::restore(&cp).map_err(|_| Rejected::Rebuild)?;
        let term = Term {
            em,
            analyzer,
            carry: parts.carry.to_vec(),
            status: parts.status,
            fidelity: parts.fidelity,
            colors: None,
        };
        Ok((term, cp))
    }

    /// Cuts a checkpoint as [`Term::save`] does and keeps only that: the
    /// terminal, its page memory and the analyzer go when this is dropped.
    pub(crate) fn pack(&mut self) -> Result<Packed, Uncut> {
        let mut blob = self.save()?;
        blob.shrink_to_fit();
        Ok(Packed {
            blob,
            cols: self.em.cols(),
            rows: self.em.rows(),
            fidelity: self.fidelity,
            counters: self.em.counters(),
            colors: self.colors,
        })
    }

    /// The analyzer's completed lines, for the debug report.
    pub(crate) fn lines(&self, n: u32) -> Vec<String> {
        self.analyzer.output(Some(n))
    }
}

/// A [`Term`] put away as the checkpoint blob it saved, a fraction of its
/// live size, with what the blob does not carry.
pub(crate) struct Packed {
    /// Format [`FORMAT`], cut and checked by [`Term::save`].
    pub(crate) blob: Vec<u8>,
    pub(crate) cols: u16,
    pub(crate) rows: u16,
    pub(crate) fidelity: Fidelity,
    counters: Counters,
    colors: Option<([u8; 3], [u8; 3])>,
}

impl Packed {
    /// The terminal back. No restore check: the blob passed one when it
    /// was saved, and the terminal being put away was swapped for its
    /// decode then, so decoding it again rebuilds that same terminal. The
    /// check walks every cell, which a wake on attach cannot afford.
    pub(crate) fn wake(&self) -> Result<Term, Rejected> {
        let (mut term, _) = Term::unpack(&self.blob)?;
        term.em.set_counters(self.counters);
        term.set_colors(self.colors);
        Ok(term)
    }
}

struct Parts<'a> {
    fidelity: Fidelity,
    status: u32,
    carry: &'a [u8],
    screen: &'a [u8],
    analysis: &'a [u8],
}

fn split(blob: &[u8]) -> Option<Parts<'_>> {
    let (&fid, rest) = blob.split_first()?;
    let fidelity = match fid {
        0 => Fidelity::Exact,
        1 => Fidelity::Approximate,
        _ => return None,
    };
    let (status, rest) = take(rest, 4)?;
    let (&n, rest) = rest.split_first()?;
    if n > 3 {
        return None;
    }
    let (carry, rest) = take(rest, usize::from(n))?;
    let (screen, rest) = sized(rest)?;
    let (analysis, rest) = sized(rest)?;
    if !rest.is_empty() {
        return None;
    }
    Some(Parts {
        fidelity,
        status: u32::from_le_bytes(status.try_into().ok()?),
        carry,
        screen,
        analysis,
    })
}

fn take(b: &[u8], n: usize) -> Option<(&[u8], &[u8])> {
    (b.len() >= n).then(|| b.split_at(n))
}

fn sized(b: &[u8]) -> Option<(&[u8], &[u8])> {
    let (len, rest) = take(b, 4)?;
    let len = u32::from_le_bytes(len.try_into().ok()?) as usize;
    take(rest, len)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fed(chunks: &[&[u8]]) -> Term {
        let mut t = Term::fresh(20, 4, 0).unwrap();
        let mut fx = Vec::new();
        for c in chunks {
            t.feed(c, true, &mut fx);
        }
        t
    }

    #[test]
    fn a_split_utf8_sequence_reaches_the_analyzer_whole() {
        let t = fed(&[b"caf\xc3", b"\xa9\r\n", b"bad \xff byte\r\n"]);
        assert_eq!(t.lines(0), ["café", "bad \u{fffd} byte"]);
    }

    #[test]
    fn a_saved_term_loads_back_and_carries_on_alike() {
        let mut a = fed(&[b"\x1b[31mone\r\ntwo \xe2\x82", b"\xac\r\n$ "]);
        let blob = a.save().unwrap();
        let mut b = Term::load(&blob).map_err(|(r, _)| r).unwrap();
        let mut fx = Vec::new();
        for t in [&mut a, &mut b] {
            t.feed(b"three\r\n(y/n) ", true, &mut fx);
        }
        assert_eq!(a.em.fingerprint(), b.em.fingerprint());
        assert_eq!(a.lines(0), b.lines(0));
        assert_eq!(a.status, b.status);
    }

    #[test]
    fn default_colours_answer_queries_and_survive_a_checkpoint() {
        let mut t = Term::fresh(20, 4, 0).unwrap();
        t.set_colors(Some(([0xd4, 0xd4, 0xd8], [0x14, 0x14, 0x16])));
        let mut fx = Vec::new();
        t.feed(b"hi\x1b]11;?\x1b\\", true, &mut fx);
        let replies: Vec<_> = fx
            .iter()
            .filter_map(|f| match f {
                Effect::Reply(b) => Some(String::from_utf8_lossy(b).into_owned()),
                _ => None,
            })
            .collect();
        assert_eq!(replies.len(), 1, "{replies:?}");
        assert!(replies[0].contains("1414/1414/1616"), "{replies:?}");
        let blob = t.save().expect("a checkpoint with default colours set");
        assert!(Term::load(&blob).is_ok());
        fx.clear();
        t.feed(b"\x1b]10;?\x1b\\", true, &mut fx);
        assert_eq!(fx.len(), 1, "still answered after the cut: {fx:?}");
    }

    #[test]
    fn a_packed_term_wakes_as_the_one_put_away_and_carries_on_alike() {
        let mut a = fed(&[b"\x1b[31mone\r\ntwo\x1b[3J \xe2\x82", b"\xac\r\n$ \x1b]0;t"]);
        a.set_colors(Some(([1, 2, 3], [4, 5, 6])));
        let packed = a.pack().unwrap();
        let mut b = packed.wake().unwrap();
        assert_eq!(a.em.fingerprint(), b.em.fingerprint());
        assert_eq!(a.em.counters(), b.em.counters());
        assert_eq!(b.em.history_clears(), 1);
        let mut fx = Vec::new();
        for t in [&mut a, &mut b] {
            t.feed(b"itle\x07three\r\n(y/n) \x1b]11;?\x1b\\", true, &mut fx);
        }
        assert_eq!(a.em.fingerprint(), b.em.fingerprint());
        assert_eq!(a.em.counters(), b.em.counters());
        assert_eq!(a.lines(0), b.lines(0));
        assert_eq!(a.status, b.status);
        let replies = fx.iter().filter(|f| matches!(f, Effect::Reply(_))).count();
        assert_eq!(replies, 2, "both answer colour queries: {fx:?}");
    }

    #[test]
    fn damaged_blobs_are_rejected_not_trusted() {
        let mut a = fed(&[b"hello"]);
        let blob = a.save().unwrap();
        for n in 0..blob.len() {
            assert!(Term::load(&blob[..n]).is_err(), "{n}");
        }
        let mut longer = blob.clone();
        longer.push(0);
        assert!(Term::load(&longer).is_err());
    }
}
