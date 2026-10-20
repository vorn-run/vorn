//! `cargo bench -p vorn-screen --bench scan`: the byte scan beside the emulator, alone, on large output.

use std::hint::black_box;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use vorn_recovery::transcript;
use vorn_term_proto::Record;

#[allow(dead_code)]
#[path = "../src/scan.rs"]
mod scan;

/// Output per corpus per run; transcripts repeat to reach it.
const BYTES: usize = 8 << 20;

/// `once` repeated to [`BYTES`], in 4 KiB reads.
fn reads(once: &[u8]) -> Vec<Vec<u8>> {
    once.iter()
        .copied()
        .cycle()
        .take(BYTES)
        .collect::<Vec<u8>>()
        .chunks(4096)
        .map(<[u8]>::to_vec)
        .collect()
}

fn transcript_reads(name: &str) -> Vec<Vec<u8>> {
    let log = transcript::builtin(name).expect("shipped transcript");
    let once: Vec<u8> = log
        .entries
        .iter()
        .filter_map(|e| match &e.rec {
            Record::Data { bytes, .. } => Some(bytes.as_slice()),
            _ => None,
        })
        .flatten()
        .copied()
        .collect();
    reads(&once)
}

fn bench(c: &mut Criterion) {
    let plain = reads(b"the quick brown fox jumps over the lazy dog 0123456789\r\n");
    let colored = reads(b"\x1b[1m\x1b[92m   Compiling\x1b[0m serde v1.0.0 \x1b[32mok\x1b[0m\r\n");
    let corpora = [
        ("plain", plain),
        ("colored", colored),
        ("agent", transcript_reads("agent")),
        ("vim", transcript_reads("vim")),
    ];
    let mut g = c.benchmark_group("scan");
    g.sample_size(20).measurement_time(Duration::from_secs(3));
    for (name, reads) in &corpora {
        g.throughput(Throughput::Bytes(reads.iter().map(|r| r.len() as u64).sum()));
        g.bench_with_input(BenchmarkId::from_parameter(name), reads, |b, reads| {
            b.iter(|| {
                let mut s = scan::Scan::default();
                let mut n = 0usize;
                for r in reads {
                    s.feed(r, |_, p| n += p.len());
                }
                black_box((n, s.clears, s.marks))
            })
        });
    }
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
