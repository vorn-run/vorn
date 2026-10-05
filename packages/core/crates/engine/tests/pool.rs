//! Sessions on the worker pool: many recovered at once, each from its own
//! checkpoint, the way a restarted vornd takes on everything sessiond holds.

mod common;

use std::collections::HashMap;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use common::{config, Sessiond};
use vorn_engine::{
    Config, Effect, Fidelity, GridIn, HubOut, Input, Open, Out, Peer, Pool, Session, State,
};
use vorn_recovery::gen::{Generator, Profile};
use vorn_recovery::Size;
use vorn_term_proto::msg::{Attach, ServerMsg};
use vorn_term_proto::{Cursor, Record};

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
        let i: usize = s.brief.session.parse().unwrap();
        assert_eq!(s.brief.state, State::Live);
        assert_eq!(
            s.brief.base,
            Some(vorn_engine::Base::Newest),
            "{}",
            s.brief.session
        );
        assert_eq!(s.screen, screens[i], "session {i}");
    }
}

/// Thirty-two sessions recover in under 200 ms. Timing on a shared CI machine
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

/// A pool whose outputs arrive on a channel.
fn pool_on_channel(cfg: Config) -> (Pool, mpsc::Receiver<(String, Out)>) {
    let (tx, rx) = mpsc::channel::<(String, Out)>();
    let tx = std::sync::Mutex::new(tx);
    let pool = Pool::new(
        2,
        cfg,
        Arc::new(move |id: &str, out: Out| {
            let _ = tx.lock().unwrap().send((id.to_owned(), out));
        }),
    )
    .unwrap();
    (pool, rx)
}

/// Plays sessiond for the pool's sessions until one leaves the pool;
/// answers what it asked for before, and how it stood last. Every attach is
/// refused when `refuse` is set.
fn until_closed(
    pool: &Pool,
    rx: &mpsc::Receiver<(String, Out)>,
    d: &mut Sessiond,
    refuse: bool,
) -> (Vec<Out>, vorn_engine::Summary) {
    let mut seen = Vec::new();
    loop {
        let (id, out) = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        match out {
            Out::Attach(_) if refuse => {
                let why = vorn_sessiond::AttachRefusal::NotRetained;
                pool.input(&id, Input::Refused(why));
            }
            Out::Attach(from) => {
                for input in d.attach(from) {
                    pool.input(&id, input);
                }
            }
            Out::Closed(summary) => return (seen, *summary),
            o => seen.push(o),
        }
    }
}

/// A session whose program ended leaves the pool once every record is
/// applied, saying how it ended, and takes its disk history with it.
#[test]
fn an_ended_session_leaves_the_pool() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        history: Some(dir.path().to_path_buf()),
        ..(*config(1 << 20)).clone()
    };
    let (pool, rx) = pool_on_channel(cfg);
    let mut d = Sessiond::new(0, SIZE);
    let mut b = vorn_recovery::LogBuilder::new(Size::new(SIZE.0, SIZE.1));
    b.data("last words\r\n").push(Record::Exit {
        code: Some(4),
        signal: None,
    });
    d.append(&b.build().entries);
    pool.open("a", Open::spawned(Cursor::start(0), Some(SIZE)));
    let (seen, last) = until_closed(&pool, &rx, &mut d, false);
    assert!(seen
        .iter()
        .any(|o| matches!(o, Out::Effect(_, Effect::Exit { code: Some(4), .. }))));
    assert_eq!(last.brief.state, State::Ended);
    assert_eq!(last.brief.exited, Some((Some(4), None)));
    assert!(last.screen.starts_with("last words"), "{:?}", last.screen);
    assert!(pool.briefs().is_empty());
    assert!(pool.sessions().is_empty());
    let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert!(left.is_empty(), "{left:?}");
}

/// A grid attach for a session that is not open is refused at once, and one
/// that reaches a session after it left is answered with an error by the
/// worker: either way the client hears, and never waits on an attachment
/// nobody holds.
#[test]
fn a_grid_attach_to_a_session_that_left_fails_closed() {
    // The worker is held inside the sink while it hands over the exit's
    // effect: the session is still placed then, and leaves only once the
    // worker goes on. An attach sent in that window is queued behind the
    // exit and reaches a worker that has closed the session, every time,
    // rather than only when this thread wins a race with the worker.
    let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let (tx, rx) = mpsc::channel::<(String, Out)>();
    let tx = std::sync::Mutex::new(tx);
    let held = Arc::clone(&gate);
    let pool = Pool::new(
        2,
        (*config(1 << 20)).clone(),
        Arc::new(move |id: &str, out: Out| {
            let exit = matches!(out, Out::Effect(_, Effect::Exit { .. }));
            let _ = tx.lock().unwrap().send((id.to_owned(), out));
            if exit {
                let (open, turn) = &*held;
                let mut open = open.lock().unwrap();
                while !*open {
                    open = turn.wait(open).unwrap();
                }
            }
        }),
    )
    .unwrap();
    let attach = |conn| GridIn::Attach {
        peer: Peer { conn, sid: 1 },
        attach: Attach {
            session: "a".into(),
            ..Attach::default()
        },
    };
    assert!(!pool.grid("a", attach(1)));
    let mut d = Sessiond::new(0, SIZE);
    let mut b = vorn_recovery::LogBuilder::new(Size::new(SIZE.0, SIZE.1));
    b.data("bye\r\n").push(Record::Exit {
        code: Some(0),
        signal: None,
    });
    d.append(&b.build().entries);
    pool.open("a", Open::spawned(Cursor::start(0), Some(SIZE)));
    loop {
        let (id, out) = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        match out {
            Out::Attach(from) => {
                for input in d.attach(from) {
                    pool.input(&id, input);
                }
            }
            // The worker waits in the sink: the session is still placed.
            Out::Effect(_, Effect::Exit { .. }) => {
                assert!(pool.grid(&id, attach(2)), "the session is still open");
                let (open, turn) = &*gate;
                *open.lock().unwrap() = true;
                turn.notify_all();
            }
            Out::Grid(HubOut::Send { conn: 2, msg }) => {
                assert!(matches!(msg, ServerMsg::Error { code: 404, .. }), "{msg:?}");
                return;
            }
            _ => {}
        }
    }
}

/// A session with nothing to carry on from is lost and leaves the pool,
/// saying why.
#[test]
fn a_lost_session_leaves_the_pool() {
    let (pool, rx) = pool_on_channel((*config(1 << 20)).clone());
    let mut d = Sessiond::new(0, SIZE);
    let mut open = Open::spawned(Cursor::start(0), Some(SIZE));
    open.newest_cp = Some(Cursor::start(0));
    pool.open("a", open);
    let (seen, last) = until_closed(&pool, &rx, &mut d, true);
    assert!(seen.contains(&Out::Lost));
    assert_eq!(last.brief.state, State::Lost);
    assert_eq!(last.brief.reason, Some("no record to carry on from"));
    assert_eq!(last.brief.rejected.len(), 4, "{:?}", last.brief.rejected);
    assert!(pool.briefs().is_empty());
}

/// The pool's briefs follow its sessions without asking the workers.
#[test]
fn briefs_follow_the_sessions() {
    let (pool, rx) = pool_on_channel((*config(1 << 20)).clone());
    let mut d = Sessiond::new(0, SIZE);
    let mut b = vorn_recovery::LogBuilder::new(Size::new(SIZE.0, SIZE.1));
    b.data("hello\r\n");
    d.append(&b.build().entries);
    pool.open("a", Open::spawned(Cursor::start(0), Some(SIZE)));
    loop {
        let (_, out) = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        match out {
            Out::Attach(from) => {
                for input in d.attach(from) {
                    pool.input("a", input);
                }
            }
            Out::Ack(c) if c == d.log.head() => break,
            _ => {}
        }
    }
    // A worker publishes its brief after the job whose outputs were just
    // read, so the cached copy can trail them for a moment.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let briefs = loop {
        let briefs = pool.briefs();
        if briefs.first().and_then(|b| b.cursor) == Some(d.log.head())
            || std::time::Instant::now() > deadline
        {
            break briefs;
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(briefs.len(), 1);
    assert_eq!(briefs[0].state, State::Live);
    assert_eq!(briefs[0].cursor, Some(d.log.head()));
}

/// Dropping a pool stops its workers at the job in hand, not after
/// everything queued.
#[test]
fn a_dropped_pool_leaves_queued_work() {
    const RECORDS: usize = 400;
    let (pool, rx) = pool_on_channel((*config(1 << 30)).clone());
    let mut b = vorn_recovery::LogBuilder::new(Size::new(SIZE.0, SIZE.1));
    let line = "x".repeat(99) + "\r\n";
    for _ in 0..RECORDS {
        b.data(line.repeat(640));
    }
    let entries = b.build().entries;
    let records = entries.len();
    pool.open("a", Open::spawned(Cursor::start(0), Some(SIZE)));
    for e in entries {
        pool.input("a", Input::Entries(vec![e]));
    }
    drop(pool);
    let acks = rx
        .try_iter()
        .filter(|(_, o)| matches!(o, Out::Ack(_)))
        .count();
    assert!(acks < records, "{acks} of {records} applied after the drop");
}
