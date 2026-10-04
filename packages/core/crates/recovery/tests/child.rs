//! Real kills: the reference engine in the `recovery-subject` process,
//! killed by the OS (SIGKILL, TerminateProcess) while records stream to it,
//! then restarted and compared with a run that was never killed.

use std::time::Duration;

use vorn_recovery::gen::{Generator, Profile};
use vorn_recovery::{
    differential, transcript, Check, ChildProcess, Error, KillPlan, ReferenceConfig, Restore,
};

const SUBJECT: &str = env!("CARGO_BIN_EXE_recovery-subject");

fn subject(
    config: ReferenceConfig,
    restore: Restore,
) -> impl FnMut() -> Result<ChildProcess, Error> {
    move || Ok(ChildProcess::new(SUBJECT, config, restore))
}

#[test]
fn random_kills_of_the_subject_process() {
    for seed in 1..=3 {
        let log = Generator::log(seed, Profile::mixed().bytes(64 << 10));
        let report = differential(
            &log,
            &KillPlan::random(seed, 4),
            subject(ReferenceConfig::default(), Restore::SessionStart),
        )
        .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        assert_eq!(report.kills.len(), 4);
        assert_eq!(report.digest, log.digest());
    }
}

#[test]
fn random_kills_recovering_from_checkpoints() {
    let config = ReferenceConfig {
        scrollback: 0,
        checkpoint_every: 1024,
    };
    for seed in 1..=3 {
        let log = Generator::log(seed, Profile::round_trip().bytes(128 << 10));
        // Exact kills: without them a slow process start (Windows) can see
        // every kill land before the child applied anything.
        let report = differential(&log, &KillPlan::random(seed, 6), || {
            Ok(ChildProcess::new(SUBJECT, config, Restore::Checkpoint).exact())
        })
        .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        // Checkpoints the subject wrote before it died were kept and used.
        assert!(
            report.resumed.iter().any(|c| c.next_rseq > 0),
            "seed {seed}: {:?}",
            report.resumed
        );
    }
}

/// Chaos: the subject is killed every 150 ± 100 ms while each transcript
/// replays over 1.5 s, and recovers from the session start each time.
#[test]
fn chaos_while_transcripts_replay() {
    for (i, (name, log)) in transcript::all().unwrap().into_iter().enumerate() {
        let plan = KillPlan::chaos(
            i as u64,
            Duration::from_millis(150),
            Duration::from_millis(100),
            Duration::from_millis(1500),
        );
        let report = differential(
            &log,
            &plan,
            subject(ReferenceConfig::default(), Restore::SessionStart),
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            report.kills.len() >= 3,
            "{name}: {} kills",
            report.kills.len()
        );
    }
}

/// Chaos with recovery from checkpoints. Over a transcript that leaves the
/// alternate screen, the one accepted loss is the inactive screen (see
/// `tests/checkpoint.rs`).
#[test]
fn chaos_recovering_from_checkpoints() {
    let config = ReferenceConfig {
        scrollback: 0,
        checkpoint_every: 256,
    };
    let log = Generator::log(77, Profile::round_trip().bytes(256 << 10));
    let plan = KillPlan::chaos(
        1,
        Duration::from_millis(120),
        Duration::from_millis(60),
        Duration::from_millis(1200),
    );
    let report = differential(&log, &plan, subject(config, Restore::Checkpoint)).unwrap();
    assert!(report.kills.len() >= 3, "{} kills", report.kills.len());
    for (name, log) in transcript::all().unwrap() {
        match differential(&log, &plan, subject(config, Restore::Checkpoint)) {
            Ok(_) => {}
            Err(Error::Mismatch(m)) if m.checks() == [Check::SavedScreen] => {}
            Err(e) => panic!("{name}: {e}"),
        }
    }
}

/// A subject that is gone is an error, not a hang.
#[test]
fn a_missing_subject_is_an_error() {
    let log = Generator::log(1, Profile::shell().bytes(1024));
    let missing = || {
        Ok(ChildProcess::new(
            "./no-such-subject",
            ReferenceConfig::default(),
            Restore::SessionStart,
        ))
    };
    assert!(matches!(
        differential(&log, &KillPlan::Never, missing),
        Err(Error::Io(_))
    ));
}
