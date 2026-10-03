//! `VORN_TRANSCRIPTS=<dir of <name>.json flush arrays> cargo run --release -p vorn-analysis --example throughput`:
//! the analysis over the server bench's own transcripts, fed per flush.
use std::time::Instant;
use vorn_analysis::Analyzer;

fn main() {
    let dir = std::env::var("VORN_TRANSCRIPTS").expect("VORN_TRANSCRIPTS");
    for name in ["agent", "spinner", "bulk"] {
        let raw = std::fs::read_to_string(format!("{dir}/{name}.json")).unwrap();
        let flushes: Vec<String> = serde_json::from_str(&raw).unwrap();
        let mb = flushes.iter().map(String::len).sum::<usize>() as f64 / 1048576.0;
        let mut best = f64::MAX;
        for _ in 0..30 {
            let t = Instant::now();
            let mut a = Analyzer::new();
            for f in &flushes {
                std::hint::black_box(a.append_str(f, true));
            }
            best = best.min(t.elapsed().as_secs_f64());
        }
        println!("{name}: {:.2} ms/MB", best * 1000.0 / mb);
        let units: Vec<Vec<u16>> = flushes.iter().map(|f| f.encode_utf16().collect()).collect();
        let mut best = f64::MAX;
        let mut out = String::new();
        for _ in 0..30 {
            let t = Instant::now();
            for u in &units {
                out.clear();
                vorn_analysis::utf16::push_utf16(&mut out, u);
                std::hint::black_box(&out);
            }
            best = best.min(t.elapsed().as_secs_f64());
        }
        println!("  utf16 -> utf8: {:.2} ms/MB", best * 1000.0 / mb);
    }
}
