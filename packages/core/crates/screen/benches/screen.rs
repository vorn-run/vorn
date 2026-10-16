//! `cargo bench -p vorn-screen`: the libghostty-vt parse and the checkpoint
//! serialize, fed per flush, and a session engine's [`Emulator`]: its parse
//! of text in many scripts and the checkpoint it cuts over deep history.

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use std::hint::black_box;
use vorn_screen::{Emulator, Screen};

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

/// About a megabyte of text in many scripts, in 64 KB flushes: accents as
/// combining marks, emoji with skin tones and joiners, CJK, and on every line
/// codepoints from blocks the lines before it did not use.
fn unicode_flushes() -> Vec<Vec<u8>> {
    const EMOJI: &[&str] = &["👍\u{1f3fd}", "👩\u{200d}💻", "❤\u{fe0f}", "🇺🇸", "✓"];
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut total = 0;
    let mut i = 0u32;
    while total < 1 << 20 {
        i += 1;
        // Greek and Cyrillic to Thai, then Latin Extended Additional and on.
        let base = 0x370 + (i * 7) % 0x900;
        let mut line = format!("\x1b[3{}m{i:>6}\x1b[39m ", i % 8);
        for k in 0..24 {
            line.extend(char::from_u32(base + k));
            if k % 6 == 0 {
                line.push('e');
                line.extend(char::from_u32(0x300 + (i + k) % 0x70));
            }
        }
        line.push_str(EMOJI[i as usize % EMOJI.len()]);
        line.push_str(" 日本語のテキスト\r\n");
        total += line.len();
        cur.push_str(&line);
        if cur.len() >= 64 * 1024 {
            out.push(std::mem::take(&mut cur).into_bytes());
        }
    }
    out.push(cur.into_bytes());
    out
}

/// A session engine's terminal holding about 4 MB of history from `flushes`.
fn deep(flushes: &[String]) -> Emulator {
    let mut em = Emulator::with_scrollback(200, 50, 4 << 20).unwrap();
    let mut fx = Vec::new();
    for f in flushes {
        em.feed(f.as_bytes(), &mut fx);
    }
    em
}

fn emulator(c: &mut Criterion, flushes: &[String]) {
    let text = unicode_flushes();
    let bytes: usize = text.iter().map(Vec::len).sum();
    let mut g = c.benchmark_group("emulator");
    g.throughput(Throughput::Bytes(bytes as u64));
    g.bench_function("parse/unicode", |b| {
        b.iter(|| {
            let mut em = Emulator::with_scrollback(200, 50, 1 << 20).unwrap();
            let mut fx = Vec::new();
            for f in &text {
                em.feed(f, &mut fx);
                fx.clear();
            }
            black_box(em.terminal().cursor_x().unwrap())
        })
    });
    g.finish();

    let mut g = c.benchmark_group("emulator/checkpoint");
    g.sample_size(10);
    let mut em = deep(flushes);
    g.bench_function("fingerprint/200x50+history", |b| {
        b.iter(|| black_box(em.fingerprint().len()))
    });
    g.bench_function("cut/200x50+history", |b| {
        b.iter(|| black_box(em.cut().unwrap().0.snapshot.len()))
    });
    g.finish();
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

    emulator(c, &flushes);
}

criterion_group!(benches, bench);
criterion_main!(benches);
