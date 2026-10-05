//! Grid mode end to end in one process: a session engine with headless
//! clients attached, frames encoded and decoded as on the socket, and each
//! client's mirror compared with the terminal (Terminal State Protocol §16).
//! The test names carry the TP test ids they implement; the fuzzing that
//! T2 also asks for is in `grid_fuzz.rs`, T18 is in vorn-term-proto and
//! vornd, and T25's encoding half in vorn-term-proto.

mod common;
mod gridrig;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gridrig::{assert_same, cutting, data_len, mirror_cols, Col, Rig, SCROLLBACK};
use libghostty_vt::key;
use vorn_engine::{GridIn, Peer, Session};
use vorn_grid::LOG_LEN;
use vorn_grid_client::Got;
use vorn_recovery::gen::{Generator, Profile};
use vorn_recovery::{LogBuilder, Rng, Size};
use vorn_screen::Emulator;
use vorn_term_mirror::row_text;
use vorn_term_proto::msg::{
    mods, CopyFormat, EventKind, Fidelity, GridPoint, InputEvent, KeyAction, KeyCode, ResyncReason,
    SelectKind, ServerMsg,
};
use vorn_term_proto::screen::row_flags;
use vorn_term_proto::{Cursor, Entry, GapReason, Record, RecordHeader, Stream};

const P1: Peer = Peer { conn: 1, sid: 1 };

fn log(size: (u16, u16), pieces: &[&str]) -> Vec<Entry> {
    let mut b = LogBuilder::new(Size::new(size.0, size.1));
    for p in pieces {
        b.data(*p);
    }
    b.build().entries
}

/// The messages connection `conn` got that are not frames.
fn messages(rig: &Rig, conn: u64) -> Vec<ServerMsg> {
    rig.got[&conn]
        .iter()
        .filter_map(|g| match g {
            Got::Message(m) => Some(m.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_client_sees_what_the_terminal_shows() {
    let mut rig = Rig::new((40, 6), SCROLLBACK);
    let mut pieces = vec![
        "hello \x1b[1;31mworld\x1b[0m\r\n".to_owned(),
        "\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\ 日本 e\u{301}\r\n".to_owned(),
    ];
    pieces.extend((0..30).map(|i| format!("line {i}\r\n")));
    let pieces: Vec<&str> = pieces.iter().map(String::as_str).collect();
    let entries = log((40, 6), &pieces);
    rig.attach(1, 1, true, None);
    rig.feed(&entries[..2]);
    rig.settle();
    assert_same(rig.em(), rig.mirror(1, 1), "after two lines");
    rig.feed(&entries[2..]);
    rig.settle();
    assert_same(rig.em(), rig.mirror(1, 1), "after scrolling");
    let m = rig.mirror(1, 1);
    assert_eq!(m.text()[4], "line 29");
    assert!(!m.history().is_empty());
}

/// TP-T1: each output byte goes through the terminal's parser exactly once,
/// whatever the grid does (renders, history, search, selection), across
/// checkpoint cuts that swap the terminal for a rebuild. The client side
/// links no terminal at all (vorn-grid-client's `links_no_terminal`).
#[test]
fn t1_each_output_byte_is_parsed_once() {
    let log = Generator::log(11, Profile::mixed().bytes(256 << 10));
    let size = (log.size.cols, log.size.rows);
    let mut rig = Rig::with_config(size, cutting(SCROLLBACK, 32 << 10));
    rig.attach(1, 1, true, None);
    for (n, chunk) in log.entries.chunks(7).enumerate() {
        rig.feed(chunk);
        rig.advance(Duration::from_millis(5));
        rig.ack_all();
        if n % 5 == 0 {
            let line = rig.mirror(1, 1).term().top_line;
            let epoch = rig.mirror(1, 1).term().sb_epoch;
            rig.grid(GridIn::FetchHistory {
                peer: P1,
                req: 1,
                sb_epoch: epoch,
                from_line: line.saturating_sub(50),
                count: 50,
            });
            rig.grid(GridIn::Search {
                peer: P1,
                req: 2,
                query: "ok".into(),
                regex: false,
                case: false,
                from_line: None,
            });
            rig.grid(GridIn::SelectAt {
                peer: P1,
                req: 3,
                at: GridPoint {
                    line,
                    col: 0,
                    sb_epoch: epoch,
                },
                kind: SelectKind::Word,
            });
        }
    }
    rig.settle();
    assert!(
        rig.s.brief().checkpoints > 0,
        "no terminal swap was exercised"
    );
    assert_eq!(rig.em().parsed_bytes(), data_len(&log.entries));
}

/// The reference a T3 check keeps: a terminal fed the records up to the
/// last frame's watermark, and the last frame's revision and watermark.
struct Watermark {
    em: Emulator,
    fed: usize,
    rev: u64,
    resume: Cursor,
    frames: u64,
}

/// TP-T3: every frame shows exactly the terminal that results from the
/// records before its `resume.next_rseq`, checked against a reference
/// terminal fed only those; `rev` rises strictly and `resume` never falls.
#[test]
fn t3_every_frame_is_the_terminal_at_its_watermark() {
    for seed in 0..6 {
        let log = Generator::log(300 + seed, Profile::mixed().bytes(192 << 10));
        let size = (log.size.cols, log.size.rows);
        let mut rig = Rig::with_config(size, cutting(SCROLLBACK, 24 << 10));
        let reference = Rc::new(RefCell::new(Watermark {
            em: Emulator::with_scrollback(u32::from(size.0), u32::from(size.1), SCROLLBACK)
                .unwrap(),
            fed: 0,
            rev: 0,
            resume: Cursor::default(),
            frames: 0,
        }));
        let entries = log.entries.clone();
        let r = Rc::clone(&reference);
        rig.on_frame = Some(Box::new(move |_s: &Session, _conn, sid, client| {
            let m = client.pane(sid).and_then(|p| p.mirror()).unwrap();
            let w = &mut *r.borrow_mut();
            assert!(m.rev() > w.rev, "rev {} after {}", m.rev(), w.rev);
            assert!(m.resume().next_rseq >= w.resume.next_rseq);
            w.rev = m.rev();
            w.resume = m.resume();
            let upto = m.resume().next_rseq as usize;
            for e in &entries[w.fed..upto] {
                match &e.rec {
                    Record::Data { bytes, .. } => w.em.feed(bytes, &mut Vec::new()),
                    Record::Resize { cols, rows, .. } => {
                        w.em.resize(u32::from(*cols), u32::from(*rows), &mut Vec::new())
                            .unwrap()
                    }
                    _ => {}
                }
            }
            w.fed = upto;
            assert_same(&w.em, m, &format!("seed {seed} frame at rseq {upto}"));
            w.frames += 1;
        }));
        rig.attach(1, 1, true, None);
        let mut rng = Rng::new(seed);
        let mut i = 0;
        while i < log.entries.len() {
            let n = (rng.range(1, 6) as usize).min(log.entries.len() - i);
            rig.feed(&log.entries[i..i + n]);
            i += n;
            rig.advance(Duration::from_millis(rng.range(0, 10)));
            if rng.chance(1, 2) {
                rig.ack_all();
            }
        }
        rig.settle();
        assert!(reference.borrow().frames > 5, "seed {seed}: too few frames");
    }
}

/// Feeds `entries` as a source writing `rate` bytes a second would: the
/// clock moves by the time each record takes.
fn burst(rig: &mut Rig, entries: &[Entry], rate: u64, mut each: impl FnMut(&mut Rig, usize)) {
    for (i, e) in entries.iter().enumerate() {
        let len = match &e.rec {
            Record::Data { bytes, .. } => bytes.len() as u64,
            _ => 0,
        };
        rig.feed(std::slice::from_ref(e));
        rig.advance(Duration::from_nanos(len * 1_000_000_000 / rate));
        each(rig, i);
    }
}

/// Records of `chunk` bytes of coloured, scrolling build output, `total`
/// bytes in all, numbered from rseq 0.
fn stream(total: usize, chunk: usize) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut n = 0u64;
    let mut offset = 0u64;
    while (offset as usize) < total {
        while text.len() < chunk {
            n += 1;
            text.push_str(&format!(
                "\x1b[3{}m{n:>8}\x1b[0m compiled crate_{} in {}ms\r\n",
                n % 8,
                n % 97,
                n % 13
            ));
        }
        let bytes: Vec<u8> = text.as_bytes()[..chunk].to_vec();
        text.drain(..chunk);
        let len = bytes.len() as u64;
        out.push(Entry {
            hdr: RecordHeader {
                epoch: 0,
                rseq: out.len() as u64,
                start_offset: offset,
            },
            at_ns: 0,
            rec: Record::Data {
                stream: Stream::Pty,
                bytes,
            },
        });
        offset += len;
    }
    out
}

/// TP-T4: a client attaching in the middle of 50 MB/s of output gets a
/// snapshot at once, keeps getting frames, and converges to the terminal.
#[test]
fn t4_attach_mid_burst_converges() {
    let entries = stream(24 << 20, 64 << 10);
    let mut rig = Rig::new((120, 40), SCROLLBACK);
    rig.attach(2, 1, true, None);
    let half = entries.len() / 2;
    burst(&mut rig, &entries, 50 << 20, |rig, i| {
        rig.ack_all();
        if i == half {
            rig.attach(1, 1, true, None);
        }
    });
    let pane = rig.clients[&1].pane(1).unwrap();
    assert!(pane.frames > 10, "frames during the burst: {}", pane.frames);
    assert_eq!(pane.snapshots, 1);
    rig.settle();
    assert_same(rig.em(), rig.mirror(1, 1), "after the burst");
    assert_same(
        rig.em(),
        rig.mirror(2, 1),
        "the client there from the start",
    );
}

/// TP-T5: two grid clients attach and detach at random during output, and a
/// bytes client parses the same records; at rest all three show the same
/// text. Bytes mode itself is the WebSocket's: the bytes client here is a
/// terminal fed every record, as xterm.js would be.
#[test]
fn t5_many_clients_agree_at_rest() {
    let log = Generator::log(5, Profile::shell().bytes(512 << 10));
    let size = (log.size.cols, log.size.rows);
    let mut rig = Rig::new(size, SCROLLBACK);
    let mut bytes_client =
        Emulator::with_scrollback(u32::from(size.0), u32::from(size.1), SCROLLBACK).unwrap();
    let mut rng = Rng::new(5);
    let mut attached = [false, false];
    for chunk in log.entries.chunks(3) {
        rig.feed(chunk);
        for e in chunk {
            match &e.rec {
                Record::Data { bytes, .. } => bytes_client.feed(bytes, &mut Vec::new()),
                Record::Resize { cols, rows, .. } => bytes_client
                    .resize(u32::from(*cols), u32::from(*rows), &mut Vec::new())
                    .unwrap(),
                _ => {}
            }
        }
        rig.advance(Duration::from_millis(rng.range(1, 10)));
        for (c, on) in attached.iter_mut().enumerate() {
            let conn = c as u64 + 1;
            if rng.chance(1, 12) {
                if *on {
                    rig.detach(conn, 1);
                } else {
                    let resume = rig
                        .clients
                        .get(&conn)
                        .and_then(|cl| cl.pane(1))
                        .and_then(|p| p.resume());
                    rig.attach(conn, 1, true, resume);
                }
                *on = !*on;
            }
        }
        rig.ack_all();
    }
    for (c, on) in attached.iter().enumerate() {
        if !on {
            rig.attach(c as u64 + 1, 1, true, None);
        }
    }
    rig.settle();
    let want = gridrig::terminal_cols(&bytes_client);
    for conn in [1, 2] {
        assert_eq!(mirror_cols(rig.mirror(conn, 1)), want, "client {conn}");
        assert_same(
            &bytes_client,
            rig.mirror(conn, 1),
            &format!("client {conn}"),
        );
    }
}

/// TP-T6: a grid client whose acks are held for 2 s at 50 MB/s costs the
/// session its two frames in flight and nothing more (the hub keeps a
/// revision and a count per attachment, never a queue), a fast client
/// beside it keeps its frame rate, and the slow one converges once it acks.
#[test]
fn t6_a_slow_client_costs_nothing_and_converges() {
    let entries = stream(100 << 20, 64 << 10);
    let mut rig = Rig::new((120, 40), SCROLLBACK);
    rig.attach(1, 1, true, None);
    rig.attach(2, 1, true, None);
    let start = rig.now;
    let mut most_in_flight = 0;
    burst(&mut rig, &entries, 50 << 20, |rig, _| {
        rig.ack(1);
        most_in_flight = most_in_flight.max(rig.unacked[&2].len());
    });
    let elapsed = rig.now - start;
    assert!(elapsed >= Duration::from_secs(2), "{elapsed:?}");
    let fast = rig.traffic[&1];
    let slow = rig.traffic[&2];
    let ticks = elapsed.as_millis() as u64 / 8;
    assert!(
        fast.frames * 10 >= ticks * 6,
        "fast client: {} frames in {ticks} ticks",
        fast.frames
    );
    assert_eq!(slow.frames, 2, "the slow client got its window and no more");
    assert_eq!(most_in_flight, 2);
    rig.settle();
    assert_same(rig.em(), rig.mirror(2, 1), "the slow client, caught up");
    // One frame covers everything since; the scroll log no longer reaches
    // back that far, so it is a snapshot.
    let pane = rig.clients[&2].pane(1).unwrap();
    assert!(pane.frames <= 4, "{}", pane.frames);
}

fn key(name: &str, mods: u16, text: Option<char>) -> InputEvent {
    InputEvent::Key {
        action: KeyAction::Press,
        key: KeyCode::from_name(name).unwrap(),
        mods,
        consumed_mods: 0,
        text: text.map(|c| c.to_string()),
        unshifted: text,
        composing: false,
    }
}

/// What Ghostty's own key encoder writes for `name` with the options set
/// explicitly rather than read from a terminal.
fn ghostty_bytes(name: &str, setup: impl FnOnce(&mut key::Encoder<'static>)) -> Vec<u8> {
    let mut enc = key::Encoder::new().unwrap();
    setup(&mut enc);
    let mut ev = key::Event::new().unwrap();
    ev.set_action(key::Action::Press)
        .set_key(vorn_grid::input::ghostty_key(
            KeyCode::from_name(name).unwrap(),
        ));
    let mut out = Vec::new();
    enc.encode_to_vec(&ev, &mut out).unwrap();
    out
}

/// TP-T11: keys are encoded per the modes in effect when they are dequeued,
/// as the program switches DECCKM, keypad mode and Kitty flags, and match
/// Ghostty's encoder set to those modes.
#[test]
fn t11_keys_follow_the_modes_in_effect() {
    let mut rig = Rig::new((40, 10), 0);
    rig.attach(1, 1, true, None);
    let entries = log(
        (40, 10),
        // DECCKM; keypad application mode (DECKPAM, with mode 1035 off so
        // Ghostty applies it whatever num lock says); a Kitty flags push and
        // pop.
        &[
            "\x1b[?1h",
            "\x1b[?1l",
            "\x1b[?1035l\x1b=",
            "\x1b>",
            "\x1b[>1u",
            "\x1b[<u",
        ],
    );
    let mut seq = 0;
    let mut press = |rig: &mut Rig, event: InputEvent| {
        seq += 1;
        rig.writes.clear();
        rig.grid(GridIn::Input {
            peer: P1,
            input_seq: seq,
            event,
        });
        rig.writes.last().cloned().unwrap()
    };
    assert_eq!(press(&mut rig, key("ArrowUp", 0, None)).0, b"\x1b[A");
    rig.feed(&entries[..1]);
    let up_app = press(&mut rig, key("ArrowUp", 0, None)).0;
    assert_eq!(up_app, b"\x1bOA");
    assert_eq!(
        up_app,
        ghostty_bytes("ArrowUp", |e| {
            e.set_cursor_key_application(true);
        })
    );
    rig.feed(&entries[1..2]);
    assert_eq!(press(&mut rig, key("ArrowUp", 0, None)).0, b"\x1b[A");
    let kp = press(&mut rig, key("NumpadEnter", 0, None)).0;
    rig.feed(&entries[2..3]);
    let kp_app = press(&mut rig, key("NumpadEnter", 0, None)).0;
    assert_ne!(kp, kp_app);
    assert_eq!(
        kp_app,
        ghostty_bytes("NumpadEnter", |e| {
            e.set_keypad_key_application(true)
                .set_ignore_keypad_with_numlock(false);
        })
    );
    rig.feed(&entries[3..4]);
    assert_eq!(press(&mut rig, key("NumpadEnter", 0, None)).0, kp);
    assert_eq!(press(&mut rig, key("Escape", 0, None)).0, b"\x1b");
    rig.feed(&entries[4..5]);
    let esc_kitty = press(&mut rig, key("Escape", 0, None)).0;
    assert_eq!(esc_kitty, b"\x1b[27u");
    assert_eq!(
        esc_kitty,
        ghostty_bytes("Escape", |e| {
            e.set_kitty_flags(key::KittyKeyFlags::DISAMBIGUATE);
        })
    );
    rig.feed(&entries[5..6]);
    assert_eq!(press(&mut rig, key("Escape", 0, None)).0, b"\x1b");
    // Each write names the event it answers, for its InputAck.
    let (bytes, ack) = press(&mut rig, key("A", mods::CTRL, Some('a')));
    assert_eq!(bytes, b"\x01");
    assert_eq!(ack, Some((P1, seq)));
}

/// TP-T13: an app that reconnects mid-output resumes with a delta while the
/// scroll log still reaches its revision and with a snapshot when it does
/// not, and converges either way.
#[test]
fn t13_app_reconnect_resumes_with_a_delta_or_a_snapshot() {
    let mut rig = Rig::new((60, 12), SCROLLBACK);
    let lines: Vec<String> = (0..4000).map(|i| format!("{i}: output line\r\n")).collect();
    let pieces: Vec<&str> = lines.iter().map(String::as_str).collect();
    let entries = log((60, 12), &pieces);
    rig.attach(1, 1, true, None);
    rig.attach(2, 1, true, None);
    rig.feed(&entries[..100]);
    rig.settle();
    // The app dies: it keeps its last frame and resume token.
    let resume = rig.clients[&1].pane(1).unwrap().resume().unwrap();
    rig.detach(1, 1);
    // Inside the log window: a few scrolling frames while it was away.
    for i in 0..5 {
        rig.feed(&entries[100 + i * 10..110 + i * 10]);
        rig.advance(Duration::from_millis(10));
        rig.ack(2);
    }
    let before = rig.got[&1].len();
    rig.attach(1, 1, true, Some(resume));
    let first = rig.got[&1][before..]
        .iter()
        .find(|g| matches!(g, Got::Frame { .. }))
        .cloned();
    assert!(
        matches!(
            first,
            Some(Got::Frame {
                snapshot: false,
                ..
            })
        ),
        "{first:?}"
    );
    rig.settle();
    assert_same(rig.em(), rig.mirror(1, 1), "resumed by delta");
    // Outside it: more scrolling frames than the log holds.
    let resume = rig.clients[&1].pane(1).unwrap().resume().unwrap();
    rig.detach(1, 1);
    let mut at = 150;
    for _ in 0..LOG_LEN + 10 {
        rig.feed(&entries[at..at + 3]);
        at += 3;
        rig.advance(Duration::from_millis(10));
        rig.ack(2);
    }
    let before = rig.got[&1].len();
    rig.attach(1, 1, true, Some(resume));
    let got = &rig.got[&1][before..];
    assert!(got.iter().any(|g| matches!(
        g,
        Got::Message(ServerMsg::Resync {
            reason: ResyncReason::NotRetained,
            ..
        })
    )));
    assert!(got
        .iter()
        .any(|g| matches!(g, Got::Frame { snapshot: true, .. })));
    rig.settle();
    assert_same(rig.em(), rig.mirror(1, 1), "resumed by snapshot");
}

/// TP-T14: vornd killed mid-output and recovered from sessiond: the app gets
/// a snapshot under a new `state_gen`, and `fidelity` is what the Session
/// Recovery Contract's conditions say for the case: exact from a good
/// checkpoint, approximate across lost output.
#[test]
fn t14_vornd_restart_resyncs_with_the_recovery_fidelity() {
    let size = (60, 12);
    let lines: Vec<String> = (0..600)
        .map(|i| format!("\x1b[3{}m{i}\x1b[0m line\r\n", i % 8))
        .collect();
    let pieces: Vec<&str> = lines.iter().map(String::as_str).collect();
    let entries = log(size, &pieces);
    for lose in [false, true] {
        let mut d = common::Sessiond::new(0, size);
        d.append(&entries[..300]);
        let cfg = cutting(SCROLLBACK, 4 << 10);
        let open = d.open();
        let (s, _) = d.recover(Arc::clone(&cfg), open);
        let mut rig = Rig::with(s);
        rig.attach(1, 1, true, None);
        rig.settle();
        let old = rig.clients[&1].pane(1).unwrap().resume().unwrap();
        // More output, then vornd dies with no last word.
        if lose {
            d.log
                .append(Record::Gap {
                    lost_bytes: 100,
                    reason: GapReason::SpoolFull,
                })
                .unwrap();
        }
        d.append(&entries[300..400]);
        let client = rig.clients.remove(&1).unwrap();
        drop(rig);
        let open = d.open();
        let (s, _) = d.recover(Arc::clone(&cfg), open);
        let mut rig = Rig::with(s);
        rig.clients.insert(1, client);
        rig.attach(1, 1, true, Some(old));
        let attached = rig.got[&1].iter().find_map(|g| match g {
            Got::Attached(a) => Some(a.clone()),
            _ => None,
        });
        let want = if lose {
            Fidelity::Approximate
        } else {
            Fidelity::Exact
        };
        assert_eq!(attached.unwrap().fidelity, want, "lose={lose}");
        assert!(rig.got[&1].iter().any(|g| matches!(
            g,
            Got::Message(ServerMsg::Resync {
                reason: ResyncReason::Restarted,
                ..
            })
        )));
        rig.settle();
        let m = rig.mirror(1, 1);
        assert_ne!(m.state_gen(), old.state_gen);
        assert_same(rig.em(), m, &format!("after the restart, lose={lose}"));
    }
}

/// TP-T15: no frame shows a state between a mode 2026 set and its reset,
/// unless the 150 ms hold expired.
#[test]
fn t15_synchronized_output_is_never_shown_half_done() {
    let mut rig = Rig::new((30, 4), 0);
    let entries = log(
        (30, 4),
        &[
            "old screen\r\n",
            "\x1b[?2026h\x1b[2J\x1b[H",
            "half",
            " drawn",
            "\r\nnew screen\x1b[?2026l",
            "\x1b[?2026h\x1b[2J\x1b[Hstuck",
        ],
    );
    let seen = Rc::new(RefCell::new(Vec::<Vec<String>>::new()));
    let s2 = Rc::clone(&seen);
    rig.on_frame = Some(Box::new(move |_s, _conn, sid, client| {
        let m = client.pane(sid).and_then(|p| p.mirror()).unwrap();
        s2.borrow_mut().push(m.text());
    }));
    rig.attach(1, 1, true, None);
    rig.feed(&entries[..1]);
    rig.settle();
    for e in &entries[1..5] {
        rig.feed(std::slice::from_ref(e));
        rig.advance(Duration::from_millis(30));
        rig.ack_all();
    }
    rig.settle();
    for text in seen.borrow().iter() {
        let all = text.join("\n");
        assert!(
            !all.contains("half") || all.contains("new screen"),
            "a frame showed the redraw half done: {text:?}"
        );
    }
    assert!(seen
        .borrow()
        .last()
        .unwrap()
        .join("\n")
        .contains("new screen"));
    // A program that never ends its update is shown once the hold expires.
    let shown = seen.borrow().len();
    rig.feed(&entries[5..]);
    rig.advance(Duration::from_millis(100));
    rig.ack_all();
    assert_eq!(seen.borrow().len(), shown, "shown before the hold expired");
    rig.advance(Duration::from_millis(60));
    rig.ack_all();
    assert!(seen.borrow().last().unwrap().join("\n").contains("stuck"));
}

/// TP-T16: `seq 1 1000000`: a frame costs the new lines it shows, never the
/// lines that scrolled past, and never more than a viewport; every row's
/// line number matches the reference (line `n` holds `n + 1`), and history
/// fetched by line matches it too.
#[test]
fn t16_scrolling_costs_the_new_lines_and_lines_are_absolute() {
    let mut text = Vec::new();
    for n in 1..=1_000_000u32 {
        text.extend_from_slice(format!("{n}\r\n").as_bytes());
    }
    let size = (80u16, 24u16);
    // Room for every line a frame scrolls, so the count is never lost.
    let mut rig = Rig::new(size, 64 << 20);
    let mut entries = Vec::new();
    let mut offset = 0u64;
    for (i, c) in text.chunks(1024).enumerate() {
        entries.push(Entry {
            hdr: RecordHeader {
                epoch: 0,
                rseq: i as u64,
                start_offset: offset,
            },
            at_ns: 0,
            rec: Record::Data {
                stream: Stream::Pty,
                bytes: c.to_vec(),
            },
        });
        offset += c.len() as u64;
    }
    rig.attach(1, 1, true, None);
    let check_lines = |rig: &Rig| {
        let m = rig.mirror(1, 1);
        // The cursor's row may hold half a number yet.
        for r in m.rows().iter().filter(|r| r.y != m.term().cursor.y) {
            let t = row_text(r);
            if !t.is_empty() {
                assert_eq!(t.parse::<u64>().unwrap(), r.line + 1, "row {}", r.y);
            }
        }
    };
    // Few lines a frame, then many: the frame's size follows the lines it
    // shows, up to the viewport.
    let mut small = Vec::new();
    let mut large = Vec::new();
    let mut frames = 0;
    let mut bytes = 0;
    for (i, e) in entries.iter().enumerate() {
        let per_frame = if i < 200 { 1 } else { 16 };
        rig.feed(std::slice::from_ref(e));
        if i % per_frame == 0 {
            rig.advance(Duration::from_millis(8));
            rig.ack_all();
            let t = rig.traffic[&1];
            if t.frames > frames + 1 || t.frames == frames {
                frames = t.frames;
                bytes = t.frame_bytes;
                continue;
            }
            let cost = t.frame_bytes - bytes;
            if i > 10 {
                if per_frame == 1 {
                    &mut small
                } else {
                    &mut large
                }
                .push(cost);
            }
            frames = t.frames;
            bytes = t.frame_bytes;
        }
        if i % 500 == 0 {
            check_lines(&rig);
        }
    }
    rig.settle();
    check_lines(&rig);
    assert_same(rig.em(), rig.mirror(1, 1), "at the end");
    let mean = |v: &[u64]| v.iter().sum::<u64>() / v.len().max(1) as u64;
    // What the whole viewport costs: a new client's snapshot.
    rig.attach(3, 1, true, None);
    let viewport = rig.traffic[&3].frame_bytes;
    // About 150 new lines a frame at first, then sixteen times that: a frame
    // never costs more than the viewport, and the lines that scrolled past
    // between frames never travel.
    assert!(
        mean(&small) <= viewport,
        "small frames: {} of {viewport}",
        mean(&small)
    );
    assert!(
        mean(&large) <= viewport,
        "large frames: {} of {viewport}",
        mean(&large)
    );
    // History by line.
    let m = rig.mirror(1, 1);
    let (top, epoch) = (m.term().top_line, m.term().sb_epoch);
    assert_eq!(epoch, 0, "the count was never lost");
    rig.grid(GridIn::FetchHistory {
        peer: P1,
        req: 7,
        sb_epoch: epoch,
        from_line: top - 200,
        count: 200,
    });
    let reply = messages(&rig, 1)
        .into_iter()
        .rev()
        .find(|m| matches!(m, ServerMsg::History { req: 7, .. }))
        .unwrap();
    let ServerMsg::History { rows, .. } = reply else {
        unreachable!()
    };
    assert_eq!(rows.len(), 200);
    for r in &rows {
        assert_eq!(row_text(r).parse::<u64>().unwrap(), r.line + 1);
    }
    let m = rig.mirror(1, 1);
    assert_eq!(m.gaps(top - 200, top), Vec::new());
    assert_eq!(row_text(m.history_line(top - 1).unwrap()), format!("{top}"));
}

/// Scrolling a few lines a frame costs those lines, not the viewport: the
/// row cache shifts as the client's mirror does.
#[test]
fn t16_a_scrolled_line_costs_one_row() {
    let size = (80u16, 24u16);
    let mut rig = Rig::new(size, SCROLLBACK);
    let lines: Vec<String> = (0..600).map(|i| format!("line {i}\r\n")).collect();
    let pieces: Vec<&str> = lines.iter().map(String::as_str).collect();
    let entries = log(size, &pieces);
    rig.attach(1, 1, true, None);
    rig.feed(&entries[..50]);
    rig.settle();
    let before = rig.traffic[&1];
    for e in &entries[50..400] {
        rig.feed(std::slice::from_ref(e));
        rig.advance(Duration::from_millis(8));
        rig.ack_all();
    }
    let t = rig.traffic[&1];
    let per_frame = (t.frame_bytes - before.frame_bytes) / (t.frames - before.frames);
    // The new line and the blank row below it (about 20 bytes framed each),
    // the cursor and the line counters: a fraction of the viewport.
    assert!(per_frame < 120, "{per_frame} bytes a one-line frame");
    // And k lines a frame cost about k rows.
    let mut at = 400;
    let mut costs = Vec::new();
    for k in [2usize, 4, 8] {
        let before = rig.traffic[&1];
        for _ in 0..10 {
            rig.feed(&entries[at..at + k]);
            at += k;
            rig.advance(Duration::from_millis(8));
            rig.ack_all();
        }
        let t = rig.traffic[&1];
        costs.push((t.frame_bytes - before.frame_bytes) / (t.frames - before.frames));
    }
    let row = 20;
    for (k, c) in [2u64, 4, 8].iter().zip(&costs) {
        assert!(
            *c <= per_frame + k * row,
            "{k} lines a frame cost {c}: {costs:?}"
        );
    }
}

/// TP-T17: a reflow, ED 3 (and DECSED 3), RIS and a scroll past the scrollback cap each
/// raise `sb_epoch` and clear the client's cached history.
#[test]
fn t17_epochs_clear_the_history_cache() {
    let size = (40u16, 8u16);
    let fill: String = (0..40).map(|i| format!("{i} fill\r\n")).collect();
    let huge: String = (0..200_000)
        .map(|i| format!("{i} past the cap\r\n"))
        .collect();
    let mut b = LogBuilder::new(Size::new(size.0, size.1));
    b.data(fill.clone());
    b.resize(Size::new(30, 8));
    b.data(fill.clone());
    b.data("\x1b[3J");
    b.data(fill.clone());
    b.data("\x1b[?3J");
    b.data(fill.clone());
    b.data("\x1bc");
    b.data(fill.clone());
    b.data(huge);
    let entries = b.build().entries;
    let mut rig = Rig::new(size, 64 << 10);
    rig.attach(1, 1, true, None);
    let mut epoch = 0;
    let mut i = 0;
    for (what, n) in [
        ("fill", 1),
        ("reflow", 2),
        ("ED 3", 2),
        ("DECSED 3", 2),
        ("RIS", 2),
        ("past the cap", 1),
    ] {
        rig.feed(&entries[i..i + n]);
        i += n;
        rig.settle();
        let m = rig.mirror(1, 1);
        if what == "fill" {
            assert!(!m.history().is_empty());
            epoch = m.term().sb_epoch;
            continue;
        }
        assert!(
            m.term().sb_epoch > epoch,
            "{what}: epoch {}",
            m.term().sb_epoch
        );
        epoch = m.term().sb_epoch;
        // Nothing cached from before the epoch survives.
        assert!(
            m.history().is_empty(),
            "{what}: {} rows cached",
            m.history().len()
        );
        assert_same(rig.em(), m, what);
    }
}

/// TP-T21: ZWJ emoji, CJK wide characters in the last column, combining
/// marks and variation selectors come through intact, with grapheme
/// clustering (mode 2027) on and off.
#[test]
fn t21_graphemes_and_widths() {
    for mode in ["", "\x1b[?2027h"] {
        let mut rig = Rig::new((10, 6), 0);
        rig.attach(1, 1, true, None);
        let entries = log(
            (10, 6),
            &[
                mode,
                "👩\u{200d}👩\u{200d}👧 family\r\n",
                "123456789日本語\r\n",
                "e\u{301} a\u{308}\u{304} \u{2764}\u{fe0f} \u{263a}\u{fe0e}\r\n",
                "x👍\u{1f3fd}y\u{200d}",
                "\u{1f1fa}\u{1f1f8}",
            ],
        );
        for e in &entries {
            rig.feed(std::slice::from_ref(e));
            rig.settle();
            assert_same(rig.em(), rig.mirror(1, 1), &format!("mode {mode:?}"));
        }
        let cols = mirror_cols(rig.mirror(1, 1));
        // The CJK character that did not fit in the last column wrapped
        // whole.
        assert!(cols
            .iter()
            .flatten()
            .any(|c| matches!(c, Col::Cell { text, wide: true, .. } if text == "日")));
    }
}

/// TP-T22: OSC 8 hyperlinks and OSC 133 prompt marks reach the mirror, and
/// a semantic selection (`SelectAt` Output) selects one command's output,
/// which `Copy` returns as text.
#[test]
fn t22_links_prompts_and_command_output() {
    let size = (40u16, 12u16);
    let mut rig = Rig::new(size, SCROLLBACK);
    rig.attach(1, 1, true, None);
    let entries = log(
        size,
        &[
            "\x1b]133;A\x1b\\$ \x1b]133;B\x1b\\ls\r\n\x1b]133;C\x1b\\",
            "one.txt\r\ntwo.txt\r\n\x1b]8;id=a;https://example.com/three\x1b\\three.txt\x1b]8;;\x1b\\\r\n",
            "\x1b]133;D;0\x1b\\\x1b]133;A\x1b\\$ \x1b]133;B\x1b\\echo hi\r\n\x1b]133;C\x1b\\hi\r\n\x1b]133;D;0\x1b\\",
            "\x1b]133;A\x1b\\$ ",
        ],
    );
    rig.feed(&entries);
    rig.settle();
    assert_same(rig.em(), rig.mirror(1, 1), "prompts and links");
    let m = rig.mirror(1, 1);
    let prompt_rows: Vec<u16> = m
        .rows()
        .iter()
        .filter(|r| r.flags & row_flags::PROMPT != 0)
        .map(|r| r.y)
        .collect();
    assert_eq!(prompt_rows, vec![0, 4, 6], "{:?}", m.text());
    let link = mirror_cols(m)[3].iter().find_map(|c| match c {
        Col::Cell { link: Some(l), .. } => Some(l.clone()),
        _ => None,
    });
    assert_eq!(link.as_deref(), Some("https://example.com/three"));
    // Select the output of `ls` from a point inside it.
    let at = GridPoint {
        line: m.rows()[2].line,
        col: 2,
        sb_epoch: m.term().sb_epoch,
    };
    let (line1, line3) = (m.rows()[1].line, m.rows()[3].line);
    rig.grid(GridIn::SelectAt {
        peer: P1,
        req: 9,
        at,
        kind: SelectKind::Output,
    });
    let Some(ServerMsg::Selection {
        range: Some((from, to)),
        ..
    }) = messages(&rig, 1)
        .into_iter()
        .rev()
        .find(|m| matches!(m, ServerMsg::Selection { .. }))
    else {
        panic!("no selection");
    };
    assert_eq!((from.line, to.line), (line1, line3));
    rig.grid(GridIn::Copy {
        peer: P1,
        req: 10,
        from,
        to,
        rect: false,
        format: CopyFormat::Plain,
    });
    let Some(ServerMsg::Copied { text, .. }) = messages(&rig, 1)
        .into_iter()
        .rev()
        .find(|m| matches!(m, ServerMsg::Copied { .. }))
    else {
        panic!("no copy");
    };
    assert_eq!(text.trim_end(), "one.txt\ntwo.txt\nthree.txt");
    // Search finds text where it is drawn.
    rig.grid(GridIn::Search {
        peer: P1,
        req: 11,
        query: "T(W)O".into(),
        regex: true,
        case: false,
        from_line: None,
    });
    let hits: Vec<_> = messages(&rig, 1)
        .into_iter()
        .filter_map(|m| match m {
            ServerMsg::SearchHits { req: 11, hits, .. } => Some(hits),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!((hits[0].from.col, hits[0].to.col), (0, 2));
    assert_eq!(hits[0].from.line, m_line(&rig, 2));
}

fn m_line(rig: &Rig, y: usize) -> u64 {
    rig.mirror(1, 1).rows()[y].line
}

/// TP-T25 through vornd: clusters of 62, 63, 64, 255 and 4,096 bytes reach
/// the mirror as the terminal holds them, and the row decoder never
/// desyncs on the cells after them.
#[test]
fn t25_long_graphemes_through_the_grid() {
    let mut rig = Rig::new((20, 4), 0);
    rig.attach(1, 1, true, None);
    let mut pieces = vec!["\x1b[?2027h".to_owned()];
    for len in [62usize, 63, 64, 255, 4096, 4097] {
        // A base and combining marks, then a plain letter after it.
        let marks = "\u{301}".repeat((len - 1) / 2);
        pieces.push(format!("a{marks}z\r\n"));
    }
    let pieces: Vec<&str> = pieces.iter().map(String::as_str).collect();
    let entries = log((20, 4), &pieces);
    for (n, e) in entries.iter().enumerate() {
        rig.feed(std::slice::from_ref(e));
        rig.settle();
        if n < entries.len() - 1 {
            assert_same(rig.em(), rig.mirror(1, 1), &format!("cluster {n}"));
        }
    }
    // One of 4,097 bytes arrives as U+FFFD, and the cell after it intact.
    let cols = mirror_cols(rig.mirror(1, 1));
    let last = &cols[cols.len() - 2];
    assert!(
        matches!(&last[0], Col::Cell { text, .. } if text == "\u{fffd}"),
        "{:?}",
        &last[..2]
    );
    assert!(matches!(&last[1], Col::Cell { text, .. } if text == "z"));
}

/// TP-T26: a client detaches while the program mints 500 styles and 50
/// links, then resumes: the delta brings every definition past its marks
/// and every cell renders right. A compaction while it is away gets it
/// ResetTables and a complete snapshot. No frame refers to an id the client
/// lacks: the rig fails on any frame the mirror refuses.
#[test]
fn t26_tables_across_reconnects_and_compaction() {
    let size = (80u16, 24u16);
    let mut rig = Rig::new(size, SCROLLBACK);
    rig.attach(1, 1, true, None);
    let styled = |from: u32, n: u32| -> String {
        (from..from + n)
            .map(|i| {
                let link = if i % 10 == 0 {
                    format!("\x1b]8;;https://example.com/{i}\x1b\\L\x1b]8;;\x1b\\")
                } else {
                    String::new()
                };
                format!(
                    "\x1b[38;2;{};{};{}m#{link}\x1b[0m{}",
                    i % 256,
                    (i / 256) % 256,
                    i / 65536,
                    if i % 40 == 39 { "\r\n" } else { "" }
                )
            })
            .collect()
    };
    let mut b = LogBuilder::new(Size::new(size.0, size.1));
    b.data("before\r\n");
    b.data(styled(1, 500));
    // Enough styles on screen, a frame at a time, to pass the table's limit.
    for k in 0..20 {
        b.data(styled(1000 + k * 400, 400));
    }
    let entries = b.build().entries;
    rig.feed(&entries[..1]);
    rig.settle();
    let resume = rig.clients[&1].pane(1).unwrap().resume().unwrap();
    rig.detach(1, 1);
    rig.feed(&entries[1..2]);
    rig.attach(1, 1, true, Some(resume));
    let first = rig.got[&1].iter().rev().find_map(|g| match g {
        Got::Frame { snapshot, .. } => Some(*snapshot),
        _ => None,
    });
    assert_eq!(first, Some(false), "a delta, not a snapshot");
    rig.settle();
    assert_same(rig.em(), rig.mirror(1, 1), "after the delta");
    assert!(rig.mirror(1, 1).style_mark() > 500);
    assert!(rig.mirror(1, 1).link_mark() > 50);
    // Away again while the table passes its limit and is compacted.
    let resume = rig.clients[&1].pane(1).unwrap().resume().unwrap();
    rig.detach(1, 1);
    rig.attach(2, 1, true, None);
    for e in &entries[2..] {
        rig.feed(std::slice::from_ref(e));
        rig.advance(std::time::Duration::from_millis(10));
        rig.ack(2);
    }
    rig.settle();
    let before = rig.got[&1].len();
    rig.attach(1, 1, true, Some(resume));
    let got = &rig.got[&1][before..];
    assert!(
        got.iter()
            .any(|g| matches!(g, Got::Message(ServerMsg::ResetTables { .. }))),
        "{got:?}"
    );
    rig.settle();
    let m = rig.mirror(1, 1);
    assert!(m.table_gen() > resume.table_gen);
    assert_same(rig.em(), m, "after the compaction");
    assert_same(rig.em(), rig.mirror(2, 1), "the client that saw it happen");
}

/// Hidden attachments get events and no frames; showing one sends one
/// frame with everything since (TP §8).
#[test]
fn hidden_attachments_get_events_only() {
    let mut rig = Rig::new((40, 8), 0);
    rig.attach(1, 1, false, None);
    let entries = log((40, 8), &["hello\x07\r\n", "\x1b]9;done\x07more\r\n"]);
    rig.feed(&entries);
    rig.settle();
    let pane = rig.clients[&1].pane(1).unwrap();
    assert_eq!(pane.frames, 0);
    let kinds: Vec<&EventKind> = pane
        .events
        .iter()
        .map(|(_, k)| k)
        .filter(|k| !matches!(k, EventKind::Status { .. }))
        .collect();
    assert_eq!(
        kinds,
        [
            &EventKind::Bell,
            &EventKind::Notify {
                title: String::new(),
                body: "done".into()
            }
        ]
    );
    rig.grid(GridIn::SetVisible {
        peer: P1,
        visible: true,
    });
    rig.settle();
    assert_eq!(rig.clients[&1].pane(1).unwrap().frames, 1);
    assert_same(rig.em(), rig.mirror(1, 1), "shown");
}

/// A program that prints and exits within one frame interval: the session
/// leaves the engine at once, so its clients get the last frame with the
/// exit, whatever the render clock or their credits say.
#[test]
fn the_last_frame_goes_out_with_the_exit() {
    let mut rig = Rig::new((40, 6), 0);
    rig.attach(1, 1, true, None);
    let mut b = LogBuilder::new(Size::new(40, 6));
    b.data("first\r\n");
    b.data("last words\r\n");
    b.push(Record::Exit {
        code: Some(0),
        signal: None,
    });
    let entries = b.build().entries;
    rig.feed(&entries[..1]);
    // Both credits spent, and the clock not due.
    rig.feed(&entries[1..]);
    assert!(rig.s.closed());
    assert!(rig.mirror(1, 1).text().iter().any(|l| l == "last words"));
    let pane = rig.clients[&1].pane(1).unwrap();
    assert!(pane
        .events
        .iter()
        .any(|(_, k)| matches!(k, EventKind::Exit { code: Some(0), .. })));
}
