//! Checkpoints carry on exactly: a terminal rebuilt from a checkpoint and the
//! terminal it was cut from, fed the same bytes and resizes afterwards, stay
//! the same in everything a fingerprint sees and ask the host for the same
//! effects. Seeded random streams over every family of sequence the
//! checkpoint has to rebuild (RC-T4, strong form: the next bytes are what
//! show a lost wrap flag, saved cursor or pending wrap).

use vorn_screen::{Checkpoint, Emulator};

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

#[derive(Debug, Clone)]
enum Op {
    Bytes(String),
    Resize(u16, u16),
}

const FAMILIES: u64 = 34;

fn piece(rng: &mut Rng, fam: u64, cols: u16, rows: u16) -> Op {
    let (c, r) = (u64::from(cols), u64::from(rows));
    let s: String = match fam {
        0 => rng
            .pick(&[
                "ls",
                "-la",
                "✓ done",
                "日本語",
                "e\u{301}",
                "x",
                "  ",
                "tab\there",
                "👍🏽",
                "a\u{fe0f}",
            ])
            .into(),
        1 => "\r\n".into(),
        2 => format!(
            "\x1b[{}m",
            rng.pick(&[
                "1",
                "2",
                "3",
                "4",
                "4:3",
                "7",
                "0",
                "31",
                "42",
                "38;5;208",
                "48;2;1;2;3",
                "58;5;9",
                "9",
                "53"
            ])
        ),
        3 => format!("\x1b[{};{}H", 1 + rng.below(r), 1 + rng.below(c)),
        4 => rng
            .pick(&[
                "\x1b[K", "\x1b[1K", "\x1b[2K", "\x1b[J", "\x1b[1J", "\x1b[2J",
            ])
            .into(),
        5 => rng
            .pick(&["\x1b[2L", "\x1b[M", "\x1b[3@", "\x1b[2P", "\x1b[4X"])
            .into(),
        6 => {
            let top = 1 + rng.below((r / 2).max(1));
            format!("\x1b[{};{}r", top, top + rng.below(r - top + 1))
        }
        7 => rng
            .pick(&[
                "\x1b[?25l",
                "\x1b[?25h",
                "\x1b[?2004h",
                "\x1b[?1h",
                "\x1b[?7l",
                "\x1b[?7h",
                "\x1b[4h",
                "\x1b[4l",
                "\x1b[20h",
                "\x1b[20l",
            ])
            .into(),
        8 => rng
            .pick(&[
                "\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\",
                "\x1b]8;id=a;file:///tmp\x07here\x1b]8;;\x07",
                "\x1b]8;;https://x\x1b\\",
            ])
            .into(),
        9 => rng
            .pick(&["\x1bD", "\x1bM", "\x1b[S", "\x1b[T", "\x1bE"])
            .into(),
        10 => rng.pick(&["\x1b7", "\x1b8"]).into(),
        11 => rng
            .pick(&["\x1b[s", "\x1b[u", "\x1b[?1048h", "\x1b[?1048l"])
            .into(),
        12 => rng
            .pick(&[
                "\x1b(0lqk\x1b(B",
                "\x1b(0",
                "\x1b(B",
                "\x1b)0\x0e",
                "\x0f",
                "\x1b(A#",
                "\x1b*0\x1bn",
                "\x1bo",
            ])
            .into(),
        13 => rng.pick(&["\x1b[3g", "\x1bH", "\t", "\x1b[2Z"]).into(),
        14 => "x".repeat(usize::from(cols) - 1) + "yz",
        15 => rng.pick(&["\x1b[3b", "a\x1b[2b", "\x1b[b"]).into(),
        16 => rng
            .pick(&[
                "\x1b[?1049h",
                "\x1b[?1049l",
                "\x1b[?47h",
                "\x1b[?47l",
                "\x1b[?1047h",
                "\x1b[?1047l",
            ])
            .into(),
        17 => rng
            .pick(&[
                "\x1b[>1u",
                "\x1b[>3u",
                "\x1b[<u",
                "\x1b[=5;1u",
                "\x1b[=2;2u",
                "\x1b[<9u",
                "\x1b[>31u",
            ])
            .into(),
        18 => rng
            .pick(&[
                "\x1b]4;1;rgb:10/20/30\x07",
                "\x1b]104\x07",
                "\x1b]10;rgb:ff/00/00\x07",
                "\x1b]11;rgb:00/00/80\x1b\\",
                "\x1b]110\x07",
            ])
            .into(),
        19 => rng
            .pick(&[
                "\x1b[1\"q",
                "\x1b[0\"q",
                "\x1b[?1000h",
                "\x1b[?1006h",
                "\x1b[?2026h",
                "\x1b[?2026l",
                "\x1bV",
                "\x1bW",
            ])
            .into(),
        20 => rng
            .pick(&[
                "\x1bN",
                "\x1bO",
                "\x1b*0\x1bn",
                "\x1bn",
                "\x1bo",
                "\x1b~",
                "\x1b|",
            ])
            .into(),
        21 => "日本".repeat(1 + rng.below((c / 2).max(1)) as usize),
        22 => rng
            .pick(&[
                "\x1b[2 q",
                "\x1b[5 q",
                "\x1b[0 q",
                "\x1b]2;title\x07",
                "\x1b]7;file:///tmp\x07",
                "\x1b]0;both\x07",
            ])
            .into(),
        23 => rng
            .pick(&["\x1b[?69h\x1b[3;10s", "\x1b[?69l", "\x1b[?6h", "\x1b[?6l"])
            .into(),
        24 => rng
            .pick(&[
                "\x1b]133;A\x07",
                "\x1b]133;B\x07",
                "\x1b]133;C\x07",
                "\x1b]133;D;0\x07",
                "\x1b]133;P;k=c\x07",
                "\x1b]133;I\x07",
                "\x1b]133;A;cl=m\x07",
                "\x1b]133;L\x07",
            ])
            .into(),
        25 => rng.pick(&["\x1b[?2027h", "\x1b[?2027l"]).into(),
        26 => rng
            .pick(&[
                "\x1b[2;5H\x1b[44m\x1b[K\x1b[0m",
                "\x1b[41m\x1b[3X\x1b[m",
                "\x1b[42m\x1b[2J\x1b[m",
            ])
            .into(),
        27 => rng
            .pick(&["\x1b#8", "\x1b[?5h", "\x1b[?5l", "\x1b[?45h", "\x1b[?12h"])
            .into(),
        28 => rng
            .pick(&[
                "\x1b[c",
                "\x1b[6n",
                "\x1b[>c",
                "\x1b[?u",
                "\x1b[?2027$p",
                "\x1b[18t",
                "\x1b[5n",
            ])
            .into(),
        29 => rng
            .pick(&[
                "\x07",
                "\x1b]52;c;aGVsbG8=\x07",
                "\x1b]9;hello\x07",
                "\x1b]777;notify;T;B\x07",
            ])
            .into(),
        30 => rng
            .pick(&["\x1b[2;3r\x1b[?6h\x1b[2;2H", "\x1b[r", "\x1b[3;1H\x1b[2M"])
            .into(),
        31 => "abcdefghijklmnopqrstuvwxyz"
            .chars()
            .take(1 + rng.below(c * 2) as usize)
            .collect(),
        32 => {
            return Op::Resize(
                (2 + rng.below(u64::from(cols) + 6)) as u16,
                (2 + rng.below(u64::from(rows) + 4)) as u16,
            )
        }
        _ => "\x1bc".into(),
    };
    Op::Bytes(s)
}

fn stream(rng: &mut Rng, n: usize, cols: &mut u16, rows: &mut u16, fams: &[u64]) -> Vec<Op> {
    (0..n)
        .map(|_| {
            let fam = fams[rng.below(fams.len() as u64) as usize];
            let op = piece(rng, fam, *cols, *rows);
            if let Op::Resize(c, r) = op {
                *cols = c;
                *rows = r;
            }
            op
        })
        .collect()
}

fn apply(em: &mut Emulator, ops: &[Op], effects: &mut Vec<vorn_screen::Effect>) {
    for op in ops {
        match op {
            Op::Bytes(s) => em.feed(s.as_bytes(), effects),
            Op::Resize(c, r) => em.resize(u32::from(*c), u32::from(*r), effects).unwrap(),
        }
    }
}

/// Both fed `ops`, each carrying on from its own rebuild before a resize, which reflows by unsaved page memory.
fn apply_both(
    a: &mut Emulator,
    b: &mut Emulator,
    ops: &[Op],
    ea: &mut Vec<vorn_screen::Effect>,
    eb: &mut Vec<vorn_screen::Effect>,
) {
    for op in ops {
        if let Op::Resize(..) = op {
            if let (Ok((_, ra)), Ok((_, rb))) = (a.cut(), b.cut()) {
                *a = ra;
                *b = rb;
            }
        }
        apply(a, std::slice::from_ref(op), ea);
        apply(b, std::slice::from_ref(op), eb);
    }
}

/// `None` when no checkpoint could be cut after `a`; otherwise whether the
/// rebuilt terminal still matches after `b`, with both fingerprints.
fn run(cols: u16, rows: u16, a: &[Op], b: &[Op]) -> Option<Result<(), (String, String)>> {
    let mut live = Emulator::new(u32::from(cols), u32::from(rows)).unwrap();
    let mut sink = Vec::new();
    apply(&mut live, a, &mut sink);
    let (cp, _) = match live.cut() {
        Ok(cut) => cut,
        Err(why) => {
            assert!(
                why == "restore check" && pending_wrap_off_the_edge(&live),
                "{why}: {a:?}"
            );
            return None;
        }
    };
    let bytes = cp.encode();
    let cp = Checkpoint::decode(&bytes).expect("decodes what it encoded");
    let mut rebuilt = Emulator::restore(&cp).unwrap();
    assert!(cp.matches(&rebuilt), "restore check");
    let (mut e1, mut e2) = (Vec::new(), Vec::new());
    apply_both(&mut live, &mut rebuilt, b, &mut e1, &mut e2);
    let (f1, f2) = (live.fingerprint(), rebuilt.fingerprint());
    if f1 == f2 && e1 == e2 {
        Some(Ok(()))
    } else {
        Some(Err((format!("{f1}\n{e1:?}"), format!("{f2}\n{e2:?}"))))
    }
}

/// The one state Ghostty's snapshot drops: a pending wrap off the last column (a right margin, a back tab).
fn pending_wrap_off_the_edge(em: &Emulator) -> bool {
    let t = em.terminal();
    t.is_cursor_pending_wrap().unwrap() && t.cursor_x().unwrap() + 1 != t.cols().unwrap()
}

fn fails(cols: u16, rows: u16, a: &[Op], b: &[Op]) -> bool {
    matches!(run(cols, rows, a, b), Some(Err(_)))
}

fn shrink(cols: u16, rows: u16, a: &mut Vec<Op>, b: &mut Vec<Op>) {
    loop {
        let mut changed = false;
        for which in 0..2 {
            let mut i = 0;
            loop {
                let len = if which == 0 { a.len() } else { b.len() };
                if i >= len {
                    break;
                }
                let (mut ta, mut tb) = (a.clone(), b.clone());
                if which == 0 {
                    ta.remove(i);
                } else {
                    tb.remove(i);
                }
                if fails(cols, rows, &ta, &tb) {
                    *a = ta;
                    *b = tb;
                    changed = true;
                } else {
                    i += 1;
                }
            }
        }
        if !changed {
            return;
        }
    }
}

fn diff(x: &str, y: &str) -> String {
    let mut out = String::new();
    for (l1, l2) in x.lines().zip(y.lines()) {
        if l1 != l2 {
            out.push_str(&format!("- {l1}\n+ {l2}\n"));
        }
    }
    out
}

fn check(seeds: std::ops::Range<u64>, fams: &[u64], len_a: usize, len_b: usize) -> (u32, u32) {
    let (mut cut, mut tried) = (0, 0);
    for seed in seeds {
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let (cols, rows) = (2 + rng.below(18) as u16, 2 + rng.below(8) as u16);
        let (mut c, mut r) = (cols, rows);
        let n = 1 + rng.below(len_a as u64) as usize;
        let mut a = stream(&mut rng, n, &mut c, &mut r, fams);
        let n = 1 + rng.below(len_b as u64) as usize;
        let mut b = stream(&mut rng, n, &mut c, &mut r, fams);
        tried += 1;
        match run(cols, rows, &a, &b) {
            None => {}
            Some(Ok(())) => cut += 1,
            Some(Err(_)) => {
                shrink(cols, rows, &mut a, &mut b);
                let (f1, f2) = run(cols, rows, &a, &b).unwrap().unwrap_err();
                panic!(
                    "seed {seed} {cols}x{rows}\nA {a:?}\nB {b:?}\n{}",
                    diff(&f1, &f2)
                );
            }
        }
    }
    (cut, tried)
}

#[test]
fn every_family_alone_carries_on() {
    for fam in 0..FAMILIES {
        let (cut, tried) = check(0..150, &[fam, 0, 1, 3, 31], 12, 12);
        assert!(cut > 0, "family {fam}: no checkpoint in {tried}");
    }
}

#[test]
fn mixed_streams_carry_on() {
    let all: Vec<u64> = (0..FAMILIES).collect();
    let (cut, tried) = check(0..3000, &all, 40, 30);
    eprintln!("cut {cut} of {tried}");
    assert!(tried - cut <= tried / 100, "cut {cut} of {tried}");
}

#[test]
fn a_shell_session_cuts_at_every_prompt() {
    // What vorn's zsh integration prints around a command.
    let mut em = Emulator::new(40, 6).unwrap();
    let mut sink = Vec::new();
    for i in 0..20 {
        let prompt = format!(
            "\x1b]133;D;0\x07\x1b]133;A\x07\x1b]2;~/src\x07~/src ❯ \x1b]133;B\x07ls -la {i}\r\n\x1b]133;C\x07"
        );
        em.feed(prompt.as_bytes(), &mut sink);
        em.feed(
            b"total 0\r\ndrwxr-xr-x  2 me  staff  64 Jan  1 00:00 .\r\n",
            &mut sink,
        );
        em.feed(
            b"\x1b]133;D;0\x07\x1b]133;A\x07~/src \xe2\x9d\xaf \x1b]133;B\x07",
            &mut sink,
        );
        let cp = em.checkpoint();
        assert!(cp.is_ok(), "step {i}: {cp:?}");
    }
}

#[test]
fn malformed_checkpoints_are_refused() {
    let mut em = Emulator::new(10, 3).unwrap();
    let mut sink = Vec::new();
    em.feed(b"hello\r\nworld", &mut sink);
    let bytes = em.checkpoint().unwrap().encode();
    for cut in 0..bytes.len() {
        assert!(Checkpoint::decode(&bytes[..cut]).is_none());
    }
    let mut longer = bytes.clone();
    longer.push(0);
    assert!(Checkpoint::decode(&longer).is_none());
}
