//! The seeded generator: deterministic per seed, sized as asked, and cutting
//! records inside UTF-8 characters and escape sequences.

use libghostty_vt::terminal::Terminal;
use vorn_recovery::gen::{Generator, Profile};
use vorn_recovery::Log;
use vorn_term_proto::Record;

fn data(log: &Log) -> impl Iterator<Item = &[u8]> {
    log.entries.iter().filter_map(|e| match &e.rec {
        Record::Data { bytes, .. } => Some(bytes.as_slice()),
        _ => None,
    })
}

#[test]
fn the_same_seed_gives_the_same_log() {
    let p = Profile::mixed().bytes(128 << 10);
    let a = Generator::log(42, p);
    assert_eq!(a, Generator::log(42, p));
    assert_ne!(a.digest(), Generator::log(43, p).digest());
    a.validate().unwrap();
}

#[test]
fn sessions_do_not_depend_on_how_many_there_are() {
    let p = Profile::shell().bytes(8 << 10);
    let three = Generator::sessions(7, 3, p);
    let five = Generator::sessions(7, 5, p);
    assert_eq!(three[..], five[..3]);
    assert_ne!(three[0], three[1]);
}

#[test]
fn hits_the_requested_size() {
    for target in [1u64, 4 << 10, 1 << 20, 3 << 20] {
        let log = Generator::log(1, Profile::mixed().bytes(target));
        let len = log.data_len();
        // It stops at the first piece that reaches the target; no piece is
        // longer than a few KiB.
        assert!(
            len >= target && len < target + (16 << 10),
            "{target}: {len}"
        );
    }
}

#[test]
fn a_resize_storm_has_its_resizes() {
    let log = Generator::log(9, Profile::resize_storm(1000));
    let resizes = log
        .entries
        .iter()
        .filter(|e| matches!(e.rec, Record::Resize { .. }))
        .count();
    assert_eq!(resizes, 1000);
    // Interleaved with output, not bunched at one end.
    let first = log
        .entries
        .iter()
        .position(|e| matches!(e.rec, Record::Resize { .. }))
        .unwrap();
    assert!(first < log.entries.len() / 10);
    assert!(log.data_len() > 10_000);
}

/// Ghostty's continuation says where each record leaves it: in a character, in a sequence, or at ground.
#[test]
fn records_split_utf8_and_sequences() {
    let log = Generator::log(5, Profile::mixed().bytes(512 << 10));
    let mut t = Terminal::new(log.size.cols, log.size.rows).unwrap();
    t.set_continuation_max_bytes(1 << 20).unwrap();
    let (mut in_utf8, mut in_seq, mut ground) = (0, 0, 0);
    for bytes in data(&log) {
        t.vt_write(bytes);
        match t
            .continuation_alloc(None)
            .unwrap()
            .as_deref()
            .unwrap_or_default()
        {
            [] => ground += 1,
            [0x1b, ..] => in_seq += 1,
            _ => in_utf8 += 1,
        }
    }
    assert!(in_utf8 > 50, "{in_utf8} boundaries inside a character");
    assert!(in_seq > 100, "{in_seq} boundaries inside a sequence");
    assert!(ground > in_seq, "{ground} boundaries at ground");
}
