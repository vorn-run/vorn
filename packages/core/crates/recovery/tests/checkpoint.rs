//! Recovery from checkpoints: the differential with the reference engine
//! restoring from its newest checkpoint, which is only as exact as the
//! formatter's round trip (`Screen::serialize`, then feeding that into a
//! fresh terminal).
//!
//! The differential runs over [`Profile::round_trip`], the generator
//! features that survive. Everything found not to survive is a named case
//! below, each pinned to the check it fails, so the day one restores
//! exactly its test says so and the profile can grow.

use vorn_recovery::gen::{Generator, Mix, Profile};
use vorn_recovery::{
    compare, differential, transcript, Check, Error, InProcess, KillPlan, Mismatch,
    ReferenceConfig, ReferenceEngine, Restore, TermState,
};
use vorn_screen::Screen;

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
/// (`alternate_screen_loses_the_primary` from the other side).
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

/// The terminal fed `before`, checkpointed and restored, then both fed
/// `after`: what a recovery from a checkpoint cut after `before` shows.
fn round_trip(
    cols: u32,
    rows: u32,
    scrollback: usize,
    before: &[u8],
    after: impl Fn(&mut Screen),
) -> Result<(), Mismatch> {
    let mut live = Screen::with_scrollback(cols, rows, scrollback).unwrap();
    live.feed(before);
    let snap = live.serialize().unwrap();
    let mut back = Screen::with_scrollback(snap.cols, snap.rows, scrollback).unwrap();
    back.feed(snap.screen.as_bytes());
    back.restore_labels(Some(&snap.title), Some(&snap.cwd));
    after(&mut live);
    after(&mut back);
    compare(
        &TermState::capture(live).unwrap(),
        &TermState::capture(back).unwrap(),
    )
}

fn lost(check: Check, result: Result<(), Mismatch>) {
    let m = result.expect_err("restores exactly now: move it out of the exceptions");
    assert!(m.checks().contains(&check), "{m}");
}

fn feed(bytes: &'static [u8]) -> impl Fn(&mut Screen) {
    move |s| {
        s.feed(bytes);
    }
}

fn nothing(_: &mut Screen) {}

/// The cursor at the last column waiting to wrap comes back as a plain CUP
/// there, so the next character overwrites instead of wrapping. The
/// reference engine does not cut a checkpoint in this state.
#[test]
fn pending_wrap_is_lost() {
    lost(Check::Cursor, round_trip(10, 3, 0, b"0123456789", nothing));
    lost(
        Check::Screen,
        round_trip(10, 3, 0, b"0123456789", feed(b"x")),
    );
}

#[test]
fn saved_cursor_is_lost() {
    lost(
        Check::SavedScreen,
        round_trip(20, 5, 0, b"\x1b[3;3H\x1b7\x1b[H", nothing),
    );
}

/// The formatter writes the active screen only.
#[test]
fn alternate_screen_loses_the_primary() {
    lost(
        Check::SavedScreen,
        round_trip(20, 5, 0, b"primary\x1b[?1049halt", nothing),
    );
}

#[test]
fn kitty_keyboard_flags_are_lost() {
    lost(
        Check::KittyKeyboard,
        round_trip(20, 5, 0, b"\x1b[>1u", nothing),
    );
}

#[test]
fn cursor_shape_is_lost() {
    lost(Check::Cursor, round_trip(20, 5, 0, b"\x1b[4 q", nothing));
}

/// DECSCA, which cut-off sequences produce now and then.
#[test]
fn character_protection_is_lost() {
    lost(Check::Cursor, round_trip(20, 5, 0, b"\x1b[1\"q", nothing));
}

/// DECSLRM homes the cursor, and the formatter writes it after the CUP:
/// the same order `Screen::serialize` already fixes for DECSTBM.
#[test]
fn left_right_margins_home_the_cursor() {
    lost(
        Check::Cursor,
        round_trip(40, 8, 0, b"\x1b[?69h\x1b[3;30s\x1b[5;10H", nothing),
    );
}

#[test]
fn origin_mode_moves_the_cursor() {
    lost(
        Check::Cursor,
        round_trip(30, 6, 0, b"\x1b[?6h\x1b[2;5r\x1b[2;3Hx", nothing),
    );
}

/// Rows erased with a background colour and left blank at the bottom of the
/// screen are trimmed as blank rows. Invisible to the comparator at the cut
/// (it uses the same formatter), visible once something prints there.
#[test]
fn background_rows_at_the_bottom_are_lost() {
    round_trip(20, 5, 0, b"ab\x1b[44m\x1b[J\x1b[0m", nothing).unwrap();
    lost(
        Check::Screen,
        round_trip(20, 5, 0, b"ab\x1b[44m\x1b[J\x1b[0m", feed(b"\x1b[3;1Hx")),
    );
}

/// A soft wrap comes back as a hard line break, so a resize reflows the
/// line differently.
#[test]
fn soft_wraps_come_back_as_hard_breaks() {
    let resize = |s: &mut Screen| s.resize(20, 4).unwrap();
    lost(
        Check::Screen,
        round_trip(10, 4, 0, b"abcdefghijKLM", resize),
    );
}

/// The formatter drops trailing blank rows; with history above, replaying
/// it into a fresh terminal pushes the screen up by that many rows.
#[test]
fn history_shifts_when_the_screen_ends_in_blank_rows() {
    lost(
        Check::Screen,
        round_trip(
            10,
            4,
            64 << 10,
            b"0\r\n1\r\n2\r\n3\r\n4\r\n5\r\n\x1b[H",
            nothing,
        ),
    );
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
