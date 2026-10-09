//! The session engine under the recovery harness: the same record log
//! through an engine that never dies and one killed and recovered from the
//! checkpoints it cut, compared by the recovery equivalence.

mod common;

use std::sync::Arc;

use common::{config, Harnessed};
use vorn_engine::Config;
use vorn_recovery::gen::{Generator, Profile};
use vorn_recovery::{
    differential, run, transcript, Error, InProcess, KillPlan, Log, LogBuilder, Report, Restore,
    Size,
};
use vorn_term_proto::Record;

fn engine(cfg: Arc<Config>) -> impl FnMut() -> Result<InProcess<Harnessed>, Error> {
    move || Ok(InProcess::new(Arc::clone(&cfg), Restore::Checkpoint))
}

/// A differential run that must have recovered from a checkpoint at least
/// once, so the restore is what is tested and not only replay from start.
fn from_checkpoints(log: &Log, plan: &KillPlan, cfg: Arc<Config>) -> Report {
    let report = differential(log, plan, engine(cfg)).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(report.digest, log.digest());
    assert!(
        report.resumed.iter().any(|c| c.next_rseq > 0),
        "every recovery replayed from the session start: {:?}",
        report.resumed
    );
    report
}

/// RC-T2: killed and recovered at random records, the terminal is the one
/// that never died, for output the checkpoints round-trip and for the
/// whole mix.
#[test]
fn differential_state() {
    for seed in 1..=8 {
        let log = Generator::log(seed, Profile::round_trip().bytes(128 << 10));
        from_checkpoints(&log, &KillPlan::random(seed, 6), config(8 << 10));
    }
    for seed in 1..=8 {
        let log = Generator::log(seed, Profile::mixed().bytes(128 << 10));
        differential(&log, &KillPlan::random(seed, 6), engine(config(8 << 10)))
            .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
    }
}

/// RC-T1 at a size CI can afford: eight sessions of seeded output, each
/// killed mid-burst at random records; the recovered stream is the
/// generated one, by hash, with no Gap. The 50 MB run is
/// `killed_mid_burst_fifty_megabytes`.
#[test]
fn killed_mid_burst() {
    mid_burst(1 << 20, 3);
}

#[test]
#[ignore = "50 MB through eight sessions; run with --ignored"]
fn killed_mid_burst_fifty_megabytes() {
    mid_burst(50_000_000 / 8, 3);
}

fn mid_burst(bytes: u64, kills: usize) {
    let logs = Generator::sessions(1, 8, Profile::mixed().bytes(bytes));
    for (i, log) in logs.iter().enumerate() {
        assert!(log
            .entries
            .iter()
            .all(|e| !matches!(e.rec, Record::Gap { .. })));
        let report = run(
            log,
            InProcess::<Harnessed>::new(config(64 << 10), Restore::Checkpoint),
            &KillPlan::random(i as u64, kills),
        )
        .unwrap_or_else(|e| panic!("session {i}: {e}"));
        assert_eq!(report.digest, log.digest(), "session {i}");
        assert_eq!(report.kills.len(), kills);
    }
}

/// RC-T3: a thousand resizes interleaved with output, killed during the
/// storm.
#[test]
fn resize_storm() {
    let log = Generator::log(3, Profile::resize_storm(1000));
    let n = log.entries.len() as u64;
    let plan = KillPlan::at([n / 4, n / 2, n / 2 + 1, 3 * n / 4]);
    let report = from_checkpoints(&log, &plan, config(2 << 10));
    assert_eq!(report.kills.len(), 4);
}

/// Recorded programs (a shell, vim, htop, an agent CLI), killed at random.
#[test]
fn transcripts() {
    for (name, log) in transcript::all().unwrap() {
        differential(&log, &KillPlan::random(11, 4), engine(config(4 << 10)))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

/// RC-T5: records ending inside a CSI, OSC, DCS or UTF-8 character get checkpoints, and recover exactly.
#[test]
fn checkpoints_inside_a_sequence() {
    let mut b = LogBuilder::new(Size::new(30, 6));
    let pieces: [&[u8]; 9] = [
        b"plain ",
        b"\x1b[3",
        b"1mred\x1b[0m ",
        b"\x1b]2;a ti",
        b"tle\x07 ",
        b"\x1bP+q544e",
        b"\x1b\\ ",
        b"caf\xc3",
        b"\xa9 done\r\n",
    ];
    for p in pieces {
        b.data(p);
    }
    let log = b.build();
    let mut e = <Harnessed as vorn_recovery::Engine>::start(&config(0), log.size).unwrap();
    for entry in &log.entries {
        let cut = vorn_recovery::Engine::apply(&mut e, entry).unwrap();
        assert!(cut.is_some(), "no checkpoint after {entry:?}");
    }
    let every: Vec<u64> = (0..log.entries.len() as u64).collect();
    from_checkpoints(&log, &KillPlan::at(every), config(0));
}

/// RC-T18: three resizes at one offset with no output between them, then
/// output that depends on the last size; killed between each pair.
#[test]
fn resizes_at_one_offset() {
    let mut b = LogBuilder::new(Size::new(40, 10));
    b.data("x".repeat(70));
    b.resize(Size::new(20, 10))
        .resize(Size::new(33, 7))
        .resize(Size::new(25, 12));
    b.data(format!("\r\n{}\x1b[5;20Hmark", "y".repeat(60)));
    let log = b.build();
    let resizes: Vec<u64> = log
        .entries
        .iter()
        .filter(|e| matches!(e.rec, Record::Resize { .. }))
        .map(|e| e.hdr.rseq)
        .collect();
    assert_eq!(resizes.len(), 3);
    assert!(log
        .entries
        .iter()
        .filter(|e| matches!(e.rec, Record::Resize { .. }))
        .all(|e| e.hdr.start_offset == 70));
    for kill in resizes {
        from_checkpoints(&log, &KillPlan::at([kill]), config(0));
    }
}
