//! The extension load test's Rust program: the source its Wasm component is
//! built from, served on stdio; with `--once`, a one-shot that answers the
//! one request it is started for and exits.

use std::io::{BufRead, Write};

use vorn_extension_sdk::serde_json::{self, json, Value};

fn main() {
    if std::env::args().any(|a| a == "--once") {
        once();
    } else {
        vorn_bench_ext_tiers::program::serve::<vorn_bench_load::Load>();
    }
}

fn once() {
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line).is_err() {
        return;
    }
    let request: Value = serde_json::from_str(line.trim()).unwrap_or_default();
    let answer = json!({
        "jsonrpc": "2.0",
        "id": request["id"],
        "result": { "method": request["params"]["method"] },
    });
    // A closed stdout means Vorn stopped waiting; there is nobody to tell.
    let _ = writeln!(std::io::stdout().lock(), "{answer}");
}
