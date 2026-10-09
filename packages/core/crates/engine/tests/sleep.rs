//! Idle sessions put their terminal away and get it back on demand: when
//! one sleeps, what wakes it, and that the terminal it wakes with is the
//! one it put away.

mod common;
mod gridrig;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gridrig::{assert_same, Rig, SCROLLBACK};
use vorn_engine::{Config, GridIn, Input, Out, Session, Viewed};
use vorn_recovery::{LogBuilder, Size};
use vorn_term_proto::{Cursor, Entry};

const IDLE: Duration = Duration::from_secs(60);
const SIZE: (u16, u16) = (80, 24);

fn idle_config(scrollback: usize) -> Config {
    Config {
        scrollback,
        idle: Some(IDLE),
        build: "test".into(),
        ..Config::default()
    }
}

/// A thousand numbered lines and a prompt, as a shell that printed them.
fn numbered() -> Vec<Entry> {
    let mut b = LogBuilder::new(Size::new(SIZE.0, SIZE.1));
    for chunk in (1..=1000).collect::<Vec<u32>>().chunks(100) {
        let text: String = chunk
            .iter()
            .map(|n| format!("\x1b[3{}mline {n}\x1b[m\r\n", n % 8))
            .collect();
        b.data(text);
    }
    b.data("\x1b]0;title\x07$ ");
    b.build().entries
}

fn session(cfg: Config, entries: &[Entry], at: Instant) -> Session {
    let mut s = Session::fresh("s", Arc::new(cfg), SIZE, Cursor::start(0)).unwrap();
    s.apply_all(entries, at, &mut Vec::new());
    s
}

fn digest(s: &Session) -> u64 {
    s.summary().digest.expect("a terminal")
}

#[test]
fn sleeps_only_once_idle_and_live() {
    let t0 = Instant::now();
    let mut s = session(idle_config(0), &numbered(), t0);
    let mut out = Vec::new();
    assert!(!s.sleep(t0 + IDLE / 2, &mut out));
    assert!(!s.asleep() && out.is_empty());
    assert!(s.sleep(t0 + IDLE, &mut out));
    assert!(s.asleep());
    assert!(s.brief().asleep);
    assert!(s.emulator().is_none());
    assert!(matches!(out.last(), Some(Out::Asleep)), "{out:?}");

    let mut never = session(Config::default(), &numbered(), t0);
    assert!(!never.sleep(t0 + IDLE * 10, &mut Vec::new()));
}

#[test]
fn a_viewer_holds_it_awake_and_the_clock_starts_when_it_goes() {
    let viewed = Arc::new(AtomicBool::new(true));
    let seen = Arc::clone(&viewed);
    let cfg = Config {
        viewed: Some(Viewed(Arc::new(move |id: &str| {
            assert_eq!(id, "s");
            seen.load(Ordering::SeqCst)
        }))),
        ..idle_config(0)
    };
    let t0 = Instant::now();
    let mut s = session(cfg, &numbered(), t0);
    let mut out = Vec::new();
    assert!(!s.sleep(t0 + IDLE, &mut out));
    assert!(!s.sleep(t0 + IDLE * 3, &mut out));
    viewed.store(false, Ordering::SeqCst);
    let gone = t0 + IDLE * 3 + Duration::from_millis(500);
    assert!(!s.sleep(gone, &mut out));
    assert!(!s.sleep(gone + IDLE / 2, &mut out));
    assert!(s.sleep(gone + IDLE, &mut out));
}

#[test]
fn a_grid_client_holds_it_awake() {
    let mut rig = Rig::with_config(SIZE, Arc::new(idle_config(SCROLLBACK)));
    rig.feed(&numbered());
    rig.attach(1, 1, true, None);
    rig.settle();
    let later = rig.now + IDLE * 2;
    assert!(!rig.s.sleep(later, &mut Vec::new()));
    rig.detach(1, 1);
    assert!(!rig
        .s
        .sleep(later + Duration::from_millis(500), &mut Vec::new()));
    assert!(rig.s.sleep(later + IDLE * 2, &mut Vec::new()));
}

/// Asleep, a session that never cut a checkpoint at its cursor cuts one,
/// which recovers the same terminal; woken and asleep again with nothing
/// new, it has nothing new to cut.
#[test]
fn sleep_cuts_a_checkpoint_only_past_the_newest() {
    let t0 = Instant::now();
    let mut s = session(idle_config(0), &numbered(), t0);
    let want = digest(&s);
    let mut out = Vec::new();
    assert!(s.sleep(t0 + IDLE, &mut out));
    let cps: Vec<_> = out
        .iter()
        .filter_map(|o| match o {
            Out::Checkpoint(cp) => Some(cp.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(cps.len(), 1, "{out:?}");
    assert_eq!(Some(cps[0].resume), s.brief().cursor);
    let back = Session::restored("s", Arc::new(idle_config(0)), &cps[0]).unwrap();
    assert_eq!(digest(&back), want);

    let mut out = Vec::new();
    s.output(1, 5, t0 + IDLE, &mut out);
    assert!(!s.asleep());
    assert!(s.sleep(t0 + IDLE * 2, &mut out));
    assert!(
        !out.iter().any(|o| matches!(o, Out::Checkpoint(_))),
        "{out:?}"
    );
}

/// Every way in wakes it to the terminal it put away, and what it answers
/// is what a session that never slept answers.
#[test]
fn every_request_wakes_it_as_it_was() {
    let t0 = Instant::now();
    let at = t0 + IDLE;
    let more = {
        let mut b = LogBuilder::new(Size::new(SIZE.0, SIZE.1));
        for e in numbered() {
            b.push(e.rec);
        }
        b.data("echo hi\r\nhi\r\n$ \x1b[?2004h");
        b.build().entries
    };
    let (before, after) = more.split_at(numbered().len());
    type Poke = fn(&mut Session, Instant, &[Entry], &mut Vec<Out>);
    let pokes: [(&str, Poke); 4] = [
        ("output", |s, now, _, out| s.output(7, 20, now, out)),
        ("snapshot", |s, now, _, out| s.snapshot(7, now, out)),
        ("records", |s, now, rest, out| {
            s.input(Input::Entries(rest.to_vec()), now, out)
        }),
        ("summary", |_, _, _, _| {}),
    ];
    for (what, poke) in pokes {
        let mut awake = session(idle_config(0), before, t0);
        let mut slept = session(idle_config(0), before, t0);
        let digest_before = digest(&slept);
        assert!(slept.sleep(at, &mut Vec::new()), "{what}");
        // Read while asleep, without waking it.
        assert_eq!(digest(&slept), digest_before, "{what}");
        assert!(slept.asleep(), "{what}");

        let (mut a, mut b) = (Vec::new(), Vec::new());
        poke(&mut awake, at, after, &mut a);
        poke(&mut slept, at, after, &mut b);
        assert_eq!(a, b, "{what}");
        assert_eq!(digest(&awake), digest(&slept), "{what}");
        let (x, y) = (awake.summary(), slept.summary());
        assert_eq!(
            (x.title, x.cwd, x.screen, x.lines, x.brief.cursor),
            (y.title, y.cwd, y.screen, y.lines, y.brief.cursor),
            "{what}"
        );
        if what != "summary" {
            assert!(!slept.asleep(), "{what}");
            assert!(slept.emulator().is_some(), "{what}");
        }
    }
}

/// A grid client attaching to a session asleep wakes it and gets the
/// screen and the history the terminal had.
#[test]
fn a_grid_attach_wakes_it_with_its_history() {
    let cfg = Arc::new(idle_config(SCROLLBACK));
    let mut rig = Rig::with_config(SIZE, Arc::clone(&cfg));
    rig.feed(&numbered());
    let want = rig.em().state_digest();
    let history = rig.em().history_clears();
    let mut out = Vec::new();
    assert!(rig.s.sleep(rig.now + IDLE, &mut out));
    rig.deliver(out);
    rig.now += IDLE;
    rig.attach(1, 1, true, None);
    rig.settle();
    assert!(!rig.s.asleep());
    assert_eq!(rig.em().state_digest(), want);
    assert_eq!(rig.em().history_clears(), history);
    assert_same(rig.em(), rig.mirror(1, 1), "after waking");
}

/// Grid requests that end attachments find none on a session asleep, and
/// leave it asleep.
#[test]
fn a_departure_does_not_wake_it() {
    let t0 = Instant::now();
    let mut s = session(idle_config(0), &numbered(), t0);
    assert!(s.sleep(t0 + IDLE, &mut Vec::new()));
    let mut out = Vec::new();
    s.input(Input::Grid(GridIn::Gone { conn: 9 }), t0 + IDLE, &mut out);
    assert!(s.asleep() && out.is_empty(), "{out:?}");
}

/// CPU time this thread has used, so a busy test runner's scheduling does
/// not count against the wake.
#[cfg(unix)]
fn thread_time() -> Duration {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `t` is a valid, writable timespec for the call's duration.
    let r = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    assert_eq!(r, 0, "clock_gettime");
    Duration::new(t.tv_sec as u64, t.tv_nsec as u32)
}

#[cfg(not(unix))]
fn thread_time() -> Duration {
    static T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    T0.get_or_init(Instant::now).elapsed()
}

/// Attach on a terminal just woken: the snapshot a bytes client attaches
/// with, asked of a session asleep, with no pathological slowdown.
#[test]
fn a_snapshot_wakes_it_quickly() {
    let t0 = Instant::now();
    let mut s = session(idle_config(SCROLLBACK), &numbered(), t0);
    let mut times = Vec::new();
    for i in 0..200u32 {
        let now = t0 + IDLE * (i + 1);
        let mut out = Vec::new();
        assert!(s.sleep(now, &mut out));
        out.clear();
        let start = thread_time();
        s.snapshot(u64::from(i), now, &mut out);
        times.push(thread_time() - start);
        assert!(
            matches!(out.last(), Some(Out::Snapshot(_, Some(_)))),
            "{out:?}"
        );
    }
    times.sort();
    let p99 = times[times.len() * 99 / 100];
    eprintln!("wake and snapshot: median {:?}, p99 {p99:?}", times[100]);
    if !cfg!(debug_assertions) {
        // The real 5 ms budget is the scale bench's "attach to snapshot" p99.
        assert!(p99 < Duration::from_millis(50), "p99 {p99:?}");
    }
}
