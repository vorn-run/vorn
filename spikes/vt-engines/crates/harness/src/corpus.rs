//! The inputs every engine is fed: what a session's PTY reads look like.
//!
//! - `build-log`: a long compiler run, plain scrolling output with colour,
//!   progress lines redrawn with `\r` and EL, and command lines long enough
//!   to wrap. Synthesized from a seed, so it carries nothing about a machine.
//! - `vim`, `htop`, `agent`: the PTY transcripts vorn-recovery ships.
//! - `seeded`: vorn-recovery's generator with every piece family on.

use vorn_recovery::gen::{Generator, Profile};
use vorn_recovery::log::Log;
use vorn_recovery::{transcript, Rng};
use vorn_term_proto::Record;

/// One PTY read, or a resize between reads.
#[derive(Debug, Clone)]
pub enum Op {
    Data(Vec<u8>),
    Resize(u16, u16),
}

#[derive(Debug, Clone)]
pub struct Corpus {
    pub cols: u16,
    pub rows: u16,
    pub ops: Vec<Op>,
}

impl Corpus {
    pub fn data_bytes(&self) -> usize {
        self.ops
            .iter()
            .map(|op| match op {
                Op::Data(b) => b.len(),
                Op::Resize(..) => 0,
            })
            .sum()
    }

    fn from_log(log: &Log) -> Corpus {
        let ops = log
            .entries
            .iter()
            .filter_map(|e| match &e.rec {
                Record::Data { bytes, .. } => Some(Op::Data(bytes.clone())),
                &Record::Resize { cols, rows, .. } => Some(Op::Resize(cols, rows)),
                Record::Gap { .. } | Record::Exit { .. } => None,
            })
            .collect();
        Corpus {
            cols: log.size.cols,
            rows: log.size.rows,
            ops,
        }
    }

    /// The same corpus played until it holds at least `bytes` of output,
    /// each pass starting from the first size again.
    fn repeated(self, bytes: usize) -> Corpus {
        let one = self.data_bytes().max(1);
        let passes = bytes.div_ceil(one);
        let mut ops = Vec::with_capacity(self.ops.len() * passes + passes);
        for _ in 0..passes {
            ops.push(Op::Resize(self.cols, self.rows));
            ops.extend(self.ops.iter().cloned());
        }
        Corpus { ops, ..self }
    }
}

/// Names of the corpora, in report order.
pub const NAMES: [&str; 5] = ["build-log", "vim", "htop", "agent", "seeded"];

/// The recovery crate's transcripts by the names this spike reports them
/// under: the one that is neither vim nor htop is the agent CLI.
fn transcript_log(name: &str) -> Log {
    let (_, bytes) = transcript::BUILTIN
        .iter()
        .find(|(n, _)| match name {
            "agent" => !matches!(*n, "vim" | "htop"),
            other => *n == other,
        })
        .unwrap_or_else(|| panic!("no transcript for {name}"));
    transcript::parse(bytes).expect("shipped transcript parses")
}

/// A corpus at least `bytes` long (transcripts repeat to get there).
pub fn load(name: &str, bytes: usize) -> Corpus {
    match name {
        "build-log" => build_log(7, bytes),
        "vim" | "htop" | "agent" => Corpus::from_log(&transcript_log(name)).repeated(bytes),
        "seeded" => seeded(7, Profile::mixed(), bytes),
        other => panic!("unknown corpus {other}"),
    }
}

/// One pass of a corpus: transcripts once, the others at `bytes`.
pub fn load_once(name: &str, bytes: usize) -> Corpus {
    match name {
        "vim" | "htop" | "agent" => Corpus::from_log(&transcript_log(name)),
        _ => load(name, bytes),
    }
}

pub fn seeded(seed: u64, profile: Profile, bytes: usize) -> Corpus {
    let log = Generator::log(seed, profile.bytes(bytes as u64));
    Corpus::from_log(&log)
}

const CRATES: &[&str] = &[
    "serde",
    "tokio",
    "memchr",
    "regex",
    "syn",
    "quote",
    "libc",
    "bytes",
    "smallvec",
    "hashbrown",
    "parking_lot",
    "crossbeam",
    "rayon",
    "itertools",
    "anyhow",
    "thiserror",
    "clap",
    "postcard",
    "criterion",
    "unicode-width",
];
const WORDS: &[&str] = &[
    "value", "buffer", "index", "state", "handle", "record", "cursor", "screen", "offset", "frame",
    "result", "config", "reader", "writer", "session",
];
const SGR_GREEN: &str = "\x1b[1m\x1b[92m";
const SGR_YELLOW: &str = "\x1b[1m\x1b[93m";
const SGR_BLUE: &str = "\x1b[1m\x1b[94m";
const SGR_RESET: &str = "\x1b[0m";

/// A compiler run's output, cut into 4 KiB reads as a PTY delivers it.
pub fn build_log(seed: u64, bytes: usize) -> Corpus {
    let mut rng = Rng::new(seed);
    let mut out = String::with_capacity(bytes + 4096);
    let total = 400;
    let mut n = 0u64;
    while out.len() < bytes {
        n += 1;
        let krate = rng.pick(CRATES);
        let (a, b, c) = (rng.below(3), rng.below(30), rng.below(20));
        let done = n % total;
        // The progress line cargo redraws in place before every message.
        let filled = (done * 40 / total) as usize;
        out.push_str(&format!(
            "\r\x1b[K{SGR_BLUE}    Building{SGR_RESET} [{}>{}] {done}/{total}: {krate}, {}\r\x1b[K",
            "=".repeat(filled),
            " ".repeat(40 - filled),
            rng.pick(CRATES),
        ));
        match rng.below(10) {
            0..=4 => out.push_str(&format!(
                "{SGR_GREEN}   Compiling{SGR_RESET} {krate}-{n} v{a}.{b}.{c}\r\n"
            )),
            5 | 6 => {
                let w = rng.pick(WORDS);
                let line = rng.range(10, 900);
                let col = rng.range(5, 60) as usize;
                out.push_str(&format!(
                    "{SGR_YELLOW}warning{SGR_RESET}\x1b[1m: unused variable: `{w}`{SGR_RESET}\r\n\
                     {SGR_BLUE}  --> {SGR_RESET}crates/{krate}/src/{w}.rs:{line}:{col}\r\n\
                     {SGR_BLUE}   |{SGR_RESET}\r\n\
                     {SGR_BLUE}{line:<3}|{SGR_RESET}     let {w} = compute(&mut self.{w}, offset + {c});\r\n\
                     {SGR_BLUE}   |{SGR_RESET}         {SGR_YELLOW}{}{SGR_RESET} {SGR_YELLOW}help: if this is intentional, prefix it with an underscore: `_{w}`{SGR_RESET}\r\n\
                     {SGR_BLUE}   |{SGR_RESET}\r\n\
                     {SGR_BLUE}   = {SGR_RESET}\x1b[1mnote{SGR_RESET}: `#[warn(unused_variables)]` on by default\r\n\r\n",
                    "^".repeat(w.len()),
                ));
            }
            7 | 8 => {
                // A verbose command line, long enough to wrap twice or more.
                out.push_str(&format!(
                    "{SGR_GREEN}     Running{SGR_RESET} `rustc --crate-name {krate} --edition=2021 crates/{krate}/src/lib.rs --error-format=json --json=diagnostic-rendered-ansi,artifacts,future-incompat --crate-type lib --emit=dep-info,metadata,link -C opt-level=3 -C embed-bitcode=no -C codegen-units=16 --cfg 'feature=\"default\"' --cfg 'feature=\"std\"' -C metadata={:016x} -C extra-filename=-{:016x} --out-dir /work/target/release/deps -L dependency=/work/target/release/deps --cap-lints allow`\r\n",
                    rng.next_u64(),
                    rng.next_u64(),
                ));
            }
            _ => {
                for _ in 0..rng.range(5, 30) {
                    let ok = rng.chance(19, 20);
                    out.push_str(&format!(
                        "test {}::{}_{} ... {}\r\n",
                        krate,
                        rng.pick(WORDS),
                        rng.pick(WORDS),
                        if ok {
                            "\x1b[32mok\x1b[0m"
                        } else {
                            "\x1b[31mFAILED\x1b[0m"
                        },
                    ));
                }
            }
        }
    }
    let ops = out
        .as_bytes()
        .chunks(4096)
        .map(|c| Op::Data(c.to_vec()))
        .collect();
    Corpus {
        cols: 120,
        rows: 40,
        ops,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vorn_recovery::log::Size;

    #[test]
    fn corpora_reach_their_size_and_repeat_from_the_first_size() {
        for name in NAMES {
            let c = load(name, 256 << 10);
            assert!(c.data_bytes() >= 256 << 10, "{name}");
        }
        let vim = load("vim", 64 << 10);
        assert!(matches!(vim.ops[0], Op::Resize(c, r) if (c, r) == (vim.cols, vim.rows)));
    }

    #[test]
    fn the_build_log_is_the_same_for_a_seed() {
        let a = build_log(3, 64 << 10);
        let b = build_log(3, 64 << 10);
        assert_eq!(format!("{:?}", a.ops), format!("{:?}", b.ops));
    }

    #[test]
    fn the_agent_transcript_is_the_third_one() {
        let c = load_once("agent", 0);
        assert!(c.data_bytes() > 1000);
    }

    #[test]
    fn sizes_come_from_the_log() {
        let c = seeded(1, Profile::mixed().size(Size::new(90, 30)), 4096);
        assert_eq!((c.cols, c.rows), (90, 30));
    }
}
