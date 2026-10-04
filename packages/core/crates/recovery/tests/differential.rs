//! The differential test, in one call: the same record log through an
//! engine that never dies and one killed and recovered, compared by the
//! equivalence. Here with the engine in this process; `child.rs` does the
//! same with real process kills.

use vorn_recovery::gen::{Generator, Profile};
use vorn_recovery::{
    compare, differential, reference, run, transcript, Check, Checkpoint, Engine, Error, InProcess,
    KillPlan, LogBuilder, ReferenceConfig, ReferenceEngine, Restore, Size, TermState,
};
use vorn_term_proto::{Entry, Record, Stream};

fn reference_engine(restore: Restore) -> impl FnMut() -> Result<InProcess<ReferenceEngine>, Error> {
    move || Ok(InProcess::new(ReferenceConfig::default(), restore))
}

#[test]
fn generated_logs_with_random_kills() {
    for seed in 1..=6 {
        let log = Generator::log(seed, Profile::mixed().bytes(128 << 10));
        let report = differential(
            &log,
            &KillPlan::random(seed, 5),
            reference_engine(Restore::SessionStart),
        )
        .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        assert_eq!(report.kills.len(), 5);
        assert!(report.replayed_records > 0);
        assert_eq!(report.digest, log.digest());
    }
}

#[test]
fn transcripts_with_random_kills() {
    for (name, log) in transcript::all().unwrap() {
        differential(
            &log,
            &KillPlan::random(11, 4),
            reference_engine(Restore::SessionStart),
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

/// T3: a thousand resizes interleaved with output, killed during the storm.
#[test]
fn resize_storm() {
    let log = Generator::log(3, Profile::resize_storm(1000));
    let n = log.entries.len() as u64;
    let plan = KillPlan::at([n / 4, n / 2, n / 2 + 1, 3 * n / 4]);
    let report = differential(&log, &plan, reference_engine(Restore::SessionStart)).unwrap();
    assert_eq!(report.kills.len(), 4);
}

/// T1 at a size CI can afford: eight sessions of seeded output, each killed
/// in the middle of a burst; the recovered stream is the generated one, by
/// hash, with no Gap. The full 50 MB run is `t1_fifty_megabytes`.
#[test]
fn killed_mid_burst_through_eight_sessions() {
    mid_burst(8, 2 << 20);
}

#[test]
#[ignore = "50 MB through eight sessions, about 6 s in release on a fast machine; run with --ignored"]
fn t1_fifty_megabytes() {
    mid_burst(8, 50_000_000 / 8);
}

fn mid_burst(sessions: usize, bytes: u64) {
    let logs = Generator::sessions(1, sessions, Profile::mixed().bytes(bytes));
    for (i, log) in logs.iter().enumerate() {
        assert!(log
            .entries
            .iter()
            .all(|e| !matches!(e.rec, Record::Gap { .. })));
        let plan = KillPlan::random(i as u64, 1);
        let report = run(
            log,
            InProcess::<ReferenceEngine>::new(ReferenceConfig::default(), Restore::Checkpoint),
            &plan,
        )
        .unwrap();
        assert_eq!(report.digest, log.digest(), "session {i}");
        assert_eq!(report.kills.len(), 1);
    }
}

/// The run that never dies is the reference run: a terminal fed every record.
#[test]
fn an_unkilled_run_is_the_reference_run() {
    let log = Generator::log(4, Profile::mixed().bytes(64 << 10));
    let report = run(
        &log,
        InProcess::<ReferenceEngine>::new(ReferenceConfig::default(), Restore::SessionStart),
        &KillPlan::Never,
    )
    .unwrap();
    let want = reference(&log, log.end(), ReferenceConfig::default().scrollback).unwrap();
    compare(&want, &report.state).unwrap();
}

/// An engine whose checkpoints claim one record more than they hold: after
/// a recovery that record is never applied.
struct LosesARecord(ReferenceEngine);

impl Engine for LosesARecord {
    type Config = ReferenceConfig;

    fn start(config: &ReferenceConfig, size: Size) -> Result<Self, Error> {
        ReferenceEngine::start(config, size).map(Self)
    }

    fn restore(config: &ReferenceConfig, cp: &Checkpoint) -> Result<Self, Error> {
        ReferenceEngine::restore(config, cp).map(Self)
    }

    fn apply(&mut self, entry: &Entry) -> Result<Option<Checkpoint>, Error> {
        Ok(self.0.apply(entry)?.map(|mut cp| {
            // Claims the next record too; the driver checks the cursor
            // against the log, so the offset is moved to match.
            cp.resume.next_rseq += 1;
            cp.resume.next_offset = u64::MAX;
            cp
        }))
    }

    fn finish(self) -> Result<TermState, Error> {
        self.0.finish()
    }
}

#[test]
fn catches_an_engine_that_loses_a_record() {
    let log = Generator::log(8, Profile::mixed().bytes(128 << 10));
    let err = differential(&log, &KillPlan::random(8, 3), || {
        Ok(InProcess::<LosesARecord>::new(
            ReferenceConfig::default(),
            Restore::Checkpoint,
        ))
    })
    .expect_err("a lost record went unnoticed");
    // The resume cursor names a byte the log does not have there.
    assert!(matches!(err, Error::Cursor { .. }), "{err}");
}

/// Claims a record more, with an offset that matches the log: only the
/// terminals can tell.
struct SkipsARecord(ReferenceEngine, Vec<u64>);

impl Engine for SkipsARecord {
    type Config = (ReferenceConfig, Vec<u64>);

    fn start(config: &Self::Config, size: Size) -> Result<Self, Error> {
        ReferenceEngine::start(&config.0, size).map(|e| Self(e, config.1.clone()))
    }

    fn restore(config: &Self::Config, cp: &Checkpoint) -> Result<Self, Error> {
        ReferenceEngine::restore(&config.0, cp).map(|e| Self(e, config.1.clone()))
    }

    fn apply(&mut self, entry: &Entry) -> Result<Option<Checkpoint>, Error> {
        Ok(self.0.apply(entry)?.map(|mut cp| {
            let at = cp.resume.next_rseq as usize;
            if let Some(&next_offset) = self.1.get(at + 1) {
                cp.resume.next_rseq += 1;
                cp.resume.next_offset = next_offset;
            }
            cp
        }))
    }

    fn finish(self) -> Result<TermState, Error> {
        self.0.finish()
    }
}

#[test]
fn catches_an_engine_that_skips_a_record() {
    let log = Generator::log(8, Profile::shell().bytes(128 << 10));
    let offsets: Vec<u64> = log.entries.iter().map(|e| e.hdr.start_offset).collect();
    let err = differential(&log, &KillPlan::random(2, 3), || {
        Ok(InProcess::<SkipsARecord>::new(
            (ReferenceConfig::default(), offsets.clone()),
            Restore::Checkpoint,
        ))
    })
    .expect_err("a skipped record went unnoticed");
    let m = err.mismatch().unwrap_or_else(|| panic!("{err}"));
    assert!(m.checks().contains(&Check::Screen), "{m}");
}

/// Holds a resize back until the next output, but cuts checkpoints as if it
/// had applied it: a recovery from such a checkpoint never sees the resize.
struct LateResize {
    inner: ReferenceEngine,
    held: Option<Entry>,
}

impl Engine for LateResize {
    type Config = ReferenceConfig;

    fn start(config: &ReferenceConfig, size: Size) -> Result<Self, Error> {
        Ok(Self {
            inner: ReferenceEngine::start(config, size)?,
            held: None,
        })
    }

    fn restore(config: &ReferenceConfig, cp: &Checkpoint) -> Result<Self, Error> {
        Ok(Self {
            inner: ReferenceEngine::restore(config, cp)?,
            held: None,
        })
    }

    fn apply(&mut self, entry: &Entry) -> Result<Option<Checkpoint>, Error> {
        if let Record::Resize { .. } = entry.rec {
            self.held = Some(entry.clone());
            // The bug: a checkpoint that is due is cut here, at a cursor
            // past the resize, from a screen that has not had it yet.
            let as_if_applied = Entry {
                rec: Record::Data {
                    stream: Stream::Pty,
                    bytes: Vec::new(),
                },
                ..entry.clone()
            };
            return self.inner.apply(&as_if_applied);
        }
        let mut cut = None;
        if let Some(resize) = self.held.take() {
            cut = self.inner.apply(&resize)?;
        }
        Ok(self.inner.apply(entry)?.or(cut))
    }

    fn finish(mut self) -> Result<TermState, Error> {
        if let Some(resize) = self.held.take() {
            self.inner.apply(&resize)?;
        }
        self.inner.finish()
    }
}

#[test]
fn catches_an_engine_that_applies_resizes_late() {
    // Whole lines between resizes, so every boundary is a safe checkpoint
    // point, on screens wide enough that nothing soft-wraps.
    let mut b = LogBuilder::new(Size::new(80, 24));
    for i in 0..40u16 {
        b.data(format!("$ echo {i}\r\n{i}\r\n"));
        b.resize(Size::new(60 + i, 10 + i % 7));
    }
    b.data("$ done\r\n");
    let log = b.build();
    // A checkpoint at every safe boundary, so one falls on each resize.
    let config = ReferenceConfig {
        scrollback: 0,
        checkpoint_every: 0,
    };
    let resizes = log
        .entries
        .iter()
        .filter(|e| matches!(e.rec, Record::Resize { .. }))
        .map(|e| e.hdr.rseq);
    let plan = KillPlan::at(resizes);
    // The reference engine recovers this log exactly...
    differential(&log, &plan, || {
        Ok(InProcess::<ReferenceEngine>::new(
            config,
            Restore::Checkpoint,
        ))
    })
    .unwrap();
    // ...and one that applies resizes late does not.
    let err = differential(&log, &plan, || {
        Ok(InProcess::<LateResize>::new(config, Restore::Checkpoint))
    })
    .expect_err("a late resize went unnoticed");
    let m = err.mismatch().unwrap_or_else(|| panic!("{err}"));
    assert!(m.checks().contains(&Check::Dimensions), "{m}");
}
