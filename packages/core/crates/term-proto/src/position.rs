//! Where a record sits in a session's log, and where a state ends: the
//! Session Recovery Contract's section 4, shared with the Terminal State
//! Protocol's section 5 so a position means the same thing on both sides.
//!
//! A record header places one record. A cursor names the first record and the
//! first byte a state does *not* include. The two never stand in for each
//! other: resuming from a header's `start_offset` would deliver that record's
//! bytes twice.

/// Which of a session's outputs a data record came from. A PTY has one; a
/// piped agent has two that share one offset space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stream {
    Pty,
    Stdout,
    Stderr,
}

/// Why bytes are missing from a log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GapReason {
    /// The ring and the spool were both full while vornd was away.
    SpoolFull,
    /// sessiond restarted the log and could not carry the ring.
    SessiondRestart,
}

/// Stamped on every record by the process that read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RecordHeader {
    /// Changes only when sessiond restarts a session's log from scratch.
    pub epoch: u32,
    /// Every record gets one: data, resize, gap and exit.
    pub rseq: u64,
    /// Bytes of output before this record, all streams together. Never wraps.
    pub start_offset: u64,
}

/// The first record and the first byte a state does not include.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Cursor {
    pub epoch: u32,
    pub next_rseq: u64,
    pub next_offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    Data {
        stream: Stream,
        bytes: Vec<u8>,
    },
    Resize {
        cols: u16,
        rows: u16,
        px_w: u16,
        px_h: u16,
        /// The resize request this answers, when one did.
        req: Option<u64>,
    },
    Gap {
        lost_bytes: u64,
        reason: GapReason,
    },
    /// Always the last record of a session.
    Exit {
        code: Option<i32>,
        signal: Option<i32>,
    },
}

impl Record {
    /// How far the record moves the offset: its bytes, the bytes a gap stands
    /// for, or nothing for a resize or an exit.
    pub fn len(&self) -> u64 {
        match self {
            Record::Data { bytes, .. } => bytes.len() as u64,
            Record::Gap { lost_bytes, .. } => *lost_bytes,
            Record::Resize { .. } | Record::Exit { .. } => 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// One record as it travels between sessiond and vornd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub hdr: RecordHeader,
    /// Monotonic clock, for diagnostics only.
    pub at_ns: u64,
    pub rec: Record,
}

impl RecordHeader {
    /// The cursor of a state that includes this record and everything before it.
    pub fn after(&self, rec: &Record) -> Cursor {
        Cursor {
            epoch: self.epoch,
            next_rseq: self.rseq + 1,
            next_offset: self.start_offset + rec.len(),
        }
    }
}

impl Entry {
    pub fn after(&self) -> Cursor {
        self.hdr.after(&self.rec)
    }
}

impl Default for Cursor {
    fn default() -> Self {
        Cursor::start(0)
    }
}

impl Cursor {
    /// The start of a session's log.
    pub const fn start(epoch: u32) -> Self {
        Cursor {
            epoch,
            next_rseq: 0,
            next_offset: 0,
        }
    }

    /// Whether a state ending here already includes the record: replay after
    /// a cursor applies exactly the records this says no to.
    pub fn includes(&self, hdr: &RecordHeader) -> bool {
        hdr.epoch == self.epoch && hdr.rseq < self.next_rseq
    }

    /// Whether `hdr` is the next record after this cursor, as the invariant
    /// requires: the same epoch, the next number, starting at the next byte.
    pub fn is_followed_by(&self, hdr: &RecordHeader) -> bool {
        hdr.epoch == self.epoch
            && hdr.rseq == self.next_rseq
            && hdr.start_offset == self.next_offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(n: usize) -> Record {
        Record::Data {
            stream: Stream::Pty,
            bytes: vec![b'x'; n],
        }
    }

    fn resize() -> Record {
        Record::Resize {
            cols: 100,
            rows: 30,
            px_w: 0,
            px_h: 0,
            req: None,
        }
    }

    #[test]
    fn the_worked_example_resumes_after_the_record() {
        // RC §4: rseq 7 at offset 100 with 20 bytes.
        let hdr = RecordHeader {
            epoch: 1,
            rseq: 7,
            start_offset: 100,
        };
        let resume = hdr.after(&data(20));
        assert_eq!(
            resume,
            Cursor {
                epoch: 1,
                next_rseq: 8,
                next_offset: 120
            }
        );
        assert!(resume.includes(&hdr));
    }

    /// RC-T17 at unit level: a cursor cut after a record includes it once and
    /// the next record follows it, for empty, one-byte and 64 KiB data and
    /// with the cursor directly before a resize and a gap.
    #[test]
    fn cursor_boundary() {
        for len in [0usize, 1, 64 * 1024] {
            for next in [
                data(5),
                resize(),
                Record::Gap {
                    lost_bytes: 9,
                    reason: GapReason::SpoolFull,
                },
            ] {
                let first = Entry {
                    hdr: RecordHeader {
                        epoch: 3,
                        rseq: 7,
                        start_offset: 100,
                    },
                    at_ns: 0,
                    rec: data(len),
                };
                let cut = first.after();
                let second = RecordHeader {
                    epoch: 3,
                    rseq: 8,
                    start_offset: 100 + len as u64,
                };
                assert!(cut.includes(&first.hdr));
                assert!(!cut.includes(&second));
                assert!(cut.is_followed_by(&second));
                // The record after it moves the offset by what it stands for.
                assert_eq!(
                    second.after(&next).next_offset,
                    100 + len as u64 + next.len()
                );
            }
        }
    }

    #[test]
    fn resizes_at_one_offset_keep_their_order() {
        // RC-T18's premise: three resizes share an offset, never an rseq.
        let mut at = Cursor::start(0);
        let mut seen = Vec::new();
        for rseq in 0..3 {
            let hdr = RecordHeader {
                epoch: 0,
                rseq,
                start_offset: 0,
            };
            assert!(at.is_followed_by(&hdr));
            at = hdr.after(&resize());
            seen.push(at);
        }
        assert!(seen.iter().all(|c| c.next_offset == 0));
        assert_eq!(seen[2].next_rseq, 3);
    }

    #[test]
    fn another_epoch_is_never_included() {
        let cut = Cursor {
            epoch: 2,
            next_rseq: 50,
            next_offset: 1_000,
        };
        let old = RecordHeader {
            epoch: 1,
            rseq: 10,
            start_offset: 10,
        };
        assert!(!cut.includes(&old));
        assert!(!cut.is_followed_by(&RecordHeader {
            epoch: 1,
            rseq: 50,
            start_offset: 1_000
        }));
    }
}
