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
//! Each frame goes to the file in one write and a failed one is cut off
//! again, and a trim writes a new file and renames it over the old one, so a
//! failure leaves the spool as it was rather than ending in a torn frame.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use vorn_term_proto::Entry;

use crate::wire::SpoolState;

const MAGIC: &[u8; 4] = b"VRNS";
const VERSION: u8 = 1;
const KIND_ENTRY: u8 = 0x01;
const HEADER_BYTES: u64 = 4 + 1 + 4;
const FRAME_PREFIX: u64 = 1 + 4 + 4;
/// One index mark per this many records: a read seeks to the mark at or
/// before its first record and decodes at most this many it does not want.
const INDEX_EVERY: usize = 64;

/// One session's spool file. Holds a contiguous run of records, oldest first.
pub struct Spool {
    path: PathBuf,
    epoch: u32,
    /// Unbuffered: every append is one whole frame in one write, so the
    /// file's length is always `bytes` and a failure can be cut off exactly.
    /// Records reach the spool one at a time as they leave the ring, so a
    /// buffer would save few calls.
    file: Option<File>,
    /// Bytes on disk, header included; what counts against the budgets.
    bytes: u64,
    /// Records in the file.
    count: usize,
    /// The first and one past the last rseq in the file, when it holds any.
    first: Option<u64>,
    first_offset: Option<u64>,
    end: u64,
    /// `(rseq, file position)` of every [`INDEX_EVERY`]th record, the first
    /// included, so a reader that fell behind reads only what it needs.
    marks: Vec<(u64, u64)>,
    /// A failed append could not be cut off. Nothing more is appended after
    /// what may be a torn frame, since no reader could get past it; a trim
    /// writes a fresh file and clears this.
    torn: bool,
    /// Leave the file when dropped: while a handoff stages the session, the
    /// file is the donor's, and once it is done, the adopter's.
    keep: bool,
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
            marks: Vec::new(),
            torn: false,
            keep: false,
        }
    }

    /// Where this spool stands, for the sessiond a session is handed to;
    /// the file stays where it is.
    pub fn state(&self) -> SpoolState {
        SpoolState {
            bytes: self.bytes,
            count: self.count as u64,
            first: self.first,
            first_offset: self.first_offset,
            end: self.end,
            marks: self.marks.clone(),
            torn: self.torn,
        }
    }

    /// The spool another sessiond kept at `path`, standing where `st` says.
    /// The file must be that long, or longer only past a torn frame, and
    /// carry `epoch`'s header. It is kept when dropped until [`Spool::keep`]
    /// says otherwise.
    pub fn adopt(path: impl Into<PathBuf>, epoch: u32, st: &SpoolState) -> io::Result<Spool> {
        let path = path.into();
        let bad = |why: &str| io::Error::new(io::ErrorKind::InvalidData, why.to_owned());
        let count = usize::try_from(st.count).map_err(|_| bad("spool count"))?;
        if count == 0 {
            if st.bytes != 0 || st.first.is_some() {
                return Err(bad("an empty spool with bytes"));
            }
            let mut s = Spool::new(path, epoch);
            s.keep = true;
            return Ok(s);
        }
        let (Some(first), Some(_)) = (st.first, st.first_offset) else {
            return Err(bad("a spool with records but no first"));
        };
        if first.checked_add(st.count) != Some(st.end)
            || st.marks.first().map(|m| m.0) != Some(first)
        {
            return Err(bad("spool records do not add up"));
        }
        open_checked(&path, epoch)?;
        let mut file = OpenOptions::new().write(true).open(&path)?;
        let len = file.metadata()?.len();
        if len < st.bytes || (len > st.bytes && !st.torn) {
            return Err(bad("the spool file is not as long as said"));
        }
        file.seek(SeekFrom::Start(st.bytes))?;
        Ok(Spool {
            path,
            epoch,
            file: Some(file),
            bytes: st.bytes,
            count,
            first: st.first,
            first_offset: st.first_offset,
            end: st.end,
            marks: st.marks.clone(),
            torn: st.torn,
            keep: true,
        })
    }

    /// Whether dropping the spool leaves its file.
    pub fn keep(&mut self, keep: bool) {
        self.keep = keep;
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
    /// On failure the file holds what it held before the call.
    pub fn append(&mut self, entry: &Entry) -> io::Result<()> {
        debug_assert!(self.first.is_none() || entry.hdr.rseq == self.end);
        if self.torn {
            return Err(io::Error::other(
                "the spool may end in a torn frame; not appending after it",
            ));
        }
        let payload = postcard::to_stdvec(entry).map_err(io::Error::other)?;
        let len = u32::try_from(payload.len()).map_err(io::Error::other)?;
        let fresh = self.file.is_none();
        let at = if fresh { HEADER_BYTES } else { self.bytes };
        let mut frame = Vec::with_capacity((HEADER_BYTES + FRAME_PREFIX) as usize + payload.len());
        if fresh {
            frame.extend_from_slice(MAGIC);
            frame.push(VERSION);
            frame.extend_from_slice(&self.epoch.to_le_bytes());
        }
        frame.push(KIND_ENTRY);
        frame.extend_from_slice(&len.to_le_bytes());
        frame.extend_from_slice(&crc32fast::hash(&payload).to_le_bytes());
        frame.extend_from_slice(&payload);
        if fresh {
            self.file = Some(
                OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&self.path)?,
            );
        }
        let f = self.file.as_mut().expect("opened above");
        if let Err(e) = f.write_all(&frame) {
            self.roll_back();
            return Err(e);
        }
        if self.count.is_multiple_of(INDEX_EVERY) {
            self.marks.push((entry.hdr.rseq, at));
        }
        self.bytes = at + FRAME_PREFIX + payload.len() as u64;
        self.count += 1;
        self.first.get_or_insert(entry.hdr.rseq);
        self.first_offset.get_or_insert(entry.hdr.start_offset);
        self.end = entry.hdr.rseq + 1;
        Ok(())
    }

    /// Cut the file back to its last whole frame after a write that may have
    /// left part of one. A file that held no frame yet is removed instead, so
    /// the next append starts it over with its header.
    fn roll_back(&mut self) {
        if self.count == 0 {
            self.file = None;
            self.bytes = 0;
            let _ = fs::remove_file(&self.path);
            return;
        }
        let at = self.bytes;
        let cut = self
            .file
            .as_mut()
            .map(|f| f.set_len(at).and_then(|()| f.seek(SeekFrom::Start(at))));
        if !matches!(cut, Some(Ok(_))) {
            self.torn = true;
        }
    }

    /// The records from `from_rseq` on, in order: at most about `max_bytes`
    /// of output, but always at least one when there is any. Reads from the
    /// index mark before `from_rseq`, never the whole file. A damaged frame
    /// ends the read: what follows it cannot be trusted to be the next
    /// record.
    pub fn read_from(&self, from_rseq: u64, max_bytes: u64) -> io::Result<Vec<Entry>> {
        if self.is_empty() || from_rseq >= self.end {
            return Ok(Vec::new());
        }
        let mark = self
            .marks
            .partition_point(|&(rseq, _)| rseq <= from_rseq)
            .saturating_sub(1);
        let at = self.marks.get(mark).map_or(HEADER_BYTES, |&(_, pos)| pos);
        let mut r = open_checked(&self.path, self.epoch)?;
        r.seek(SeekFrom::Start(at))?;
        let mut out = Vec::new();
        let mut taken = 0u64;
        while let Some(entry) = next_frame(&mut r) {
            if entry.hdr.rseq < from_rseq {
                continue;
            }
            let n = entry.rec.len();
            if !out.is_empty() && taken + n > max_bytes {
                break;
            }
            taken += n;
            out.push(entry);
        }
        Ok(out)
    }

    /// Drop every record before `rseq`. The records kept are written to a new
    /// file that is renamed over this one, or the file is removed when
    /// nothing in it is still needed. On failure the spool is unchanged.
    pub fn trim_before(&mut self, rseq: u64) -> io::Result<()> {
        match self.first {
            Some(first) if first < rseq => {}
            _ => return Ok(()),
        }
        let keep = self.read_from(rseq, u64::MAX)?;
        if keep.is_empty() {
            return self.clear();
        }
        // Until the rename, `fresh` owns its file and removes it on any error.
        let mut fresh = Spool::new(self.path.with_extension("trim"), self.epoch);
        for e in &keep {
            fresh.append(e)?;
        }
        // Windows does not replace a file that is open; close ours first.
        self.file = None;
        if let Err(e) = fs::rename(&fresh.path, &self.path) {
            self.reopen();
            return Err(e);
        }
        // `fresh` is left with only the temporary name, which is gone, so its
        // drop removes nothing.
        self.file = fresh.file.take();
        self.bytes = fresh.bytes;
        self.count = fresh.count;
        self.first = fresh.first;
        self.first_offset = fresh.first_offset;
        self.end = fresh.end;
        self.marks = std::mem::take(&mut fresh.marks);
        self.torn = false;
        Ok(())
    }

    /// Open the file again to append at its end, after a failed trim closed
    /// it. If that fails too, the spool takes no more appends.
    fn reopen(&mut self) {
        let at = self.bytes;
        let reopened = OpenOptions::new()
            .write(true)
            .open(&self.path)
            .and_then(|mut f| f.seek(SeekFrom::Start(at)).map(|_| f));
        match reopened {
            Ok(f) => self.file = Some(f),
            Err(_) => self.torn = true,
        }
    }

    /// Remove the file.
    pub fn clear(&mut self) -> io::Result<()> {
        self.file = None;
        self.bytes = 0;
        self.count = 0;
        self.first = None;
        self.first_offset = None;
        self.end = 0;
        self.marks.clear();
        self.torn = false;
        match fs::remove_file(&self.path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

impl Drop for Spool {
    fn drop(&mut self) {
        if !self.keep {
            let _ = self.clear();
        }
    }
}

fn encoded_len(entry: &Entry) -> u64 {
    postcard::experimental::serialized_size(entry).unwrap_or(usize::MAX) as u64
}

/// Open a spool file for reading, past its header, checking it is this
/// session's.
fn open_checked(path: &Path, epoch: u32) -> io::Result<BufReader<File>> {
    let mut r = BufReader::new(File::open(path)?);
    let mut head = [0u8; HEADER_BYTES as usize];
    r.read_exact(&mut head)?;
    if &head[..4] != MAGIC || head[4] != VERSION || head[5..9] != epoch.to_le_bytes() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not this session's spool",
        ));
    }
    Ok(r)
}

/// The next whole frame's record, or None at the end of the file or at a
/// damaged or torn frame.
fn next_frame(r: &mut impl Read) -> Option<Entry> {
    let mut prefix = [0u8; FRAME_PREFIX as usize];
    r.read_exact(&mut prefix).ok()?;
    if prefix[0] != KIND_ENTRY {
        return None;
    }
    let len = u32::from_le_bytes(prefix[1..5].try_into().expect("4 bytes")) as usize;
    let crc = u32::from_le_bytes(prefix[5..9].try_into().expect("4 bytes"));
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload).ok()?;
    if crc32fast::hash(&payload) != crc {
        return None;
    }
    postcard::from_bytes::<Entry>(&payload).ok()
}

/// Read a spool file's records up to the first damaged or torn frame.
pub fn read_file(path: &Path, epoch: u32) -> io::Result<Vec<Entry>> {
    let mut r = open_checked(path, epoch)?;
    let mut out = Vec::new();
    while let Some(e) = next_frame(&mut r) {
        out.push(e);
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
        let back = s.read_from(4, u64::MAX).unwrap();
        assert_eq!(back, (4..10).map(|i| entry(i, 10)).collect::<Vec<_>>());
        assert_eq!(s.bytes(), fs::metadata(s.path()).unwrap().len());
    }

    /// A spool handed to another sessiond carries on in the same file, and
    /// only the side that holds the session removes it.
    #[test]
    fn an_adopted_spool_carries_on_in_the_same_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("h.log");
        let mut donor = Spool::new(&path, 2);
        for i in 0..70 {
            donor.append(&entry(i, 10)).unwrap();
        }
        let st = donor.state();
        let mut adopted = Spool::adopt(&path, 2, &st).unwrap();
        donor.keep(true);
        drop(donor);
        assert!(path.exists(), "the donor leaves the file");
        adopted.append(&entry(70, 10)).unwrap();
        assert_eq!(
            adopted.read_from(0, u64::MAX).unwrap(),
            (0..71).map(|i| entry(i, 10)).collect::<Vec<_>>()
        );
        assert_eq!(adopted.read_from(65, u64::MAX).unwrap().len(), 6);
        drop(adopted);
        assert!(path.exists(), "kept until the adopter owns it");
        let mut owned = Spool::adopt(&path, 2, &{
            let mut s = st.clone();
            s.bytes = fs::metadata(&path).unwrap().len();
            s.count += 1;
            s.end += 1;
            s
        })
        .unwrap();
        owned.keep(false);
        drop(owned);
        assert!(!path.exists());

        assert!(Spool::adopt(&path, 2, &st).is_err(), "no file");
        let mut other = Spool::new(&path, 3);
        other.append(&entry(0, 10)).unwrap();
        assert!(
            Spool::adopt(&path, 2, &other.state()).is_err(),
            "wrong epoch"
        );
        let mut short = other.state();
        short.bytes -= 1;
        assert!(Spool::adopt(&path, 3, &short).is_err(), "wrong length");
        let empty = Spool::new(dir.path().join("none.log"), 2).state();
        assert!(Spool::adopt(dir.path().join("none.log"), 2, &empty)
            .unwrap()
            .is_empty());
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
        assert_eq!(s.read_from(0, u64::MAX).unwrap().len(), 4);
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

    #[test]
    fn a_read_from_far_in_stops_at_its_byte_budget() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Spool::new(dir.path().join("b.log"), 2);
        for i in 0..200 {
            s.append(&entry(i, 10)).unwrap();
        }
        let rseqs = |v: Vec<Entry>| v.iter().map(|e| e.hdr.rseq).collect::<Vec<_>>();
        assert_eq!(rseqs(s.read_from(150, 25).unwrap()), vec![150, 151]);
        // Always at least one record, however small the budget.
        assert_eq!(rseqs(s.read_from(150, 0).unwrap()), vec![150]);
        // Exactly on an index mark, and just before one.
        assert_eq!(rseqs(s.read_from(128, 0).unwrap()), vec![128]);
        assert_eq!(rseqs(s.read_from(127, 10).unwrap()), vec![127]);
        assert_eq!(s.read_from(0, u64::MAX).unwrap().len(), 200);
        assert_eq!(s.read_from(199, u64::MAX).unwrap().len(), 1);
        assert!(s.read_from(200, u64::MAX).unwrap().is_empty());
        // The index survives a trim.
        s.trim_before(70).unwrap();
        assert_eq!(rseqs(s.read_from(140, 15).unwrap()), vec![140]);
        assert_eq!(rseqs(s.read_from(0, 0).unwrap()), vec![70]);
    }

    /// A write that left part of a frame is cut off, so the records appended
    /// after it can still be read.
    #[test]
    fn a_failed_append_leaves_no_torn_frame() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("r.log");
        let mut s = Spool::new(&path, 2);
        for i in 0..3 {
            s.append(&entry(i, 20)).unwrap();
        }
        // What a write that failed halfway leaves behind.
        s.file
            .as_mut()
            .unwrap()
            .write_all(&[KIND_ENTRY, 40, 0, 0, 0, 1, 2])
            .unwrap();
        s.roll_back();
        for i in 3..6 {
            s.append(&entry(i, 20)).unwrap();
        }
        assert_eq!(read_file(&path, 2).unwrap().len(), 6);
        assert_eq!(s.bytes(), fs::metadata(&path).unwrap().len());
    }

    /// When a failed append cannot be cut off, the spool refuses appends
    /// rather than write records nobody could read, and a trim starts a
    /// fresh file.
    #[test]
    fn a_spool_that_cannot_cut_a_failed_append_takes_no_more() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.log");
        let mut s = Spool::new(&path, 2);
        for i in 0..3 {
            s.append(&entry(i, 20)).unwrap();
        }
        // A handle that can neither write nor truncate.
        s.file = Some(File::open(&path).unwrap());
        assert!(s.append(&entry(3, 20)).is_err());
        s.file = Some(OpenOptions::new().write(true).open(&path).unwrap());
        assert!(s.append(&entry(3, 20)).is_err(), "torn: no more appends");
        assert_eq!(s.read_from(0, u64::MAX).unwrap().len(), 3);
        s.trim_before(1).unwrap();
        s.append(&entry(3, 20)).unwrap();
        let back = read_file(&path, 2).unwrap();
        assert_eq!(back, (1..4).map(|i| entry(i, 20)).collect::<Vec<_>>());
    }

    /// A trim that cannot write its new file leaves the old one whole and
    /// still taking appends.
    #[test]
    fn a_failed_trim_leaves_the_spool_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.log");
        let mut s = Spool::new(&path, 2);
        for i in 0..6 {
            s.append(&entry(i, 30)).unwrap();
        }
        let bytes = s.bytes();
        // Something already sits where the new file would be written.
        fs::create_dir(path.with_extension("trim")).unwrap();
        assert!(s.trim_before(3).is_err());
        assert_eq!(s.first(), Some(0));
        assert_eq!(s.bytes(), bytes);
        s.append(&entry(6, 30)).unwrap();
        let back = read_file(&path, 2).unwrap();
        assert_eq!(back, (0..7).map(|i| entry(i, 30)).collect::<Vec<_>>());
        assert_eq!(s.bytes(), fs::metadata(&path).unwrap().len());
    }
}
