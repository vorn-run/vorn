//! RC-T4, seeded: random VT into a terminal, serialize it, feed that into a
//! fresh terminal, and serialize again. The two must be byte for byte the
//! same, which is the restore check the Session Recovery Contract (§4) runs
//! before it trusts a checkpoint.
//!
//! Every input that does not survive becomes a named fixture below, with the
//! feature it loses, so a Ghostty bump that fixes or breaks one shows up here.

use vorn_screen::Screen;

/// xorshift, so every run feeds the same bytes and a failure names its seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len() as u64) as usize]
    }
}

const WORDS: &[&str] = &[
    "ls",
    "-la",
    "error:",
    "✓ done",
    "日本語",
    "e\u{301}",
    "👩‍👩‍👧",
    "tab\there",
    "x",
    "  ",
];

/// One piece of output from the families the round trip has to carry:
/// text, styles, cursor movement, erasing, scroll regions, modes, links.
fn piece(rng: &mut Rng, cols: u64, rows: u64) -> String {
    match rng.below(14) {
        0..=3 => rng.pick(WORDS).to_owned(),
        4 => "\r\n".to_owned(),
        5 => {
            let sgr = [
                "1",
                "2",
                "3",
                "4",
                "4:3",
                "5",
                "7",
                "8",
                "9",
                "53",
                "0",
                "22",
                "31",
                "42",
                "91",
                "38;5;208",
                "48;5;17",
                "38;2;10;200;30",
                "48;2;1;2;3",
                "58;5;9",
            ];
            format!("\x1b[{}m", rng.pick(&sgr))
        }
        6 => format!("\x1b[{};{}H", 1 + rng.below(rows), 1 + rng.below(cols)),
        7 => rng
            .pick(&["\x1b[K", "\x1b[1K", "\x1b[2K", "\x1b[J", "\x1b[1J"])
            .to_owned(),
        8 => rng
            .pick(&["\x1b[2L", "\x1b[M", "\x1b[3@", "\x1b[2P", "\x1b[4X"])
            .to_owned(),
        9 => {
            let top = 1 + rng.below(rows / 2);
            format!("\x1b[{};{}r", top, top + rng.below(rows - top))
        }
        10 => rng
            .pick(&[
                "\x1b[?25l",
                "\x1b[?25h",
                "\x1b[?2004h",
                "\x1b[?1h",
                "\x1b[?7l",
                "\x1b[?7h",
                "\x1b[4h",
                "\x1b[4l",
            ])
            .to_owned(),
        11 => rng
            .pick(&[
                "\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\",
                "\x1b]8;id=a;file:///tmp\x07here\x1b]8;;\x07",
            ])
            .to_owned(),
        12 => rng
            .pick(&["\x1bD", "\x1bM", "\x1b7", "\x1b8", "\x1b[S", "\x1b[T"])
            .to_owned(),
        _ => rng
            .pick(&["\x1b(0lqk\x1b(B", "\x1b[3g", "\x1bH", "\x1b[s", "\x1b[u"])
            .to_owned(),
    }
}

fn round_trip(input: &str, cols: u32, rows: u32) -> (String, String) {
    let mut first = Screen::new(cols, rows).unwrap();
    first.feed(input.as_bytes());
    let once = first.serialize().unwrap().screen;
    let mut second = Screen::new(cols, rows).unwrap();
    second.feed(once.as_bytes());
    let twice = second.serialize().unwrap().screen;
    (once, twice)
}

#[test]
fn random_output_restores_exactly() {
    let mut failures = Vec::new();
    for seed in 1..=300u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let (cols, rows) = (20 + rng.below(100), 5 + rng.below(40));
        let input: String = (0..rng.below(200))
            .map(|_| piece(&mut rng, cols, rows))
            .collect();
        let (once, twice) = round_trip(&input, cols as u32, rows as u32);
        if once != twice {
            failures.push(seed);
        }
    }
    assert!(
        failures.is_empty(),
        "seeds that did not restore: {failures:?}"
    );
}

/// DECSTBM homes the cursor, so the formatter writes the region before the cursor.
#[test]
fn a_scrolling_region_keeps_the_cursor() {
    let (once, twice) = round_trip("\x1b[2;20r\x1b[7;9Hx", 40, 24);
    assert_eq!(once, twice);
    assert!(once.contains("\x1b[2;20r\x1b[7;10H"), "{once:?}");
}

/// Accepted differences: input whose restore is not exact, and why. Each is
/// pinned, so the day it restores exactly this test says so.
mod fixtures {
    use super::round_trip;

    /// The formatter formats the active screen only (RC §4): with the
    /// alternate screen up, the primary screen is not in the output, and a
    /// checkpoint carries it separately as `primary_scrollback`.
    /// Origin mode makes the formatter's absolute cursor position relative to
    /// the scrolling region on replay, so the cursor lands lower than it was.
    #[test]
    fn origin_mode_moves_the_cursor() {
        let (once, twice) = round_trip("\x1b[?6h\x1b[2;5r\x1b[2;3Hx", 30, 6);
        assert_ne!(once, twice);
    }

    #[test]
    fn alternate_screen_hides_the_primary() {
        let (once, _) = round_trip("primary text\x1b[?1049halt text", 40, 5);
        assert!(once.contains("alt text"));
        assert!(!once.contains("primary text"));
    }
}
