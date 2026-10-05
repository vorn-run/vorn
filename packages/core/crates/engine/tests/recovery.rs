//! A vornd killed and a new one taking the session on from sessiond: the
//! restore base it picks, replay mode, effects and the disk history. The
//! sessiond here is its real record log, without the sockets.

mod common;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use common::{config, reference, state, writes_in_replay, Sessiond};
use vorn_engine::{Base, Config, Effect, EffectId, Fidelity, Input, Open, Out, Session, State};
use vorn_recovery::gen::{Generator, Profile};
use vorn_recovery::{compare, LogBuilder, Size};
use vorn_sessiond::AttachRefusal;
use vorn_sessiond_wire::Checkpoint;
use vorn_term_proto::{Cursor, Entry, GapReason, Record};

const SIZE: (u16, u16) = (40, 8);

/// Shell-like output with everything that makes an effect along the way:
/// bells, clipboard writes, notifications, cwd changes and queries, a
/// resize and the alternate screen.
fn eventful() -> Vec<Entry> {
    let mut b = LogBuilder::new(Size::new(SIZE.0, SIZE.1));
    let base = Generator::log(
        5,
        Profile::round_trip()
            .bytes(64 << 10)
            .size(Size::new(SIZE.0, SIZE.1)),
    );
    let mut n = 0;
    for e in &base.entries {
        if let Record::Data { bytes, .. } = &e.rec {
            b.data(bytes.clone());
            n += 1;
            match n % 9 {
                1 => b.data("\x07ding\r\n"),
                3 => b.data("\x1b]52;c;aGVsbG8=\x07"),
                4 => b.data("\x1b]9;build done\x07"),
                5 => b.data(format!("\x1b]5522;cwd;/srv/{n}\x07")),
                6 => b.data("\x1b[c\x1b[6n"),
                7 if n == 16 => b.resize(Size::new(SIZE.0 + 7, SIZE.1)),
                8 if n == 26 => b.data("\x1b[?1049hfull screen\x1b[?1049l"),
                _ => &mut b,
            };
        }
    }
    b.build().entries
}

fn effects(outs: &[Out]) -> Vec<(EffectId, Effect)> {
    outs.iter()
        .filter_map(|o| match o {
            Out::Effect(id, e) => Some((id.clone(), e.clone())),
            _ => None,
        })
        .collect()
}

fn ids(fx: &[(EffectId, Effect)], f: impl Fn(&Effect) -> bool) -> Vec<EffectId> {
    fx.iter()
        .filter(|(_, e)| f(e))
        .map(|(id, _)| id.clone())
        .collect()
}

fn at_most_once(e: &Effect) -> bool {
    matches!(e, Effect::Bell | Effect::Clipboard { .. })
}

/// The session run with no kill: every effect it makes, and its terminal.
fn calm(entries: &[Entry], cfg: Arc<Config>) -> Vec<(EffectId, Effect)> {
    let mut s = Session::fresh("s", cfg, SIZE, Cursor::start(0)).unwrap();
    let mut out = Vec::new();
    s.apply_all(entries, Instant::now(), &mut out);
    effects(&out)
}

/// The first vornd: takes the session on as it spawns it, reads records up
/// to `k` of the `sent` it was sent, and dies.
fn first_vornd(d: &mut Sessiond, cfg: Arc<Config>, k: u64) -> Vec<Out> {
    let mut first = Vec::new();
    let mut s = Session::open(
        "s",
        cfg,
        Open::spawned(Cursor::start(0), Some(SIZE)),
        Instant::now(),
        &mut first,
    );
    let mut seen = Vec::new();
    d.serve(&mut s, first, &mut seen, Some(k));
    seen
}

/// A vornd killed after any record, with
/// sessiond having sent it a few more it never read, is rebuilt exactly
/// from a valid restore base, writes nothing to the program while it
/// replays, and makes each effect by its rule: bells and clipboard writes
/// at most once, everything else at least once under the same ids a vornd
/// that never died would give them.
#[test]
fn killed_at_any_record_recovers_exactly() {
    let entries = eventful();
    let n = entries.len() as u64;
    let cfg = config(512);
    let want = reference(Arc::clone(&cfg), SIZE, &entries);
    let calm_fx = calm(&entries, Arc::clone(&cfg));
    assert!(calm_fx.iter().any(|(_, e)| *e == Effect::Bell));
    let mut bases = HashMap::new();
    for k in 0..n {
        let sent = (k + 3).min(n - 1);
        let mut d = Sessiond::new(0, SIZE);
        d.append(&entries[..=sent as usize]);
        let before = first_vornd(&mut d, Arc::clone(&cfg), k);
        d.append(&entries[sent as usize + 1..]);

        let (s, after) = {
            let open = d.open();
            d.recover(Arc::clone(&cfg), open)
        };
        let summary = s.brief();
        *bases.entry(summary.base.unwrap()).or_insert(0) += 1;
        assert_eq!(
            writes_in_replay(&after),
            0,
            "k {k}: wrote to the program in replay"
        );
        assert!(
            after.contains(&Out::Ready(Fidelity::Exact)),
            "k {k}: {summary:?}"
        );
        assert_eq!(s.fidelity(), Fidelity::Exact, "k {k}: {:?}", summary.reason);
        compare(&want, &state(s)).unwrap_or_else(|m| panic!("k {k}: {m}"));

        let (fx1, fx2) = (effects(&before), effects(&after));
        // At most once: nothing twice, and nothing from a record the dead
        // vornd was sent but never read.
        let once1 = ids(&fx1, at_most_once);
        let once2 = ids(&fx2, at_most_once);
        let both: HashSet<_> = once1.iter().chain(&once2).collect();
        assert_eq!(
            both.len(),
            once1.len() + once2.len(),
            "k {k}: a bell or clipboard write twice"
        );
        assert!(
            both.iter().all(|id| id.rseq <= k || id.rseq > sent),
            "k {k}"
        );
        let lost = ids(&calm_fx, at_most_once)
            .into_iter()
            .filter(|id| id.rseq > k && id.rseq <= sent)
            .count();
        assert_eq!(
            both.len() + lost,
            ids(&calm_fx, at_most_once).len(),
            "k {k}"
        );
        // At least once, under the calm run's ids.
        let calm_rest: HashSet<_> = calm_fx
            .iter()
            .filter(|(_, e)| !at_most_once(e))
            .map(|x| format!("{x:?}"))
            .collect();
        let got: HashSet<_> = fx1
            .iter()
            .chain(&fx2)
            .filter(|(_, e)| !at_most_once(e))
            .map(|x| format!("{x:?}"))
            .collect();
        assert_eq!(got, calm_rest, "k {k}");
    }
    // The cadence puts checkpoints every few records, so most recoveries
    // start from one.
    assert!(
        bases.get(&Base::Newest).copied().unwrap_or(0) > n as usize / 2,
        "{bases:?}"
    );
}

/// RC-T7: a log with DA1 and DSR 6 queries; the recovered vornd writes
/// nothing while it replays them, and answers the next one, live.
#[test]
fn no_replayed_query_replies() {
    let mut b = LogBuilder::new(Size::new(SIZE.0, SIZE.1));
    b.data("$ ").data("\x1b[c").data("x\x1b[6n").data("\r\n$ ");
    let entries = b.build().entries;
    let cfg = config(1 << 20);
    let mut d = Sessiond::new(0, SIZE);
    d.append(&entries);
    let before = first_vornd(&mut d, Arc::clone(&cfg), entries.len() as u64);
    assert_eq!(
        before.iter().filter(|o| matches!(o, Out::Write(_))).count(),
        2,
        "answered live"
    );

    let (mut s, after) = {
        let open = d.open();
        d.recover(Arc::clone(&cfg), open)
    };
    assert_eq!(s.brief().base, Some(Base::Newest));
    assert!(
        after.iter().all(|o| !matches!(o, Out::Write(_))),
        "{after:?}"
    );

    let mut b = LogBuilder::new(Size::new(SIZE.0, SIZE.1));
    b.data("\x1b[6n");
    d.append(&b.build().entries);
    let mut live = Vec::new();
    let next = d.log.entries_from(s.brief().cursor.unwrap()).unwrap();
    s.input(Input::Entries(next), Instant::now(), &mut live);
    assert!(
        live.iter()
            .any(|o| matches!(o, Out::Write(w) if w.starts_with(b"\x1b["))),
        "{live:?}"
    );
}

/// RC-T6: after recoveries at many points, the disk history holds each
/// record once, in order.
#[test]
fn no_duplicate_history() {
    let entries = eventful();
    let n = entries.len() as u64;
    let dir = tempfile::tempdir().unwrap();
    let cfg = Arc::new(Config {
        history: Some(dir.path().to_path_buf()),
        ..(*config(1024)).clone()
    });
    for k in [0, 3, n / 3, n / 2, n - 2] {
        let path = dir.path().join("s.log");
        let _ = std::fs::remove_file(&path);
        let mut d = Sessiond::new(0, SIZE);
        d.append(&entries[..=(k as usize + 2).min(entries.len() - 1)]);
        first_vornd(&mut d, Arc::clone(&cfg), k);
        d.append(&entries[(k as usize + 3).min(entries.len())..]);
        // Two kills in a row: the second vornd dies right after taking over.
        let (s, _) = {
            let open = d.open();
            d.recover(Arc::clone(&cfg), open)
        };
        drop(s);
        let (_s, _) = {
            let open = d.open();
            d.recover(Arc::clone(&cfg), open)
        };
        let bytes = std::fs::read(&path).unwrap();
        let (_, start, frames, _) = vorn_pipeline::history::parse(&bytes).unwrap();
        assert_eq!(start, Cursor::start(0));
        let got: Vec<u64> = frames.iter().map(|f| f.rseq).collect();
        let want: Vec<u64> = entries
            .iter()
            .filter(|e| matches!(e.rec, Record::Data { .. } | Record::Resize { .. }))
            .map(|e| e.hdr.rseq)
            .collect();
        assert_eq!(got, want, "kill at {k}");
    }
}

/// Records of `len` bytes each, so a cadence of `per * len` cuts after
/// every `per` records.
fn even_records(count: usize, len: usize) -> Vec<Entry> {
    let mut b = LogBuilder::new(Size::new(SIZE.0, SIZE.1));
    for i in 0..count {
        let mut line = format!("{i:>5} ");
        line.push_str(&"abcdefghij".repeat(len / 10)[..len - line.len() - 2]);
        line.push_str("\r\n");
        b.data(line);
    }
    b.build().entries
}

/// RC-T19, the recovery half: checkpoints at rseq 100 and 200, the newer
/// one corrupt. Recovery turns it down, restores the fallback at 100 and
/// replays records 100 onward exactly.
#[test]
fn a_corrupt_newest_falls_back() {
    let entries = even_records(250, 50);
    let cfg = config(100 * 50);
    let mut d = Sessiond::new(0, SIZE);
    d.append(&entries[..220]);
    first_vornd(&mut d, Arc::clone(&cfg), 219);
    assert_eq!(d.log.newest_cp().map(|c| c.next_rseq), Some(200));
    assert_eq!(d.log.retain_from().next_rseq, 100);
    d.append(&entries[220..]);
    // Damaged in storage, after sessiond checked its CRC.
    d.damage = Some(200);

    let (s, after) = {
        let open = d.open();
        d.recover(Arc::clone(&cfg), open)
    };
    let summary = s.brief();
    assert_eq!(summary.base, Some(Base::Fallback), "{summary:?}");
    assert_eq!(summary.rejected.len(), 1, "{summary:?}");
    assert!(after.contains(&Out::Attach(vorn_sessiond::AttachFrom::FallbackCheckpoint)));
    assert_eq!(s.fidelity(), Fidelity::Exact);
    compare(&reference(cfg, SIZE, &entries), &state(s)).unwrap();
}

/// A session driven by hand: sessiond's answers are given directly.
fn by_hand(open: Open) -> (Session, Vec<Out>) {
    let mut out = Vec::new();
    let s = Session::open("s", config(1 << 20), open, Instant::now(), &mut out);
    (s, out)
}

/// A real checkpoint of a session that printed `text`, cut at the end.
fn checkpoint_of(text: &str) -> (Checkpoint, Vec<Entry>) {
    let mut b = LogBuilder::new(Size::new(SIZE.0, SIZE.1));
    b.data(text);
    let entries = b.build().entries;
    let mut s = Session::fresh("s", config(0), SIZE, Cursor::start(0)).unwrap();
    let mut out = Vec::new();
    s.apply_all(&entries, Instant::now(), &mut out);
    let cp = out
        .into_iter()
        .find_map(|o| match o {
            Out::Checkpoint(cp) => Some(cp),
            _ => None,
        })
        .expect("a checkpoint");
    (cp, entries)
}

fn reseal(mut cp: Checkpoint, f: impl FnOnce(&mut Vec<u8>)) -> Checkpoint {
    f(&mut cp.blob);
    cp.blob_crc32 = crc32fast::hash(&cp.blob);
    cp
}

fn open_with_checkpoints(entries: &[Entry]) -> Open {
    let end = entries.last().unwrap().after();
    Open {
        epoch: 0,
        oldest: Cursor::start(0),
        head: end,
        sent: end,
        newest_cp: Some(end),
        size: SIZE,
        spawn_size: None,
        pty: true,
        exited: None,
    }
}

/// RC-T23: a checkpoint in a format this vornd does not read, one with a
/// bad CRC, one with a part missing and one that fails the restore check
/// are each turned down, and the next base tried; the session is marked
/// approximate only when none qualifies.
#[test]
fn invalid_restore_bases() {
    let (good, entries) = checkpoint_of("hello world\r\n$ ");
    let unreadable = Checkpoint {
        format: 99,
        ..good.clone()
    };
    let bad_crc = Checkpoint {
        blob_crc32: good.blob_crc32 ^ 1,
        ..good.clone()
    };
    let truncated = reseal(good.clone(), |b| b.truncate(b.len() - 9));
    let wrong_screen = reseal(good.clone(), |b| {
        let at = b
            .windows(5)
            .position(|w| w == b"hello")
            .expect("the text is in the blob");
        b[at] = b'j';
    });
    let restored = Session::restored("s", config(0), &wrong_screen);
    assert_eq!(restored.err(), Some(vorn_engine::Rejected::RestoreCheck));

    // Newest unreadable, fallback with a bad CRC: replay from the start.
    let (mut s, mut out) = by_hand(open_with_checkpoints(&entries));
    s.input(
        Input::Checkpoint(unreadable.clone()),
        Instant::now(),
        &mut out,
    );
    s.input(Input::Checkpoint(bad_crc.clone()), Instant::now(), &mut out);
    s.input(Input::Entries(entries.clone()), Instant::now(), &mut out);
    let sum = s.brief();
    assert_eq!(sum.base, Some(Base::SessionStart));
    assert_eq!(sum.rejected.len(), 2, "{sum:?}");
    assert_eq!(s.fidelity(), Fidelity::Exact);
    let attaches: Vec<_> = out.iter().filter(|o| matches!(o, Out::Attach(_))).collect();
    assert_eq!(
        attaches.len(),
        3,
        "newest, fallback, session start: {attaches:?}"
    );

    // Newest with a part missing, fallback good: the fallback.
    let (mut s, mut out) = by_hand(open_with_checkpoints(&entries));
    s.input(
        Input::Checkpoint(truncated.clone()),
        Instant::now(),
        &mut out,
    );
    s.input(Input::Checkpoint(good.clone()), Instant::now(), &mut out);
    assert_eq!(s.brief().base, Some(Base::Fallback));
    assert_eq!(s.fidelity(), Fidelity::Exact);
    assert!(out.contains(&Out::Ready(Fidelity::Exact)));

    // Nothing qualifies and the start is gone: the checkpoint that rebuilt,
    // marked approximate.
    let (mut s, mut out) = by_hand(open_with_checkpoints(&entries));
    s.input(
        Input::Checkpoint(wrong_screen.clone()),
        Instant::now(),
        &mut out,
    );
    s.input(
        Input::Checkpoint(unreadable.clone()),
        Instant::now(),
        &mut out,
    );
    s.input(
        Input::Refused(AttachRefusal::NotRetained),
        Instant::now(),
        &mut out,
    );
    let sum = s.brief();
    assert_eq!(sum.base, Some(Base::Best), "{sum:?}");
    assert_eq!(sum.rejected.len(), 3, "{sum:?}");
    assert_eq!(s.fidelity(), Fidelity::Approximate);
    assert!(out.contains(&Out::Attach(vorn_sessiond::AttachFrom::Cursor(
        wrong_screen.resume
    ))));
    assert!(out.contains(&Out::Ready(Fidelity::Approximate)));

    // And with the records after that checkpoint gone too, the oldest
    // record into a blank terminal.
    out.clear();
    s.input(
        Input::Refused(AttachRefusal::NotRetained),
        Instant::now(),
        &mut out,
    );
    let sum = s.brief();
    assert_eq!(sum.base, Some(Base::Oldest), "{sum:?}");
    assert_eq!(s.fidelity(), Fidelity::Approximate);
    assert!(!out.contains(&Out::Lost), "{out:?}");
    assert!(
        out.iter()
            .any(|o| matches!(o, Out::Attach(vorn_sessiond::AttachFrom::Cursor(_)))),
        "{out:?}"
    );
}

/// A clean stop cuts past every record applied: a resize or the exit as
/// much as output, so the next vornd has nothing to replay.
#[test]
fn a_clean_stop_covers_records_without_output() {
    let mut b = LogBuilder::new(Size::new(SIZE.0, SIZE.1));
    b.data("one\r\n")
        .resize(Size::new(SIZE.0 + 2, SIZE.1))
        .push(Record::Exit {
            code: Some(0),
            signal: None,
        });
    let entries = b.build().entries;
    let mut s = Session::fresh("s", config(1 << 20), SIZE, Cursor::start(0)).unwrap();
    let now = Instant::now();
    let mut out = Vec::new();
    for e in &entries {
        out.clear();
        s.apply_all(std::slice::from_ref(e), now, &mut out);
        s.shutdown(&mut out);
        let cut = out.iter().find_map(|o| match o {
            Out::Checkpoint(cp) => Some(cp.resume),
            _ => None,
        });
        assert_eq!(cut, Some(e.after()), "after {:?}", e.rec);
        // And nothing more until another record is applied.
        out.clear();
        s.shutdown(&mut out);
        assert!(out.is_empty(), "{out:?}");
    }
}

/// A session is marked exact only when it was rebuilt exactly. Approximate
/// from an unreadable checkpoint with nothing else to go on, across a gap, and from the
/// session start of a session resized before vornd knew its spawn size.
/// Exact from a good checkpoint and by replay from rseq 0.
#[test]
fn exact_only_when_earned() {
    let (good, entries) = checkpoint_of("one\r\ntwo\r\n");
    let unreadable = Checkpoint {
        format: 2,
        ..good.clone()
    };

    let (mut s, mut out) = by_hand(open_with_checkpoints(&entries));
    s.input(
        Input::Checkpoint(unreadable.clone()),
        Instant::now(),
        &mut out,
    );
    s.input(
        Input::Refused(AttachRefusal::NoSuchCheckpoint),
        Instant::now(),
        &mut out,
    );
    s.input(
        Input::Refused(AttachRefusal::NotRetained),
        Instant::now(),
        &mut out,
    );
    assert_eq!(s.brief().base, Some(Base::Oldest));
    assert_eq!(s.fidelity(), Fidelity::Approximate);

    let (mut s, mut out) = by_hand(open_with_checkpoints(&entries));
    s.input(Input::Checkpoint(good.clone()), Instant::now(), &mut out);
    assert_eq!(s.fidelity(), Fidelity::Exact);
    assert!(out.contains(&Out::Ready(Fidelity::Exact)));
    compare(&reference(config(0), SIZE, &entries), &state(s)).unwrap();

    // Replay from rseq 0, spawn size known: exact.
    let mut d = Sessiond::new(0, SIZE);
    d.append(&entries);
    let mut open = d.open();
    open.spawn_size = Some(SIZE);
    let (s, out) = d.recover(config(1 << 20), open);
    assert_eq!(s.brief().base, Some(Base::SessionStart));
    assert!(out.contains(&Out::Ready(Fidelity::Exact)));

    // The same with a resize in the log and no spawn size: approximate.
    let mut b = LogBuilder::new(Size::new(SIZE.0, SIZE.1));
    b.data("a").resize(Size::new(20, 5)).data("b");
    let mut d = Sessiond::new(0, SIZE);
    d.append(&b.build().entries);
    let (s, out) = {
        let open = d.open();
        d.recover(config(1 << 20), open)
    };
    assert_eq!(s.brief().reason, Some("spawn size unknown"));
    assert!(out.contains(&Out::Ready(Fidelity::Approximate)));

    // Across a gap: approximate, and a nudge to redraw once live, not
    // while replaying.
    let mut d = Sessiond::new(0, SIZE);
    d.append(&entries);
    d.log
        .append(Record::Gap {
            lost_bytes: 100,
            reason: GapReason::SpoolFull,
        })
        .unwrap();
    d.log
        .append(Record::Data {
            stream: vorn_term_proto::Stream::Pty,
            bytes: b"after".to_vec(),
        })
        .unwrap();
    let mut open = d.open();
    open.spawn_size = Some(SIZE);
    let (s, out) = d.recover(config(1 << 20), open);
    assert_eq!(s.fidelity(), Fidelity::Approximate);
    assert_eq!(s.brief().reason, Some("output lost"));
    let ready = out.iter().position(|o| matches!(o, Out::Ready(_))).unwrap();
    let nudge = out
        .iter()
        .position(|o| matches!(o, Out::Nudge { .. }))
        .unwrap();
    assert!(nudge > ready);
    assert!(
        s.summary().screen.starts_with("after"),
        "{:?}",
        s.summary().screen
    );
}

/// A vornd that spawned the session cuts a checkpoint at its start, so a
/// later recovery knows the spawn size even after the session resized.
#[test]
fn a_spawned_session_keeps_its_spawn_size() {
    let mut b = LogBuilder::new(Size::new(SIZE.0, SIZE.1));
    b.data("x".repeat(60)).resize(Size::new(20, 5)).data("y");
    let entries = b.build().entries;
    let cfg = config(1 << 20);
    let mut d = Sessiond::new(0, SIZE);
    d.append(&entries);
    first_vornd(&mut d, Arc::clone(&cfg), 0);
    assert_eq!(d.log.newest_cp(), Some(Cursor::start(0)));
    let (s, out) = {
        let open = d.open();
        d.recover(Arc::clone(&cfg), open)
    };
    assert_eq!(s.brief().base, Some(Base::Newest));
    assert!(out.contains(&Out::Ready(Fidelity::Exact)));
    assert_eq!(s.brief().state, State::Live);
    compare(&reference(cfg, SIZE, &entries), &state(s)).unwrap();
}

/// A clean stop after the program exited cuts a checkpoint past the Exit
/// record, which leaves nothing to replay: the next vornd still knows how
/// the program ended, from sessiond.
#[test]
fn an_exit_inside_the_newest_checkpoint_is_still_known() {
    let mut b = LogBuilder::new(Size::new(SIZE.0, SIZE.1));
    b.data("bye\r\n").push(Record::Exit {
        code: Some(3),
        signal: None,
    });
    let entries = b.build().entries;
    let cfg = config(1 << 20);
    let mut d = Sessiond::new(0, SIZE);
    d.append(&entries);
    let mut first = Vec::new();
    let mut s = Session::open(
        "s",
        Arc::clone(&cfg),
        Open::spawned(Cursor::start(0), Some(SIZE)),
        Instant::now(),
        &mut first,
    );
    let mut seen = Vec::new();
    d.serve(&mut s, first, &mut seen, None);
    let mut last = Vec::new();
    s.shutdown(&mut last);
    d.serve(&mut s, last, &mut seen, None);
    drop(s);
    assert_eq!(d.log.newest_cp(), Some(d.log.head()));

    let (s, after) = {
        let open = d.open();
        d.recover(Arc::clone(&cfg), open)
    };
    assert!(after.contains(&Out::Ready(Fidelity::Exact)));
    let summary = s.summary();
    assert_eq!(summary.brief.base, Some(Base::Newest));
    assert_eq!(summary.brief.exited, Some((Some(3), None)));
    assert!(summary.screen.starts_with("bye"), "{:?}", summary.screen);
}

/// A summary carries the terminal's state digest: the same for a session
/// fed the records in one batch or one at a time, different once the state
/// moves on, and absent before there is a terminal.
#[test]
fn the_summary_carries_the_state_digest() {
    let entries = eventful();
    let start = Cursor::start(entries[0].hdr.epoch);
    let cfg = config(1 << 40);
    let mut whole = Session::fresh("a", Arc::clone(&cfg), SIZE, start).unwrap();
    let mut piecemeal = Session::fresh("b", Arc::clone(&cfg), SIZE, start).unwrap();
    let now = Instant::now();
    whole.apply_all(&entries, now, &mut Vec::new());
    for e in &entries {
        piecemeal.apply_all(std::slice::from_ref(e), now, &mut Vec::new());
    }
    let digest = whole.summary().digest.expect("a terminal");
    assert_eq!(Some(digest), piecemeal.summary().digest);
    assert_eq!(digest, whole.emulator().unwrap().state_digest());

    let blank = Session::fresh("c", cfg, SIZE, start).unwrap();
    assert_ne!(blank.summary().digest, Some(digest));

    // Waiting on sessiond for a checkpoint: no terminal yet.
    let mut open = Open::spawned(start, Some(SIZE));
    open.newest_cp = Some(start);
    let waiting = Session::open("d", config(1 << 40), open, now, &mut Vec::new());
    assert_eq!(waiting.brief().state, State::Attaching);
    assert_eq!(waiting.summary().digest, None);
}
