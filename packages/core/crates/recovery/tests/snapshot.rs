//! Ghostty's snapshots as checkpoints under the recovery harness: exact at every record, mid-sequence included.

use vorn_recovery::gen::{Generator, Profile};
use vorn_recovery::{
    compare, differential, transcript, Checkpoint, Engine, Error, InProcess, KillPlan, Log,
    LogBuilder, Restore, Size, TermState,
};
use vorn_screen::Emulator;
use vorn_term_proto::{Entry, Record};

const SCROLLBACK: usize = 64 << 10;

/// Every sequence family a checkpoint carries, title stack included, in one-byte records.
fn split_everywhere() -> Log {
    let text = concat!(
        "\x1b[22;0t\x1b]2;vim \u{2014} main.rs\x07\x1b[?1049h\x1b[22;2t",
        "\x1b7\x1b(0lqk\x1b(B\x1b[2;5r\x1b[?6h\x1b[3;4H\x1b8",
        "\x1b[>5u\x1b[=3;2u\x1b[1;2;3;38;2;10;20;30mstyled\x1b[m",
        "\x1b]8;id=x;https://example.com\x1b\\link\x1b]8;;\x1b\\",
        "\x1bP+q544e\x1b\\\x1b_Gignored\x1b\\",
        "e\u{301} \u{1f469}\u{200d}\u{1f467} \u{65e5}\u{672c}",
        "\x1bN\x1b*0q\x1b[?2004h\x1b]133;A\x07$ \x1b]133;B\x07",
        "\x1b[23;0t\x1b[?1049l\x1b[23;2t\x1b]7;file://h/tmp\x1b\\",
        "\x1b]5522;cwd;/srv\x07tail\r\n",
    );
    let mut b = LogBuilder::new(Size::new(30, 8));
    for byte in text.as_bytes() {
        b.data(vec![*byte]);
    }
    b.resize(Size::new(24, 6));
    b.data("after the resize\x1b[22;0t\x1b[2J");
    b.build()
}

/// Generator logs, the recorded programs (vim and htop push titles) and the log above.
fn corpora() -> Vec<(String, Log)> {
    let mut all = Vec::new();
    for seed in 1..=8 {
        let p = Profile::mixed().bytes(48 << 10);
        all.push((format!("mixed {seed}"), Generator::log(seed, p)));
        let p = Profile::round_trip().bytes(48 << 10);
        all.push((format!("round trip {seed}"), Generator::log(seed, p)));
    }
    for (name, log) in transcript::all().unwrap() {
        all.push((name.to_owned(), log));
    }
    all.push(("split everywhere".to_owned(), split_everywhere()));
    all
}

fn apply(em: &mut Emulator, e: &Entry) {
    match &e.rec {
        Record::Data { bytes, .. } => em.feed(bytes, &mut Vec::new()),
        &Record::Resize { cols, rows, .. } => em
            .resize(cols.into(), rows.into(), &mut Vec::new())
            .unwrap(),
        Record::Gap { .. } | Record::Exit { .. } => {}
    }
}

fn replay(log: &Log, upto: usize) -> Emulator {
    let size = log.size;
    let mut em = Emulator::with_scrollback(size.cols.into(), size.rows.into(), SCROLLBACK).unwrap();
    for e in &log.entries[..upto] {
        apply(&mut em, e);
    }
    em
}

fn continuation(em: &Emulator) -> Vec<u8> {
    em.terminal()
        .continuation_alloc(None)
        .unwrap()
        .map(|b| b.to_vec())
        .unwrap_or_default()
}

/// The one state a snapshot drops: a pending wrap off the last column (a right margin, a back tab).
fn pending_wrap_off_the_edge(em: &Emulator) -> bool {
    let t = em.terminal();
    t.is_cursor_pending_wrap().unwrap() && t.cursor_x().unwrap() + 1 != t.cols().unwrap()
}

/// At every record: cut, restore, and compare with a replay, unfinished sequence included.
#[test]
fn every_record_restores_exactly() {
    let (mut cut, mut declined) = (0, 0);
    for (name, log) in corpora() {
        let mut live = replay(&log, 0);
        for k in 0..log.entries.len() {
            apply(&mut live, &log.entries[k]);
            let rebuilt = match live.cut() {
                Ok((_, rebuilt)) => rebuilt,
                Err(why) => {
                    assert!(
                        why == "restore check" && pending_wrap_off_the_edge(&live),
                        "{name} record {k}: {why}"
                    );
                    declined += 1;
                    continue;
                }
            };
            cut += 1;
            let want = replay(&log, k + 1);
            assert_eq!(
                continuation(&rebuilt),
                continuation(&want),
                "{name} record {k}"
            );
            let (a, b) = (
                TermState::capture(want).unwrap(),
                TermState::capture(rebuilt).unwrap(),
            );
            compare(&a, &b).unwrap_or_else(|m| panic!("{name} record {k}: {m}"));
        }
    }
    eprintln!("cut {cut}, declined {declined}");
    assert!(declined * 100 <= cut, "cut {cut}, declined {declined}");
}

/// An engine that cuts a Ghostty snapshot after every record it can.
#[derive(Debug)]
struct EveryRecord(Emulator);

impl Engine for EveryRecord {
    type Config = ();

    fn start(_: &(), size: Size) -> Result<Self, Error> {
        let em = Emulator::with_scrollback(size.cols.into(), size.rows.into(), SCROLLBACK)?;
        Ok(Self(em))
    }

    fn restore(_: &(), cp: &Checkpoint) -> Result<Self, Error> {
        let bad = || Error::Engine("undecodable checkpoint".into());
        let saved = vorn_screen::Checkpoint::decode(&cp.blob).ok_or_else(bad)?;
        let em = Emulator::restore(&saved)?;
        if !saved.matches(&em) {
            return Err(Error::Engine("restore check".into()));
        }
        Ok(Self(em))
    }

    fn apply(&mut self, entry: &Entry) -> Result<Option<Checkpoint>, Error> {
        apply(&mut self.0, entry);
        Ok(self.0.checkpoint().ok().map(|cp| Checkpoint {
            resume: entry.after(),
            size: Size::new(self.0.cols(), self.0.rows()),
            blob: cp.encode(),
        }))
    }

    fn finish(self) -> Result<TermState, Error> {
        TermState::capture(self.0)
    }
}

/// Killed after every record and recovered from snapshots, a session ends where one never killed does.
#[test]
fn killed_at_every_record_recovers_exactly() {
    for (name, log) in corpora() {
        let every: Vec<u64> = log.entries.iter().map(|e| e.hdr.rseq).collect();
        let report = differential(&log, &KillPlan::at(every), || {
            Ok(InProcess::<EveryRecord>::new((), Restore::Checkpoint))
        })
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(report.digest, log.digest(), "{name}");
        let from_start = report.resumed.iter().filter(|c| c.next_rseq == 0).count();
        assert!(
            from_start * 100 <= report.resumed.len(),
            "{name}: {from_start} of {} recoveries from the start",
            report.resumed.len()
        );
    }
}
