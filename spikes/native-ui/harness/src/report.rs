//! Turns the raw arrays into the numbers in the report.

use serde_json::{json, Value};

fn pct(sorted: &[f64], p: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    Some(sorted[i])
}

fn floats(v: &Value) -> Vec<f64> {
    v.as_array()
        .map(|a| a.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default()
}

fn r1(x: Option<f64>) -> Value {
    x.map_or(Value::Null, |x| json!((x * 10.0).round() / 10.0))
}

/// The headline numbers of one run.
pub fn summarize(r: &Value) -> Value {
    let c = &r["client_report"];
    let mut s = json!({});

    let mut lat = floats(&c["latency_ms"]);
    lat.sort_by(f64::total_cmp);
    if !lat.is_empty() {
        s["latency_p50_ms"] = r1(pct(&lat, 0.5));
        s["latency_p95_ms"] = r1(pct(&lat, 0.95));
        s["latency_p99_ms"] = r1(pct(&lat, 0.99));
        s["latency_n"] = json!(lat.len());
        s["latency_lost"] = c["lost"].clone();
    }

    let at = floats(&c["frame_at_ms"]);
    if at.len() > 1 && r["mode"] == "load" {
        let period = c["period_ms"]
            .as_f64()
            .filter(|p| *p > 1.0)
            .unwrap_or(1000.0 / r["hz"].as_f64().unwrap_or(60.0));
        let mut dt: Vec<f64> = at.windows(2).map(|w| w[1] - w[0]).collect();
        let dropped: f64 = dt
            .iter()
            .map(|d| ((d / period).round() - 1.0).max(0.0))
            .sum();
        let span = at[at.len() - 1] - at[0];
        dt.sort_by(f64::total_cmp);
        let mut work = floats(&c["frame_work_ms"]);
        work.sort_by(f64::total_cmp);
        s["frames"] = json!(at.len());
        s["fps"] = r1(Some((at.len() - 1) as f64 * 1000.0 / span));
        s["refresh_hz"] = r1(Some(1000.0 / period));
        s["frame_interval_p50_ms"] = r1(pct(&dt, 0.5));
        s["frame_interval_p95_ms"] = r1(pct(&dt, 0.95));
        s["frame_interval_max_ms"] = r1(dt.last().copied());
        s["frame_work_p50_ms"] = r1(pct(&work, 0.5));
        s["frame_work_p95_ms"] = r1(pct(&work, 0.95));
        s["dropped_frames"] = json!(dropped);
        s["dropped_pct"] = r1(Some(100.0 * dropped / (dropped + at.len() as f64)));
    }

    // CPU and memory over the measured window: from the "window" mark (the
    // client's settle time is over) to the last sample.
    let rows = r["samples"]["rows"].as_array().cloned().unwrap_or_default();
    let window_at = r["samples"]["marks"]
        .as_array()
        .and_then(|m| m.iter().find(|x| x[0] == "window"))
        .and_then(|x| x[1].as_f64());
    if let (Some(w), true) = (window_at, rows.len() > 2) {
        let win: Vec<&Value> = rows
            .iter()
            .filter(|row| row[0].as_f64().unwrap_or(0.0) >= w)
            .collect();
        // The last sample may be the exiting process; leave it out.
        let win = &win[..win.len().saturating_sub(1)];
        if win.len() >= 2 {
            let (a, b) = (win[0], win[win.len() - 1]);
            let secs = b[0].as_f64().unwrap() - a[0].as_f64().unwrap();
            let cpu = |i: usize| {
                let d = b[i].as_f64().unwrap_or(0.0) - a[i].as_f64().unwrap_or(0.0);
                Some(100.0 * d / 1e9 / secs)
            };
            let mb = |i: usize, f: fn(f64, f64) -> f64| {
                let x = win
                    .iter()
                    .map(|row| row[i].as_f64().unwrap_or(0.0))
                    .fold(0.0, f);
                Some(x / 1048576.0)
            };
            s["client_cpu_pct"] = r1(cpu(1));
            s["vornd_cpu_pct"] = r1(cpu(4));
            s["sessiond_cpu_pct"] = r1(cpu(6));
            s["windowserver_cpu_pct"] = r1(cpu(9));
            s["client_rss_max_mb"] = r1(mb(2, f64::max));
            s["client_footprint_max_mb"] = r1(mb(3, f64::max));
            s["vornd_rss_max_mb"] = r1(mb(5, f64::max));
            s["helpers"] = win[win.len() - 1][7].clone();
            s["window_s"] = r1(Some(secs));
        }
    }
    s["cold_start_ms"] = r1(r["cold_start_ms"].as_f64());
    s
}

/// Every run in results/raw as one markdown table, also written to
/// results/summary.md.
pub fn main() {
    let raw = crate::spike_root().join("results/raw");
    let mut runs: Vec<Value> = std::fs::read_dir(&raw)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_str(&std::fs::read_to_string(e.path()).ok()?).ok())
        .collect();
    runs.sort_by_key(|r| r["name"].as_str().unwrap_or("").to_owned());
    let cols = [
        "latency_p50_ms",
        "latency_p95_ms",
        "latency_lost",
        "fps",
        "frame_interval_p95_ms",
        "frame_work_p50_ms",
        "frame_work_p95_ms",
        "dropped_frames",
        "dropped_pct",
        "client_cpu_pct",
        "client_footprint_max_mb",
        "client_rss_max_mb",
        "vornd_cpu_pct",
        "windowserver_cpu_pct",
        "cold_start_ms",
    ];
    let mut md = String::from("| run | ");
    md.push_str(&cols.join(" | "));
    md.push_str(" |\n|---|");
    md.push_str(&"---|".repeat(cols.len()));
    md.push('\n');
    for r in &runs {
        let s = summarize(r);
        md.push_str(&format!("| {} |", r["name"].as_str().unwrap_or("?")));
        for c in cols {
            let v = &s[c];
            md.push_str(&format!(
                " {} |",
                if v.is_null() { "".into() } else { v.to_string() }
            ));
        }
        md.push('\n');
    }
    print!("{md}");
    let _ = std::fs::write(crate::spike_root().join("results/summary.md"), md);
}
