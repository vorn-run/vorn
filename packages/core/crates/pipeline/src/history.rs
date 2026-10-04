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
//! A record that does not follow the last one (records went missing, or
//! the epoch changed) starts a new segment: a new file whose header names
//! where it starts, so no file has a hole a reader cannot see. So does a
//! file grown past its cap, which keeps disk use bounded: the segment in use
//! is `<name>`, the one before it `<name>.1`, and older ones are dropped.
//!
//! A file torn by a crash mid-append is cut back to its last whole frame
//! when it is opened, so appending carries on after good data. Opening does
//! not read the whole file to find that frame: a small sidecar,
//! `<name>.tail`, records a frame boundary and the cursor there. It is
//! rewritten every [`TAIL_EVERY`] bytes and when the writer is dropped, and
//! only what follows it is read. Without a sidecar that matches, the file is
//! read from its header, which the cap bounds.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use vorn_term_proto::{Cursor, Entry, Record, Stream};

use crate::frame::{self, KIND_DATA, KIND_RESIZE, PREFIX_BYTES, RECORD_HEADER_BYTES};

const MAGIC: &[u8; 4] = b"VRNL";
const FORMAT_VERSION: u8 = 2;
/// Magic, version, generation, then the start cursor.
pub const HEADER_BYTES: usize = 4 + 1 + 4 + 4 + 8 + 8;

/// The cap on one segment unless [`History::with_cap`] sets another.
pub const DEFAULT_CAP: u64 = 32 << 20;
/// How much is appended between rewrites of the tail sidecar: about the
/// most an open after a crash reads.
pub const TAIL_EVERY: u64 = 256 << 10;

const TAIL_MAGIC: &[u8; 4] = b"VRNT";
/// Magic, generation, start cursor, length, the cursor there, CRC.
const TAIL_BYTES: usize = 4 + 4 + 20 + 8 + 20 + 4;

/// One session's history log, open for appending.
#[derive(Debug)]
pub struct History {
    file: File,
    path: PathBuf,
    generation: u32,
    /// Where the segment in use starts.
    start: Cursor,
    /// After the last record the log holds.
    next: Cursor,
    /// Bytes in the segment in use.
    len: u64,
    /// `len` when the sidecar was last written.
    hinted: u64,
    cap: u64,
}

/// One frame read back from a log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub rseq: u64,
    pub start_offset: u64,
    pub record: Record,
}

/// Where a file's whole frames end, and the cursor there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Tail {
    generation: u32,
    start: Cursor,
    len: u64,
    next: Cursor,
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
        let size = file.metadata()?.len();
        let mut head = [0u8; HEADER_BYTES];
        let header = if size >= HEADER_BYTES as u64 {
            file.read_exact(&mut head)?;
            read_header(&head)
        } else {
            None
        };
        let mut h = History {
            file,
            path: path.to_path_buf(),
            generation,
            start,
            next: start,
            len: 0,
            hinted: 0,
            cap: DEFAULT_CAP,
        };
        match header {
            Some((gen, from)) if gen == generation && from.epoch == start.epoch => {
                h.carry_on(from, size)?;
            }
            _ => h.restart(start)?,
        }
        Ok(h)
    }

    /// Caps each segment at about `bytes`: a record that would take it past
    /// that starts a new one instead.
    pub fn with_cap(mut self, bytes: u64) -> History {
        self.cap = bytes;
        self
    }

    /// Deletes the log at `path`, its older segment and its sidecar, as when
    /// the session they belong to is gone. Files already missing are fine.
    pub fn remove(path: &Path) -> io::Result<()> {
        for p in [path.to_path_buf(), older_path(path), tail_path(path)] {
            match std::fs::remove_file(&p) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        Ok(())
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// Where the segment in use starts.
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
        let hdr = &entry.hdr;
        if hdr.epoch < self.next.epoch || self.next.includes(hdr) {
            return Ok(false);
        }
        let at = Cursor {
            epoch: hdr.epoch,
            next_rseq: hdr.rseq,
            next_offset: hdr.start_offset,
        };
        if !self.next.is_followed_by(hdr) {
            // The records between are not in this log: a reader must not
            // take this one as following the last.
            self.segment(at)?;
        }
        let place = frame::Record {
            rseq: hdr.rseq,
            start_offset: hdr.start_offset,
        };
        let mut out = Vec::new();
        match &entry.rec {
            Record::Data { stream, bytes } => {
                frame::data_from(&mut out, place, stream_byte(*stream), bytes)
            }
            &Record::Resize { cols, rows, .. } => frame::resize(&mut out, place, cols, rows),
            Record::Gap { .. } | Record::Exit { .. } => {}
        }
        if !out.is_empty() {
            let has_frames = self.len > HEADER_BYTES as u64;
            if has_frames && self.len + out.len() as u64 > self.cap {
                self.segment(at)?;
            }
            self.file.write_all(&out)?;
            self.len += out.len() as u64;
        }
        self.next = entry.after();
        if self.len - self.hinted >= TAIL_EVERY {
            self.write_tail()?;
        }
        Ok(!out.is_empty())
    }

    /// Picks up a log of this generation from its last whole frame, reading
    /// only what follows the sidecar's boundary when the sidecar matches.
    fn carry_on(&mut self, from: Cursor, size: u64) -> io::Result<()> {
        let tail = read_tail(&tail_path(&self.path))
            .filter(|t| t.generation == self.generation && t.start == from && t.len <= size);
        let (at, mut next) = tail.map_or((HEADER_BYTES as u64, from), |t| (t.len, t.next));
        self.file.seek(SeekFrom::Start(at))?;
        let mut rest = Vec::new();
        self.file.read_to_end(&mut rest)?;
        let (frames, good) = frames(&rest);
        if let Some(f) = frames.last() {
            next = Cursor {
                epoch: from.epoch,
                next_rseq: f.rseq + 1,
                next_offset: f.start_offset + f.record.len(),
            };
        }
        self.len = at + good as u64;
        self.file.set_len(self.len)?;
        self.file.seek(SeekFrom::End(0))?;
        self.start = from;
        self.next = next;
        self.write_tail()
    }

    /// Keeps the segment in use as the older one and starts a new one at
    /// `at`.
    fn segment(&mut self, at: Cursor) -> io::Result<()> {
        std::fs::rename(&self.path, older_path(&self.path))?;
        self.file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.path)?;
        self.restart(at)
    }

    /// Makes the file an empty log starting at `at`.
    fn restart(&mut self, at: Cursor) -> io::Result<()> {
        self.file.set_len(0)?;
        self.file.seek(SeekFrom::Start(0))?;
        let h = header(self.generation, at);
        self.file.write_all(&h)?;
        self.start = at;
        self.next = at;
        self.len = h.len() as u64;
        self.write_tail()
    }

    /// Records where the file's whole frames end. A sidecar torn by a crash
    /// fails its CRC and is ignored.
    fn write_tail(&mut self) -> io::Result<()> {
        let t = Tail {
            generation: self.generation,
            start: self.start,
            len: self.len,
            next: self.next,
        };
        std::fs::write(tail_path(&self.path), encode_tail(&t))?;
        self.hinted = self.len;
        Ok(())
    }
}

impl Drop for History {
    /// Leaves the sidecar at the end, so the next open reads nothing past
    /// it.
    fn drop(&mut self) {
        if self.hinted != self.len {
            let _ = self.write_tail();
        }
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

fn tail_path(path: &Path) -> PathBuf {
    with_suffix(path, ".tail")
}

fn older_path(path: &Path) -> PathBuf {
    with_suffix(path, ".1")
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
    put_cursor(&mut h, start);
    h
}

fn put_cursor(out: &mut Vec<u8>, c: Cursor) {
    out.extend_from_slice(&c.epoch.to_le_bytes());
    out.extend_from_slice(&c.next_rseq.to_le_bytes());
    out.extend_from_slice(&c.next_offset.to_le_bytes());
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    b.get(i..i + 4)
        .and_then(|s| s.try_into().ok())
        .map_or(0, u32::from_le_bytes)
}

fn u64_at(b: &[u8], i: usize) -> u64 {
    b.get(i..i + 8)
        .and_then(|s| s.try_into().ok())
        .map_or(0, u64::from_le_bytes)
}

fn cursor_at(b: &[u8], i: usize) -> Cursor {
    Cursor {
        epoch: u32_at(b, i),
        next_rseq: u64_at(b, i + 4),
        next_offset: u64_at(b, i + 12),
    }
}

/// The generation and start cursor of a version 2 header.
fn read_header(bytes: &[u8]) -> Option<(u32, Cursor)> {
    if bytes.len() < HEADER_BYTES || &bytes[..4] != MAGIC || bytes[4] != FORMAT_VERSION {
        return None;
    }
    Some((u32_at(bytes, 5), cursor_at(bytes, 9)))
}

fn encode_tail(t: &Tail) -> Vec<u8> {
    let mut out = Vec::with_capacity(TAIL_BYTES);
    out.extend_from_slice(TAIL_MAGIC);
    out.extend_from_slice(&t.generation.to_le_bytes());
    put_cursor(&mut out, t.start);
    out.extend_from_slice(&t.len.to_le_bytes());
    put_cursor(&mut out, t.next);
    let crc = crc32fast::hash(&out);
    out.extend_from_slice(&crc.to_le_bytes());
    out
}

fn read_tail(path: &Path) -> Option<Tail> {
    let b = std::fs::read(path).ok()?;
    if b.len() != TAIL_BYTES || &b[..4] != TAIL_MAGIC {
        return None;
    }
    if crc32fast::hash(&b[..TAIL_BYTES - 4]) != u32_at(&b, TAIL_BYTES - 4) {
        return None;
    }
    let len = u64_at(&b, 28);
    if len < HEADER_BYTES as u64 {
        return None;
    }
    Some(Tail {
        generation: u32_at(&b, 4),
        start: cursor_at(&b, 8),
        len,
        next: cursor_at(&b, 36),
    })
}

/// Reads a log: its generation, start cursor, every whole frame, and how
/// many bytes those take. `None` for a file that is not a version 2 log.
/// Reading stops at the first torn or damaged frame, or a kind it does not
/// know, as the server's reader does.
pub fn parse(bytes: &[u8]) -> Option<(u32, Cursor, Vec<Frame>, usize)> {
    let (generation, start) = read_header(bytes)?;
    let (frames, good) = frames(&bytes[HEADER_BYTES..]);
    Some((generation, start, frames, HEADER_BYTES + good))
}

/// Every whole frame at the start of `bytes`, and how many bytes they take.
fn frames(bytes: &[u8]) -> (Vec<Frame>, usize) {
    let mut frames = Vec::new();
    let mut at = 0;
    while bytes.len() >= at + PREFIX_BYTES {
        let kind = bytes[at];
        let len = u32_at(bytes, at + 1) as usize;
        let crc = u32_at(bytes, at + 5);
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
    (frames, at)
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

    /// The start cursor and every frame's rseq of the log at `path`.
    fn read(path: &Path) -> (Cursor, Vec<u64>) {
        let bytes = std::fs::read(path).unwrap();
        let (_, start, frames, _) = parse(&bytes).unwrap();
        (start, frames.iter().map(|f| f.rseq).collect())
    }

    fn rseqs(path: &Path) -> Vec<u64> {
        read(path).1
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

    /// Opening reads only past the sidecar's boundary: a byte damaged
    /// before it is not seen (a full read would stop there and cut the
    /// file), and appending carries on at the end.
    #[test]
    fn the_sidecar_spares_reading_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.log");
        let mut h = History::open(&path, 1, Cursor::start(1)).unwrap();
        let mut offset = 0;
        for rseq in 0..100 {
            h.append(&entry(rseq, offset, data("0123456789"))).unwrap();
            offset += 10;
        }
        drop(h);
        let mut bytes = std::fs::read(&path).unwrap();
        // In the second frame.
        bytes[HEADER_BYTES + 36 + 30] ^= 0xff;
        std::fs::write(&path, &bytes).unwrap();
        let mut h = History::open(&path, 1, Cursor::start(1)).unwrap();
        assert_eq!(h.cursor().next_rseq, 100);
        h.append(&entry(100, offset, data("x"))).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().len() as usize,
            bytes.len() + 9 + 16 + 2
        );

        // Without the sidecar the whole file is read, and cut at the damage.
        drop(h);
        std::fs::remove_file(tail_path(&path)).unwrap();
        let h = History::open(&path, 1, Cursor::start(1)).unwrap();
        assert_eq!(h.cursor().next_rseq, 1);
    }

    /// A record that does not follow the last one starts a new segment, so
    /// neither file has a hole.
    #[test]
    fn a_hole_starts_a_new_segment() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.log");
        let mut h = History::open(&path, 1, Cursor::start(1)).unwrap();
        h.append(&entry(0, 0, data("ab"))).unwrap();
        h.append(&entry(1, 2, data("cd"))).unwrap();
        h.append(&entry(5, 20, data("ef"))).unwrap();
        let at = Cursor {
            epoch: 1,
            next_rseq: 5,
            next_offset: 20,
        };
        assert_eq!(h.start(), at);
        assert_eq!(read(&older_path(&path)), (Cursor::start(1), vec![0, 1]));
        assert_eq!(read(&path), (at, vec![5]));
        // And reopened, the log is the new segment.
        drop(h);
        let h = History::open(&path, 1, Cursor::start(1)).unwrap();
        assert_eq!(h.cursor().next_rseq, 6);
    }

    /// Past its cap a log moves on to a new segment, and keeps one older
    /// segment only.
    #[test]
    fn segments_are_capped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.log");
        let mut h = History::open(&path, 1, Cursor::start(1))
            .unwrap()
            .with_cap(1000);
        let chunk = "x".repeat(100);
        let mut offset = 0;
        for rseq in 0..100 {
            h.append(&entry(rseq, offset, data(&chunk))).unwrap();
            offset += 100;
        }
        let (start, now) = read(&path);
        let (older_start, older) = read(&older_path(&path));
        assert!(std::fs::metadata(&path).unwrap().len() <= 1000);
        assert!(std::fs::metadata(older_path(&path)).unwrap().len() <= 1000);
        assert_eq!(now.last(), Some(&99));
        assert_eq!(start.next_rseq, now[0]);
        assert_eq!(older_start.next_rseq, older[0]);
        assert_eq!(older.last().map(|r| r + 1), Some(now[0]));
        let files = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(files, 3, "the log, the one before and the sidecar");
    }

    #[test]
    fn remove_takes_every_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.log");
        let mut h = History::open(&path, 1, Cursor::start(1)).unwrap();
        h.append(&entry(0, 0, data("ab"))).unwrap();
        h.append(&entry(3, 9, data("cd"))).unwrap();
        drop(h);
        History::remove(&path).unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        History::remove(&path).unwrap();
    }
}
