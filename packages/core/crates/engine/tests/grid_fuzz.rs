//! The gate's protocol condition: a headless mirror matches vornd cell for
//! cell under fuzzing (TP-T2). Seeded VT from the recovery harness's
//! generator and the recorded transcripts (vim, htop, an agent CLI) go
//! through a session in random batches, with a client attached whose acks
//! are held back at random, so frames are skipped and coalesced per the
//! credit rules; after every settled frame the mirror is compared with the
//! terminal. Checkpoints are cut along the way, so frames also cross
//! terminal swaps.
//!
//! `VORN_GRID_SEEDS=n` runs more seeds than the default.

mod gridrig;

use std::time::Duration;

use gridrig::{assert_same, cutting, Rig, SCROLLBACK};
use vorn_engine::{GridIn, Peer};
use vorn_recovery::gen::{Generator, Profile};
use vorn_recovery::{transcript, Log, Rng};
use vorn_term_proto::msg::GridResume;

/// Seeds that once failed, kept as regression cases.
const REGRESSIONS: &[u64] = &[
    // A combining joiner Ghostty marked no row dirty for.
    178, // Scrolls and resizes coalesced into one delta.
    939,
];

fn seeds() -> u64 {
    std::env::var("VORN_GRID_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(48)
}

/// Runs `log` through a session with two clients, checking both at random
/// settled points and at the end. The first stays attached (or attaches
/// late); the second comes and goes, resuming with what its mirror holds,
/// and is hidden and shown again.
fn run(log: &Log, seed: u64, what: &str) {
    let size = (log.size.cols, log.size.rows);
    let mut rig = Rig::with_config(size, cutting(SCROLLBACK, 24 << 10));
    let mut rng = Rng::new(seed ^ 0x5eed_9e1d);
    let late = rng.chance(1, 3);
    if !late {
        rig.attach(1, 1, true, None);
    }
    rig.attach(2, 1, true, None);
    let mut second = Second::Attached;
    let entries = &log.entries;
    let mut i = 0;
    let mut checks = 0;
    while i < entries.len() {
        let n = (rng.range(1, 5) as usize).min(entries.len() - i);
        rig.feed(&entries[i..i + n]);
        i += n;
        rig.advance(Duration::from_millis(rng.range(0, 12)));
        if late && i > entries.len() / 3 && !rig.clients.contains_key(&1) {
            rig.attach(1, 1, true, None);
        }
        if rng.chance(1, 3) {
            rig.ack(1);
        }
        if rng.chance(1, 2) {
            rig.ack(2);
        }
        let before = second;
        second = match (second, rng.below(16)) {
            (Second::Attached, 0) => {
                let resume = rig.clients[&2].pane(1).and_then(|p| p.resume());
                rig.detach(2, 1);
                Second::Away(resume)
            }
            (Second::Attached, 1) => {
                rig.grid(GridIn::SetVisible {
                    peer: Peer { conn: 2, sid: 1 },
                    visible: false,
                });
                Second::Hidden
            }
            (Second::Away(resume), 2) => {
                rig.attach(2, 1, true, resume);
                Second::Attached
            }
            (Second::Hidden, 3) => {
                rig.grid(GridIn::SetVisible {
                    peer: Peer { conn: 2, sid: 1 },
                    visible: true,
                });
                Second::Attached
            }
            (s, _) => s,
        };
        if std::env::var("VORN_GRID_TRACE").is_ok() {
            eprintln!(
                "{what}: fed ..{i}, second {before:?} -> {second:?}, unacked {:?}, c1 {:?}, due {:?}",
                rig.unacked,
                (rig.clients.keys().collect::<Vec<_>>(), rig.got.get(&1).map(|g| g.len()), entries.len()),
                rig.s.due().map(|d| d.saturating_duration_since(rig.now))
            );
        }
        if rng.chance(1, 6) && rig.clients.contains_key(&1) {
            rig.settle();
            let at = format!("{what} at record {i}");
            if std::env::var("VORN_GRID_TRACE").is_ok() {
                for g in &rig.got[&1] {
                    eprintln!(
                        "  got {}",
                        format!("{g:?}").chars().take(300).collect::<String>()
                    );
                }
            }
            assert_same(rig.em(), rig.mirror(1, 1), &at);
            if second == Second::Attached {
                assert_same(rig.em(), rig.mirror(2, 1), &format!("{at}, second client"));
            }
            checks += 1;
        }
    }
    if !rig.clients.contains_key(&1) {
        rig.attach(1, 1, true, None);
    }
    match second {
        Second::Away(resume) => rig.attach(2, 1, true, resume),
        Second::Hidden => rig.grid(GridIn::SetVisible {
            peer: Peer { conn: 2, sid: 1 },
            visible: true,
        }),
        Second::Attached => {}
    }
    rig.settle();
    assert_same(rig.em(), rig.mirror(1, 1), &format!("{what} at the end"));
    assert_same(
        rig.em(),
        rig.mirror(2, 1),
        &format!("{what} at the end, second client"),
    );
    let _ = checks;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Second {
    Attached,
    Hidden,
    Away(Option<GridResume>),
}

#[test]
fn generated_sessions_mirror_cell_for_cell() {
    let n = seeds();
    for seed in REGRESSIONS.iter().copied().chain(0..n) {
        let profile = match seed % 3 {
            0 => Profile::mixed(),
            1 => Profile::shell(),
            _ => Profile::full_screen(),
        };
        let log = Generator::log(seed, profile.bytes(48 << 10));
        run(&log, seed, &format!("seed {seed}"));
    }
}

#[test]
fn resize_storms_mirror_cell_for_cell() {
    for seed in 0..8 {
        let log = Generator::log(1000 + seed, Profile::resize_storm(60));
        run(&log, seed, &format!("resize storm {seed}"));
    }
}

#[test]
fn recorded_programs_mirror_cell_for_cell() {
    for (name, log) in transcript::all().unwrap() {
        for seed in 0..4 {
            run(&log, seed, &format!("{name} with seed {seed}"));
        }
    }
}
