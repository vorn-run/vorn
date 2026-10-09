//! What the engine's tests share: sessiond's own record log standing in for
//! sessiond, and the engine under the recovery harness.

#![allow(dead_code)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use vorn_engine::{Cadence, Config, Input, Open, Out, Session};
use vorn_recovery::{Checkpoint as HarnessCheckpoint, Error, Size, TermState};
use vorn_sessiond::{AttachFrom, Budget, Overflow, SessionLog, SpoolPool};
use vorn_sessiond_wire::{Checkpoint, Kind, SessionInfo};
use vorn_term_proto::{Cursor, Entry};

/// A config that cuts a checkpoint every `bytes` of output.
pub fn config(bytes: u64) -> Arc<Config> {
    Arc::new(Config {
        cadence: Cadence {
            bytes,
            ..Cadence::default()
        },
        build: "test".into(),
        ..Config::default()
    })
}

/// One session's log in sessiond, with nothing else of sessiond around it.
pub struct Sessiond {
    pub log: SessionLog,
    pub kind: Kind,
    /// The checkpoint resuming at this rseq comes out of storage with a
    /// byte changed.
    pub damage: Option<u64>,
    _dir: tempfile::TempDir,
}

impl Sessiond {
    pub fn new(epoch: u32, size: (u16, u16)) -> Sessiond {
        let dir = tempfile::tempdir().unwrap();
        let budget = Budget {
            ring_bytes: 1 << 30,
            spool_bytes: 1 << 30,
        };
        Sessiond {
            log: SessionLog::new(
                epoch,
                budget,
                Overflow::Drop,
                dir.path().join("s.log"),
                SpoolPool::default(),
                size,
            ),
            kind: Kind::Pty,
            damage: None,
            _dir: dir,
        }
    }

    /// Appends records as a session's reader does; their headers come out
    /// as the log numbers them.
    pub fn append<'a>(&mut self, entries: impl IntoIterator<Item = &'a Entry>) {
        for e in entries {
            self.log.append(e.rec.clone()).unwrap();
        }
    }

    pub fn welcome(&mut self) -> SessionInfo {
        self.log.settle_gap();
        let (cols, rows) = self.log.size();
        SessionInfo {
            session: "s".into(),
            kind: self.kind,
            pid: 1,
            epoch: self.log.epoch(),
            oldest: self.log.oldest(),
            head: self.log.head(),
            newest_cp: self.log.newest_cp(),
            retain_from: self.log.retain_from(),
            sent: self.log.sent(),
            cols,
            rows,
            exited: self.log.exited(),
            spooled_bytes: 0,
        }
    }

    /// Answers an attach as the server does, marking everything in it sent.
    pub fn attach(&mut self, from: AttachFrom) -> Vec<Input> {
        match self.log.attach(from) {
            Err(why) => vec![Input::Refused(why)],
            Ok((cp, entries)) => {
                let mut out = Vec::new();
                if let Some(mut cp) = cp {
                    if Some(cp.resume.next_rseq) == self.damage {
                        let mid = cp.blob.len() / 2;
                        cp.blob[mid] ^= 0x20;
                    }
                    out.push(Input::Checkpoint(cp));
                }
                if let Some(last) = entries.last() {
                    self.log.mark_sent(last.after());
                }
                if !entries.is_empty() {
                    out.push(Input::Entries(entries));
                }
                out
            }
        }
    }

    /// Carries out what a session asked, feeding sessiond's answers back
    /// to it, until it asks nothing more of sessiond. Every output is kept
    /// in `seen`; with `keep` set, entries beyond it are held back, as a
    /// vornd that dies before reading them would.
    pub fn serve(
        &mut self,
        s: &mut Session,
        mut pending: Vec<Out>,
        seen: &mut Vec<Out>,
        keep: Option<u64>,
    ) {
        while !pending.is_empty() {
            let mut next = Vec::new();
            for o in pending.drain(..) {
                match &o {
                    Out::Attach(from) => {
                        for input in self.attach(*from) {
                            let input = match (input, keep) {
                                (Input::Entries(mut es), Some(upto)) => {
                                    es.retain(|e| e.hdr.rseq <= upto);
                                    Input::Entries(es)
                                }
                                (i, _) => i,
                            };
                            s.input(input, Instant::now(), &mut next);
                        }
                    }
                    Out::Checkpoint(cp) => {
                        let _ = self.log.put_checkpoint(cp.clone());
                    }
                    Out::Ack(c) => self.log.ack(*c),
                    _ => {}
                }
                seen.push(o);
            }
            pending = next;
        }
    }

    /// A vornd taking the session on, run until it asks nothing more.
    pub fn recover(&mut self, cfg: Arc<Config>, open: Open) -> (Session, Vec<Out>) {
        let mut first = Vec::new();
        let mut s = Session::open("s", cfg, open, Instant::now(), &mut first);
        let mut seen = Vec::new();
        self.serve(&mut s, first, &mut seen, None);
        (s, seen)
    }

    /// A vornd's session as sessiond's Welcome describes it.
    pub fn open(&mut self) -> Open {
        Open::from_info(&self.welcome())
    }

    pub fn records(&mut self) -> Vec<Entry> {
        self.log.attach(AttachFrom::SessionStart).unwrap().1
    }
}

/// The terminal a session ended with, for the comparator.
pub fn state(s: Session) -> TermState {
    TermState::capture(s.into_emulator().expect("a terminal")).unwrap()
}

/// What a terminal fed every record of `entries` from a blank `size`
/// looks like.
pub fn reference(cfg: Arc<Config>, size: (u16, u16), entries: &[Entry]) -> TermState {
    let epoch = entries.first().map_or(0, |e| e.hdr.epoch);
    let mut s = Session::fresh("s", cfg, size, Cursor::start(epoch)).unwrap();
    s.apply_all(entries, Instant::now(), &mut Vec::new());
    state(s)
}

/// The writes a session made before it was live.
pub fn writes_in_replay(outs: &[Out]) -> usize {
    outs.iter()
        .take_while(|o| !matches!(o, Out::Ready(_)))
        .filter(|o| matches!(o, Out::Write(_)))
        .count()
}

/// The session engine under the recovery harness: one session, its
/// checkpoints handed to the harness's store as sessiond would keep them.
/// With [`Config::idle`] at zero it is put to sleep after every record, so
/// every record after the first wakes it.
pub struct Harnessed(Session, bool);

fn harnessed(cfg: &Config, s: Session) -> Harnessed {
    Harnessed(s, cfg.idle == Some(Duration::ZERO))
}

/// The harness's blob is the whole checkpoint as sessiond stores it,
/// format and CRC included, so a recovery checks them as vornd does.
fn to_harness(cp: Checkpoint) -> HarnessCheckpoint {
    HarnessCheckpoint {
        resume: cp.resume,
        size: Size::new(cp.cols, cp.rows),
        blob: postcard::to_stdvec(&cp).unwrap(),
    }
}

impl vorn_recovery::Engine for Harnessed {
    type Config = Arc<Config>;

    fn start(cfg: &Arc<Config>, size: Size) -> Result<Self, Error> {
        Session::fresh(
            "s",
            Arc::clone(cfg),
            (size.cols, size.rows),
            Cursor::start(0),
        )
        .map(|s| harnessed(cfg, s))
        .map_err(Error::Screen)
    }

    fn restore(cfg: &Arc<Config>, cp: &HarnessCheckpoint) -> Result<Self, Error> {
        let stored: Checkpoint = postcard::from_bytes(&cp.blob)?;
        Session::restored("s", Arc::clone(cfg), &stored)
            .map(|s| harnessed(cfg, s))
            .map_err(|why| Error::Engine(why.as_str().into()))
    }

    fn apply(&mut self, entry: &Entry) -> Result<Option<HarnessCheckpoint>, Error> {
        let mut out = Vec::new();
        let now = Instant::now();
        self.0.apply_all(std::slice::from_ref(entry), now, &mut out);
        if self.1 {
            self.0.sleep(now, &mut out);
        }
        let mut cut = None;
        for o in out {
            match o {
                Out::Checkpoint(cp) => cut = Some(to_harness(cp)),
                Out::Lost => return Err(Error::Engine("session lost".into())),
                _ => {}
            }
        }
        Ok(cut)
    }

    fn finish(self) -> Result<TermState, Error> {
        TermState::capture(self.0.into_emulator().ok_or(Error::Dead)?)
    }
}
