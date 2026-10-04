//! `cargo bench -p vorn-screen`: the libghostty-vt parse and the checkpoint
//! serialize, without napi, fed per flush as the server feeds them.

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use std::hint::black_box;
use vorn_screen::Screen;

/// About a megabyte of coloured build output and redrawn status lines, in 64 KB flushes.
fn flushes() -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut total = 0;
    let mut i = 0u32;
    while total < 1 << 20 {
        i += 1;
        let line = if i.is_multiple_of(5) {
            format!("\r\x1b[2K\x1b[38;5;174m⠙\x1b[39m Thinking… ({}s)", i / 40)
        } else {
            format!(
                "\x1b[32m✓\x1b[39m compiled \x1b[1msrc/module_{i}.rs\x1b[22m in {}ms\r\n",
                i % 97
            )
        };
        total += line.len();
        cur.push_str(&line);
        if cur.len() >= 64 * 1024 {
            out.push(std::mem::take(&mut cur));
        }
    }
    out.push(cur);
    out
}

fn bench(c: &mut Criterion) {
    let flushes = flushes();
    let bytes: usize = flushes.iter().map(String::len).sum();
    let mut g = c.benchmark_group("screen");
    g.throughput(Throughput::Bytes(bytes as u64));
    g.bench_function("parse/build_log", |b| {
        b.iter(|| {
            let mut s = Screen::new(200, 50).unwrap();
            for f in &flushes {
                black_box(s.feed(f.as_bytes()));
            }
        })
    });
    g.finish();

    let mut s = Screen::new(200, 50).unwrap();
    for f in &flushes {
        s.feed(f.as_bytes());
    }
    c.bench_function("screen/serialize/200x50", |b| {
        b.iter(|| black_box(s.serialize().unwrap()))
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
