//! Sessions on the worker pool: many recovered at once, each from its own
//! checkpoint, the way a restarted vornd takes on everything sessiond holds.

mod common;

use std::collections::HashMap;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use common::{config, Sessiond};
use vorn_engine::{Config, Fidelity, Open, Out, Pool, Session, State};
use vorn_recovery::gen::{Generator, Profile};
use vorn_recovery::Size;
use vorn_term_proto::Cursor;

const SIZE: (u16, u16) = (100, 30);

/// `n` sessions in sessiond, each with output past its newest checkpoint,
/// left by a vornd that died; and what each screen should be.
fn sessions(n: usize, cfg: &Arc<Config>) -> (Vec<Sessiond>, Vec<String>) {
    let profile = Profile::round_trip()
        .bytes(256 << 10)
        .size(Size::new(SIZE.0, SIZE.1));
    let mut held = Vec::new();
    let mut screens = Vec::new();
    for log in Generator::sessions(9, n, profile) {
        let mut d = Sessiond::new(0, SIZE);
        d.append(&log.entries);
        let mut first = Vec::new();
        let mut s = Session::open(
            "s",
            Arc::clone(cfg),
            Open::spawned(Cursor::start(0), Some(SIZE)),
            Instant::now(),
            &mut first,
        );
        d.serve(&mut s, first, &mut Vec::new(), None);
        assert!(d.log.newest_cp().is_some_and(|c| c.next_rseq > 0));
        let mut want = Session::fresh("s", Arc::clone(cfg), SIZE, Cursor::start(0)).unwrap();
        want.apply_all(&log.entries, Instant::now(), &mut Vec::new());
        screens.push(want.summary().screen);
        held.push(d);
    }
    (held, screens)
}

/// Recovers every session in `held` on a pool, playing sessiond for each;
/// returns how long until every one was live, and the pool's report.
fn recover(held: &mut [Sessiond], cfg: &Arc<Config>) -> (Duration, Vec<vorn_engine::Summary>) {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let (tx, rx) = mpsc::channel::<(String, Out)>();
    let tx = std::sync::Mutex::new(tx);
    let pool = Pool::new(
        threads,
        (**cfg).clone(),
        Arc::new(move |id: &str, out: Out| {
            let _ = tx.lock().unwrap().send((id.to_owned(), out));
        }),
    )
    .unwrap();
    let opens: Vec<Open> = held.iter_mut().map(|d| d.open()).collect();
    let started = Instant::now();
    for (i, open) in opens.into_iter().enumerate() {
        pool.open(&i.to_string(), open);
    }
    let mut ready = HashMap::new();
    while ready.len() < held.len() {
        let (id, out) = rx
            .recv_timeout(Duration::from_secs(30))
            .expect("sessions answer");
        let d = &mut held[id.parse::<usize>().unwrap()];
        match out {
            Out::Attach(from) => {
                for input in d.attach(from) {
                    pool.input(&id, input);
                }
            }
            Out::Ready(f) => {
                ready.insert(id, f);
            }
            Out::Lost => panic!("session {id} lost"),
            _ => {}
        }
    }
    let took = started.elapsed();
    assert!(ready.values().all(|f| *f == Fidelity::Exact), "{ready:?}");
    let report = pool.sessions();
    pool.shutdown(false);
    (took, report)
}

/// 32 sessions with checkpoints, recovered at once: each from its newest
/// checkpoint, exact, with the screen a vornd that never died has.
#[test]
fn thirty_two_sessions_recover_from_their_checkpoints() {
    let cfg = config(64 << 10);
    let (mut held, screens) = sessions(32, &cfg);
    let (took, report) = recover(&mut held, &cfg);
    eprintln!("32 sessions recovered in {took:?}");
    assert_eq!(report.len(), 32);
    for s in &report {
        let i: usize = s.session.parse().unwrap();
        assert_eq!(s.state, State::Live);
        assert_eq!(s.base, Some(vorn_engine::Base::Newest), "{}", s.session);
        assert_eq!(s.screen, screens[i], "session {i}");
    }
}

/// The recovery time the design asks for. Timing on a shared CI machine
/// says little, so this runs on demand.
#[test]
#[ignore = "timing; run with --ignored on an idle machine"]
fn thirty_two_sessions_recover_in_under_200ms() {
    let cfg = config(64 << 10);
    let (mut held, _) = sessions(32, &cfg);
    // Recovering changes nothing a second recovery reads: nothing is cut
    // before the newest checkpoint is passed, and the head is reached
    // without passing it.
    let best = (0..3).map(|_| recover(&mut held, &cfg).0).min().unwrap();
    eprintln!("32 sessions recovered in {best:?} (best of 3)");
    assert!(best < Duration::from_millis(200), "{best:?}");
}

/// A clean shutdown cuts a last checkpoint for each session, however far
/// off the cadence's next one is.
#[test]
fn a_clean_shutdown_cuts_a_last_checkpoint() {
    let cfg = config(1 << 30);
    let (tx, rx) = mpsc::channel::<(String, Out)>();
    let tx = std::sync::Mutex::new(tx);
    let pool = Pool::new(
        2,
        (*cfg).clone(),
        Arc::new(move |id: &str, out: Out| {
            let _ = tx.lock().unwrap().send((id.to_owned(), out));
        }),
    )
    .unwrap();
    let mut d = Sessiond::new(0, SIZE);
    let mut b = vorn_recovery::LogBuilder::new(Size::new(SIZE.0, SIZE.1));
    b.data("some output\r\n$ ");
    d.append(&b.build().entries);
    pool.open("a", Open::spawned(Cursor::start(0), Some(SIZE)));
    let mut cuts = Vec::new();
    loop {
        let (_, out) = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        match out {
            Out::Attach(from) => {
                for input in d.attach(from) {
                    pool.input("a", input);
                }
            }
            Out::Checkpoint(cp) => cuts.push(cp.resume),
            Out::Ack(c) if c == d.log.head() => break,
            _ => {}
        }
    }
    // Only the one at the session start: the cadence is far off.
    assert_eq!(cuts, [Cursor::start(0)]);
    pool.shutdown(true);
    let last: Vec<Cursor> = rx
        .try_iter()
        .filter_map(|(_, o)| match o {
            Out::Checkpoint(cp) => Some(cp.resume),
            _ => None,
        })
        .collect();
    assert_eq!(last, [d.log.head()]);
}
