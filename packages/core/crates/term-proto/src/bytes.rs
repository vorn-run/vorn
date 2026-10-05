//! The raw-byte stream a bytes client (xterm.js on the web, the phone, the
//! desktop renderer) reads: Terminal State Protocol §7 Bytes and Resized,
//! §12's reconnect rule and §14's frame version 2.
//!
//! A bytes client parses the session itself, so what it is sent must be the
//! session's records exactly, in rseq order, and it must always know its
//! [`Cursor`]: that is what lets it continue after a reconnect, or a vornd
//! restart, without a snapshot. So data records travel in [`BytesFrame`]s
//! that name the records they hold, and a resize travels in-band, between
//! two frames, as [`Piece::Resized`], at its place in the stream.
//!
//! Frame version 2, all integers big-endian, each u64 as two u32 halves so
//! JavaScript reads it exactly up to 2^53:
//!
//! ```text
//! [u8 version = 2][u8 id length][id][u32 epoch]
//! [u64 first_rseq][u64 last_rseq][u64 start_offset][data]
//! ```
//!
//! `data` is the bytes of data records `first_rseq..=last_rseq`, joined,
//! with `data[0]` at `start_offset`. After it the client's cursor is
//! `{epoch, last_rseq + 1, start_offset + data.len()}`. Version 1
//! (`[1][u32 seq][u8 id length][id][output]`) carried only a flush number,
//! so a version 1 client always reattaches with a snapshot.

use crate::position::{Cursor, Entry, Record};

/// The frame version this module writes.
pub const FRAME_V2: u8 = 2;

/// The most output one frame carries when it joins records (WP4's flush
/// size). A single record larger than this travels alone: splitting it would
/// give the client a cursor in the middle of a record.
pub const MAX_FLUSH: usize = 64 << 10;

/// Ids are UUIDs; the length must fit its byte.
pub const MAX_ID_BYTES: usize = 255;

/// Bytes before the id: the version and the id length.
const LEAD: usize = 2;
/// Bytes after the id and before the data: epoch and three u64s.
const FIXED: usize = 4 + 3 * 8;

/// Why bytes are not a version 2 frame, or one could not be written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// Shorter than its header says.
    Truncated,
    /// Another version, such as 1.
    Version(u8),
    /// An empty id, or one longer than [`MAX_ID_BYTES`].
    Id,
    /// `last_rseq` before `first_rseq`.
    Range,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::Truncated => f.write_str("bytes frame shorter than its header"),
            FrameError::Version(v) => write!(f, "bytes frame version {v}, expected {FRAME_V2}"),
            FrameError::Id => write!(f, "bytes frame id must be 1..{MAX_ID_BYTES} bytes"),
            FrameError::Range => f.write_str("bytes frame ends before it starts"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Data records `first_rseq..=last_rseq` of one session, as a client reads
/// them. Borrows the frame it was decoded from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BytesFrame<'a> {
    pub session: &'a str,
    pub epoch: u32,
    pub first_rseq: u64,
    pub last_rseq: u64,
    pub start_offset: u64,
    pub data: &'a [u8],
}

impl<'a> BytesFrame<'a> {
    /// The client's cursor once it has applied this frame.
    pub fn resume(&self) -> Cursor {
        Cursor {
            epoch: self.epoch,
            next_rseq: self.last_rseq + 1,
            next_offset: self.start_offset + self.data.len() as u64,
        }
    }

    /// Appends the encoded frame to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) -> Result<(), FrameError> {
        let id = self.session.as_bytes();
        if id.is_empty() || id.len() > MAX_ID_BYTES {
            return Err(FrameError::Id);
        }
        if self.last_rseq < self.first_rseq {
            return Err(FrameError::Range);
        }
        out.reserve(LEAD + id.len() + FIXED + self.data.len());
        out.push(FRAME_V2);
        // Checked above.
        out.push(id.len() as u8);
        out.extend_from_slice(id);
        put_header(
            out,
            self.epoch,
            self.first_rseq,
            self.last_rseq,
            self.start_offset,
        );
        out.extend_from_slice(self.data);
        Ok(())
    }

    /// Reads a version 2 frame.
    pub fn decode(bytes: &'a [u8]) -> Result<BytesFrame<'a>, FrameError> {
        let (&version, rest) = bytes.split_first().ok_or(FrameError::Truncated)?;
        if version != FRAME_V2 {
            return Err(FrameError::Version(version));
        }
        let (&n, rest) = rest.split_first().ok_or(FrameError::Truncated)?;
        let n = usize::from(n);
        if n == 0 {
            return Err(FrameError::Id);
        }
        if rest.len() < n + FIXED {
            return Err(FrameError::Truncated);
        }
        let (id, rest) = rest.split_at(n);
        let session = std::str::from_utf8(id).map_err(|_| FrameError::Id)?;
        let (fixed, data) = rest.split_at(FIXED);
        let epoch = u32::from_be_bytes([fixed[0], fixed[1], fixed[2], fixed[3]]);
        let first_rseq = u64_at(fixed, 4);
        let last_rseq = u64_at(fixed, 12);
        let start_offset = u64_at(fixed, 20);
        if last_rseq < first_rseq {
            return Err(FrameError::Range);
        }
        Ok(BytesFrame {
            session,
            epoch,
            first_rseq,
            last_rseq,
            start_offset,
            data,
        })
    }
}

fn put_header(out: &mut Vec<u8>, epoch: u32, first: u64, last: u64, start: u64) {
    out.extend_from_slice(&epoch.to_be_bytes());
    for v in [first, last, start] {
        out.extend_from_slice(&v.to_be_bytes());
    }
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut word = [0u8; 8];
    word.copy_from_slice(&b[at..at + 8]);
    u64::from_be_bytes(word)
}

/// Why a stream cannot go on from a client's cursor: it gets a Resync and a
/// VtSnapshot instead (TP §12, RC §6 flow G).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lost {
    /// A Gap record: the bytes it stands for cannot be sent.
    Gap,
    /// The next record held does not follow the cursor: records between
    /// them are not retained.
    NotRetained,
    /// The cursor names another epoch of the log.
    WrongEpoch,
}

impl Lost {
    /// The reason as the wire names it in `terminal:resync`.
    pub fn as_str(self) -> &'static str {
        match self {
            Lost::Gap => "gap",
            Lost::NotRetained => "notRetained",
            Lost::WrongEpoch => "wrongEpoch",
        }
    }
}

/// One step of a bytes client's stream, in record order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    /// An encoded [`BytesFrame`].
    Frame(Vec<u8>),
    /// A Resize record: the client resizes here, before the next frame.
    Resized { rseq: u64, cols: u16, rows: u16 },
    /// The Exit record, always last.
    Exit {
        rseq: u64,
        code: Option<i32>,
        signal: Option<i32>,
    },
    /// The stream stops here; the cursor is left before the record that
    /// stopped it.
    Lost(Lost),
}

/// Turns the records after `from` into what a bytes client is sent, and
/// moves `from` past them.
///
/// Records `from` already includes are skipped, so a run that overlaps what
/// the client has is safe to pass. Consecutive data records are joined into
/// frames of at most `max_flush` bytes (one larger record alone). Stops at
/// the first record that cannot be sent (a Gap, or one that does not follow
/// the cursor) with [`Piece::Lost`].
pub fn pack(
    session: &str,
    from: &mut Cursor,
    entries: &[Entry],
    max_flush: usize,
    out: &mut Vec<Piece>,
) -> Result<(), FrameError> {
    if session.is_empty() || session.len() > MAX_ID_BYTES {
        return Err(FrameError::Id);
    }
    let mut frame = Open::default();
    for e in entries {
        if from.includes(&e.hdr) {
            continue;
        }
        if e.hdr.epoch != from.epoch {
            frame.close(session, from, out);
            out.push(Piece::Lost(Lost::WrongEpoch));
            return Ok(());
        }
        // The open frame's records count as included for continuity.
        let at = frame.cursor(from);
        if !at.is_followed_by(&e.hdr) {
            frame.close(session, from, out);
            out.push(Piece::Lost(Lost::NotRetained));
            return Ok(());
        }
        match &e.rec {
            Record::Data { bytes, .. } => {
                if frame.len() > 0 && frame.len() + bytes.len() > max_flush {
                    frame.close(session, from, out);
                }
                frame.push(e.hdr.rseq, e.hdr.start_offset, bytes);
            }
            &Record::Resize { cols, rows, .. } => {
                frame.close(session, from, out);
                out.push(Piece::Resized {
                    rseq: e.hdr.rseq,
                    cols,
                    rows,
                });
                *from = e.after();
            }
            &Record::Exit { code, signal } => {
                frame.close(session, from, out);
                out.push(Piece::Exit {
                    rseq: e.hdr.rseq,
                    code,
                    signal,
                });
                *from = e.after();
            }
            Record::Gap { .. } => {
                frame.close(session, from, out);
                out.push(Piece::Lost(Lost::Gap));
                return Ok(());
            }
        }
    }
    frame.close(session, from, out);
    Ok(())
}

/// A frame being filled: its records and their bytes, encoded once closed.
#[derive(Default)]
struct Open {
    first: Option<(u64, u64)>,
    last: u64,
    data: Vec<u8>,
}

impl Open {
    fn len(&self) -> usize {
        self.data.len()
    }

    fn push(&mut self, rseq: u64, start_offset: u64, bytes: &[u8]) {
        if self.first.is_none() {
            self.first = Some((rseq, start_offset));
        }
        self.last = rseq;
        self.data.extend_from_slice(bytes);
    }

    /// Where the stream stands with this frame included.
    fn cursor(&self, from: &Cursor) -> Cursor {
        match self.first {
            None => *from,
            Some((_, start)) => Cursor {
                epoch: from.epoch,
                next_rseq: self.last + 1,
                next_offset: start + self.data.len() as u64,
            },
        }
    }

    fn close(&mut self, session: &str, from: &mut Cursor, out: &mut Vec<Piece>) {
        let Some((first, start)) = self.first.take() else {
            return;
        };
        let f = BytesFrame {
            session,
            epoch: from.epoch,
            first_rseq: first,
            last_rseq: self.last,
            start_offset: start,
            data: &self.data,
        };
        let mut buf = Vec::new();
        // The id was checked by `pack`, and records only move forward.
        if f.encode_into(&mut buf).is_ok() {
            *from = f.resume();
            out.push(Piece::Frame(buf));
        }
        self.data.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position::{GapReason, RecordHeader, Stream};

    /// A log of records with sessiond's headers, built in order.
    struct Log {
        at: Cursor,
        entries: Vec<Entry>,
    }

    impl Log {
        fn new(epoch: u32) -> Log {
            Log {
                at: Cursor::start(epoch),
                entries: Vec::new(),
            }
        }

        fn starting(at: Cursor) -> Log {
            Log {
                at,
                entries: Vec::new(),
            }
        }

        fn push(&mut self, rec: Record) -> &mut Log {
            let hdr = RecordHeader {
                epoch: self.at.epoch,
                rseq: self.at.next_rseq,
                start_offset: self.at.next_offset,
            };
            let e = Entry { hdr, at_ns: 0, rec };
            self.at = e.after();
            self.entries.push(e);
            self
        }

        fn data(&mut self, bytes: &[u8]) -> &mut Log {
            self.push(Record::Data {
                stream: Stream::Pty,
                bytes: bytes.to_vec(),
            })
        }

        fn resize(&mut self, cols: u16, rows: u16) -> &mut Log {
            self.push(Record::Resize {
                cols,
                rows,
                px_w: 0,
                px_h: 0,
                req: None,
            })
        }
    }

    /// What a client following the pieces ends with: its bytes, its resizes
    /// with the byte count they came at, and its cursor.
    fn follow(pieces: &[Piece], mut at: Cursor) -> (Vec<u8>, Vec<(u64, u16, u16)>, Cursor) {
        let mut bytes = Vec::new();
        let mut sizes = Vec::new();
        for p in pieces {
            match p {
                Piece::Frame(f) => {
                    let f = BytesFrame::decode(f).unwrap();
                    assert_eq!(f.first_rseq, at.next_rseq, "frames continue the cursor");
                    assert_eq!(f.start_offset, at.next_offset);
                    bytes.extend_from_slice(f.data);
                    at = f.resume();
                }
                &Piece::Resized { rseq, cols, rows } => {
                    assert_eq!(rseq, at.next_rseq);
                    sizes.push((bytes.len() as u64, cols, rows));
                    at.next_rseq += 1;
                }
                Piece::Exit { rseq, .. } => {
                    assert_eq!(*rseq, at.next_rseq);
                    at.next_rseq += 1;
                }
                Piece::Lost(_) => {}
            }
        }
        (bytes, sizes, at)
    }

    #[test]
    fn a_frame_round_trips_with_u64s_past_32_bits() {
        let f = BytesFrame {
            session: "a-session",
            epoch: 7,
            first_rseq: (1 << 40) + 3,
            last_rseq: (1 << 40) + 9,
            start_offset: (1 << 52) + 1,
            data: b"\x1b[31mhi\xf0\x9f",
        };
        let mut wire = Vec::new();
        f.encode_into(&mut wire).unwrap();
        assert_eq!(BytesFrame::decode(&wire), Ok(f));
        assert_eq!(
            f.resume(),
            Cursor {
                epoch: 7,
                next_rseq: (1 << 40) + 10,
                next_offset: (1 << 52) + 1 + 9,
            }
        );
    }

    /// The exact bytes `tests/terminal-frame.test.ts` decodes, so the two
    /// ends agree on the layout.
    #[test]
    fn the_layout_the_typescript_decoder_reads() {
        let mut wire = Vec::new();
        BytesFrame {
            session: "s",
            epoch: 1,
            first_rseq: 2,
            last_rseq: 3,
            start_offset: 5,
            data: b"ab",
        }
        .encode_into(&mut wire)
        .unwrap();
        #[rustfmt::skip]
        let want = [
            2, 1, b's', 0, 0, 0, 1,
            0, 0, 0, 0, 0, 0, 0, 2,
            0, 0, 0, 0, 0, 0, 0, 3,
            0, 0, 0, 0, 0, 0, 0, 5,
            b'a', b'b',
        ];
        assert_eq!(wire, want);
    }

    #[test]
    fn refuses_what_is_not_a_v2_frame() {
        let mut wire = Vec::new();
        BytesFrame {
            session: "s",
            epoch: 0,
            first_rseq: 0,
            last_rseq: 0,
            start_offset: 0,
            data: b"x",
        }
        .encode_into(&mut wire)
        .unwrap();
        for n in 0..wire.len() - 1 {
            assert!(BytesFrame::decode(&wire[..n]).is_err(), "{n}");
        }
        let mut v1 = wire.clone();
        v1[0] = 1;
        assert_eq!(BytesFrame::decode(&v1), Err(FrameError::Version(1)));
        let mut backwards = wire.clone();
        // first_rseq = 1, last_rseq = 0.
        backwards[2 + 1 + 4 + 7] = 1;
        assert_eq!(BytesFrame::decode(&backwards), Err(FrameError::Range));
        let long = "x".repeat(MAX_ID_BYTES + 1);
        let f = BytesFrame {
            session: &long,
            ..BytesFrame::decode(&wire).unwrap()
        };
        assert_eq!(f.encode_into(&mut Vec::new()), Err(FrameError::Id));
    }

    /// TP-T23 at unit level: a data record at offset 100 holding 20 bytes
    /// and a cursor cut right after it. The client's next byte is 120, and
    /// nothing is doubled or missing, for cursors before, between and after
    /// two resizes at the same offset.
    #[test]
    fn no_duplicate_bytes_at_a_boundary() {
        let mut log = Log::starting(Cursor {
            epoch: 1,
            next_rseq: 7,
            next_offset: 100,
        });
        log.data(b"0123456789abcdefghij")
            .resize(100, 30)
            .resize(90, 20)
            .data(b"after");
        let all = log.entries.clone();
        let whole: Vec<u8> = b"0123456789abcdefghijafter".to_vec();
        // Cut after the data record (rseq 8, offset 120), after each resize,
        // and from the start of the data record.
        for (cut, bytes_before) in [(7u64, 0usize), (8, 20), (9, 20), (10, 20)] {
            let mut from = Cursor {
                epoch: 1,
                next_rseq: cut,
                next_offset: 100 + bytes_before as u64,
            };
            let start = from;
            let mut out = Vec::new();
            pack("s", &mut from, &all, MAX_FLUSH, &mut out).unwrap();
            let (bytes, sizes, end) = follow(&out, start);
            assert_eq!(bytes, whole[bytes_before..], "cut at {cut}");
            assert_eq!(end, log.at);
            if cut == 8 {
                let Piece::Resized { rseq, .. } = out[0] else {
                    panic!("{out:?}")
                };
                assert_eq!(rseq, 8, "the first thing after 120 is the resize");
            }
            // Resizes come in their order, each once, before the bytes after them.
            let expect: Vec<_> = [(8, 100, 30), (9, 90, 20)]
                .into_iter()
                .filter(|(r, _, _)| *r >= cut)
                .map(|(_, c, r)| ((20 - bytes_before) as u64, c, r))
                .collect();
            assert_eq!(sizes, expect, "cut at {cut}");
        }
    }

    #[test]
    fn joins_records_into_flushes_of_at_most_max_flush() {
        let mut log = Log::new(0);
        for _ in 0..10 {
            log.data(&[b'x'; 30 << 10]);
        }
        log.data(&[b'y'; 100 << 10]).data(b"tail");
        let mut from = Cursor::start(0);
        let mut out = Vec::new();
        pack("s", &mut from, &log.entries, MAX_FLUSH, &mut out).unwrap();
        let sizes: Vec<usize> = out
            .iter()
            .map(|p| match p {
                Piece::Frame(f) => BytesFrame::decode(f).unwrap().data.len(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            sizes,
            [
                60 << 10,
                60 << 10,
                60 << 10,
                60 << 10,
                60 << 10,
                100 << 10,
                4
            ]
        );
        assert_eq!(from, log.at);
        assert_eq!(follow(&out, Cursor::start(0)).0.len(), (400 << 10) + 4);
    }

    #[test]
    fn stops_at_a_gap_and_at_records_it_does_not_have() {
        let mut log = Log::new(2);
        log.data(b"abc").push(Record::Gap {
            lost_bytes: 50,
            reason: GapReason::SpoolFull,
        });
        log.data(b"def");
        let mut from = Cursor::start(2);
        let mut out = Vec::new();
        pack("s", &mut from, &log.entries, MAX_FLUSH, &mut out).unwrap();
        assert_eq!(out.last(), Some(&Piece::Lost(Lost::Gap)));
        assert_eq!(from.next_rseq, 1, "left before the gap");

        // A cursor past the first record held, but before what is held next.
        let mut from = Cursor {
            epoch: 2,
            next_rseq: 0,
            next_offset: 0,
        };
        let mut out = Vec::new();
        pack("s", &mut from, &log.entries[2..], MAX_FLUSH, &mut out).unwrap();
        assert_eq!(out, [Piece::Lost(Lost::NotRetained)]);

        let mut other = Cursor::start(3);
        let mut out = Vec::new();
        pack("s", &mut other, &log.entries, MAX_FLUSH, &mut out).unwrap();
        assert_eq!(out, [Piece::Lost(Lost::WrongEpoch)]);
    }

    /// RC-T17's client half at unit level: data records of 0, 1 and 64 KiB,
    /// the cursor directly before a resize and a gap. A stream resumed from
    /// the cursor cut after record 7 sees byte 100 exactly once.
    #[test]
    fn a_resumed_stream_sees_each_byte_once() {
        for len in [0usize, 1, 64 << 10] {
            let mut log = Log::starting(Cursor {
                epoch: 0,
                next_rseq: 0,
                next_offset: 0,
            });
            log.data(&[b'a'; 100]);
            for _ in 1..7 {
                log.data(b"");
            }
            log.data(&vec![b'b'; len]).resize(10, 10).data(b"cd");
            let whole: Vec<u8> = log
                .entries
                .iter()
                .flat_map(|e| match &e.rec {
                    Record::Data { bytes, .. } => bytes.clone(),
                    _ => Vec::new(),
                })
                .collect();
            for cut in 0..log.entries.len() {
                let start = if cut == 0 {
                    Cursor::start(0)
                } else {
                    log.entries[cut - 1].after()
                };
                // What the client had, then what it is sent from its cursor.
                let mut from = Cursor::start(0);
                let mut first = Vec::new();
                pack("s", &mut from, &log.entries[..cut], MAX_FLUSH, &mut first).unwrap();
                assert_eq!(from, start);
                let mut rest = Vec::new();
                pack("s", &mut from, &log.entries, MAX_FLUSH, &mut rest).unwrap();
                let (a, _, _) = follow(&first, Cursor::start(0));
                let (b, _, end) = follow(&rest, start);
                assert_eq!([a, b].concat(), whole, "len {len} cut {cut}");
                assert_eq!(end, log.at);
            }
        }
    }
}
