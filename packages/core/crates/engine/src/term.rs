//! One session's terminal and what reads the output beside it: the
//! emulator, the output analyzer and the state that ties them to a point in
//! the stream. A checkpoint blob is all of it, as bytes, and a terminal left
//! idle is kept as nothing more ([`Packed`]).
//!
//! Blob layout, format [`FORMAT`]: the fidelity byte, the last agent status
//! (u32), the UTF-8 bytes a record split for the analyzer (a length byte and
//! up to three bytes), then the screen checkpoint, the analyzer's state and
//! the program status records, each as a u32 length and its bytes. All
//! integers little-endian. Format 2 had no records.

use vorn_analysis::rules::{Cursor, Rules, Screen};
use vorn_analysis::Analyzer;
use vorn_screen::{Checkpoint, Counters, Effect, Emulator, Theme, Uncut};

use crate::program::Store;

/// The checkpoint format this engine writes. It reads this and the one
/// before it; a checkpoint in any other format is not a restore base.
pub const FORMAT: u16 = 3;

/// The format before program status records, read as having none.
pub(crate) const FORMAT_NO_PROGRAMS: u16 = 2;

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
    /// The theme colour queries are answered with, kept across a reset.
    colors: Option<Theme>,
    /// The agent's screen rules, which decide its status when one holds; the
    /// analyzer's patterns decide when none does. Not part of a checkpoint:
    /// the session sets them.
    pub(crate) rules: Option<&'static Rules>,
    /// The rows the rules last read, kept for their allocations.
    rows: Vec<String>,
    /// The program's OSC 7501 records.
    pub(crate) programs: Store,
}

/// Every terminal lets its programs draw kitty images sent in the stream;
/// loading them from files waits for [`Term::set_local`].
fn drawing_images(mut em: Emulator) -> Emulator {
    em.set_images(true);
    em
}

impl Term {
    pub(crate) fn fresh(cols: u16, rows: u16, scrollback: usize) -> vorn_screen::Result<Term> {
        Ok(Term {
            em: drawing_images(Emulator::with_scrollback(
                u32::from(cols),
                u32::from(rows),
                scrollback,
            )?),
            analyzer: Analyzer::new(),
            carry: Vec::new(),
            status: vorn_analysis::STATUS_NONE,
            fidelity: Fidelity::Exact,
            colors: None,
            rules: None,
            rows: Vec::new(),
            programs: Store::default(),
        })
    }

    /// Says whether the program runs on this machine: only then may kitty
    /// images load from files, temp files and shared memory. Not part of a
    /// checkpoint: the session sets it.
    pub(crate) fn set_local(&mut self, local: bool) {
        self.em.set_image_files(local);
    }

    /// Sets the theme, which OSC 4 and 10 to 12 queries are answered
    /// with. Not part of a checkpoint: each vornd sets its own.
    pub(crate) fn set_colors(&mut self, colors: Option<Theme>) {
        self.colors = colors;
        if colors.is_some() {
            // A terminal that refuses it answers with Ghostty's colours, as before.
            let _ = self.em.set_theme(colors.as_ref());
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
        let patterns = self.analyzer.append_str(&text, analyze);
        let status = match self.rules {
            Some(rules) if analyze => self.screen_status(rules).unwrap_or(patterns),
            _ => patterns,
        };
        if status == vorn_analysis::STATUS_NONE || status == self.status {
            return None;
        }
        self.status = status;
        Some(status)
    }

    /// The status the agent's screen rules read on the screen now:
    /// `STATUS_NONE` while a synchronized frame is half drawn, `None` when no
    /// rule holds.
    fn screen_status(&mut self, rules: &Rules) -> Option<u32> {
        if self.em.render_hold().is_some() {
            return Some(vorn_analysis::STATUS_NONE);
        }
        let (row, col, visible) = self.em.cursor();
        let height = self.em.rows();
        let top = match rules.span(height, row) {
            Some((from, to)) => {
                self.em.rows_text(from, to, &mut self.rows);
                from
            }
            None => {
                self.rows.clear();
                0
            }
        };
        let screen = Screen {
            rows: &self.rows,
            top,
            height,
            cursor: Cursor { row, col, visible },
            title: self.em.title(),
        };
        rules.status(&screen).map(|s| s.code())
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
        let local = self.em.image_files();
        self.em = drawing_images(Emulator::with_scrollback(
            u32::from(cols),
            u32::from(rows),
            scrollback,
        )?);
        self.carry.clear();
        self.fidelity = Fidelity::Approximate;
        self.set_colors(self.colors);
        self.set_local(local);
        Ok(())
    }

    /// Cuts a checkpoint here, carrying on from the terminal rebuilt from
    /// it (see [`Emulator::checkpoint`]), and returns the blob.
    pub(crate) fn save(&mut self) -> Result<Vec<u8>, Uncut> {
        // Cut without them, so the restore check compares like with like:
        // a rebuild has Ghostty's defaults until its vornd sets its own.
        if self.colors.is_some() {
            let _ = self.em.set_theme(None);
        }
        let cut = self.em.checkpoint();
        self.set_colors(self.colors);
        let screen = cut?.encode();
        let analysis = self.analyzer.save();
        let mut programs = Vec::new();
        self.programs.encode(&mut programs);
        let mut out = Vec::with_capacity(20 + screen.len() + analysis.len() + programs.len());
        out.push(match self.fidelity {
            Fidelity::Exact => 0,
            Fidelity::Approximate => 1,
        });
        out.extend_from_slice(&self.status.to_le_bytes());
        // At most three bytes: a longer run was decoded already.
        out.push(self.carry.len() as u8);
        out.extend_from_slice(&self.carry);
        for part in [&screen, &analysis, &programs] {
            out.extend_from_slice(&(part.len() as u32).to_le_bytes());
            out.extend_from_slice(part);
        }
        Ok(out)
    }

    /// The terminal a blob of `format` holds, after the restore check. A
    /// blob whose screen rebuilds but fails the check comes back as
    /// `Err((RestoreCheck, Some(term)))`, the best a session without a
    /// valid base has.
    pub(crate) fn load(blob: &[u8], format: u16) -> Result<Term, (Rejected, Option<Box<Term>>)> {
        let (term, cp) = Term::unpack(blob, format).map_err(|why| (why, None))?;
        if cp.matches(&term.em) {
            Ok(term)
        } else {
            Err((Rejected::RestoreCheck, Some(Box::new(term))))
        }
    }

    /// The terminal a blob holds, with its screen checkpoint, unchecked.
    fn unpack(blob: &[u8], format: u16) -> Result<(Term, Checkpoint), Rejected> {
        let parts = split(blob, format).ok_or(Rejected::Decode)?;
        let programs = match parts.programs {
            Some(b) => Store::decode(b).ok_or(Rejected::Decode)?,
            None => Store::default(),
        };
        let cp = Checkpoint::decode(parts.screen).ok_or(Rejected::Decode)?;
        let analyzer = Analyzer::restore(parts.analysis).ok_or(Rejected::Decode)?;
        let em = drawing_images(Emulator::restore(&cp).map_err(|_| Rejected::Rebuild)?);
        let term = Term {
            em,
            analyzer,
            carry: parts.carry.to_vec(),
            status: parts.status,
            fidelity: parts.fidelity,
            colors: None,
            rules: None,
            rows: Vec::new(),
            programs,
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
    colors: Option<Theme>,
}

impl Packed {
    /// The terminal back. No restore check: the blob passed one when it
    /// was saved, and the terminal being put away was swapped for its
    /// decode then, so decoding it again rebuilds that same terminal. The
    /// check walks every cell, which a wake on attach cannot afford.
    pub(crate) fn wake(&self) -> Result<Term, Rejected> {
        let (mut term, _) = Term::unpack(&self.blob, FORMAT)?;
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
    programs: Option<&'a [u8]>,
}

fn split(blob: &[u8], format: u16) -> Option<Parts<'_>> {
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
    let (programs, rest) = match format {
        FORMAT => {
            let (p, rest) = sized(rest)?;
            (Some(p), rest)
        }
        FORMAT_NO_PROGRAMS => (None, rest),
        _ => return None,
    };
    if !rest.is_empty() {
        return None;
    }
    Some(Parts {
        fidelity,
        status: u32::from_le_bytes(status.try_into().ok()?),
        carry,
        screen,
        analysis,
        programs,
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

    fn ruled() -> Term {
        static RULES: std::sync::OnceLock<Rules> = std::sync::OnceLock::new();
        let rules = RULES.get_or_init(|| {
            Rules::parse(
                r#"
agent = "test"
[[rule]]
status = "waiting"
any = ['Proceed\?']
[[rule]]
status = "working"
rows = [-2, -1]
any = ['esc to interrupt']
[[rule]]
status = "idle"
rows = [-1, -1]
any = ['^>']
"#,
            )
            .unwrap()
        });
        let mut t = Term::fresh(30, 4, 0).unwrap();
        t.rules = Some(rules);
        t
    }

    #[test]
    fn screen_rules_decide_the_status_from_what_is_shown() {
        let mut t = ruled();
        let mut fx = Vec::new();
        let mut feed = |t: &mut Term, b: &[u8]| t.feed(b, true, &mut fx);
        assert_eq!(
            feed(&mut t, b"\x1b[4;1H> "),
            Some(vorn_analysis::STATUS_IDLE)
        );
        assert_eq!(feed(&mut t, b"x"), None);
        assert_eq!(
            feed(&mut t, b"\x1b[3;1H* esc to interrupt\x1b[4;1H\x1b[2K"),
            Some(vorn_analysis::STATUS_RUNNING)
        );
        assert_eq!(
            feed(&mut t, b"\x1b[2;1HProceed?"),
            Some(vorn_analysis::STATUS_WAITING)
        );
        // With no rule holding, the analyzer's patterns decide: errors only
        // come from them.
        assert_eq!(
            feed(&mut t, b"\x1b[2J\x1b[1;1Hwork\r\nError: it broke\r\n"),
            Some(vorn_analysis::STATUS_ERROR)
        );
    }

    #[test]
    fn a_synchronized_frame_is_read_once_it_is_drawn() {
        let mut t = ruled();
        let mut fx = Vec::new();
        assert_eq!(t.feed(b"\x1b[?2026h\x1b[4;1H> ", true, &mut fx), None);
        assert_eq!(
            t.feed(b"\x1b[?2026l", true, &mut fx),
            Some(vorn_analysis::STATUS_IDLE)
        );
        // Without analysis, the rules are not read either.
        let mut t = ruled();
        assert_eq!(t.feed(b"\x1b[4;1H> ", false, &mut fx), None);
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
        let mut b = Term::load(&blob, FORMAT).map_err(|(r, _)| r).unwrap();
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
        let mut ansi = [[0; 3]; 16];
        ansi[1] = [0xef, 0x44, 0x44];
        t.set_colors(Some(Theme {
            cursor: Some([0xd4, 0xd4, 0xd8]),
            ansi: Some(ansi),
            ..Theme::new([0xd4, 0xd4, 0xd8], [0x14, 0x14, 0x16])
        }));
        let mut fx = Vec::new();
        t.feed(
            b"hi\x1b]11;?\x1b\\\x1b]12;?\x1b\\\x1b]4;1;?\x1b\\\x1b]4;16;?\x1b\\",
            true,
            &mut fx,
        );
        let replies: Vec<_> = fx
            .iter()
            .filter_map(|f| match f {
                Effect::Reply(b) => Some(String::from_utf8_lossy(b).into_owned()),
                _ => None,
            })
            .collect();
        assert_eq!(replies.len(), 4, "{replies:?}");
        assert!(replies[0].contains("1414/1414/1616"), "{replies:?}");
        assert!(
            replies[1].contains("d4d4/d4d4/d8d8"),
            "the cursor: {replies:?}"
        );
        assert!(
            replies[2].contains("efef/4444/4444"),
            "the theme's red: {replies:?}"
        );
        assert!(
            replies[3].contains("0000/0000/0000"),
            "Ghostty's cube: {replies:?}"
        );
        let blob = t.save().expect("a checkpoint with default colours set");
        assert!(Term::load(&blob, FORMAT).is_ok());
        fx.clear();
        t.feed(b"\x1b]10;?\x1b\\", true, &mut fx);
        assert_eq!(fx.len(), 1, "still answered after the cut: {fx:?}");
    }

    #[test]
    fn a_packed_term_wakes_as_the_one_put_away_and_carries_on_alike() {
        let mut a = fed(&[b"\x1b[31mone\r\ntwo\x1b[3J \xe2\x82", b"\xac\r\n$ \x1b]0;t"]);
        a.set_colors(Some(Theme::new([1, 2, 3], [4, 5, 6])));
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
    fn a_format_2_blob_loads_with_no_records() {
        let mut a = fed(&[b"hello"]);
        a.programs.report(vorn_screen::ProgramReport {
            state: vorn_screen::ProgramState::Working,
            kind: None,
            progress: Some(3),
            id: "a".into(),
            app: String::new(),
            title: String::new(),
            msg: "m".into(),
        });
        a.programs.take_changed();
        let blob = a.save().unwrap();
        let mut programs = Vec::new();
        a.programs.encode(&mut programs);
        let old = &blob[..blob.len() - 4 - programs.len()];
        let b = Term::load(old, FORMAT_NO_PROGRAMS)
            .map_err(|(r, _)| r)
            .unwrap();
        assert_eq!(b.programs, Store::default());
        assert!(Term::load(old, FORMAT).is_err());
        assert!(Term::load(&blob, FORMAT_NO_PROGRAMS).is_err());
        let c = Term::load(&blob, FORMAT).map_err(|(r, _)| r).unwrap();
        assert_eq!(c.programs, a.programs);
    }

    #[test]
    fn damaged_blobs_are_rejected_not_trusted() {
        let mut a = fed(&[b"hello"]);
        let blob = a.save().unwrap();
        for n in 0..blob.len() {
            assert!(Term::load(&blob[..n], FORMAT).is_err(), "{n}");
        }
        let mut longer = blob.clone();
        longer.push(0);
        assert!(Term::load(&longer, FORMAT).is_err());
    }
}
