//! The kill driver: replays a record log into a target, kills it at chosen,
//! random or timed points, recovers it and replays what it lost; and the
//! differential test built on it, in one call.
//!
//! The driver plays sessiond: it holds the log, delivers records in rseq
//! order, and after a kill asks the recovered target where to resume, checks
//! that the answer is a record boundary it actually delivered, and replays
//! from there. A [`Target`] is the thing that dies: an engine in this process
//! ([`InProcess`]) or a real process ([`crate::ChildProcess`]).

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use vorn_term_proto::{Cursor, Entry, Record};

use crate::compare::{compare, TermState};
use crate::engine::{Engine, Restore, Resume, Store};
use crate::log::{Digest, Log, Size};
use crate::rng::Rng;
use crate::Error;

/// What the driver kills and recovers: a vornd, later also a client.
pub trait Target {
    /// Starts the session at its spawn size.
    fn start(&mut self, size: Size) -> Result<(), Error>;

    /// Hands over one record. A target may still be working on earlier ones
    /// when this returns; a kill takes whatever it had not finished.
    fn deliver(&mut self, entry: &Entry) -> Result<(), Error>;

    /// Kills it with no chance to clean up: no flush, no last checkpoint.
    fn kill(&mut self) -> Result<(), Error>;

    /// Restarts it and recovers what it can; says where replay resumes.
    fn recover(&mut self) -> Result<Resume, Error>;

    /// Lets it take in everything delivered, then reads its terminal.
    fn finish(self) -> Result<TermState, Error>
    where
        Self: Sized;
}

/// Kills the target on a timer while the log replays at a set pace, the way
/// a crashing vornd meets a session that keeps printing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chaos {
    pub seed: u64,
    /// Time between kills, give or take `jitter`.
    pub every: Duration,
    pub jitter: Duration,
    /// How long first delivery of the whole log takes; replay after a kill
    /// runs at full speed.
    pub over: Duration,
}

/// When to kill the target. A kill "at rseq r" comes right after record r
/// was delivered, before r + 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KillPlan {
    Never,
    /// After each of these rseqs, once each, in order of delivery.
    At(Vec<u64>),
    /// After `kills` distinct rseqs drawn from the log with `seed`.
    Random {
        seed: u64,
        kills: usize,
    },
    Chaos(Chaos),
}

impl KillPlan {
    pub fn at(rseqs: impl IntoIterator<Item = u64>) -> Self {
        KillPlan::At(rseqs.into_iter().collect())
    }

    pub fn random(seed: u64, kills: usize) -> Self {
        KillPlan::Random { seed, kills }
    }

    /// Kills every `every` ± `jitter` while the log is delivered over `over`.
    pub fn chaos(seed: u64, every: Duration, jitter: Duration, over: Duration) -> Self {
        KillPlan::Chaos(Chaos {
            seed,
            every,
            jitter,
            over,
        })
    }

    /// The rseqs a non-timed plan kills after, for a log of `n` records.
    fn rseqs(&self, n: u64) -> BTreeSet<u64> {
        match self {
            KillPlan::Never | KillPlan::Chaos(_) => BTreeSet::new(),
            KillPlan::At(at) => at.iter().copied().filter(|&r| r < n).collect(),
            KillPlan::Random { seed, kills } => {
                let mut rng = Rng::new(*seed);
                let mut set = BTreeSet::new();
                let want = (*kills).min(usize::try_from(n).unwrap_or(usize::MAX));
                while set.len() < want {
                    set.insert(rng.below(n));
                }
                set
            }
        }
    }
}

/// What a run did, and the state it ended in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The rseqs the target was killed after, in order.
    pub kills: Vec<u64>,
    /// Where each recovery resumed.
    pub resumed: Vec<Cursor>,
    /// Records handed over, replays included.
    pub records_delivered: u64,
    pub bytes_delivered: u64,
    /// Records handed over more than once.
    pub replayed_records: u64,
    /// The data the final state was built from: the log up to the last
    /// resume cursor, then every byte delivered after it. Equal to
    /// [`Log::digest`] exactly when nothing was lost or doubled.
    pub digest: Digest,
    pub state: TermState,
}

/// Replays `log` into `target`, killing it as `plan` says.
pub fn run<T: Target>(log: &Log, mut target: T, plan: &KillPlan) -> Result<Report, Error> {
    log.validate()?;
    let n = log.entries.len();
    let mut kill_at = plan.rseqs(n as u64);
    let chaos = match plan {
        KillPlan::Chaos(c) => Some((*c, Rng::new(c.seed))),
        _ => None,
    };
    let started = Instant::now();
    let mut next_kill = chaos.as_ref().map(|(c, _)| started + c.every);
    let mut chaos = chaos;

    let mut report = Tally::default();
    target.start(log.size)?;
    // The first record not yet delivered at all, which only grows.
    let mut frontier = 0usize;
    let mut i = 0usize;
    while i < n {
        let entry = &log.entries[i];
        if i >= frontier {
            if let Some((c, _)) = &chaos {
                // Pace first delivery: record i is due i/n of the way through.
                let due = started + c.over.mul_f64(i as f64 / n as f64);
                let now = Instant::now();
                if due > now {
                    std::thread::sleep(due - now);
                }
            }
            frontier = i + 1;
        } else {
            report.replayed_records += 1;
        }
        target.deliver(entry)?;
        report.records_delivered += 1;
        if let Record::Data { bytes, .. } = &entry.rec {
            report.bytes_delivered += bytes.len() as u64;
            report.digest.update(bytes);
        }

        let rseq = entry.hdr.rseq;
        let timed_out = next_kill.is_some_and(|t| Instant::now() >= t);
        if kill_at.remove(&rseq) || timed_out {
            target.kill()?;
            report.kills.push(rseq);
            let resumed = match target.recover()? {
                Resume::SessionStart => Cursor::start(log.epoch()),
                Resume::From(c) => c,
            };
            let delivered = entry.after();
            if resumed.epoch == delivered.epoch && resumed.next_rseq > delivered.next_rseq {
                return Err(Error::ResumedAhead { resumed, delivered });
            }
            let before = log.upto(resumed)?;
            report.digest = digest_of(before);
            report.resumed.push(resumed);
            i = before.len();
            if let Some((c, rng)) = &mut chaos {
                next_kill = Some(Instant::now() + jittered(c, rng));
            }
            continue;
        }
        i += 1;
    }
    let state = target.finish()?;
    Ok(Report {
        kills: report.kills,
        resumed: report.resumed,
        records_delivered: report.records_delivered,
        bytes_delivered: report.bytes_delivered,
        replayed_records: report.replayed_records,
        digest: report.digest,
        state,
    })
}

/// A report while the run is under way.
#[derive(Default)]
struct Tally {
    kills: Vec<u64>,
    resumed: Vec<Cursor>,
    records_delivered: u64,
    bytes_delivered: u64,
    replayed_records: u64,
    digest: Digest,
}

/// The differential test in one call: the same log through a target that
/// never dies and one killed as `plan` says, each made by `make`; their
/// terminals must be equivalent and the killed one built from the log's
/// bytes exactly once. Returns the killed run's report.
pub fn differential<T: Target>(
    log: &Log,
    plan: &KillPlan,
    mut make: impl FnMut() -> Result<T, Error>,
) -> Result<Report, Error> {
    let calm = run(log, make()?, &KillPlan::Never)?;
    let killed = run(log, make()?, plan)?;
    compare(&calm.state, &killed.state)?;
    let expected = log.digest();
    if killed.digest != expected {
        return Err(Error::Stream {
            expected,
            got: killed.digest,
        });
    }
    Ok(killed)
}

/// A terminal fed every record before `cursor`, in order, and nothing else:
/// the reference run a recovered state at that cursor is held to.
pub fn reference(log: &Log, cursor: Cursor, scrollback: usize) -> Result<TermState, Error> {
    let mut screen = vorn_screen::Screen::with_scrollback(
        log.size.cols.into(),
        log.size.rows.into(),
        scrollback,
    )?;
    for e in log.upto(cursor)? {
        match &e.rec {
            Record::Data { bytes, .. } => {
                screen.feed(bytes);
            }
            &Record::Resize { cols, rows, .. } => screen.resize(cols.into(), rows.into())?,
            Record::Gap { .. } => return Err(Error::Unsupported("Gap records")),
            Record::Exit { .. } => {}
        }
    }
    TermState::capture(screen)
}

fn digest_of(entries: &[Entry]) -> Digest {
    let mut d = Digest::default();
    for e in entries {
        if let Record::Data { bytes, .. } = &e.rec {
            d.update(bytes);
        }
    }
    d
}

fn jittered(c: &Chaos, rng: &mut Rng) -> Duration {
    let jitter = u64::try_from(c.jitter.as_micros()).unwrap_or(u64::MAX);
    let every = u64::try_from(c.every.as_micros()).unwrap_or(u64::MAX);
    let lo = every.saturating_sub(jitter);
    let hi = every.saturating_add(jitter);
    Duration::from_micros(rng.range(lo, hi).max(1))
}

/// An engine in this process. A kill drops it without a word; the
/// checkpoint store, which stands in for sessiond, lives on.
#[derive(Debug)]
pub struct InProcess<E: Engine> {
    config: E::Config,
    restore: Restore,
    size: Option<Size>,
    engine: Option<E>,
    store: Store,
}

impl<E: Engine> InProcess<E> {
    pub fn new(config: E::Config, restore: Restore) -> Self {
        Self {
            config,
            restore,
            size: None,
            engine: None,
            store: Store::new(),
        }
    }

    /// The checkpoints stored so far.
    pub fn store(&self) -> &Store {
        &self.store
    }
}

impl<E: Engine> Target for InProcess<E> {
    fn start(&mut self, size: Size) -> Result<(), Error> {
        self.size = Some(size);
        self.engine = Some(E::start(&self.config, size)?);
        Ok(())
    }

    fn deliver(&mut self, entry: &Entry) -> Result<(), Error> {
        let engine = self.engine.as_mut().ok_or(Error::Dead)?;
        if let Some(cp) = engine.apply(entry)? {
            self.store.put(cp);
        }
        Ok(())
    }

    fn kill(&mut self) -> Result<(), Error> {
        self.engine = None;
        Ok(())
    }

    fn recover(&mut self) -> Result<Resume, Error> {
        if self.restore == Restore::Checkpoint {
            for cp in self.store.candidates() {
                if let Ok(engine) = E::restore(&self.config, cp) {
                    self.engine = Some(engine);
                    return Ok(Resume::From(cp.resume));
                }
            }
        }
        let size = self.size.ok_or(Error::Dead)?;
        self.engine = Some(E::start(&self.config, size)?);
        Ok(Resume::SessionStart)
    }

    fn finish(self) -> Result<TermState, Error> {
        self.engine.ok_or(Error::Dead)?.finish()
    }
}
