//! The seeded generator: deterministic per seed, sized as asked, and cutting
//! records inside UTF-8 characters and escape sequences, where the scanner
//! (and Ghostty itself) say a checkpoint may not go.

use vorn_recovery::gen::{Generator, Mix, Profile};
use vorn_recovery::{Log, Scanner, Size};
use vorn_screen::Screen;
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

#[test]
fn records_split_utf8_and_sequences() {
    let log = Generator::log(5, Profile::mixed().bytes(512 << 10));
    let mut s = Scanner::new();
    let (mut in_utf8, mut in_seq, mut safe) = (0, 0, 0);
    for bytes in data(&log) {
        s.feed(bytes);
        if s.utf8_open() {
            in_utf8 += 1;
        } else if !s.is_safe() {
            in_seq += 1;
        } else {
            safe += 1;
        }
    }
    assert!(in_utf8 > 50, "{in_utf8} boundaries inside a character");
    assert!(in_seq > 100, "{in_seq} boundaries inside a sequence");
    assert!(safe > in_seq, "{safe} safe boundaries");
}

/// The scanner against Ghostty: at each record boundary, a fresh terminal
/// fed everything so far and then `Z` moves its cursor one cell right only
/// if its parser was in ground with no UTF-8 open. In a CSI `Z` is a final
/// byte (CBT, backwards), in an escape it dispatches, in a string it is
/// payload, and after an open UTF-8 lead it prints a replacement first.
#[test]
fn the_scanner_agrees_with_ghostty() {
    let mix = Mix {
        // Margins make "one cell right" depend on more than the cursor.
        margins: 0,
        ..Mix::EVERYTHING
    };
    let profile = Profile {
        max_record: 24,
        ..Profile::mixed().mix(mix).bytes(12 << 10)
    };
    let (mut agreed_safe, mut agreed_unsafe) = (0, 0);
    for seed in 1..=4 {
        let log = Generator::log(seed, profile);
        let mut scanner = Scanner::new();
        let mut fed: Vec<u8> = Vec::new();
        let mut size = log.size;
        let mut resizes: Vec<(usize, Size)> = Vec::new();
        for e in &log.entries {
            match &e.rec {
                Record::Data { bytes, .. } => {
                    scanner.feed(bytes);
                    fed.extend_from_slice(bytes);
                }
                &Record::Resize { cols, rows, .. } => {
                    size = Size::new(cols, rows);
                    resizes.push((fed.len(), size));
                    continue;
                }
                _ => continue,
            }
            let mut t = Screen::new(log.size.cols.into(), log.size.rows.into()).unwrap();
            let mut at = 0;
            for &(offset, sz) in &resizes {
                t.feed(&fed[at..offset]);
                t.resize(sz.cols.into(), sz.rows.into()).unwrap();
                at = offset;
            }
            t.feed(&fed[at..]);
            let term = t.terminal();
            let (x, y) = (term.cursor_x().unwrap(), term.cursor_y().unwrap());
            if term.is_cursor_pending_wrap().unwrap() || x + 2 >= size.cols {
                continue;
            }
            t.feed(b"Z");
            let term = t.terminal();
            let moved_one = term.cursor_x().unwrap() == x + 1 && term.cursor_y().unwrap() == y;
            assert_eq!(
                scanner.is_safe(),
                moved_one,
                "seed {seed}, rseq {}: scanner says {:?} (utf8 open: {}), after {:?}",
                e.hdr.rseq,
                scanner.state(),
                scanner.utf8_open(),
                String::from_utf8_lossy(&fed[fed.len().saturating_sub(40)..])
            );
            if moved_one {
                agreed_safe += 1;
            } else {
                agreed_unsafe += 1;
            }
        }
    }
    assert!(
        agreed_safe > 100 && agreed_unsafe > 50,
        "{agreed_safe} safe, {agreed_unsafe} unsafe"
    );
}
