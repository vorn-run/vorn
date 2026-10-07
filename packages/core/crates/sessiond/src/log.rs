//! One session's record log (RC §4 and §8): every record sessiond reads or
//! performs, numbered, held in a memory ring with a disk spool behind it, and
//! the two checkpoints vornd stored.
//!
//! The log is a plain data structure: no threads, no I/O but the spool. The
//! session's reader appends under the session lock, so rseq order is the
//! order sessiond saw things happen (RC §7 rule 1).
//!
//! Watermarks, each a cursor:
//!
//! - `head`: after the last record appended;
//! - `sent`: after the last record written to vornd's socket;
//! - `delivered`: after the last record vornd acked;
//! - `newest_cp`: the newest checkpoint's resume cursor, where recovery starts;
//! - `retain_from`: the older checkpoint's resume cursor, the only one
//!   sessiond trims behind, so the fallback always has every record after it.

use std::collections::VecDeque;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use vorn_term_proto::{Cursor, Entry, GapReason, Record, RecordHeader};

use crate::spool::Spool;
use crate::wire::{AttachFrom, AttachRefusal, Checkpoint, ExitInfo, Manifest};

/// What a session may hold before it spills to disk, and how much disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub ring_bytes: u64,
    pub spool_bytes: u64,
}

impl Budget {
    /// An interactive terminal.
    pub const PTY: Budget = Budget {
        ring_bytes: 4 << 20,
        spool_bytes: 64 << 20,
    };
    /// A piped agent: it prints more, and its transcript is its work.
    pub const PIPED: Budget = Budget {
        ring_bytes: 8 << 20,
        spool_bytes: 64 << 20,
    };
}

/// What happens when the ring and the spool are both full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overflow {
    /// Interactive: give up the fallback checkpoint, then drop new output and
    /// record a Gap for it. The shell stays responsive.
    Drop,
    /// Piped agent: stop reading. The agent blocks on its next write and
    /// nothing is lost.
    Block,
}

/// The spool budget all sessions share (512 MiB by default). Whichever cap is
/// hit first, the session's or this one, applies.
#[derive(Debug, Clone)]
pub struct SpoolPool {
    used: Arc<AtomicU64>,
    cap: u64,
}

impl Default for SpoolPool {
    fn default() -> Self {
        SpoolPool::new(512 << 20)
    }
}

impl SpoolPool {
    pub fn new(cap: u64) -> Self {
        SpoolPool {
            used: Arc::new(AtomicU64::new(0)),
            cap,
        }
    }

    pub fn used(&self) -> u64 {
        self.used.load(Ordering::Relaxed)
    }

    fn take(&self, n: u64) -> bool {
        self.used
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |u| {
                (u + n <= self.cap).then_some(u + n)
            })
            .is_ok()
    }

    fn give(&self, n: u64) {
        self.used.fetch_sub(n, Ordering::Relaxed);
    }
}

#[derive(Debug)]
pub enum AppendError {
    /// Exit is always the last record.
    Exited,
    /// A blocking session is full. The record comes back so the reader can
    /// hold it and stop reading until there is room.
    ///
    /// A spool that cannot be written counts as full: an interactive session
    /// drops the record and a Gap stands for it, a blocking one gets it back
    /// to retry once there is room, so the loss is never silent.
    Full(Record),
}

/// Why sessiond would not store a checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointRefusal {
    BadCrc,
    WrongEpoch,
    /// Its resume cursor is not a record boundary this log holds.
    NotRetained,
    /// Older than the newest one stored.
    Older,
}

/// What a ring entry costs beyond its bytes: header, enum and deque slot.
const ENTRY_OVERHEAD: u64 = 48;

fn ring_cost(rec: &Record) -> u64 {
    match rec {
        Record::Data { bytes, .. } => bytes.len() as u64 + ENTRY_OVERHEAD,
        _ => ENTRY_OVERHEAD,
    }
}

fn now_ns() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

/// The cursor of a state that ends just before this record.
fn before(hdr: &RecordHeader) -> Cursor {
    Cursor {
        epoch: hdr.epoch,
        next_rseq: hdr.rseq,
        next_offset: hdr.start_offset,
    }
}

pub struct SessionLog {
    epoch: u32,
    budget: Budget,
    overflow: Overflow,
    head: Cursor,
    sent: Cursor,
    delivered: Cursor,
    retain_from: Cursor,
    newest: Option<Checkpoint>,
    fallback: Option<Checkpoint>,
    /// The newest records; everything older that is still retained is in
    /// the spool, so the two together are one contiguous run.
    ring: VecDeque<Entry>,
    ring_bytes: u64,
    spool: Spool,
    pool: SpoolPool,
    /// Output read while full that no Gap stands for yet.
    lost: u64,
    exit: Option<ExitInfo>,
    size: (u16, u16),
    /// Added to this process's record clock: a log handed over from another
    /// sessiond carries on from that one's, so `at_ns` never runs back.
    clock: u64,
}

impl SessionLog {
    pub fn new(
        epoch: u32,
        budget: Budget,
        overflow: Overflow,
        spool_path: impl Into<PathBuf>,
        pool: SpoolPool,
        size: (u16, u16),
    ) -> Self {
        let start = Cursor::start(epoch);
        SessionLog {
            epoch,
            budget,
            overflow,
            head: start,
            sent: start,
            delivered: start,
            retain_from: start,
            newest: None,
            fallback: None,
            ring: VecDeque::new(),
            ring_bytes: 0,
            spool: Spool::new(spool_path, epoch),
            pool,
            lost: 0,
            exit: None,
            size,
            clock: 0,
        }
    }

    /// Fill in `m`'s log fields, for handing the session to another
    /// sessiond. Settles any pending Gap first, so the ring is the whole
    /// story.
    pub(crate) fn describe(&mut self, m: &mut Manifest) {
        self.settle_gap();
        m.epoch = self.epoch;
        m.ring_budget = self.budget.ring_bytes;
        m.spool_budget = self.budget.spool_bytes;
        m.blocking = self.overflow == Overflow::Block;
        m.head = self.head;
        m.sent = self.sent;
        m.delivered = self.delivered;
        m.retain_from = self.retain_from;
        (m.cols, m.rows) = self.size;
        m.exit = self.exit;
        m.clock_ns = self.clock_ns();
        m.ring_entries = self.ring.len() as u64;
        m.spool = self.spool.state();
        m.newest = self.newest.is_some();
        m.fallback = self.fallback.is_some();
    }

    /// The records in memory, oldest first.
    pub(crate) fn ring(&self) -> &VecDeque<Entry> {
        &self.ring
    }

    pub(crate) fn checkpoints(&self) -> (Option<&Checkpoint>, Option<&Checkpoint>) {
        (self.newest.as_ref(), self.fallback.as_ref())
    }

    /// The log another sessiond described in `m`, with the ring and
    /// checkpoints it sent, carrying on in the spool file at `spool_path`.
    /// Checked to be one contiguous run ending at `m.head`. Its spool's
    /// bytes count against `pool` from here on, and the file is kept when
    /// the log is dropped until [`SessionLog::keep_spool`] says otherwise.
    pub(crate) fn adopt(
        m: &Manifest,
        ring: VecDeque<Entry>,
        newest: Option<Checkpoint>,
        fallback: Option<Checkpoint>,
        spool_path: impl Into<PathBuf>,
        pool: SpoolPool,
    ) -> io::Result<SessionLog> {
        let bad = |why: &str| io::Error::new(io::ErrorKind::InvalidData, why.to_owned());
        if ring.len() as u64 != m.ring_entries {
            return Err(bad("ring entries missing"));
        }
        if m.head.epoch != m.epoch {
            return Err(bad("head from another epoch"));
        }
        if let Some(first) = ring.front() {
            if m.spool.count > 0 && first.hdr.rseq != m.spool.end {
                return Err(bad("the ring does not follow the spool"));
            }
            let mut at = before(&first.hdr);
            for e in &ring {
                if !at.is_followed_by(&e.hdr) {
                    return Err(bad("the ring is not contiguous"));
                }
                at = e.after();
            }
            if at != m.head {
                return Err(bad("the ring does not end at head"));
            }
        } else if m.spool.count > 0 && m.spool.end != m.head.next_rseq {
            return Err(bad("the spool does not end at head"));
        }
        for cp in newest.iter().chain(&fallback) {
            if !cp.crc_ok() || cp.resume.epoch != m.epoch {
                return Err(bad("a damaged checkpoint"));
            }
        }
        if newest.is_some() != m.newest || fallback.is_some() != m.fallback {
            return Err(bad("checkpoints missing"));
        }
        let spool = Spool::adopt(spool_path, m.epoch, &m.spool)?;
        if !pool.take(spool.bytes()) {
            return Err(bad("no room in the spool budget"));
        }
        let ring_bytes = ring.iter().map(|e| ring_cost(&e.rec)).sum();
        Ok(SessionLog {
            epoch: m.epoch,
            budget: Budget {
                ring_bytes: m.ring_budget,
                spool_bytes: m.spool_budget,
            },
            overflow: if m.blocking {
                Overflow::Block
            } else {
                Overflow::Drop
            },
            head: m.head,
            sent: m.sent,
            delivered: m.delivered,
            retain_from: m.retain_from,
            newest,
            fallback,
            ring,
            ring_bytes,
            spool,
            pool,
            lost: 0,
            exit: m.exit,
            size: (m.cols, m.rows),
            clock: m.clock_ns.saturating_sub(now_ns()),
        })
    }

    /// Whether dropping the log leaves its spool file, as it must while the
    /// session is being handed over or once another sessiond has it.
    pub(crate) fn keep_spool(&mut self, keep: bool) {
        self.spool.keep(keep);
    }

    /// The clock `at_ns` is read from.
    fn clock_ns(&self) -> u64 {
        now_ns() + self.clock
    }

    /// Append a record. Data that does not fit is dropped (and a Gap will
    /// stand for it) or handed back, by the session's overflow policy; the
    /// return says whether it was kept. Resize, Gap and Exit are always kept.
    pub fn append(&mut self, rec: Record) -> Result<bool, AppendError> {
        if self.exit.is_some() {
            return Err(AppendError::Exited);
        }
        if let Record::Data { .. } = rec {
            let extra = if self.lost > 0 { ENTRY_OVERHEAD } else { 0 };
            // A spool write that failed made no room; see `AppendError::Full`.
            if !self.make_room(ring_cost(&rec) + extra).unwrap_or(false) {
                return match self.overflow {
                    Overflow::Drop => {
                        self.lost += rec.len();
                        Ok(false)
                    }
                    Overflow::Block => Err(AppendError::Full(rec)),
                };
            }
        } else {
            // Control records are never refused, not even when the spool
            // cannot be written: they cost almost nothing, and Exit must land.
            let _ = self.make_room(ring_cost(&rec));
        }
        self.settle_gap();
        if let Record::Exit { code, signal } = rec {
            self.exit = Some(ExitInfo { code, signal });
        }
        if let Record::Resize { cols, rows, .. } = rec {
            self.size = (cols, rows);
        }
        self.push(rec);
        Ok(true)
    }

    /// Write the Gap for output dropped while full, if any, so the log says
    /// how many bytes are missing before anything reads past them.
    pub fn settle_gap(&mut self) {
        if self.lost > 0 {
            let lost_bytes = std::mem::take(&mut self.lost);
            self.push(Record::Gap {
                lost_bytes,
                reason: GapReason::SpoolFull,
            });
        }
    }

    fn push(&mut self, rec: Record) {
        let entry = Entry {
            hdr: RecordHeader {
                epoch: self.epoch,
                rseq: self.head.next_rseq,
                start_offset: self.head.next_offset,
            },
            at_ns: self.clock_ns(),
            rec,
        };
        self.head = entry.after();
        self.ring_bytes += ring_cost(&entry.rec);
        self.ring.push_back(entry);
    }

    /// Make room in the ring by dropping what is behind `retain_from` and
    /// spooling the rest. False when the spool is full too.
    fn make_room(&mut self, cost: u64) -> io::Result<bool> {
        while self.ring_bytes + cost > self.budget.ring_bytes {
            let Some(front) = self.ring.front() else {
                // One record larger than the whole ring: keep it anyway.
                break;
            };
            if !self.retain_from.includes(&front.hdr) {
                let n = self.spool.cost_here(front);
                if self.spool.bytes() + n > self.budget.spool_bytes || !self.pool.take(n) {
                    if self.overflow == Overflow::Drop && self.give_up_fallback()? {
                        continue;
                    }
                    return Ok(false);
                }
                let before = self.spool.bytes();
                if let Err(e) = self.spool.append(front) {
                    self.pool.give(n);
                    return Err(e);
                }
                debug_assert_eq!(self.spool.bytes() - before, n);
            }
            let gone = self.ring.pop_front().expect("front exists");
            self.ring_bytes -= ring_cost(&gone.rec);
        }
        Ok(true)
    }

    /// RC §8: the first thing an interactive session gives up is the fallback
    /// checkpoint and the records only it needs. No Gap is needed for that,
    /// because the newest checkpoint still has every record after it.
    fn give_up_fallback(&mut self) -> io::Result<bool> {
        let Some(newest) = &self.newest else {
            return Ok(false);
        };
        if newest.resume.next_rseq <= self.retain_from.next_rseq {
            return Ok(false);
        }
        self.retain_from = newest.resume;
        self.fallback = None;
        self.trim()?;
        Ok(true)
    }

    /// Drop everything before `retain_from`.
    fn trim(&mut self) -> io::Result<()> {
        while self
            .ring
            .front()
            .is_some_and(|e| self.retain_from.includes(&e.hdr))
        {
            let gone = self.ring.pop_front().expect("front exists");
            self.ring_bytes -= ring_cost(&gone.rec);
        }
        let before = self.spool.bytes();
        let res = self.spool.trim_before(self.retain_from.next_rseq);
        // A rewrite never grows the file, and a failed one leaves it as it was.
        self.pool.give(before.saturating_sub(self.spool.bytes()));
        res
    }

    /// Store a checkpoint vornd cut. It becomes the newest; the one it
    /// replaces becomes the fallback and sets `retain_from`, and only then is
    /// anything behind it trimmed.
    pub fn put_checkpoint(&mut self, cp: Checkpoint) -> Result<(), CheckpointRefusal> {
        if !cp.crc_ok() {
            return Err(CheckpointRefusal::BadCrc);
        }
        if cp.resume.epoch != self.epoch {
            return Err(CheckpointRefusal::WrongEpoch);
        }
        if let Some(n) = &self.newest {
            if cp.resume.next_rseq < n.resume.next_rseq {
                return Err(CheckpointRefusal::Older);
            }
        }
        if !self.holds_boundary(cp.resume) {
            return Err(CheckpointRefusal::NotRetained);
        }
        if let Some(prev) = self.newest.replace(cp) {
            self.retain_from = prev.resume;
            self.fallback = Some(prev);
        }
        // A failed trim leaves the spool whole, holding records from before
        // `retain_from` that the next trim drops: the run stays contiguous
        // and nothing vornd may ask for is gone.
        let _ = self.trim();
        Ok(())
    }

    /// Whether `c` names a record boundary inside `[oldest, head]`.
    fn holds_boundary(&mut self, c: Cursor) -> bool {
        if c.epoch != self.epoch
            || c.next_rseq < self.oldest().next_rseq
            || c.next_rseq > self.head.next_rseq
        {
            return false;
        }
        if c.next_rseq == self.head.next_rseq {
            return c == self.head;
        }
        let hdr = match self.ring.iter().find(|e| e.hdr.rseq == c.next_rseq) {
            Some(e) => Some(e.hdr),
            None => self
                .spool
                .read_from(c.next_rseq, 0)
                .ok()
                .and_then(|v| v.first().map(|e| e.hdr)),
        };
        hdr.is_some_and(|h| before(&h) == c)
    }

    /// Read a session for an attaching vornd: the checkpoint it asked for, if
    /// any, and every record after it up to head.
    pub fn attach(
        &mut self,
        from: AttachFrom,
    ) -> Result<(Option<Checkpoint>, Vec<Entry>), AttachRefusal> {
        self.settle_gap();
        let (cp, cursor) = match from {
            AttachFrom::NewestCheckpoint => {
                let cp = self.newest.clone().ok_or(AttachRefusal::NoSuchCheckpoint)?;
                let at = cp.resume;
                (Some(cp), at)
            }
            AttachFrom::FallbackCheckpoint => {
                let cp = self
                    .fallback
                    .clone()
                    .ok_or(AttachRefusal::NoSuchCheckpoint)?;
                let at = cp.resume;
                (Some(cp), at)
            }
            AttachFrom::SessionStart => (None, Cursor::start(self.epoch)),
            AttachFrom::Cursor(c) => {
                if c.epoch != self.epoch {
                    return Err(AttachRefusal::WrongEpoch);
                }
                (None, c)
            }
        };
        let entries = self.entries_from(cursor)?;
        Ok((cp, entries))
    }

    /// Every record from `c` to head, checked to be one contiguous run that
    /// starts exactly at `c`.
    pub fn entries_from(&mut self, c: Cursor) -> Result<Vec<Entry>, AttachRefusal> {
        if c.epoch != self.epoch {
            return Err(AttachRefusal::WrongEpoch);
        }
        if c.next_rseq < self.oldest().next_rseq || c.next_rseq > self.head.next_rseq {
            return Err(AttachRefusal::NotRetained);
        }
        let mut out = Vec::new();
        let ring_first = self.ring.front().map(|e| e.hdr.rseq);
        if ring_first.is_none_or(|f| c.next_rseq < f) {
            out = self
                .spool
                .read_from(c.next_rseq, u64::MAX)
                .map_err(|_| AttachRefusal::NotRetained)?;
        }
        out.extend(
            self.ring
                .iter()
                .filter(|e| e.hdr.rseq >= c.next_rseq)
                .cloned(),
        );
        // One contiguous run from c to head, or nothing usable: a damaged
        // spool or a cursor that is not a record boundary.
        let mut at = c;
        for e in &out {
            if !at.is_followed_by(&e.hdr) {
                return Err(AttachRefusal::NotRetained);
            }
            at = e.after();
        }
        if at != self.head {
            return Err(AttachRefusal::NotRetained);
        }
        Ok(out)
    }

    /// The records after `from`, at most about `max_bytes` of output but
    /// always at least one: what a live connection sends next. Reads the
    /// ring directly; a reader that fell behind into the spool reads only
    /// this batch from it, so a frame never grows with the spool.
    pub fn read_batch(
        &mut self,
        from: Cursor,
        max_bytes: u64,
    ) -> Result<Vec<Entry>, AttachRefusal> {
        self.settle_gap();
        if from == self.head {
            return Ok(Vec::new());
        }
        if from.epoch != self.epoch {
            return Err(AttachRefusal::WrongEpoch);
        }
        if from.next_rseq < self.oldest().next_rseq || from.next_rseq > self.head.next_rseq {
            return Err(AttachRefusal::NotRetained);
        }
        let ring_first = self
            .ring
            .front()
            .map_or(self.head.next_rseq, |e| e.hdr.rseq);
        let mut out = Vec::new();
        if from.next_rseq < ring_first {
            out = self
                .spool
                .read_from(from.next_rseq, max_bytes)
                .map_err(|_| AttachRefusal::NotRetained)?;
        }
        let mut taken: u64 = out.iter().map(|e| e.rec.len()).sum();
        // The ring carries on only where the spool's part ran up to it: one
        // that stopped short hit the budget, or a damaged frame.
        let reached_ring = out.last().map_or(from.next_rseq >= ring_first, |e| {
            e.hdr.rseq + 1 == ring_first
        });
        if reached_ring {
            let skip = from.next_rseq.saturating_sub(ring_first) as usize;
            for e in self.ring.iter().skip(skip) {
                let n = e.rec.len();
                if !out.is_empty() && taken + n > max_bytes {
                    break;
                }
                taken += n;
                out.push(e.clone());
            }
        }
        // One contiguous run starting exactly at `from`, or nothing usable.
        let mut at = from;
        for e in &out {
            if !at.is_followed_by(&e.hdr) {
                return Err(AttachRefusal::NotRetained);
            }
            at = e.after();
        }
        if out.is_empty() {
            return Err(AttachRefusal::NotRetained);
        }
        Ok(out)
    }

    /// Records up to `c` were written to vornd's socket.
    pub fn mark_sent(&mut self, c: Cursor) {
        if c.epoch == self.epoch && c.next_rseq > self.sent.next_rseq {
            self.sent = c;
        }
    }

    /// vornd fed records up to `c` into its terminal.
    pub fn ack(&mut self, c: Cursor) {
        if c.epoch == self.epoch
            && c.next_rseq > self.delivered.next_rseq
            && c.next_rseq <= self.head.next_rseq
        {
            self.delivered = c;
        }
    }

    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    pub fn head(&self) -> Cursor {
        self.head
    }

    pub fn sent(&self) -> Cursor {
        self.sent
    }

    pub fn delivered(&self) -> Cursor {
        self.delivered
    }

    pub fn retain_from(&self) -> Cursor {
        self.retain_from
    }

    pub fn newest_cp(&self) -> Option<Cursor> {
        self.newest.as_ref().map(|c| c.resume)
    }

    /// The first record still held. The spool holds the older part of the
    /// run, so it comes first.
    pub fn oldest(&self) -> Cursor {
        if let (Some(rseq), Some(offset)) = (self.spool.first(), self.spool.first_offset()) {
            return Cursor {
                epoch: self.epoch,
                next_rseq: rseq,
                next_offset: offset,
            };
        }
        self.ring.front().map_or(self.head, |e| before(&e.hdr))
    }

    pub fn exited(&self) -> Option<ExitInfo> {
        self.exit
    }

    pub fn size(&self) -> (u16, u16) {
        self.size
    }

    pub fn spooled_bytes(&self) -> u64 {
        self.spool.bytes()
    }

    pub fn ring_bytes(&self) -> u64 {
        self.ring_bytes
    }

    /// Bytes dropped while full that no Gap stands for yet.
    pub fn lost(&self) -> u64 {
        self.lost
    }
}

impl Drop for SessionLog {
    /// A released session's bytes go back to the pool; its spool file goes
    /// with the spool unless a handoff keeps it.
    fn drop(&mut self) {
        self.pool.give(self.spool.bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vorn_term_proto::Stream;

    fn data(n: usize, fill: u8) -> Record {
        Record::Data {
            stream: Stream::Pty,
            bytes: vec![fill; n],
        }
    }

    fn resize(cols: u16) -> Record {
        Record::Resize {
            cols,
            rows: 24,
            px_w: 0,
            px_h: 0,
            req: None,
        }
    }

    fn log(
        dir: &tempfile::TempDir,
        budget: Budget,
        overflow: Overflow,
        pool: SpoolPool,
    ) -> SessionLog {
        SessionLog::new(
            1,
            budget,
            overflow,
            dir.path().join("s.log"),
            pool,
            (80, 24),
        )
    }

    fn big() -> Budget {
        Budget {
            ring_bytes: 1 << 30,
            spool_bytes: 1 << 30,
        }
    }

    fn checkpoint(at: Cursor) -> Checkpoint {
        let blob = format!("screen at {}", at.next_rseq).into_bytes();
        Checkpoint {
            session: "s".into(),
            resume: at,
            cols: 80,
            rows: 24,
            format: 1,
            vornd_build: "test".into(),
            blob_crc32: crc32fast::hash(&blob),
            blob,
        }
    }

    /// The cursor after record `rseq` when every record before it is `n` bytes.
    fn after(rseq: u64, n: u64) -> Cursor {
        Cursor {
            epoch: 1,
            next_rseq: rseq + 1,
            next_offset: (rseq + 1) * n,
        }
    }

    fn bytes_of(entries: &[Entry]) -> Vec<u8> {
        let mut out = Vec::new();
        for e in entries {
            if let Record::Data { bytes, .. } = &e.rec {
                out.extend(bytes);
            }
        }
        out
    }

    #[test]
    fn records_are_numbered_in_the_order_they_arrive() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = log(&dir, big(), Overflow::Drop, SpoolPool::default());
        l.append(data(20, b'a')).unwrap();
        l.append(resize(100)).unwrap();
        l.append(resize(120)).unwrap();
        l.append(data(5, b'b')).unwrap();
        let (_, all) = l.attach(AttachFrom::SessionStart).unwrap();
        let hdrs: Vec<_> = all
            .iter()
            .map(|e| (e.hdr.rseq, e.hdr.start_offset))
            .collect();
        assert_eq!(hdrs, vec![(0, 0), (1, 20), (2, 20), (3, 20)]);
        assert_eq!(
            l.head(),
            Cursor {
                epoch: 1,
                next_rseq: 4,
                next_offset: 25
            }
        );
        assert_eq!(l.size(), (120, 24));
    }

    /// RC-T17 on the log: a cursor cut after rseq 7 at offset 100 with 20
    /// bytes reads from rseq 8 and byte 120, so byte 100 comes back once.
    #[test]
    fn a_cursor_resumes_after_the_record_it_includes() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = log(&dir, big(), Overflow::Drop, SpoolPool::default());
        for i in 0..7 {
            l.append(data(if i == 6 { 4 } else { 16 }, b'a' + i))
                .unwrap();
        }
        assert_eq!(l.head().next_offset, 100);
        l.append(data(20, b'x')).unwrap(); // rseq 7 at 100
        l.append(resize(90)).unwrap();
        l.append(data(3, b'y')).unwrap();
        let cut = Cursor {
            epoch: 1,
            next_rseq: 8,
            next_offset: 120,
        };
        let (_, rest) = l.attach(AttachFrom::Cursor(cut)).unwrap();
        assert_eq!(rest[0].hdr.rseq, 8);
        assert_eq!(bytes_of(&rest), b"yyy");
        let (_, all) = l.attach(AttachFrom::SessionStart).unwrap();
        let mut stream = bytes_of(&all[..8]);
        stream.extend(bytes_of(&rest));
        assert_eq!(stream, bytes_of(&all));
        // An offset that is not the record's boundary is not a resume token.
        let wrong = Cursor {
            next_offset: 100,
            ..cut
        };
        assert_eq!(
            l.attach(AttachFrom::Cursor(wrong)),
            Err(AttachRefusal::NotRetained)
        );
        let other = Cursor { epoch: 2, ..cut };
        assert_eq!(
            l.attach(AttachFrom::Cursor(other)),
            Err(AttachRefusal::WrongEpoch)
        );
        let ahead = Cursor {
            next_rseq: 11,
            next_offset: 123,
            ..cut
        };
        assert_eq!(
            l.attach(AttachFrom::Cursor(ahead)),
            Err(AttachRefusal::NotRetained)
        );
        // At head: nothing missing, nothing to send.
        assert_eq!(l.attach(AttachFrom::Cursor(l.head())).unwrap().1, vec![]);
    }

    #[test]
    fn exit_is_the_last_record() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = log(&dir, big(), Overflow::Drop, SpoolPool::default());
        l.append(data(4, b'a')).unwrap();
        l.append(Record::Exit {
            code: Some(7),
            signal: None,
        })
        .unwrap();
        assert!(matches!(l.append(data(1, b'b')), Err(AppendError::Exited)));
        assert!(matches!(l.append(resize(10)), Err(AppendError::Exited)));
        assert_eq!(l.exited().and_then(|e| e.code), Some(7));
        let (_, all) = l.attach(AttachFrom::SessionStart).unwrap();
        assert!(matches!(all.last().unwrap().rec, Record::Exit { .. }));
    }

    /// RC-T19: with checkpoints at rseq 100 and 200, recovery from the
    /// fallback replays 100 onward exactly; records below 200 are trimmed
    /// only once a checkpoint at 300 is stored.
    #[test]
    fn fallback_retention() {
        let dir = tempfile::tempdir().unwrap();
        // A ring that holds about 50 records, so most of this is on disk.
        let budget = Budget {
            ring_bytes: 50 * (10 + ENTRY_OVERHEAD),
            spool_bytes: 1 << 20,
        };
        let pool = SpoolPool::default();
        let mut l = log(&dir, budget, Overflow::Drop, pool.clone());
        for i in 0..250 {
            l.append(data(10, i as u8)).unwrap();
        }
        assert!(l.spooled_bytes() > 0);
        assert_eq!(pool.used(), l.spooled_bytes());
        let (_, all) = l.attach(AttachFrom::SessionStart).unwrap();

        l.put_checkpoint(checkpoint(after(99, 10))).unwrap();
        assert_eq!(l.retain_from().next_rseq, 0);
        l.put_checkpoint(checkpoint(after(199, 10))).unwrap();
        assert_eq!(l.newest_cp().unwrap().next_rseq, 200);
        assert_eq!(l.retain_from().next_rseq, 100);
        assert_eq!(l.oldest().next_rseq, 100);

        // The newer one failed its restore check: vornd asks for the fallback.
        let (cp, rest) = l.attach(AttachFrom::FallbackCheckpoint).unwrap();
        assert_eq!(cp.unwrap().resume.next_rseq, 100);
        assert_eq!(rest.len(), 150);
        assert_eq!(bytes_of(&rest), bytes_of(&all[100..]));
        assert_eq!(
            l.attach(AttachFrom::SessionStart),
            Err(AttachRefusal::NotRetained)
        );

        for i in 250..320 {
            l.append(data(10, i as u8)).unwrap();
        }
        assert_eq!(l.oldest().next_rseq, 100);
        l.put_checkpoint(checkpoint(after(299, 10))).unwrap();
        assert_eq!(l.retain_from().next_rseq, 200);
        assert_eq!(l.oldest().next_rseq, 200);
        let (cp, rest) = l.attach(AttachFrom::FallbackCheckpoint).unwrap();
        assert_eq!(cp.unwrap().resume.next_rseq, 200);
        assert_eq!(rest.first().unwrap().hdr.rseq, 200);
        assert_eq!(pool.used(), l.spooled_bytes());
    }

    #[test]
    fn a_checkpoint_is_stored_only_at_a_boundary_the_log_holds() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = log(&dir, big(), Overflow::Drop, SpoolPool::default());
        for _ in 0..10 {
            l.append(data(10, b'a')).unwrap();
        }
        let mut bad = checkpoint(after(4, 10));
        bad.blob[0] ^= 1;
        assert_eq!(l.put_checkpoint(bad), Err(CheckpointRefusal::BadCrc));
        let mut mid = checkpoint(after(4, 10));
        mid.resume.next_offset += 3;
        mid.blob_crc32 = crc32fast::hash(&mid.blob);
        assert_eq!(l.put_checkpoint(mid), Err(CheckpointRefusal::NotRetained));
        let past = checkpoint(after(12, 10));
        assert_eq!(l.put_checkpoint(past), Err(CheckpointRefusal::NotRetained));
        let other = Checkpoint {
            resume: Cursor {
                epoch: 9,
                ..after(4, 10)
            },
            ..checkpoint(after(4, 10))
        };
        assert_eq!(l.put_checkpoint(other), Err(CheckpointRefusal::WrongEpoch));
        l.put_checkpoint(checkpoint(after(6, 10))).unwrap();
        assert_eq!(
            l.put_checkpoint(checkpoint(after(4, 10))),
            Err(CheckpointRefusal::Older)
        );
        l.put_checkpoint(checkpoint(l.head())).unwrap();
        assert_eq!(l.attach(AttachFrom::NewestCheckpoint).unwrap().1, vec![]);
    }

    /// RC-T8 at unit level: while vornd is away, what fits in the spool comes
    /// back whole; past it an interactive session records one Gap with the
    /// exact byte count, and an agent session refuses the record so nothing
    /// is lost.
    #[test]
    fn spool_overflow() {
        let budget = Budget {
            ring_bytes: 4 << 10,
            spool_bytes: 40 << 10,
        };
        let chunk = 1000;

        let dir = tempfile::tempdir().unwrap();
        let mut l = log(&dir, budget, Overflow::Drop, SpoolPool::default());
        for i in 0..30 {
            assert!(l.append(data(chunk, i)).unwrap());
        }
        let (_, all) = l.attach(AttachFrom::SessionStart).unwrap();
        assert_eq!(all.len(), 30);
        assert!(all.iter().all(|e| !matches!(e.rec, Record::Gap { .. })));

        let dir = tempfile::tempdir().unwrap();
        let mut l = log(&dir, budget, Overflow::Drop, SpoolPool::default());
        let mut kept = 0u64;
        let mut lost = 0u64;
        for i in 0..100 {
            if l.append(data(chunk, i)).unwrap() {
                kept += chunk as u64;
            } else {
                lost += chunk as u64;
            }
        }
        assert!(lost > 0);
        let (_, all) = l.attach(AttachFrom::SessionStart).unwrap();
        let gaps: Vec<_> = all
            .iter()
            .filter_map(|e| match e.rec {
                Record::Gap { lost_bytes, .. } => Some(lost_bytes),
                _ => None,
            })
            .collect();
        assert_eq!(gaps, vec![lost]);
        assert_eq!(bytes_of(&all).len() as u64, kept);
        assert_eq!(l.head().next_offset, 100 * chunk as u64);

        let dir = tempfile::tempdir().unwrap();
        let mut l = log(&dir, budget, Overflow::Block, SpoolPool::default());
        let mut refused = None;
        for i in 0..100u8 {
            match l.append(data(chunk, i)) {
                Ok(true) => {}
                Err(AppendError::Full(rec)) => {
                    refused = Some((i, rec));
                    break;
                }
                other => panic!("{other:?}"),
            }
        }
        let (i, rec) = refused.expect("an agent blocks before 100 KB");
        assert_eq!(rec, data(chunk, i));
        let (_, all) = l.attach(AttachFrom::SessionStart).unwrap();
        assert_eq!(all.len(), i as usize);
        assert_eq!(l.lost(), 0);
    }

    /// RC-T20: an interactive session gives up the fallback before it drops a
    /// byte, and only further overflow writes a Gap, with the exact count.
    #[test]
    fn fallback_before_gap() {
        let budget = Budget {
            ring_bytes: 4 << 10,
            spool_bytes: 40 << 10,
        };
        let chunk = 1000u64;
        let dir = tempfile::tempdir().unwrap();
        let mut l = log(&dir, budget, Overflow::Drop, SpoolPool::default());
        for i in 0..20 {
            l.append(data(chunk as usize, i)).unwrap();
        }
        l.put_checkpoint(checkpoint(after(4, chunk))).unwrap();
        l.put_checkpoint(checkpoint(after(14, chunk))).unwrap();
        assert_eq!(l.retain_from().next_rseq, 5);

        let mut n = 20u8;
        while l.retain_from().next_rseq == 5 {
            assert!(
                l.append(data(chunk as usize, n)).unwrap(),
                "no byte dropped first"
            );
            n += 1;
        }
        assert_eq!(l.retain_from().next_rseq, 15);
        assert_eq!(
            l.attach(AttachFrom::FallbackCheckpoint),
            Err(AttachRefusal::NoSuchCheckpoint)
        );
        assert_eq!(l.lost(), 0);

        let mut lost = 0;
        for _ in 0..60 {
            if !l.append(data(chunk as usize, n)).unwrap() {
                lost += chunk;
            }
            n = n.wrapping_add(1);
        }
        assert!(lost > 0);
        let (cp, rest) = l.attach(AttachFrom::NewestCheckpoint).unwrap();
        assert_eq!(cp.unwrap().resume.next_rseq, 15);
        let gaps: Vec<_> = rest
            .iter()
            .filter_map(|e| match e.rec {
                Record::Gap { lost_bytes, .. } => Some(lost_bytes),
                _ => None,
            })
            .collect();
        assert_eq!(gaps, vec![lost]);
    }

    #[test]
    fn a_resize_while_full_comes_after_the_gap_for_what_was_dropped() {
        let budget = Budget {
            ring_bytes: 2 << 10,
            spool_bytes: 4 << 10,
        };
        let dir = tempfile::tempdir().unwrap();
        let mut l = log(&dir, budget, Overflow::Drop, SpoolPool::default());
        while l.append(data(500, b'a')).unwrap() {}
        let lost = l.lost();
        assert_eq!(lost, 500);
        l.append(resize(50)).unwrap();
        let (_, all) = l.attach(AttachFrom::SessionStart).unwrap();
        let n = all.len();
        assert!(matches!(
            all[n - 2].rec,
            Record::Gap {
                lost_bytes: 500,
                ..
            }
        ));
        assert!(matches!(all[n - 1].rec, Record::Resize { cols: 50, .. }));
    }

    #[test]
    fn the_shared_spool_cap_applies_across_sessions() {
        let budget = Budget {
            ring_bytes: 2 << 10,
            spool_bytes: 1 << 20,
        };
        let pool = SpoolPool::new(10 << 10);
        let dir = tempfile::tempdir().unwrap();
        let mut a = SessionLog::new(
            1,
            budget,
            Overflow::Block,
            dir.path().join("a"),
            pool.clone(),
            (80, 24),
        );
        let mut b = SessionLog::new(
            1,
            budget,
            Overflow::Block,
            dir.path().join("b"),
            pool.clone(),
            (80, 24),
        );
        let mut full = 0;
        for i in 0..40u8 {
            for l in [&mut a, &mut b] {
                if matches!(l.append(data(500, i)), Err(AppendError::Full(_))) {
                    full += 1;
                }
            }
        }
        assert!(full > 0);
        assert!(pool.used() <= 10 << 10);
        assert_eq!(pool.used(), a.spooled_bytes() + b.spooled_bytes());
        let b_bytes = b.spooled_bytes();
        drop(a);
        // A released session's spool goes back to the pool.
        assert!(!dir.path().join("a").exists());
        assert_eq!(pool.used(), b_bytes);
    }

    /// A spool that cannot be written is full, never a silent loss: an
    /// interactive session records a Gap for what it dropped, a blocking one
    /// hands the record back, and Exit still lands.
    #[test]
    fn a_spool_write_failure_is_a_gap_or_backpressure() {
        let budget = Budget {
            ring_bytes: 4 * (100 + ENTRY_OVERHEAD),
            spool_bytes: 1 << 20,
        };
        let dir = tempfile::tempdir().unwrap();
        let unwritable = dir.path().join("missing").join("s.log");

        let mut l = SessionLog::new(
            1,
            budget,
            Overflow::Drop,
            &unwritable,
            SpoolPool::default(),
            (80, 24),
        );
        let mut kept = 0;
        for i in 0..10 {
            if l.append(data(100, i)).unwrap() {
                kept += 1;
            }
        }
        assert_eq!(kept, 4);
        assert_eq!(l.lost(), 600);
        l.append(Record::Exit {
            code: Some(0),
            signal: None,
        })
        .unwrap();
        assert_eq!(l.head().next_offset, 1000);
        let (_, all) = l.attach(AttachFrom::Cursor(after(3, 100))).unwrap();
        assert!(matches!(
            all[0].rec,
            Record::Gap {
                lost_bytes: 600,
                ..
            }
        ));
        assert!(matches!(all[1].rec, Record::Exit { .. }));
        assert_eq!(l.spooled_bytes(), 0);

        let mut l = SessionLog::new(
            1,
            budget,
            Overflow::Block,
            &unwritable,
            SpoolPool::default(),
            (80, 24),
        );
        for i in 0..4 {
            assert!(l.append(data(100, i)).unwrap());
        }
        match l.append(data(100, 4)) {
            Err(AppendError::Full(rec)) => assert_eq!(rec, data(100, 4)),
            other => panic!("{other:?}"),
        }
        assert_eq!(l.lost(), 0);
    }

    #[test]
    fn a_batch_reads_from_the_ring_and_falls_back_to_the_spool() {
        let budget = Budget {
            ring_bytes: 10 * (100 + ENTRY_OVERHEAD),
            spool_bytes: 1 << 20,
        };
        let dir = tempfile::tempdir().unwrap();
        let mut l = log(&dir, budget, Overflow::Drop, SpoolPool::default());
        for i in 0..30 {
            l.append(data(100, i)).unwrap();
        }
        let b = l.read_batch(after(24, 100), 250).unwrap();
        assert_eq!(
            b.iter().map(|e| e.hdr.rseq).collect::<Vec<_>>(),
            vec![25, 26]
        );
        // A single record larger than the batch still comes.
        assert_eq!(l.read_batch(after(24, 100), 1).unwrap().len(), 1);
        // Behind the ring: a batch from the spool, held to the same budget,
        // however much the spool holds.
        let b = l.read_batch(after(2, 100), 250).unwrap();
        assert_eq!(b.iter().map(|e| e.hdr.rseq).collect::<Vec<_>>(), vec![3, 4]);
        assert_eq!(l.read_batch(after(2, 100), 0).unwrap().len(), 1);
        // A batch runs on from the spool's last record into the ring.
        let b = l.read_batch(after(17, 100), 350).unwrap();
        assert_eq!(
            b.iter().map(|e| e.hdr.rseq).collect::<Vec<_>>(),
            vec![18, 19, 20]
        );
        // Read batch by batch from the start, the whole log comes back once.
        let mut at = Cursor::start(1);
        let mut seen = Vec::new();
        while at != l.head() {
            let b = l.read_batch(at, 450).unwrap();
            assert!(b.iter().map(|e| e.rec.len()).sum::<u64>() <= 450);
            at = b.last().unwrap().after();
            seen.extend(b);
        }
        assert_eq!(seen, l.attach(AttachFrom::SessionStart).unwrap().1);
        assert_eq!(l.read_batch(l.head(), 250).unwrap(), vec![]);
        let wrong = Cursor {
            next_offset: 1,
            ..after(24, 100)
        };
        assert_eq!(l.read_batch(wrong, 250), Err(AttachRefusal::NotRetained));
    }

    #[test]
    fn watermarks_only_move_forward() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = log(&dir, big(), Overflow::Drop, SpoolPool::default());
        for _ in 0..5 {
            l.append(data(10, b'a')).unwrap();
        }
        l.mark_sent(after(3, 10));
        l.mark_sent(after(1, 10));
        assert_eq!(l.sent(), after(3, 10));
        l.ack(after(2, 10));
        l.ack(after(9, 10));
        l.ack(after(0, 10));
        assert_eq!(l.delivered(), after(2, 10));
    }

    #[test]
    fn a_log_handed_over_keeps_its_spool_file() {
        let budget = Budget {
            ring_bytes: 4 << 10,
            spool_bytes: 40 << 10,
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.log");
        let mut l = log(&dir, budget, Overflow::Drop, SpoolPool::default());
        for i in 0..10 {
            l.append(data(1000, i)).unwrap();
        }
        assert!(l.spooled_bytes() > 0);
        l.keep_spool(true);
        drop(l);
        assert!(path.exists());

        let mut l = log(&dir, budget, Overflow::Drop, SpoolPool::default());
        for i in 0..10 {
            l.append(data(1000, i)).unwrap();
        }
        drop(l);
        assert!(!path.exists());
    }
}
