//! Workflow runs against a test vornd: trigger to first step, the engine's cost per step, gates, schedules and memory per run.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::daemon::TestVornd;
use crate::live::{wall_ms, Live, Note};
use crate::stats::summary;
use crate::sys;

/// How long a burst of runs may take before the bench calls it stuck.
const BURST_LIMIT: Duration = Duration::from_secs(180);

/// A manual workflow of `steps` trivial conditions in a chain, with a gate after the first when asked.
pub fn chain(id: &str, steps: usize, gate: bool, trigger: Value) -> Value {
    let mut nodes = vec![
        json!({ "id": "t", "type": "trigger", "label": "Start", "position": { "x": 0, "y": 0 }, "config": trigger }),
    ];
    let mut edges = Vec::new();
    let mut prev = "t".to_owned();
    let link = |from: &str, to: &str, edges: &mut Vec<Value>| {
        edges.push(json!({ "id": format!("e-{from}-{to}"), "source": from, "target": to }));
    };
    for i in 1..=steps {
        let node = format!("s{i}");
        nodes.push(json!({
            "id": node, "type": "condition", "label": format!("Step {i}"), "position": { "x": 0, "y": 120 * i },
            "config": { "variable": "1", "operator": "equals", "value": "1" }
        }));
        link(&prev, &node, &mut edges);
        prev = node;
        if gate && i == 1 {
            nodes.push(json!({ "id": "g", "type": "approval", "label": "Review", "position": { "x": 200, "y": 120 }, "config": { "message": "Carry on?" } }));
            link(&prev, "g", &mut edges);
            prev = "g".into();
        }
    }
    json!({ "id": id, "name": format!("Bench {id}"), "icon": "Play", "iconColor": "#10b981", "enabled": true, "workspaceId": "personal", "nodes": nodes, "edges": edges })
}

fn manual() -> Value {
    json!({ "triggerType": "manual" })
}

async fn start(bin: &Path) -> Result<(TestVornd, Live), String> {
    let bin = bin.to_owned();
    let v = tokio::task::spawn_blocking(move || TestVornd::start(&bin))
        .await
        .map_err(|e| e.to_string())??;
    let live = Live::connect(&v.home).await?;
    let (mut config, _) = live.call("config:load", Value::Null).await?;
    config["defaults"]["hasSeenOnboarding"] = json!(2);
    live.call("config:save", config).await?;
    Ok((v, live))
}

/// What one run did, as its broadcasts showed it.
#[derive(Default, Clone)]
struct Seen {
    sent: Option<Instant>,
    sent_wall: f64,
    /// When the engine says its first step and the run started and ended, Unix ms.
    first_wall: Option<f64>,
    done_wall: Option<f64>,
    first: Option<Instant>,
    waiting: Option<Instant>,
    done: Option<Instant>,
    run_id: Option<String>,
    status: String,
    steps_done: usize,
}

fn iso_ms(iso: Option<&String>) -> Option<f64> {
    iso.and_then(|t| crate::stats::iso_ms(t))
}

fn moved(note: &Note) -> bool {
    note.run.as_ref().is_some_and(|r| {
        r.steps
            .iter()
            .any(|s| s.node_id != "t" && s.status != "pending")
    })
}

/// Samples a process's footprint every 20 ms until stopped: the highest seen.
struct Sampler {
    stop: Arc<AtomicBool>,
    peak: Arc<Mutex<f64>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Sampler {
    fn start(pid: u32) -> Sampler {
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(Mutex::new(0.0_f64));
        let (s, p) = (Arc::clone(&stop), Arc::clone(&peak));
        let thread = std::thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                if let Some((f, _)) = sys::memory(pid) {
                    let mut p = p.lock().unwrap_or_else(|e| e.into_inner());
                    *p = f64::max(*p, f);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        Sampler {
            stop,
            peak,
            thread: Some(thread),
        }
    }

    fn finish(mut self) -> f64 {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        *self.peak.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Triggers every workflow at once and follows each run to its end (or its gate).
async fn burst(
    live: &mut Live,
    ids: &[String],
    until_gate: bool,
) -> Result<(HashMap<String, Seen>, usize, usize), String> {
    live.drain();
    let mut seen: HashMap<String, Seen> = HashMap::new();
    let mut answers = Vec::new();
    for id in ids {
        let sent_wall = wall_ms();
        let (sent, rx) = live.send("workflow:runManual", json!({ "workflowId": id }));
        seen.insert(
            id.clone(),
            Seen {
                sent: Some(sent),
                sent_wall,
                ..Seen::default()
            },
        );
        answers.push(rx);
    }
    for rx in answers {
        let (_, a) = rx.await.map_err(|_| "closed".to_owned())?;
        a?;
    }
    let (mut notes, mut bytes) = (0, 0);
    let until = tokio::time::Instant::now() + BURST_LIMIT;
    let settled = |s: &Seen| {
        if until_gate {
            s.waiting.is_some() || s.done.is_some()
        } else {
            s.done.is_some()
        }
    };
    while !seen.values().all(settled) {
        let Ok(Some(note)) = tokio::time::timeout_at(until, live.notes.recv()).await else {
            let left = seen.values().filter(|s| !settled(s)).count();
            return Err(format!("{left} of {} runs did not finish", ids.len()));
        };
        let Some(run) = &note.run else { continue };
        notes += 1;
        bytes += note.bytes;
        let Some(s) = seen.get_mut(&run.workflow_id) else {
            continue;
        };
        if s.run_id.as_ref().is_some_and(|r| r != &run.run_id) {
            continue;
        }
        s.run_id = Some(run.run_id.clone());
        if s.first.is_none() && moved(&note) {
            s.first = Some(note.at);
        }
        if s.first_wall.is_none() {
            s.first_wall = iso_ms(
                run.steps
                    .iter()
                    .find(|st| st.node_id == "s1")
                    .and_then(|st| st.started_at.as_ref()),
            );
        }
        if s.waiting.is_none()
            && run
                .steps
                .iter()
                .any(|st| st.node_id == "g" && st.status == "waiting")
        {
            s.waiting = Some(note.at);
        }
        if s.done.is_none() && run.status != "running" {
            s.done = Some(note.at);
            s.done_wall = iso_ms(run.completed_at.as_ref());
            s.status = run.status.clone();
            s.steps_done = run
                .steps
                .iter()
                .filter(|st| st.node_id != "t" && st.status == "success")
                .count();
        }
    }
    Ok((seen, notes, bytes))
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Runs of `steps` steps, `concurrent` at once.
pub async fn runs(bin: &Path, steps: usize, concurrent: usize) -> Result<Value, String> {
    let (v, mut live) = start(bin).await?;
    let pid = v.pid();
    let ids: Vec<String> = (0..concurrent).map(|i| format!("wf-{i:03}")).collect();
    for id in ids.iter().chain(std::iter::once(&"warm".to_owned())) {
        live.call(
            "workflow:create",
            json!({ "workflow": chain(id, steps, false, manual()) }),
        )
        .await?;
    }
    for _ in 0..3 {
        burst(&mut live, &["warm".to_owned()], false).await?;
    }
    let bursts = (30 / concurrent).clamp(1, 10);
    let (mut first, mut per_step, mut rate, mut mem_run) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut seen_first, mut cpu_step) = (Vec::new(), Vec::new());
    let (mut notes, mut bytes, mut failed) = (0, 0, 0);
    for _ in 0..bursts {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let base = sys::memory(pid).map_or(0.0, |m| m.0);
        let sampler = Sampler::start(pid);
        let cpu0 = sys::cpu(pid);
        let (seen, n, b) = burst(&mut live, &ids, false).await?;
        let cpu1 = sys::cpu(pid);
        let peak = sampler.finish();
        if let (Some(a), Some(z)) = (cpu0, cpu1) {
            cpu_step.push((z - a) * 1e6 / (concurrent * steps) as f64);
        }
        notes += n;
        bytes += b;
        mem_run.push((peak - base).max(0.0) * 1024.0 / concurrent as f64);
        let start = seen.values().filter_map(|s| s.sent).min();
        let end = seen.values().filter_map(|s| s.done).max();
        if let (Some(a), Some(z)) = (start, end) {
            rate.push((concurrent * steps) as f64 / (z - a).as_secs_f64());
        }
        for s in seen.values() {
            if s.status != "success" || s.steps_done != steps {
                failed += 1;
            }
            if let (Some(sent), Some(f)) = (s.sent, s.first) {
                seen_first.push(ms(f - sent));
            }
            if let Some(f) = s.first_wall {
                first.push(f - s.sent_wall);
            }
            if let (Some(f), Some(d)) = (s.first_wall, s.done_wall) {
                per_step.push((d - f) * 1000.0 / steps as f64);
            }
        }
    }
    Ok(json!({
        "kind": "flows",
        "steps": steps,
        "concurrent": concurrent,
        "bursts": bursts,
        "failed_runs": failed,
        "trigger_to_first_ms": summary(&first),
        "seen_first_ms": summary(&seen_first),
        "per_step_us": summary(&per_step),
        "cpu_us_per_step": summary(&cpu_step),
        "steps_per_s": summary(&rate),
        "peak_kb_per_run": summary(&mem_run),
        "run_notes": notes,
        "run_note_mb": bytes as f64 / (1024.0 * 1024.0),
        "vornd_mb_end": sys::memory(pid).map(|m| m.0),
    }))
}

/// Runs parked at a gate, `concurrent` at once, then approved one by one.
pub async fn gates(bin: &Path, concurrent: usize) -> Result<Value, String> {
    let (v, mut live) = start(bin).await?;
    let pid = v.pid();
    let ids: Vec<String> = (0..concurrent).map(|i| format!("gate-{i:03}")).collect();
    for id in ids.iter().chain(std::iter::once(&"warm".to_owned())) {
        live.call(
            "workflow:create",
            json!({ "workflow": chain(id, 2, true, manual()) }),
        )
        .await?;
    }
    let (mut request, mut answer, mut resume, mut parked_kb) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut walked_on = Vec::new();
    let rounds = (10 / concurrent).max(1);
    for round in 0..=rounds {
        let warm = round == 0;
        let list = if warm {
            vec!["warm".to_owned()]
        } else {
            ids.clone()
        };
        tokio::time::sleep(Duration::from_millis(300)).await;
        let base = sys::memory(pid).map_or(0.0, |m| m.0);
        let (seen, _, _) = burst(&mut live, &list, true).await?;
        tokio::time::sleep(Duration::from_millis(200)).await;
        if !warm {
            let parked = sys::memory(pid).map_or(0.0, |m| m.0);
            parked_kb.push((parked - base).max(0.0) * 1024.0 / list.len() as f64);
        }
        for (wf, s) in &seen {
            let (Some(run), Some(waiting)) = (s.run_id.clone(), s.waiting) else {
                return Err(format!("{wf} never reached its gate"));
            };
            if let Some(sent) = s.sent {
                if !warm {
                    request.push(ms(waiting - sent));
                }
            }
            live.drain();
            let sent_wall = wall_ms();
            let (sent, rx) = live.send(
                "workflow:resolveGate",
                json!({ "runId": run, "nodeId": "g", "decision": "approve" }),
            );
            let (at, a) = rx.await.map_err(|_| "closed".to_owned())?;
            if a?["accepted"] != json!(true) {
                return Err(format!("the gate of {wf} was not accepted"));
            }
            let until = tokio::time::Instant::now() + Duration::from_secs(30);
            let mut resumed = None;
            let next = loop {
                let Ok(Some(note)) = tokio::time::timeout_at(until, live.notes.recv()).await else {
                    return Err(format!("{wf} did not walk on after its gate"));
                };
                let Some(r) = &note.run else { continue };
                if r.run_id != run {
                    continue;
                }
                if resumed.is_none()
                    && r.steps
                        .iter()
                        .any(|st| st.node_id == "g" && st.status == "success")
                {
                    resumed = Some(note.at);
                }
                if let Some(at) = iso_ms(
                    r.steps
                        .iter()
                        .find(|st| st.node_id == "s2")
                        .and_then(|st| st.started_at.as_ref()),
                ) {
                    break at;
                }
            };
            let resumed = resumed.unwrap_or(sent);
            if !warm {
                answer.push(ms(at - sent));
                resume.push(ms(resumed - sent));
                walked_on.push(next - sent_wall);
            }
        }
        // Let the runs finish before the next round triggers them again.
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    Ok(json!({
        "kind": "gates",
        "concurrent": concurrent,
        "trigger_to_gate_ms": summary(&request),
        "answer_ms": summary(&answer),
        "answer_to_resume_ms": summary(&walked_on),
        "seen_resume_ms": summary(&resume),
        "parked_kb_per_run": summary(&parked_kb),
    }))
}

/// Workflows due at their own second within the next half minute, and how late each starts.
pub async fn schedule(bin: &Path) -> Result<Value, String> {
    let (_v, mut live) = start(bin).await?;
    const COUNT: u64 = 12;
    let now = wall_ms();
    let second = (now / 1000.0).floor() as u64;
    let mut due: HashMap<String, f64> = HashMap::new();
    for i in 0..COUNT {
        let at = second + 6 + 2 * i;
        let cron = format!("{} * * * * *", at % 60);
        let id = format!("sched-{i:02}");
        live.call("workflow:create", json!({ "workflow": chain(&id, 1, false, json!({ "triggerType": "recurring", "cron": cron })) })).await?;
        due.insert(id, at as f64 * 1000.0);
    }
    live.drain();
    let (mut late, mut seen_late) = (Vec::new(), Vec::new());
    let until = tokio::time::Instant::now() + Duration::from_secs(6 + 2 * COUNT + 15);
    while !due.is_empty() {
        let Ok(Some(note)) = tokio::time::timeout_at(until, live.notes.recv()).await else {
            break;
        };
        let Some(r) = &note.run else { continue };
        if let Some(at) = due.remove(&r.workflow_id) {
            seen_late.push(note.wall_ms - at);
            if let Some(started) = iso_ms(r.started_at.as_ref()) {
                late.push(started - at);
            }
        }
    }
    let earliest = late.iter().copied().fold(f64::INFINITY, f64::min);
    let jitter: Vec<f64> = late.iter().map(|l| l - earliest).collect();
    Ok(json!({
        "kind": "schedule",
        "jitter_ms": summary(&jitter),
        "fired": late.len(),
        "missed": due.len(),
        "late_ms": summary(&late),
        "seen_late_ms": summary(&seen_late),
    }))
}
