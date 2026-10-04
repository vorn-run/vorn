//! Record logs: what sessiond would hand vornd, built in memory.
//!
//! A [`Log`] is one session's records in order, each stamped with the header
//! sessiond would give it ([`RecordHeader`]: epoch, rseq, start offset), and
//! the size the session started at. The harness keeps one epoch per log:
//! sessiond restarting a log (a new epoch after a Gap) is not modelled.

use serde::{Deserialize, Serialize};
use vorn_term_proto::{Cursor, Entry, Record, RecordHeader, Stream};

use crate::Error;

/// A terminal size in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

impl Size {
    pub const fn new(cols: u16, rows: u16) -> Self {
        Self { cols, rows }
    }
}

impl std::fmt::Display for Size {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}x{}", self.cols, self.rows)
    }
}

/// FNV-1a 64 over a byte stream, with its length: enough to say "the same
/// bytes in the same order" about 50 MB without keeping a copy of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Digest {
    pub len: u64,
    pub fnv: u64,
}

impl Default for Digest {
    fn default() -> Self {
        Self {
            len: 0,
            fnv: 0xcbf2_9ce4_8422_2325,
        }
    }
}

impl Digest {
    pub fn update(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.fnv ^= u64::from(b);
            self.fnv = self.fnv.wrapping_mul(0x0100_0000_01b3);
        }
        self.len += bytes.len() as u64;
    }

    pub fn of(bytes: &[u8]) -> Self {
        let mut d = Self::default();
        d.update(bytes);
        d
    }
}

/// One session's record log: entries numbered from rseq 0 in one epoch, with
/// no holes, so entry `i` has rseq `i`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Log {
    /// The size the session was spawned at, before any Resize record.
    pub size: Size,
    pub entries: Vec<Entry>,
}

impl Log {
    /// The epoch every entry carries.
    pub fn epoch(&self) -> u32 {
        self.entries.first().map_or(0, |e| e.hdr.epoch)
    }

    /// The cursor after the last record: a state that includes everything.
    pub fn end(&self) -> Cursor {
        self.entries
            .last()
            .map_or(Cursor::start(self.epoch()), Entry::after)
    }

    /// Bytes of output in the log, all streams together.
    pub fn data_len(&self) -> u64 {
        self.end().next_offset
    }

    /// The digest of every Data record's bytes in order: what a recovered
    /// session must have seen, once each, for T1-style checks.
    pub fn digest(&self) -> Digest {
        let mut d = Digest::default();
        for e in &self.entries {
            if let Record::Data { bytes, .. } = &e.rec {
                d.update(bytes);
            }
        }
        d
    }

    /// The records a state ending at `cursor` does not include: replay after
    /// a cursor. Fails when the cursor is not a record boundary of this log
    /// (another epoch, past the end, or an offset that disagrees with the
    /// record there), which is what a recovery that lost or invented bytes
    /// looks like from outside.
    pub fn after(&self, cursor: Cursor) -> Result<&[Entry], Error> {
        let at = self.check(cursor)?;
        Ok(&self.entries[at..])
    }

    /// The records a state ending at `cursor` includes: what a reference
    /// terminal for that state is fed.
    pub fn upto(&self, cursor: Cursor) -> Result<&[Entry], Error> {
        let at = self.check(cursor)?;
        Ok(&self.entries[..at])
    }

    /// The index of the first record `cursor` excludes.
    fn check(&self, cursor: Cursor) -> Result<usize, Error> {
        let bad = || Error::Cursor {
            cursor,
            end: self.end(),
        };
        if cursor.epoch != self.epoch() {
            return Err(bad());
        }
        let at = usize::try_from(cursor.next_rseq).map_err(|_| bad())?;
        let offset = match self.entries.get(at) {
            Some(e) => e.hdr.start_offset,
            None if at == self.entries.len() => self.end().next_offset,
            None => return Err(bad()),
        };
        if offset != cursor.next_offset {
            return Err(bad());
        }
        Ok(at)
    }

    /// Checks the numbering a log must have: one epoch, rseq from 0 without
    /// holes, offsets that add up. Transcripts and hand-built logs pass
    /// through here.
    pub fn validate(&self) -> Result<(), Error> {
        let mut at = Cursor::start(self.epoch());
        for e in &self.entries {
            if !at.is_followed_by(&e.hdr) {
                return Err(Error::Log(format!(
                    "record {:?} does not follow {:?}",
                    e.hdr, at
                )));
            }
            if let Record::Resize { cols, rows, .. } = e.rec {
                if cols == 0 || rows == 0 {
                    return Err(Error::Log(format!("resize to {cols}x{rows}")));
                }
            }
            at = e.after();
        }
        if self.size.cols == 0 || self.size.rows == 0 {
            return Err(Error::Log(format!("starting size {}", self.size)));
        }
        Ok(())
    }
}

/// Stamps headers onto records in the order they are pushed, as sessiond's
/// reader does: every record gets the next rseq, and data moves the offset.
#[derive(Debug, Clone)]
pub struct LogBuilder {
    size: Size,
    next: Cursor,
    at_ns: u64,
    entries: Vec<Entry>,
}

impl LogBuilder {
    pub fn new(size: Size) -> Self {
        Self::with_epoch(size, 0)
    }

    pub fn with_epoch(size: Size, epoch: u32) -> Self {
        Self {
            size,
            next: Cursor::start(epoch),
            at_ns: 0,
            entries: Vec::new(),
        }
    }

    /// Where the next record goes.
    pub fn cursor(&self) -> Cursor {
        self.next
    }

    /// Moves the diagnostic clock the next records are stamped with.
    pub fn advance_ns(&mut self, ns: u64) -> &mut Self {
        self.at_ns = self.at_ns.saturating_add(ns);
        self
    }

    pub fn push(&mut self, rec: Record) -> &mut Self {
        let hdr = RecordHeader {
            epoch: self.next.epoch,
            rseq: self.next.next_rseq,
            start_offset: self.next.next_offset,
        };
        self.next = hdr.after(&rec);
        self.entries.push(Entry {
            hdr,
            at_ns: self.at_ns,
            rec,
        });
        self
    }

    /// One PTY read.
    pub fn data(&mut self, bytes: impl Into<Vec<u8>>) -> &mut Self {
        self.push(Record::Data {
            stream: Stream::Pty,
            bytes: bytes.into(),
        })
    }

    pub fn resize(&mut self, size: Size) -> &mut Self {
        self.push(Record::Resize {
            cols: size.cols,
            rows: size.rows,
            px_w: 0,
            px_h: 0,
            req: None,
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn build(self) -> Log {
        Log {
            size: self.size,
            entries: self.entries,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Log {
        let mut b = LogBuilder::new(Size::new(80, 24));
        b.data("hello ").resize(Size::new(100, 30)).data("world");
        b.build()
    }

    #[test]
    fn headers_number_records_and_bytes() {
        let log = sample();
        log.validate().unwrap();
        let hdrs: Vec<_> = log
            .entries
            .iter()
            .map(|e| (e.hdr.rseq, e.hdr.start_offset))
            .collect();
        // The resize shares an offset with the record after it, never an rseq.
        assert_eq!(hdrs, [(0, 0), (1, 6), (2, 6)]);
        assert_eq!(log.data_len(), 11);
        assert_eq!(log.digest(), Digest::of(b"hello world"));
    }

    #[test]
    fn replay_after_a_cursor() {
        let log = sample();
        let cut = log.entries[0].after();
        assert_eq!(log.after(cut).unwrap().len(), 2);
        assert_eq!(log.upto(cut).unwrap().len(), 1);
        assert!(log.after(log.end()).unwrap().is_empty());
        assert_eq!(log.after(Cursor::start(0)).unwrap().len(), 3);
    }

    #[test]
    fn a_cursor_off_the_boundaries_is_refused() {
        let log = sample();
        let mut lying = log.entries[0].after();
        lying.next_offset += 1;
        assert!(matches!(log.after(lying), Err(Error::Cursor { .. })));
        let past = Cursor {
            next_rseq: 9,
            ..log.end()
        };
        assert!(log.after(past).is_err());
        let other_epoch = Cursor::start(1);
        assert!(log.after(other_epoch).is_err());
    }

    #[test]
    fn validate_finds_holes() {
        let mut log = sample();
        log.entries.remove(1);
        assert!(log.validate().is_err());
    }

    #[test]
    fn digest_is_fnv1a() {
        // FNV-1a 64 test vector.
        assert_eq!(Digest::of(b"a").fnv, 0xaf63_dc4c_8601_ec8c);
        assert_eq!(Digest::of(b"").len, 0);
    }
}
