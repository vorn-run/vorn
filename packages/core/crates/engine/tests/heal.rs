//! After a settled resize a program that does not start its screen over
//! (clear it, or switch screens) within `HEAL_AFTER` is asked to redraw:
//! one row shorter, then back. Full-screen and inline programs are asked; a
//! shell at its prompt, which redraws itself, is not, and neither is a
//! program that repainted, nor the nudge's own resizes.

mod gridrig;

use std::time::{Duration, Instant};

use gridrig::config;
use vorn_engine::{Out, Session, HEAL_AFTER};
use vorn_recovery::{LogBuilder, Size};
use vorn_term_proto::{Cursor, Entry};

/// A live session at 80x24 and the records it is fed, one batch at a time.
struct Live {
    s: Session,
    log: LogBuilder,
    fed: usize,
    now: Instant,
}

impl Live {
    fn new() -> Live {
        Live {
            s: Session::fresh("s", config(1 << 20), (80, 24), Cursor::start(0)).unwrap(),
            log: LogBuilder::new(Size::new(80, 24)),
            fed: 0,
            now: Instant::now(),
        }
    }

    /// Applies what was added to the log since the last call.
    fn feed(&mut self) -> Vec<Out> {
        let entries: Vec<Entry> = self.log.clone().build().entries[self.fed..].to_vec();
        self.fed += entries.len();
        let mut out = Vec::new();
        self.s.apply_all(&entries, self.now, &mut out);
        out
    }

    fn output(&mut self, bytes: &[u8]) -> Vec<Out> {
        self.log.data(bytes.to_vec());
        self.feed()
    }

    fn resize(&mut self, cols: u16, rows: u16) -> Vec<Out> {
        self.log.resize(Size::new(cols, rows));
        self.feed()
    }

    /// Moves the clock by `d`, running what fell due.
    fn wait(&mut self, d: Duration) -> Vec<Out> {
        self.now += d;
        let mut out = Vec::new();
        self.s.frame(self.now, &mut out);
        out
    }
}

fn nudges(out: &[Out]) -> Vec<(u16, u16)> {
    out.iter()
        .filter_map(|o| match o {
            Out::Nudge { cols, rows } => Some((*cols, *rows)),
            _ => None,
        })
        .collect()
}

/// An alternate-screen program that draws with cursor moves and never clears.
const FULL_SCREEN: &[u8] = b"\x1b[?1049h\x1b[2J\x1b[H\x1b[3;5Hstatus\x1b[24;1H> input";

#[test]
fn a_full_screen_program_that_does_not_repaint_is_asked_to_once() {
    let mut l = Live::new();
    l.output(FULL_SCREEN);
    assert!(nudges(&l.resize(100, 30)).is_empty(), "not at once");
    let due = l.s.due().expect("the heal is due");
    assert!(due <= l.now + HEAL_AFTER, "{due:?}");
    assert!(nudges(&l.wait(HEAL_AFTER / 2)).is_empty(), "nor early");
    // Only cursor moves since: no repaint.
    l.output(b"\x1b[3;5Hstatus 2");
    assert_eq!(nudges(&l.wait(HEAL_AFTER)), [(100, 30)]);
    // The nudge's own resizes ask for nothing more.
    l.resize(100, 29);
    l.resize(100, 30);
    assert!(nudges(&l.wait(HEAL_AFTER * 4)).is_empty());
    // A later settled resize is watched again.
    l.resize(90, 30);
    assert_eq!(nudges(&l.wait(HEAL_AFTER)), [(90, 30)]);
}

#[test]
fn a_program_that_repaints_by_itself_is_left_alone() {
    let mut l = Live::new();
    l.output(FULL_SCREEN);
    l.resize(100, 30);
    // The program starts over at the new size, inside a synchronized update.
    l.output(b"\x1b[?2026h\x1b[2J\x1b[H\x1b[3;5Hstatus\x1b[?2026l");
    assert!(nudges(&l.wait(HEAL_AFTER)).is_empty());
}

#[test]
fn a_shell_at_its_prompt_is_left_alone_and_an_inline_program_is_not() {
    let mut l = Live::new();
    l.output(b"\x1b]133;A\x07$ ");
    l.resize(100, 30);
    assert!(
        nudges(&l.wait(HEAL_AFTER)).is_empty(),
        "a prompt redraws itself"
    );
    // A command runs and draws inline on the primary screen.
    l.output(b"top\r\n\x1b]133;C\x07\x1b[5;1Hprocesses 120\x1b[6;1Hload 1.2");
    l.resize(90, 30);
    assert_eq!(nudges(&l.wait(HEAL_AFTER)), [(90, 30)]);
}
