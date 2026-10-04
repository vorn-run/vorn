//! A session's disk history as a session engine keeps it: one log file in
//! the server's history log format, version 2 (`history/log.ts`), written
//! once per record.
//!
//! The header names the cursor the log starts at, and the writer knows the
//! cursor after the last record it holds. A record below that is one the
//! file already has: a recovering engine replays records the dead one
//! already wrote, and they are skipped here rather than written twice. Gap
//! and Exit records have no frame kind in this format yet; they are
//! skipped too, and the next record after them is written at its own place.
//!
//! A file torn by a crash mid-append is cut back to its last whole frame
//! when it is opened, so appending carries on after good data.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use vorn_term_proto::{Cursor, Entry, Record, Stream};

use crate::frame::{self, KIND_DATA, KIND_RESIZE, PREFIX_BYTES, RECORD_HEADER_BYTES};

const MAGIC: &[u8; 4] = b"VRNL";
const FORMAT_VERSION: u8 = 2;
/// Magic, version, generation, then the start cursor.
pub const HEADER_BYTES: usize = 4 + 1 + 4 + 4 + 8 + 8;

/// One session's history log, open for appending.
#[derive(Debug)]
pub struct History {
    file: File,
    generation: u32,
    start: Cursor,
    /// After the last record the file holds.
    next: Cursor,
}

/// One frame read back from a log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub rseq: u64,
    pub start_offset: u64,
    pub record: Record,
}

impl History {
    /// Opens the log at `path` for `generation`. A log of the same
    /// generation and epoch carries on from its last whole frame; anything
    /// else is replaced by an empty log starting at `start`.
    pub fn open(path: &Path, generation: u32, start: Cursor) -> io::Result<History> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        if let Some((gen, from, frames, good)) = parse(&bytes) {
            if gen == generation && from.epoch == start.epoch {
                let next = frames.last().map_or(from, |f| Cursor {
                    epoch: from.epoch,
                    next_rseq: f.rseq + 1,
                    next_offset: f.start_offset + f.record.len(),
                });
                file.set_len(good as u64)?;
                file.seek(SeekFrom::End(0))?;
                return Ok(History {
                    file,
                    generation,
                    start: from,
                    next,
                });
            }
        }
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&header(generation, start))?;
        Ok(History {
            file,
            generation,
            start,
            next: start,
        })
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// Where the log starts.
    pub fn start(&self) -> Cursor {
        self.start
    }

    /// The first record the log does not hold.
    pub fn cursor(&self) -> Cursor {
        self.next
    }

    /// Writes `entry` unless the log already holds it. Returns whether it
    /// wrote anything.
    pub fn append(&mut self, entry: &Entry) -> io::Result<bool> {
        if entry.hdr.epoch != self.next.epoch || entry.hdr.rseq < self.next.next_rseq {
            return Ok(false);
        }
        let at = frame::Record {
            rseq: entry.hdr.rseq,
            start_offset: entry.hdr.start_offset,
        };
        let mut out = Vec::new();
        match &entry.rec {
            Record::Data { stream, bytes } => {
                frame::data_from(&mut out, at, stream_byte(*stream), bytes)
            }
            &Record::Resize { cols, rows, .. } => frame::resize(&mut out, at, cols, rows),
            Record::Gap { .. } | Record::Exit { .. } => {}
        }
        if !out.is_empty() {
            self.file.write_all(&out)?;
        }
        self.next = entry.after();
        Ok(!out.is_empty())
    }
}

fn stream_byte(s: Stream) -> u8 {
    match s {
        Stream::Pty => frame::STREAM_PTY,
        Stream::Stdout => frame::STREAM_STDOUT,
        Stream::Stderr => frame::STREAM_STDERR,
    }
}

fn header(generation: u32, start: Cursor) -> Vec<u8> {
    let mut h = Vec::with_capacity(HEADER_BYTES);
    h.extend_from_slice(MAGIC);
    h.push(FORMAT_VERSION);
    h.extend_from_slice(&generation.to_le_bytes());
    h.extend_from_slice(&start.epoch.to_le_bytes());
    h.extend_from_slice(&start.next_rseq.to_le_bytes());
    h.extend_from_slice(&start.next_offset.to_le_bytes());
    h
}

/// Reads a log: its generation, start cursor, every whole frame, and how
/// many bytes those take. `None` for a file that is not a version 2 log.
/// Reading stops at the first torn or damaged frame, or a kind it does not
/// know, as the server's reader does.
pub fn parse(bytes: &[u8]) -> Option<(u32, Cursor, Vec<Frame>, usize)> {
    if bytes.len() < HEADER_BYTES || &bytes[..4] != MAGIC || bytes[4] != FORMAT_VERSION {
        return None;
    }
    let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap_or([0; 4]));
    let u64_at = |b: &[u8], i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap_or([0; 8]));
    let generation = u32_at(5);
    let start = Cursor {
        epoch: u32_at(9),
        next_rseq: u64_at(bytes, 13),
        next_offset: u64_at(bytes, 21),
    };
    let mut frames = Vec::new();
    let mut at = HEADER_BYTES;
    while bytes.len() >= at + PREFIX_BYTES {
        let kind = bytes[at];
        let len = u32_at(at + 1) as usize;
        let crc = u32_at(at + 5);
        let Some(payload) = bytes.get(at + PREFIX_BYTES..at + PREFIX_BYTES + len) else {
            break;
        };
        if payload.len() < RECORD_HEADER_BYTES || crc32fast::hash(payload) != crc {
            break;
        }
        let (rseq, start_offset) = (u64_at(payload, 0), u64_at(payload, 8));
        let body = &payload[RECORD_HEADER_BYTES..];
        let record = match kind {
            KIND_DATA if !body.is_empty() => Record::Data {
                stream: match body[0] {
                    frame::STREAM_STDOUT => Stream::Stdout,
                    frame::STREAM_STDERR => Stream::Stderr,
                    _ => Stream::Pty,
                },
                bytes: body[1..].to_vec(),
            },
            KIND_RESIZE if body.len() == 8 => Record::Resize {
                cols: u16::from_le_bytes([body[0], body[1]]),
                rows: u16::from_le_bytes([body[2], body[3]]),
                px_w: 0,
                px_h: 0,
                req: None,
            },
            _ => break,
        };
        frames.push(Frame {
            rseq,
            start_offset,
            record,
        });
        at += PREFIX_BYTES + len;
    }
    Some((generation, start, frames, at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vorn_term_proto::RecordHeader;

    fn entry(rseq: u64, offset: u64, rec: Record) -> Entry {
        Entry {
            hdr: RecordHeader {
                epoch: 1,
                rseq,
                start_offset: offset,
            },
            at_ns: 0,
            rec,
        }
    }

    fn data(s: &str) -> Record {
        Record::Data {
            stream: Stream::Pty,
            bytes: s.as_bytes().to_vec(),
        }
    }

    fn rseqs(path: &Path) -> Vec<u64> {
        let bytes = std::fs::read(path).unwrap();
        parse(&bytes).unwrap().2.iter().map(|f| f.rseq).collect()
    }

    /// RC-T6 at the writer: records a recovering engine offers again are
    /// held once, and the log carries on after them.
    #[test]
    fn a_replayed_record_is_written_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.log");
        let log = [
            entry(0, 0, data("ab")),
            entry(
                1,
                2,
                Record::Resize {
                    cols: 10,
                    rows: 5,
                    px_w: 0,
                    px_h: 0,
                    req: None,
                },
            ),
            entry(2, 2, data("cd")),
            entry(
                3,
                4,
                Record::Gap {
                    lost_bytes: 3,
                    reason: vorn_term_proto::GapReason::SpoolFull,
                },
            ),
            entry(4, 7, data("e")),
        ];
        let mut h = History::open(&path, 3, Cursor::start(1)).unwrap();
        for e in &log[..3] {
            assert!(h.append(e).unwrap());
        }
        drop(h);
        // A new engine replays from rseq 1.
        let mut h = History::open(&path, 3, Cursor::start(1)).unwrap();
        assert_eq!(h.cursor().next_rseq, 3);
        for e in &log[1..] {
            h.append(e).unwrap();
        }
        assert_eq!(h.cursor(), log[4].after());
        assert_eq!(rseqs(&path), [0, 1, 2, 4]);
    }

    #[test]
    fn a_torn_tail_is_cut_and_written_over() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.log");
        let mut h = History::open(&path, 1, Cursor::start(1)).unwrap();
        h.append(&entry(0, 0, data("hello"))).unwrap();
        h.append(&entry(1, 5, data("world"))).unwrap();
        drop(h);
        let len = std::fs::metadata(&path).unwrap().len();
        let f = OpenOptions::new().write(true).open(&path).unwrap();
        f.set_len(len - 2).unwrap();
        let mut h = History::open(&path, 1, Cursor::start(1)).unwrap();
        assert_eq!(h.cursor().next_rseq, 1);
        h.append(&entry(1, 5, data("world"))).unwrap();
        assert_eq!(rseqs(&path), [0, 1]);
    }

    #[test]
    fn another_generation_starts_a_new_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.log");
        let mut h = History::open(&path, 1, Cursor::start(1)).unwrap();
        h.append(&entry(0, 0, data("old"))).unwrap();
        drop(h);
        let from = Cursor {
            epoch: 1,
            next_rseq: 9,
            next_offset: 40,
        };
        let h = History::open(&path, 2, from).unwrap();
        assert_eq!(h.cursor(), from);
        assert!(rseqs(&path).is_empty());
    }
}
