//! `cargo bench -p vorn-screen --bench corpora`: `Emulator` against bare libghostty-vt, target 0.8x or better.

use std::hint::black_box;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use libghostty_vt::terminal::Terminal;
use vorn_recovery::gen::{Generator, Mix, Profile};
use vorn_recovery::{transcript, Log, Rng};
use vorn_screen::Emulator;
use vorn_term_proto::Record;

/// Output per corpus per run; transcripts repeat to reach it.
const BYTES: usize = 4 << 20;
/// History both terminals keep, in bytes of Ghostty's page memory.
const SCROLLBACK: usize = 4 << 20;

/// PTY reads at one size, as a session's terminal is fed them.
struct Corpus {
    cols: u16,
    rows: u16,
    flushes: Vec<Vec<u8>>,
}

impl Corpus {
    /// A log's output at the size of its first resize, repeated to [`BYTES`]: parsing alone.
    fn from_log(log: &Log) -> Corpus {
        let (mut cols, mut rows) = (log.size.cols, log.size.rows);
        if let Some((c, r)) = log.entries.iter().find_map(|e| match e.rec {
            Record::Resize { cols, rows, .. } => Some((cols, rows)),
            _ => None,
        }) {
            (cols, rows) = (c, r);
        }
        let once: Vec<Vec<u8>> = log
            .entries
            .iter()
            .filter_map(|e| match &e.rec {
                Record::Data { bytes, .. } => Some(bytes.clone()),
                _ => None,
            })
            .collect();
        let len: usize = once.iter().map(Vec::len).sum();
        let flushes = once
            .iter()
            .cycle()
            .take(once.len() * BYTES.div_ceil(len.max(1)))
            .cloned()
            .collect();
        Corpus {
            cols,
            rows,
            flushes,
        }
    }

    fn bytes(&self) -> u64 {
        self.flushes.iter().map(|f| f.len() as u64).sum()
    }
}

fn transcript_corpus(name: &str) -> Corpus {
    Corpus::from_log(&transcript::builtin(name).expect("shipped transcript"))
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
];
const WORDS: &[&str] = &[
    "value", "buffer", "index", "state", "cursor", "screen", "offset",
];

/// A compiler run with a redrawn progress line and wrapping command lines, in 4 KiB reads.
fn build_log() -> Corpus {
    let mut rng = Rng::new(7);
    let mut out = String::with_capacity(BYTES + 4096);
    let (green, yellow, blue, reset) = (
        "\x1b[1m\x1b[92m",
        "\x1b[1m\x1b[93m",
        "\x1b[1m\x1b[94m",
        "\x1b[0m",
    );
    let mut n = 0u64;
    while out.len() < BYTES {
        n += 1;
        let krate = rng.pick(CRATES);
        let done = n % 400;
        let filled = (done * 40 / 400) as usize;
        out.push_str(&format!(
            "\r\x1b[K{blue}    Building{reset} [{}>{}] {done}/400: {krate}\r\x1b[K",
            "=".repeat(filled),
            " ".repeat(40 - filled),
        ));
        match rng.below(10) {
            0..=4 => out.push_str(&format!("{green}   Compiling{reset} {krate}-{n} v0.{}.0\r\n", rng.below(30))),
            5 | 6 => {
                let w = rng.pick(WORDS);
                out.push_str(&format!(
                    "{yellow}warning{reset}\x1b[1m: unused variable: `{w}`{reset}\r\n{blue}  --> {reset}crates/{krate}/src/{w}.rs:{}:9\r\n{blue}   |{reset}         {yellow}{}{reset} help: prefix it with an underscore\r\n\r\n",
                    rng.range(10, 900),
                    "^".repeat(w.len()),
                ));
            }
            7 | 8 => out.push_str(&format!(
                "{green}     Running{reset} `rustc --crate-name {krate} --edition=2021 crates/{krate}/src/lib.rs --error-format=json --crate-type lib -C opt-level=3 -C metadata={:016x} --out-dir /work/target/release/deps -L dependency=/work/target/release/deps --cap-lints allow`\r\n",
                rng.next_u64(),
            )),
            _ => {
                for _ in 0..rng.range(5, 30) {
                    let result = if rng.chance(19, 20) { "\x1b[32mok\x1b[0m" } else { "\x1b[31mFAILED\x1b[0m" };
                    out.push_str(&format!("test {krate}::{}_{} ... {result}\r\n", rng.pick(WORDS), rng.pick(WORDS)));
                }
            }
        }
    }
    Corpus {
        cols: 120,
        rows: 40,
        flushes: out.as_bytes().chunks(4096).map(<[u8]>::to_vec).collect(),
    }
}

/// The generator's every piece family but resizes.
fn seeded() -> Corpus {
    let mix = Mix {
        resize: 0,
        ..Mix::EVERYTHING
    };
    Corpus::from_log(&Generator::log(
        7,
        Profile::mixed().mix(mix).bytes(BYTES as u64),
    ))
}

fn bare(c: &Corpus) {
    let mut t = Terminal::new(c.cols, c.rows).unwrap();
    t.set_scrollback_max_bytes(Some(SCROLLBACK)).unwrap();
    for f in &c.flushes {
        t.vt_write(f);
    }
    black_box(t.cursor_x().unwrap());
}

fn screen(c: &Corpus) {
    let mut em = Emulator::with_scrollback(c.cols.into(), c.rows.into(), SCROLLBACK).unwrap();
    let mut effects = Vec::new();
    for f in &c.flushes {
        em.feed(f, &mut effects);
        effects.clear();
    }
    black_box(em.terminal().cursor_x().unwrap());
}

fn bench(c: &mut Criterion) {
    let corpora = [
        ("build-log", build_log()),
        ("vim", transcript_corpus("vim")),
        ("htop", transcript_corpus("htop")),
        ("agent", transcript_corpus("claude")),
        ("seeded", seeded()),
    ];
    for (name, corpus) in &corpora {
        let mut g = c.benchmark_group(format!("corpus/{name}"));
        g.throughput(Throughput::Bytes(corpus.bytes()));
        g.sample_size(10).measurement_time(Duration::from_secs(4));
        g.bench_with_input(BenchmarkId::new("ghostty", name), corpus, |b, c| {
            b.iter(|| bare(c))
        });
        g.bench_with_input(BenchmarkId::new("vorn-screen", name), corpus, |b, c| {
            b.iter(|| screen(c))
        });
        g.finish();
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
