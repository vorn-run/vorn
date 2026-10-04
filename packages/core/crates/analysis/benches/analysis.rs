//! `cargo bench -p vorn-analysis`: the analysis alone, without napi, on two of
//! the shapes of output the server bench uses, a spinner and a bulk dump. Generated here so the crate
//! needs nothing outside it; the server bench (`yarn bench`) is the number the
//! work packages are accepted on.

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use std::hint::black_box;
use vorn_analysis::Analyzer;

/// xorshift, so every run feeds the same bytes.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Roughly a megabyte as an agent TUI and a build write it, in read-sized chunks.
fn transcripts() -> Vec<(&'static str, Vec<String>)> {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let (mut spinner, mut bulk) = (Vec::new(), Vec::new());
    let (mut sb, mut bb) = (0, 0);
    let mut tick = 0;
    while sb < 1 << 20 {
        tick += 1;
        let c = if rng.next().is_multiple_of(25) {
            "\r\x1b[2K\r\n\x1b[1m\x1b[38;5;114m⏺\x1b[39m Read\x1b[22m(src/lib.rs)\r\n  \x1b[2m⎿  Read 120 lines\x1b[22m\r\n\r\n".to_owned()
        } else {
            format!(
                "\r\x1b[2K\x1b[38;5;174m{}\x1b[39m Thinking… \x1b[2m({}s · ↓ {} tokens · esc to interrupt)\x1b[22m",
                SPINNER[tick % 10],
                tick / 12,
                tick * 7
            )
        };
        sb += c.len();
        spinner.push(c);
    }
    while bb < 1 << 20 {
        let mut c = String::new();
        while c.len() < 4000 {
            let n = rng.next() % 900;
            c.push_str(&format!(
                "\x1b[32m✓\x1b[39m compiled src/module_{n}.rs in {}ms\r\n",
                n % 97
            ));
        }
        bb += c.len();
        bulk.push(c);
    }
    vec![("spinner", spinner), ("bulk", bulk)]
}

fn bench(c: &mut Criterion) {
    let mut g = c.benchmark_group("analysis");
    for (name, chunks) in transcripts() {
        let bytes: usize = chunks.iter().map(String::len).sum();
        g.throughput(Throughput::Bytes(bytes as u64));
        g.bench_function(format!("per_chunk/{name}"), |b| {
            b.iter(|| {
                let mut a = Analyzer::new();
                for ch in &chunks {
                    black_box(a.append_str(ch, true));
                }
            })
        });
        // What the server pays when it analyzes once per flush instead.
        let flushes: Vec<String> = chunks.chunks(100).map(|c| c.concat()).collect();
        g.bench_function(format!("per_flush/{name}"), |b| {
            b.iter(|| {
                let mut a = Analyzer::new();
                for f in &flushes {
                    black_box(a.append_str(f, true));
                }
            })
        });
    }
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
