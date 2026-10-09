//! The engine under test, and a reference engine to stand in for vornd's.
//!
//! An [`Engine`] is what vornd runs per session: it is fed records in order,
//! keeps a terminal, and now and then cuts a [`Checkpoint`] for sessiond to
//! keep. When vornd dies, a new engine comes back from a checkpoint (or from
//! the session's start) and is fed the records after it. vornd's own session
//! engine implements this trait to run under the harness.
//!
//! [`ReferenceEngine`] is a **test double**, not vornd's engine: a
//! vorn-screen [`Screen`] fed in record order, cutting checkpoints with
//! [`Screen::serialize`] at safe boundaries. It is exact by construction when
//! recovering from the session start, and as exact as the formatter's round
//! trip when recovering from a checkpoint, which is what lets the harness's
//! own tests find where that round trip loses state.

use serde::{Deserialize, Serialize};
use vorn_screen::Screen;
use vorn_term_proto::{Cursor, Entry, Record};

use crate::compare::TermState;
use crate::log::Size;
use crate::Error;

/// A checkpoint as an engine hands it to storage: where it ends, the size it
/// was cut at, and an engine-defined blob the storage never reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    /// The first record and byte the checkpoint does not include.
    pub resume: Cursor,
    pub size: Size,
    pub blob: Vec<u8>,
}

/// What a restarted engine recovers from, in sessiond's terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Restore {
    /// Replay every record from rseq 0 into a fresh engine.
    SessionStart,
    /// The newest checkpoint, the older one when the newest will not
    /// restore, the session start when neither will; then replay after it.
    Checkpoint,
}

/// Where a recovered target wants records from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resume {
    SessionStart,
    From(Cursor),
}

/// A session engine: what vornd runs per terminal. The harness owns the
/// record log and the checkpoint store; the engine owns only its terminal.
pub trait Engine: Sized {
    /// Whatever the engine is configured with; cloned into each incarnation.
    type Config: Clone + std::fmt::Debug;

    /// A new session at its spawn size, before any record.
    fn start(config: &Self::Config, size: Size) -> Result<Self, Error>;

    /// Back from a checkpoint this engine (or an earlier incarnation) cut.
    fn restore(config: &Self::Config, checkpoint: &Checkpoint) -> Result<Self, Error>;

    /// Applies one record, in rseq order. Returns a checkpoint when the
    /// engine cut one after this record.
    fn apply(&mut self, entry: &Entry) -> Result<Option<Checkpoint>, Error>;

    /// The terminal as it stands, for the comparator.
    fn finish(self) -> Result<TermState, Error>;
}

/// sessiond's checkpoint storage, reduced to what recovery reads: the newest
/// checkpoint and the one before it. It belongs to the harness, so it
/// survives the engine's death.
#[derive(Debug, Clone, Default)]
pub struct Store {
    newest: Option<Checkpoint>,
    fallback: Option<Checkpoint>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    /// Keeps `cp` as the newest, the newest before it as the fallback. A
    /// checkpoint that is not past the newest is ignored, as an engine
    /// replaying old records cuts the same checkpoints again.
    pub fn put(&mut self, cp: Checkpoint) {
        if let Some(n) = &self.newest {
            if cp.resume.epoch == n.resume.epoch && cp.resume.next_rseq <= n.resume.next_rseq {
                return;
            }
        }
        self.fallback = self.newest.replace(cp);
    }

    pub fn newest(&self) -> Option<&Checkpoint> {
        self.newest.as_ref()
    }

    pub fn fallback(&self) -> Option<&Checkpoint> {
        self.fallback.as_ref()
    }

    /// Newest first.
    pub fn candidates(&self) -> impl Iterator<Item = &Checkpoint> {
        self.newest.iter().chain(self.fallback.iter())
    }
}

/// How the reference engine keeps its terminal and when it cuts checkpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceConfig {
    /// Ghostty's scrollback limit, in bytes of page memory.
    pub scrollback: usize,
    /// Output bytes between checkpoints: the next safe boundary after this
    /// many bytes gets one.
    pub checkpoint_every: u64,
}

impl Default for ReferenceConfig {
    fn default() -> Self {
        Self {
            scrollback: 256 << 10,
            checkpoint_every: 16 << 10,
        }
    }
}

/// The reference engine's checkpoint blob: the formatter's VT for the
/// screen, and the title and cwd, which are not in it.
#[derive(Debug, Serialize, Deserialize)]
struct Blob {
    screen: String,
    title: String,
    cwd: String,
}

/// A stand-in for vornd's session engine (see the module docs).
pub struct ReferenceEngine {
    config: ReferenceConfig,
    screen: Screen,
    size: Size,
    since_cut: u64,
}

impl std::fmt::Debug for ReferenceEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReferenceEngine")
            .field("config", &self.config)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl ReferenceEngine {
    /// The terminal itself, for tests that look further than the comparator.
    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    fn cut(&mut self, resume: Cursor) -> Result<Checkpoint, Error> {
        let snap = self.screen.serialize()?;
        let blob = postcard::to_stdvec(&Blob {
            screen: snap.screen,
            title: snap.title,
            cwd: snap.cwd,
        })?;
        self.since_cut = 0;
        Ok(Checkpoint {
            resume,
            size: self.size,
            blob,
        })
    }
}

impl Engine for ReferenceEngine {
    type Config = ReferenceConfig;

    fn start(config: &ReferenceConfig, size: Size) -> Result<Self, Error> {
        Ok(Self {
            config: *config,
            screen: Screen::with_scrollback(size.cols.into(), size.rows.into(), config.scrollback)?,
            size,
            since_cut: 0,
        })
    }

    fn restore(config: &ReferenceConfig, cp: &Checkpoint) -> Result<Self, Error> {
        let blob: Blob = postcard::from_bytes(&cp.blob)?;
        let mut engine = Self::start(config, cp.size)?;
        engine.screen.feed(blob.screen.as_bytes());
        engine
            .screen
            .restore_labels(Some(&blob.title), Some(&blob.cwd));
        Ok(engine)
    }

    fn apply(&mut self, entry: &Entry) -> Result<Option<Checkpoint>, Error> {
        match &entry.rec {
            Record::Data { bytes, .. } => {
                self.screen.feed(bytes);
                self.since_cut += bytes.len() as u64;
            }
            &Record::Resize { cols, rows, .. } => {
                self.screen.resize(cols.into(), rows.into())?;
                self.size = Size::new(cols, rows);
            }
            Record::Gap { .. } => return Err(Error::Unsupported("Gap records")),
            Record::Exit { .. } => {}
        }
        // The formatter writes the cursor as a CUP, which cannot say "at the
        // last column, about to wrap": a checkpoint cut then would print the
        // next character over the last one. Waiting for the next record is
        // cheaper than carrying the flag; see `tests/checkpoint.rs`.
        if self.screen.terminal().is_vt_ground()?
            && self.since_cut >= self.config.checkpoint_every
            && !self.screen.terminal().is_cursor_pending_wrap()?
        {
            return self.cut(entry.after()).map(Some);
        }
        Ok(None)
    }

    fn finish(self) -> Result<TermState, Error> {
        TermState::capture(self.screen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::LogBuilder;

    fn cp(rseq: u64) -> Checkpoint {
        Checkpoint {
            resume: Cursor {
                epoch: 0,
                next_rseq: rseq,
                next_offset: rseq,
            },
            size: Size::new(10, 5),
            blob: Vec::new(),
        }
    }

    #[test]
    fn the_store_keeps_the_two_newest() {
        let mut s = Store::new();
        s.put(cp(3));
        s.put(cp(7));
        s.put(cp(5));
        s.put(cp(9));
        let kept: Vec<u64> = s.candidates().map(|c| c.resume.next_rseq).collect();
        assert_eq!(kept, [9, 7]);
    }

    #[test]
    fn checkpoints_wait_for_a_safe_boundary() {
        let config = ReferenceConfig {
            scrollback: 0,
            checkpoint_every: 1,
        };
        let mut b = LogBuilder::new(Size::new(20, 4));
        b.data(b"ab\x1b[3").data(b"1mred\xe6\x97").data(b"\xa5");
        let log = b.build();
        let mut e = ReferenceEngine::start(&config, log.size).unwrap();
        let cuts: Vec<bool> = log
            .entries
            .iter()
            .map(|en| e.apply(en).unwrap().is_some())
            .collect();
        assert_eq!(cuts, [false, false, true]);
    }

    #[test]
    fn a_checkpoint_restores_title_cwd_and_screen() {
        let config = ReferenceConfig {
            scrollback: 0,
            checkpoint_every: 0,
        };
        let mut b = LogBuilder::new(Size::new(20, 4));
        b.data("\x1b]2;t\x07\x1b]5522;cwd;/srv\x07hello");
        let log = b.build();
        let mut e = ReferenceEngine::start(&config, log.size).unwrap();
        let cp = e.apply(&log.entries[0]).unwrap().unwrap();
        let back = ReferenceEngine::restore(&config, &cp).unwrap();
        let (a, b) = (e.finish().unwrap(), back.finish().unwrap());
        crate::compare(&a, &b).unwrap();
        assert_eq!(b.title, "t");
        assert_eq!(b.cwd, "/srv");
    }
}
