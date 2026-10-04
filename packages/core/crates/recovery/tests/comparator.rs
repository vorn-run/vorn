//! The comparator: equal for terminals fed the same output, and each check
//! made to fail once, alone, by a terminal broken in just that way.

use vorn_recovery::gen::{Generator, Profile};
use vorn_recovery::{compare, transcript, Check, Log, Mismatch, TermState};
use vorn_screen::Screen;
use vorn_term_proto::Record;

const SCROLLBACK: usize = 64 << 10;

fn fed(log: &Log, whole: bool) -> TermState {
    let mut s =
        Screen::with_scrollback(log.size.cols.into(), log.size.rows.into(), SCROLLBACK).unwrap();
    let mut pending = Vec::new();
    for e in &log.entries {
        match &e.rec {
            Record::Data { bytes, .. } if whole => pending.extend_from_slice(bytes),
            Record::Data { bytes, .. } => {
                s.feed(bytes);
            }
            &Record::Resize { cols, rows, .. } => {
                s.feed(&pending);
                pending.clear();
                s.resize(cols.into(), rows.into()).unwrap();
            }
            _ => {}
        }
    }
    s.feed(&pending);
    TermState::capture(s).unwrap()
}

/// Two 40x10 terminals fed `common`, then each its own bytes.
fn pair(common: &[u8], left: &[u8], right: &[u8]) -> Result<(), Mismatch> {
    let side = |own: &[u8]| {
        let mut s = Screen::with_scrollback(40, 10, SCROLLBACK).unwrap();
        s.feed(common);
        s.feed(own);
        TermState::capture(s).unwrap()
    };
    compare(&side(left), &side(right))
}

fn only(check: Check, result: Result<(), Mismatch>) {
    let m = result.expect_err("the broken terminal compared equal");
    assert_eq!(m.checks(), [check], "{m}");
    // The diff names what differs, for whoever reads a failing test.
    assert!(!m.diffs[0].detail.is_empty());
}

const BASE: &[u8] = b"$ ls\r\nfile one\r\n\x1b[1;32mgreen\x1b[0m two\r\n$ ";

#[test]
fn equal_for_the_same_log() {
    for seed in 1..=8 {
        let log = Generator::log(seed, Profile::mixed().bytes(96 << 10));
        compare(&fed(&log, false), &fed(&log, false)).unwrap();
    }
    for (name, log) in transcript::all().unwrap() {
        compare(&fed(&log, false), &fed(&log, false)).unwrap_or_else(|m| panic!("{name}: {m}"));
    }
}

/// How output was cut into records does not matter to the terminal: the
/// same bytes in one feed per resize compare equal to one per record.
#[test]
fn equal_however_the_output_was_cut() {
    for seed in 1..=8 {
        let log = Generator::log(seed, Profile::mixed().bytes(96 << 10));
        compare(&fed(&log, false), &fed(&log, true)).unwrap_or_else(|m| panic!("seed {seed}: {m}"));
    }
}

#[test]
fn dimensions_check() {
    let side = |resize: bool| {
        let mut s = Screen::with_scrollback(40, 10, SCROLLBACK).unwrap();
        s.feed(BASE);
        if resize {
            // One more column: nothing reflows, the cursor stays.
            s.resize(41, 10).unwrap();
        }
        TermState::capture(s).unwrap()
    };
    only(Check::Dimensions, compare(&side(false), &side(true)));
}

#[test]
fn screen_check() {
    only(Check::Screen, pair(BASE, b"hello", b"hellp"));
}

#[test]
fn screen_check_sees_scrollback() {
    // The same visible rows; one side has one line more of history.
    let lines = |from: u32| -> Vec<u8> {
        (from..12)
            .flat_map(|i| format!("{i}\r\n").into_bytes())
            .collect()
    };
    only(Check::Screen, pair(b"", &lines(0), &lines(1)));
}

#[test]
fn cursor_check() {
    only(Check::Cursor, pair(BASE, b"", b"\x1b[C"));
}

#[test]
fn cursor_check_sees_the_pen() {
    // A lost SGR changes nothing on screen yet, only what prints next.
    only(
        Check::Cursor,
        pair(BASE, b"\x1b[1;4;38;5;208m", b"\x1b[1;4m"),
    );
}

#[test]
fn modes_check() {
    only(
        Check::Modes,
        pair(BASE, b"\x1b[?2004h\x1b[?1000h", b"\x1b[?1000h"),
    );
}

#[test]
fn kitty_keyboard_check() {
    only(Check::KittyKeyboard, pair(BASE, b"\x1b[>1u", b""));
}

#[test]
fn kitty_keyboard_check_sees_below_the_top() {
    // The same flags on top; a lost push underneath.
    only(
        Check::KittyKeyboard,
        pair(BASE, b"\x1b[>5u\x1b[>1u", b"\x1b[>1u"),
    );
}

#[test]
fn active_screen_check() {
    // Mode 47 is set on both; leaving through 1047 switches back without
    // touching it, so only the active screen differs.
    only(
        Check::ActiveScreen,
        pair(b"", b"\x1b[?47h", b"\x1b[?47h\x1b[?1047l"),
    );
}

#[test]
fn saved_screen_check() {
    // The alternate screen lost what was drawn on it.
    only(
        Check::SavedScreen,
        pair(BASE, b"\x1b[?47hx\x08\x1b[?47l", b"\x1b[?47h\x1b[?47l"),
    );
}

#[test]
fn saved_screen_check_sees_the_saved_cursor() {
    only(
        Check::SavedScreen,
        pair(BASE, b"\x1b[3;3H\x1b7\x1b[H", b"\x1b[H"),
    );
}

#[test]
fn scrolling_region_check() {
    // DECSTBM homes the cursor, so both sides end at home.
    only(Check::ScrollingRegion, pair(BASE, b"\x1b[2;5r", b"\x1b[H"));
}

#[test]
fn title_check() {
    only(Check::Title, pair(BASE, b"\x1b]2;vim\x07", b""));
}

#[test]
fn cwd_check() {
    only(
        Check::Cwd,
        pair(
            BASE,
            b"\x1b]7;file://box/srv\x07",
            b"\x1b]7;file://box/tmp\x07",
        ),
    );
}

#[test]
fn every_check_has_a_test() {
    // A reminder that lives next to the tests above: a new Check needs one.
    assert_eq!(Check::ALL.len(), 10);
}
