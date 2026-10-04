//! Recovery from checkpoints.
//!
//! The reference engine restores from its newest checkpoint, which is only
//! as exact as the formatter's round trip (`Screen::serialize`, then feeding
//! that into a fresh terminal): its differential runs over
//! [`Profile::round_trip`], the generator features that survive, and the
//! seeds at the end pin what does not.
//!
//! The session engine's checkpoints ([`Emulator::checkpoint`]) restore the
//! state the formatter loses. The named cases below are each such state,
//! round-tripped through them and compared by every check.

use vorn_recovery::gen::{Generator, Mix, Profile};
use vorn_recovery::{
    compare, differential, transcript, Check, Error, InProcess, KillPlan, Mismatch,
    ReferenceConfig, ReferenceEngine, Restore, TermState,
};
use vorn_screen::{Checkpoint, Emulator};

fn engine(config: ReferenceConfig) -> impl FnMut() -> Result<InProcess<ReferenceEngine>, Error> {
    move || Ok(InProcess::new(config, Restore::Checkpoint))
}

/// Checkpoints every KiB, no history: the server's model keeps none, and
/// with history the round trip loses more (see the cases below).
const OFTEN: ReferenceConfig = ReferenceConfig {
    scrollback: 0,
    checkpoint_every: 1024,
};

#[test]
fn differential_from_checkpoints() {
    for seed in 1..=40 {
        let log = Generator::log(seed, Profile::round_trip().bytes(128 << 10));
        let report = differential(&log, &KillPlan::random(seed, 6), engine(OFTEN))
            .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        // Recovery came from checkpoints, not from the session start.
        assert!(
            report.resumed.iter().any(|c| c.next_rseq > 0),
            "seed {seed}"
        );
    }
}

/// The transcripts recover exactly except for one thing: a checkpoint cut
/// after a program left the alternate screen (htop, vim) does not carry
/// what it left there, so the inactive screen differs
/// (the formatter writes the active screen only).
#[test]
fn differential_from_checkpoints_over_transcripts() {
    for (name, log) in transcript::all().unwrap() {
        let config = ReferenceConfig {
            checkpoint_every: 256,
            ..OFTEN
        };
        let n = log.entries.len() as u64;
        match differential(&log, &KillPlan::at((1..n).step_by(3)), engine(config)) {
            Ok(_) => {}
            Err(Error::Mismatch(m)) if m.checks() == [Check::SavedScreen] => {
                assert!(m.to_string().contains("inactive screen"), "{name}: {m}");
            }
            Err(e) => panic!("{name}: {e}"),
        }
    }
}

/// The terminal fed `before`, checkpointed the way the session engine does
/// it ([`Emulator::checkpoint`]) and rebuilt, then both fed `after`: what a
/// recovery from a checkpoint cut after `before` shows. The terminal that
/// was checkpointed is compared as it was, not as it carries on from its
/// rebuild, so nothing the rebuild drops can hide.
fn round_trip(
    cols: u32,
    rows: u32,
    scrollback: usize,
    before: &[u8],
    after: impl Fn(&mut Emulator),
) -> Result<(), Mismatch> {
    let mut live = Emulator::with_scrollback(cols, rows, scrollback).unwrap();
    live.feed(before, &mut Vec::new());
    let (cp, _) = live
        .cut()
        .unwrap_or_else(|why| panic!("no checkpoint: {why}"));
    let mut back = Emulator::restore(&Checkpoint::decode(&cp.encode()).unwrap()).unwrap();
    after(&mut live);
    after(&mut back);
    compare(
        &TermState::capture(live).unwrap(),
        &TermState::capture(back).unwrap(),
    )
}

fn feed(bytes: &'static [u8]) -> impl Fn(&mut Emulator) {
    move |e| e.feed(bytes, &mut Vec::new())
}

fn nothing(_: &mut Emulator) {}

/// The cursor at the last column waiting to wrap: the next character wraps.
#[test]
fn pending_wrap_is_restored() {
    round_trip(10, 3, 0, b"0123456789", nothing).unwrap();
    round_trip(10, 3, 0, b"0123456789", feed(b"x")).unwrap();
}

#[test]
fn saved_cursor_is_restored() {
    round_trip(20, 5, 0, b"\x1b[3;3H\x1b7\x1b[H", nothing).unwrap();
    round_trip(
        20,
        5,
        0,
        b"\x1b[3;3H\x1b[1;31m\x1b7\x1b[H\x1b[0m",
        feed(b"\x1b8x"),
    )
    .unwrap();
}

/// The screen that is not showing, and its saved cursor.
#[test]
fn alternate_screen_keeps_the_primary() {
    round_trip(20, 5, 0, b"primary\x1b[?1049halt", nothing).unwrap();
    round_trip(20, 5, 0, b"primary\x1b[?1049halt", feed(b"\x1b[?1049lmore")).unwrap();
}

#[test]
fn kitty_keyboard_flags_are_restored() {
    round_trip(20, 5, 0, b"\x1b[>1u", nothing).unwrap();
    round_trip(20, 5, 0, b"\x1b[>1u\x1b[>5u\x1b[?1049h\x1b[>3u", nothing).unwrap();
}

#[test]
fn cursor_shape_is_restored() {
    round_trip(20, 5, 0, b"\x1b[4 q", nothing).unwrap();
}

/// DECSCA, which cut-off sequences produce now and then.
#[test]
fn character_protection_is_restored() {
    round_trip(20, 5, 0, b"\x1b[1\"q", nothing).unwrap();
    round_trip(
        20,
        5,
        0,
        b"\x1b[1\"qab\x1b[0\"q",
        feed(b"\x1b[1;1H\x1b[?2K"),
    )
    .unwrap();
}

#[test]
fn left_right_margins_keep_the_cursor() {
    round_trip(40, 8, 0, b"\x1b[?69h\x1b[3;30s\x1b[5;10H", nothing).unwrap();
}

#[test]
fn origin_mode_keeps_the_cursor() {
    round_trip(30, 6, 0, b"\x1b[?6h\x1b[2;5r\x1b[2;3Hx", nothing).unwrap();
}

/// Rows erased with a background colour and left blank at the bottom.
#[test]
fn background_rows_at_the_bottom_are_restored() {
    round_trip(20, 5, 0, b"ab\x1b[44m\x1b[J\x1b[0m", nothing).unwrap();
    round_trip(20, 5, 0, b"ab\x1b[44m\x1b[J\x1b[0m", feed(b"\x1b[3;1Hx")).unwrap();
}

/// A soft wrap stays a soft wrap, so a resize reflows the line the same.
#[test]
fn soft_wraps_stay_soft() {
    let resize = |e: &mut Emulator| e.resize(20, 4, &mut Vec::new()).unwrap();
    round_trip(10, 4, 0, b"abcdefghijKLM", resize).unwrap();
}

/// History above a screen that ends in blank rows stays where it was.
#[test]
fn history_stays_put_when_the_screen_ends_in_blank_rows() {
    round_trip(
        10,
        4,
        64 << 10,
        b"0\r\n1\r\n2\r\n3\r\n4\r\n5\r\n\x1b[H",
        nothing,
    )
    .unwrap();
}

/// A combining mark stays on its base character.
#[test]
fn combining_marks_stay_on_their_cell() {
    round_trip(10, 3, 0, b"oke\xcc\x81", nothing).unwrap();
    round_trip(10, 3, 0, b"oke\xcc\x81", feed(b"\x1b[1;2H\x1b[P")).unwrap();
}

/// Blank cells a redraw left styled.
#[test]
fn styled_blanks_are_restored() {
    round_trip(
        10,
        3,
        0,
        b"\x1b[44mab\x1b[2X\x1b[0m",
        feed(b"\x1b[1;1H\x1b[4@"),
    )
    .unwrap();
}

/// A combining mark that comes back attached to the cell before its base
/// character ("ok\u{301}e" for "oke\u{301}"). Found by the seed below with
/// RIS and prompts left out of the mix (a reset hides it); not reduced to a
/// smaller input yet.
#[test]
fn combining_marks_can_move_a_cell() {
    let mix = Mix {
        reset: 0,
        prompt: 0,
        ..Mix::ROUND_TRIP
    };
    let seed = 184;
    let log = Generator::log(seed, Profile::round_trip().mix(mix).bytes(128 << 10));
    let err = differential(&log, &KillPlan::random(seed, 6), engine(OFTEN))
        .expect_err("restores exactly now");
    let m = err.mismatch().unwrap_or_else(|| panic!("{err}"));
    assert_eq!(m.checks(), [Check::Screen], "seed {seed}: {m}");
    assert!(m.to_string().contains("\\u{301}e"), "seed {seed}: {m}");
}

/// Full-screen redraws (CUP, ICH, DCH, ECH, EL) leave blank cells whose
/// style the formatter does not write; they show once text moves into
/// them. Seeds that show it, pinned.
#[test]
fn redraws_leave_styled_blanks() {
    let mix = Mix {
        redraw: 15,
        ..Mix::ROUND_TRIP
    };
    for seed in [210, 245] {
        let log = Generator::log(seed, Profile::round_trip().mix(mix).bytes(128 << 10));
        let err = differential(&log, &KillPlan::random(seed, 6), engine(OFTEN))
            .expect_err("restores exactly now");
        let m = err.mismatch().unwrap_or_else(|| panic!("{err}"));
        assert_eq!(m.checks(), [Check::Screen], "seed {seed}: {m}");
    }
}
