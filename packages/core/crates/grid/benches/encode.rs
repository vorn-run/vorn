//! `cargo bench -p vorn-grid`: the cost of grid mode at 32 busy terminals,
//! next to the cost of parsing their output (Terminal State Protocol §17:
//! encoding at 32 terminals must cost less than the parse it rides on).
//!
//! Each of 32 terminals, 200x50 with scrollback, is fed its own seeded
//! output from the recovery harness's generator, round robin, 8 KB a
//! terminal at a time: about 1 MB/s each, 32 MB/s in all, at one render
//! update per 8 ms frame. `parse` times only the terminals parsing it;
//! `encode` times only the render updates: dirty rows re-encoded and
//! compared with the row cache, line counting, tables and `TermState`. Both
//! report throughput in bytes of output, so their ratio is the encoder's
//! share of the parse budget. Three inputs: the generator's shell output
//! (prompts, colour, links, progress bars, and combining marks in nearly
//! every frame), everything the generator makes (full-screen redraws, the
//! alternate screen, resizes and the rest), and a build log with a spinner
//! redrawn in place, as `vorn-screen`'s parse bench feeds, the way agents
//! and build tools print.

use std::hint::black_box;
use std::time::{Duration, Instant};

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use vorn_grid::Grid;
use vorn_recovery::gen::{Generator, Profile};
use vorn_recovery::{Log, Size};
use vorn_screen::Emulator;
use vorn_term_proto::{Cursor, Record};

const TERMINALS: usize = 32;
const PER_TERMINAL: u64 = 512 << 10;
const FRAME_BYTES: usize = 8 << 10;
const SCROLLBACK: usize = 1 << 20;

/// A build log with a spinner line redrawn in place, `PER_TERMINAL` bytes,
/// started at a different line for each terminal.
fn build_log(t: usize) -> Log {
    let mut b = vorn_recovery::LogBuilder::new(Size::new(200, 50));
    let mut total = 0u64;
    let mut i = t as u32 * 1000;
    while total < PER_TERMINAL {
        let mut chunk = String::new();
        while chunk.len() < 4096 {
            i += 1;
            chunk.push_str(&if i.is_multiple_of(5) {
                format!("\r\x1b[2K\x1b[38;5;174m⠙\x1b[39m Thinking… ({}s)", i / 40)
            } else {
                format!(
                    "\x1b[32m✓\x1b[39m compiled \x1b[1msrc/module_{i}.rs\x1b[22m in {}ms\r\n",
                    i % 97
                )
            });
        }
        total += chunk.len() as u64;
        b.data(chunk);
    }
    b.build()
}

/// Each terminal's output, cut into what arrives between two frames.
fn frames(profile: Option<Profile>) -> (Vec<Vec<Vec<Record>>>, u64) {
    let logs: Vec<Log> = match profile {
        Some(p) => Generator::sessions(
            42,
            TERMINALS,
            p.size(Size::new(200, 50)).bytes(PER_TERMINAL),
        ),
        None => (0..TERMINALS).map(build_log).collect(),
    };
    let mut total = 0;
    let per = logs
        .iter()
        .map(|log| {
            let mut frames = Vec::new();
            let mut cur = Vec::new();
            let mut bytes = 0;
            for e in &log.entries {
                if let Record::Data { bytes: b, .. } = &e.rec {
                    bytes += b.len();
                    total += b.len() as u64;
                }
                cur.push(e.rec.clone());
                if bytes >= FRAME_BYTES {
                    frames.push(std::mem::take(&mut cur));
                    bytes = 0;
                }
            }
            frames.push(cur);
            frames
        })
        .collect();
    (per, total)
}

fn apply(em: &mut Emulator, recs: &[Record], fx: &mut Vec<vorn_screen::Effect>) {
    for r in recs {
        match r {
            Record::Data { bytes, .. } => em.feed(bytes, fx),
            Record::Resize { cols, rows, .. } => {
                let _ = em.resize(u32::from(*cols), u32::from(*rows), fx);
            }
            _ => {}
        }
        fx.clear();
    }
}

fn terminals() -> Vec<Emulator> {
    (0..TERMINALS)
        .map(|_| Emulator::with_scrollback(200, 50, SCROLLBACK).unwrap())
        .collect()
}

/// Runs every terminal's frames round robin; times the parse, the render
/// updates, or both, as asked.
fn run(per: &[Vec<Vec<Record>>], encode: bool) -> (Duration, Duration) {
    let mut ems = terminals();
    let mut grids: Vec<Grid> = (0..TERMINALS).map(|_| Grid::new().unwrap()).collect();
    let mut fx = Vec::new();
    let (mut parse, mut enc) = (Duration::ZERO, Duration::ZERO);
    let most = per.iter().map(Vec::len).max().unwrap_or(0);
    for f in 0..most {
        for (t, frames) in per.iter().enumerate() {
            let Some(recs) = frames.get(f) else { continue };
            let start = Instant::now();
            apply(&mut ems[t], recs, &mut fx);
            parse += start.elapsed();
            if encode {
                let start = Instant::now();
                black_box(grids[t].update(&ems[t], Cursor::default()).unwrap());
                enc += start.elapsed();
            }
        }
    }
    (parse, enc)
}

fn bench(c: &mut Criterion) {
    for (name, profile) in [
        ("build-log", None),
        ("shell", Some(Profile::shell())),
        ("mixed", Some(Profile::mixed())),
    ] {
        let (per, total) = frames(profile);
        let mut g = c.benchmark_group(format!("grid/32x200x50/{name}"));
        g.throughput(Throughput::Bytes(total));
        g.sample_size(10);
        g.measurement_time(Duration::from_secs(20));
        g.bench_function("parse", |b| {
            b.iter_custom(|n| (0..n).map(|_| run(&per, false).0).sum())
        });
        g.bench_function("encode", |b| {
            b.iter_custom(|n| (0..n).map(|_| run(&per, true).1).sum())
        });
        g.finish();
        let (parse, enc) = run(&per, true);
        let mb = total as f64 / f64::from(1 << 20);
        println!(
            "grid/32x200x50/{name}: {mb:.1} MB, parse {:.2} ms/MB, encode {:.2} ms/MB ({:.0}% of parse)",
            parse.as_secs_f64() * 1e3 / mb,
            enc.as_secs_f64() * 1e3 / mb,
            100.0 * enc.as_secs_f64() / parse.as_secs_f64()
        );
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
