//! The disk spool behind a session's ring (RC §8): records at or after
//! `retain_from` that no longer fit in memory.
//!
//! The file uses the history log's framing, a CRC-32 per frame, so a torn or
//! damaged tail ends a read at the last whole record instead of handing vornd
//! a wrong byte:
//!
//! ```text
//! header = 'VRNS'  u8 version  u32le epoch
//! frame  = u8 kind  u32le payload_len  u32le crc32  payload      kind 0x01: a postcard Entry
//! ```
//!
//! Appends are not fsynced: the spool only has to survive vornd, not the machine.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use vorn_term_proto::Entry;

const MAGIC: &[u8; 4] = b"VRNS";
const VERSION: u8 = 1;
const KIND_ENTRY: u8 = 0x01;
const HEADER_BYTES: u64 = 4 + 1 + 4;
const FRAME_PREFIX: u64 = 1 + 4 + 4;

/// One session's spool file. Holds a contiguous run of records, oldest first.
pub struct Spool {
    path: PathBuf,
    epoch: u32,
    file: Option<BufWriter<File>>,
    /// Bytes on disk, header included; what counts against the budgets.
    bytes: u64,
    /// Records in the file.
    count: usize,
    /// The first and one past the last rseq in the file, when it holds any.
    first: Option<u64>,
    first_offset: Option<u64>,
    end: u64,
}

impl Spool {
    /// A spool at `path`, created on the first append. Any file already there
    /// belongs to an earlier sessiond and is replaced.
    pub fn new(path: impl Into<PathBuf>, epoch: u32) -> Self {
        Spool {
            path: path.into(),
            epoch,
            file: None,
            bytes: 0,
            count: 0,
            first: None,
            first_offset: None,
            end: 0,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The first rseq on disk.
    pub fn first(&self) -> Option<u64> {
        self.first
    }

    /// The first record's start offset.
    pub fn first_offset(&self) -> Option<u64> {
        self.first_offset
    }

    /// What appending this entry would add to the file, the header included
    /// when the file does not exist yet.
    pub fn cost_here(&self, entry: &Entry) -> u64 {
        FRAME_PREFIX + encoded_len(entry) + if self.file.is_none() { HEADER_BYTES } else { 0 }
    }

    /// Append one record. Records must arrive in rseq order with no holes.
    pub fn append(&mut self, entry: &Entry) -> io::Result<()> {
        debug_assert!(self.first.is_none() || entry.hdr.rseq == self.end);
        let payload = postcard::to_stdvec(entry).map_err(io::Error::other)?;
        if self.file.is_none() {
            let mut f = BufWriter::new(
                OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&self.path)?,
            );
            f.write_all(MAGIC)?;
            f.write_all(&[VERSION])?;
            f.write_all(&self.epoch.to_le_bytes())?;
            self.file = Some(f);
            self.bytes = HEADER_BYTES;
        }
        let f = self.file.as_mut().expect("opened above");
        f.write_all(&[KIND_ENTRY])?;
        f.write_all(&(payload.len() as u32).to_le_bytes())?;
        f.write_all(&crc32fast::hash(&payload).to_le_bytes())?;
        f.write_all(&payload)?;
        self.bytes += FRAME_PREFIX + payload.len() as u64;
        self.count += 1;
        self.first.get_or_insert(entry.hdr.rseq);
        self.first_offset.get_or_insert(entry.hdr.start_offset);
        self.end = entry.hdr.rseq + 1;
        Ok(())
    }

    /// Every record from `from_rseq` on, in order. A damaged frame ends the
    /// read: what follows it cannot be trusted to be the next record.
    pub fn read_from(&mut self, from_rseq: u64) -> io::Result<Vec<Entry>> {
        if self.is_empty() || from_rseq >= self.end {
            return Ok(Vec::new());
        }
        if let Some(f) = self.file.as_mut() {
            f.flush()?;
        }
        let mut out = Vec::new();
        for entry in read_file(&self.path, self.epoch)? {
            if entry.hdr.rseq >= from_rseq {
                out.push(entry);
            }
        }
        Ok(out)
    }

    /// Drop every record before `rseq`. The file is rewritten, or removed
    /// when nothing in it is still needed.
    pub fn trim_before(&mut self, rseq: u64) -> io::Result<()> {
        match self.first {
            Some(first) if first < rseq => {}
            _ => return Ok(()),
        }
        let keep = self.read_from(rseq)?;
        self.clear()?;
        for e in &keep {
            self.append(e)?;
        }
        Ok(())
    }

    /// Remove the file.
    pub fn clear(&mut self) -> io::Result<()> {
        self.file = None;
        self.bytes = 0;
        self.count = 0;
        self.first = None;
        self.first_offset = None;
        self.end = 0;
        match fs::remove_file(&self.path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

impl Drop for Spool {
    fn drop(&mut self) {
        let _ = self.clear();
    }
}

fn encoded_len(entry: &Entry) -> u64 {
    postcard::experimental::serialized_size(entry).unwrap_or(usize::MAX) as u64
}

/// Read a spool file's records up to the first damaged or torn frame.
pub fn read_file(path: &Path, epoch: u32) -> io::Result<Vec<Entry>> {
    let mut r = BufReader::new(File::open(path)?);
    let mut head = [0u8; HEADER_BYTES as usize];
    r.read_exact(&mut head)?;
    if &head[..4] != MAGIC || head[4] != VERSION || head[5..9] != epoch.to_le_bytes() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not this session's spool",
        ));
    }
    let mut out = Vec::new();
    loop {
        let mut prefix = [0u8; FRAME_PREFIX as usize];
        if r.read_exact(&mut prefix).is_err() {
            break;
        }
        let len = u32::from_le_bytes(prefix[1..5].try_into().unwrap()) as usize;
        let crc = u32::from_le_bytes(prefix[5..9].try_into().unwrap());
        if prefix[0] != KIND_ENTRY {
            break;
        }
        let mut payload = vec![0u8; len];
        if r.read_exact(&mut payload).is_err() || crc32fast::hash(&payload) != crc {
            break;
        }
        match postcard::from_bytes::<Entry>(&payload) {
            Ok(e) => out.push(e),
            Err(_) => break,
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vorn_term_proto::{Record, RecordHeader, Stream};

    fn entry(rseq: u64, n: usize) -> Entry {
        Entry {
            hdr: RecordHeader {
                epoch: 2,
                rseq,
                start_offset: rseq * 10,
            },
            at_ns: rseq,
            rec: Record::Data {
                stream: Stream::Pty,
                bytes: vec![b'a' + (rseq % 26) as u8; n],
            },
        }
    }

    #[test]
    fn records_come_back_in_order_from_any_rseq() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Spool::new(dir.path().join("a.log"), 2);
        for i in 0..10 {
            let e = entry(i, 10);
            let before = s.bytes();
            let cost = s.cost_here(&e);
            s.append(&e).unwrap();
            assert_eq!(s.bytes() - before, cost);
        }
        let back = s.read_from(4).unwrap();
        assert_eq!(back, (4..10).map(|i| entry(i, 10)).collect::<Vec<_>>());
        assert_eq!(s.bytes(), fs::metadata(s.path()).unwrap().len());
    }

    #[test]
    fn trimming_rewrites_or_removes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.log");
        let mut s = Spool::new(&path, 2);
        for i in 0..6 {
            s.append(&entry(i, 100)).unwrap();
        }
        let full = s.bytes();
        s.trim_before(3).unwrap();
        assert_eq!(s.first(), Some(3));
        assert!(s.bytes() < full);
        s.append(&entry(6, 1)).unwrap();
        assert_eq!(s.read_from(0).unwrap().len(), 4);
        s.trim_before(7).unwrap();
        assert!(s.is_empty());
        assert!(!path.exists());
    }

    #[test]
    fn a_damaged_frame_ends_the_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.log");
        let mut s = Spool::new(&path, 2);
        let mut starts = Vec::new();
        for i in 0..4 {
            starts.push(s.bytes().max(HEADER_BYTES) as usize);
            s.append(&entry(i, 50)).unwrap();
        }
        s.read_from(0).unwrap(); // flushes
        let mut bytes = fs::read(&path).unwrap();
        // Flip a byte inside the third record's payload.
        bytes[starts[2] + FRAME_PREFIX as usize + 20] ^= 0xff;
        fs::write(&path, &bytes).unwrap();
        let back = read_file(&path, 2).unwrap();
        assert_eq!(back.len(), 2);
        // A torn tail is the same.
        fs::write(&path, &bytes[..bytes.len() - 3]).unwrap();
        assert!(read_file(&path, 2).unwrap().len() <= 3);
        // Another session's or epoch's file is refused.
        assert!(read_file(&path, 3).is_err());
    }
}
