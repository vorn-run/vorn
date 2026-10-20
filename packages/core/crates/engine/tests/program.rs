//! OSC 7501 program status records through the session actor: kept per
//! session, dropped at a prompt or exit, carried by checkpoints, and the
//! query answered only live.

mod common;

use std::sync::Arc;
use std::time::Instant;

use common::{config, writes_in_replay, Sessiond};
use vorn_engine::{BlockedKind, Effect, Input, Open, Out, ProgramState, Programs, Session, FORMAT};
use vorn_recovery::{LogBuilder, Size};
use vorn_term_proto::{Cursor, Entry, Record};

const SIZE: (u16, u16) = (40, 8);

fn b64(s: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in s.as_bytes().chunks(3) {
        let n = c
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn osc(fields: &str) -> String {
    format!("\x1b]7501;{fields}\x1b\\")
}

fn programs(outs: &[Out]) -> Vec<Programs> {
    outs.iter()
        .filter_map(|o| match o {
            Out::Effect(_, Effect::Program(p)) => Some(p.clone()),
            _ => None,
        })
        .collect()
}

fn run(entries: &[Entry]) -> (Session, Vec<Out>) {
    let mut s = Session::fresh("s", config(1 << 20), SIZE, Cursor::start(0)).unwrap();
    let mut out = Vec::new();
    s.apply_all(entries, Instant::now(), &mut out);
    (s, out)
}

/// The spec's examples, one record each: working, blocked on auth, working
/// again and done, with progress and messages, and a child record.
fn script() -> Vec<Entry> {
    let mut b = LogBuilder::new(Size::new(SIZE.0, SIZE.1));
    scripted(&mut b);
    b.build().entries
}

fn scripted(b: &mut LogBuilder) {
    b.data(osc(&format!(
        "state=working:progress=10:app=deploy:msg={}",
        b64("Building")
    )))
    .data(osc(&format!(
        "state=blocked:kind=auth:msg={}",
        b64("Sign in to continue")
    )))
    .data(osc(&format!(
        "state=working:progress=60:msg={}",
        b64("Uploading")
    )))
    .data(osc(&format!(
        "state=working:id=build/test:progress=50:title={}",
        b64("tests")
    )))
    .data(osc(&format!(
        "state=done:app=deploy:msg={}",
        b64("Deployed")
    )));
}

#[test]
fn a_script_drives_working_blocked_working_done_with_a_child() {
    let (_, out) = run(&script());
    let seen = programs(&out);
    let root: Vec<_> = seen
        .iter()
        .map(|p| {
            let r = &p.records[0];
            assert_eq!(r.id, "");
            (r.state, r.kind, r.progress, r.msg.as_str())
        })
        .collect();
    assert_eq!(
        root,
        [
            (ProgramState::Working, None, Some(10), "Building"),
            (
                ProgramState::Blocked,
                Some(BlockedKind::Auth),
                None,
                "Sign in to continue"
            ),
            (ProgramState::Working, None, Some(60), "Uploading"),
            (ProgramState::Working, None, Some(60), "Uploading"),
            (ProgramState::Done, None, None, "Deployed"),
        ]
    );
    assert!(seen.iter().all(|p| p.native));
    let last = seen.last().unwrap();
    assert_eq!(last.records.len(), 2);
    let child = &last.records[1];
    assert_eq!(
        (child.id.as_str(), child.title.as_str(), child.progress),
        ("build/test", "tests", Some(50))
    );
    // A record without an app shows its ancestor's.
    assert_eq!(child.app, "deploy");
    assert_eq!(seen[1].records[0].app, "", "a report replaces its record");
}

#[test]
fn an_exit_or_a_prompt_clears_working_and_blocked() {
    let mut b = LogBuilder::new(Size::new(SIZE.0, SIZE.1));
    b.data(osc("state=working:id=a"))
        .data(osc("state=blocked:kind=permission:id=b"))
        .data(osc("state=error:id=c"))
        .push(Record::Exit {
            code: None,
            signal: Some(9),
        });
    let (_, out) = run(&b.build().entries);
    let last = programs(&out).pop().unwrap();
    assert!(!last.native);
    assert_eq!(last.records.len(), 1);
    assert_eq!(last.records[0].state, ProgramState::Error);
    // The record's change comes before the exit.
    let at = |f: fn(&Effect) -> bool| {
        out.iter()
            .position(|o| matches!(o, Out::Effect(_, e) if f(e)))
            .unwrap()
    };
    assert!(
        out.iter()
            .rposition(|o| matches!(o, Out::Effect(_, Effect::Program(_))))
            .unwrap()
            < at(|e| matches!(e, Effect::Exit { .. }))
    );

    let mut b = LogBuilder::new(Size::new(SIZE.0, SIZE.1));
    b.data(osc("state=working")).data("\x1b]133;A\x07$ ");
    let (_, out) = run(&b.build().entries);
    let last = programs(&out).pop().unwrap();
    assert!(last.records.is_empty() && !last.native);
}

#[test]
fn osc_progress_shows_on_the_root_until_the_program_reports() {
    let mut b = LogBuilder::new(Size::new(SIZE.0, SIZE.1));
    b.data("\x1b]9;4;1;25\x07")
        .data(osc("state=working:progress=70"))
        .data("\x1b]9;4;1;90\x07");
    let (_, out) = run(&b.build().entries);
    let seen = programs(&out);
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert!(!seen[0].native && seen[0].records[0].from_progress);
    assert_eq!(seen[0].records[0].progress, Some(25));
    assert_eq!(seen[1].records[0].progress, Some(70));
}

#[test]
fn the_query_is_answered_once_live_and_never_in_replay() {
    let mut b = LogBuilder::new(Size::new(SIZE.0, SIZE.1));
    b.data("$ ").data("\x1b]7501;?\x1b\\");
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
    let mut before = Vec::new();
    d.serve(&mut s, first, &mut before, Some(entries.len() as u64));
    let writes: Vec<_> = before
        .iter()
        .filter_map(|o| match o {
            Out::Write(w) => Some(String::from_utf8_lossy(w).into_owned()),
            _ => None,
        })
        .collect();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert!(writes[0].starts_with("\x1b]7501;?"), "{writes:?}");

    let (_, after) = {
        let open = d.open();
        d.recover(Arc::clone(&cfg), open)
    };
    assert_eq!(writes_in_replay(&after), 0);
    assert!(after.iter().all(|o| !matches!(o, Out::Write(_))));
}

#[test]
fn size_reports_follow_the_cards_pixels_across_resizes() {
    let mut b = LogBuilder::new(Size::new(SIZE.0, SIZE.1));
    let queries = "\x1b[14t\x1b[16t\x1b[18t";
    // 10 by 20 pixel cells, then the same font at a scale of 1.5 on a larger card.
    for (cols, rows, px_w, px_h) in [(40, 8, 400, 160), (60, 12, 900, 360)] {
        b.push(Record::Resize {
            cols,
            rows,
            px_w,
            px_h,
            req: None,
        });
        b.data(queries);
    }
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
    let mut out = Vec::new();
    d.serve(&mut s, first, &mut out, Some(entries.len() as u64));
    let writes: Vec<_> = out
        .iter()
        .filter_map(|o| match o {
            Out::Write(w) => Some(String::from_utf8_lossy(w).into_owned()),
            _ => None,
        })
        .collect();
    assert_eq!(
        writes.concat(),
        "\x1b[4;160;400t\x1b[6;20;10t\x1b[8;8;40t\x1b[4;360;900t\x1b[6;30;15t\x1b[8;12;60t"
    );
}

#[test]
fn records_survive_a_checkpoint() {
    let mut b = LogBuilder::new(Size::new(SIZE.0, SIZE.1));
    scripted(&mut b);
    let n = b.len();
    b.data(osc("state=clear:id=build"));
    let all = b.build().entries;
    let (entries, rest) = all.split_at(n);
    let mut s = Session::fresh("s", config(0), SIZE, Cursor::start(0)).unwrap();
    let mut out = Vec::new();
    s.apply_all(entries, Instant::now(), &mut out);
    let cp = out
        .iter()
        .rev()
        .find_map(|o| match o {
            Out::Checkpoint(cp) => Some(cp.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(cp.format, FORMAT);
    let want = programs(&out).pop().unwrap();

    // Taken on again from that checkpoint, a clear reports what is left.
    let end = entries.last().unwrap().after();
    let open = Open {
        epoch: 0,
        oldest: Cursor::start(0),
        head: end,
        sent: end,
        newest_cp: Some(end),
        size: SIZE,
        spawn_size: None,
        pty: true,
        exited: None,
    };
    let mut out = Vec::new();
    let mut s = Session::open("s", config(1 << 20), open, Instant::now(), &mut out);
    s.input(Input::Checkpoint(cp), Instant::now(), &mut out);
    let mut live = Vec::new();
    s.input(Input::Entries(rest.to_vec()), Instant::now(), &mut live);
    let got = programs(&live).pop().expect("records changed");
    assert_eq!(got.records, want.records[..1]);
}
