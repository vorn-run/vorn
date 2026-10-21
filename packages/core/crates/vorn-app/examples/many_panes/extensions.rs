//! The extensions a load run installs, the calls that keep them busy and
//! what they cost: Node, Python and Bash programs and Rust Wasm components,
//! services and one-shots, written into the run's temporary HOME and linked
//! into its vornd. Their ids start `bench.`, never `vorn.`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use vorn_app::client::{Endpoint, Rpc};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Lang {
    Node,
    Python,
    Bash,
    Rust,
}

impl Lang {
    pub fn name(self) -> &'static str {
        match self {
            Lang::Node => "node",
            Lang::Python => "python",
            Lang::Bash => "bash",
            Lang::Rust => "rust",
        }
    }
}

/// The twenty, 16 services and 4 one-shots. The first four are one service
/// of each language, so a short run still has every kind of service.
const MIX: [(Lang, bool); 20] = [
    (Lang::Node, true),
    (Lang::Python, true),
    (Lang::Bash, true),
    (Lang::Rust, true),
    (Lang::Node, false),
    (Lang::Python, false),
    (Lang::Bash, false),
    (Lang::Node, true),
    (Lang::Python, true),
    (Lang::Rust, true),
    (Lang::Bash, true),
    (Lang::Bash, false),
    (Lang::Node, true),
    (Lang::Python, true),
    (Lang::Rust, true),
    (Lang::Node, true),
    (Lang::Python, true),
    (Lang::Rust, true),
    (Lang::Node, true),
    (Lang::Rust, true),
];

pub const MAX: usize = MIX.len();

/// One installed bench extension.
pub struct Ext {
    pub id: String,
    pub lang: Lang,
    /// A service stays up between calls; a one-shot is a process per call.
    pub service: bool,
    pub dir: PathBuf,
}

impl Ext {
    /// The action the bench calls: the Rust component's cheapest one, the
    /// programs' `work`, which for a service asks Vorn for its sessions.
    pub fn action(&self) -> &'static str {
        if self.lang == Lang::Rust {
            "key"
        } else {
            "work"
        }
    }
}

/// Writes the first `n` of the mix into `dir`.
pub fn generate(dir: &Path, n: usize) -> Result<Vec<Ext>, String> {
    let mut seen: HashMap<(Lang, bool), usize> = HashMap::new();
    MIX.iter()
        .take(n)
        .map(|&(lang, service)| {
            let k = seen.entry((lang, service)).or_default();
            *k += 1;
            let kind = if service { "svc" } else { "once" };
            let id = format!("bench.{}-{kind}-{k}", lang.name());
            let ext = Ext {
                dir: dir.join(&id),
                id,
                lang,
                service,
            };
            write(&ext)?;
            Ok(ext)
        })
        .collect()
}

fn write(ext: &Ext) -> Result<(), String> {
    let put = |name: &str, text: &[u8]| {
        std::fs::write(ext.dir.join(name), text).map_err(|e| format!("{name}: {e}"))
    };
    std::fs::create_dir_all(&ext.dir).map_err(|e| e.to_string())?;
    let id = &ext.id;
    let mut manifest = format!(
        "id = \"{id}\"\nname = \"{id}\"\nversion = \"1.0.0\"\napi = \"vorn:extension@1.0\"\n"
    );
    // The id rides along as an argument, so `ps` tells whose process it is.
    let (runner, file, program): (&str, &str, &str) = match (ext.lang, ext.service) {
        (Lang::Node, true) => ("node", "service.mjs", NODE_SERVICE),
        (Lang::Node, false) => ("node", "once.mjs", NODE_ONCE),
        (Lang::Python, true) => ("python3", "service.py", PYTHON_SERVICE),
        (Lang::Python, false) => ("python3", "once.py", PYTHON_ONCE),
        (Lang::Bash, true) => ("bash", "service.sh", BASH_SERVICE),
        (Lang::Bash, false) => ("bash", "once.sh", BASH_ONCE),
        (Lang::Rust, _) => {
            std::fs::copy(component()?, ext.dir.join("turns.wasm")).map_err(|e| e.to_string())?;
            manifest += RUST_MANIFEST;
            return put("vorn-extension.toml", manifest.as_bytes());
        }
    };
    put(file, program.as_bytes())?;
    let command = format!("command = [\"{runner}\", \"{file}\", \"{id}\"]\n");
    if ext.service {
        let events = if ext.lang == Lang::Bash { "[]" } else { EVENTS };
        manifest += &format!(
            "\n[declares]\nuses = [\"sessions.read\"]\n\n[service]\n{command}events = {events}\n"
        );
    } else {
        manifest += &format!("\n[actions.work]\ntitle = \"Work\"\n{command}");
    }
    put("vorn-extension.toml", manifest.as_bytes())
}

/// The example `turns` component, built once a run.
fn component() -> Result<PathBuf, String> {
    static BUILT: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    BUILT
        .get_or_init(|| vorn_guest_build::component("vorn-example-turns"))
        .clone()
}

const EVENTS: &str = r#"["session.created", "session.status", "session.exited"]"#;

const RUST_MANIFEST: &str = r#"
[declares]
uses = ["sessions.read"]

[service]
component = "turns.wasm"
events = ["session.created", "session.status", "session.exited"]

[[contributes.badges]]
id = "count"
tree = true
"#;

const NODE_SERVICE: &str = r#"// A bench service: each call asks Vorn for its sessions, as a sidebar would.
import process from 'node:process'
import readline from 'node:readline'

const send = (m) => process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', ...m })}\n`)
const waiting = new Map()
let asked = 0
let heard = 0
readline.createInterface({ input: process.stdin }).on('line', (line) => {
  const m = JSON.parse(line)
  if (m.method === undefined) {
    const reply = waiting.get(m.id)
    waiting.delete(m.id)
    return reply?.(m)
  }
  if (m.method === 'guest.init') return send({ id: m.id, result: null })
  if (m.method === 'guest.on-event') {
    heard += m.params.batch.length
    return
  }
  if (m.method === 'guest.handle') {
    const rid = `r${++asked}`
    waiting.set(rid, (r) =>
      send(r.error ? { id: m.id, error: r.error } : { id: m.id, result: { bytes: JSON.stringify(r.result ?? null).length, heard } })
    )
    return send({ id: rid, method: 'host.call', params: { method: 'sessions.list', args: {} } })
  }
  if (m.id !== undefined) send({ id: m.id, error: { code: -32601, message: `no ${m.method}` } })
})
"#;

const NODE_ONCE: &str = r#"// A bench one-shot: one request in, one answer out.
import process from 'node:process'
import readline from 'node:readline'

const input = readline.createInterface({ input: process.stdin })
input.once('line', (line) => {
  const { params } = JSON.parse(line)
  process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', id: 1, result: { method: params.method } })}\n`)
  input.close()
})
"#;

const PYTHON_SERVICE: &str = r#"# A bench service: each call asks Vorn for its sessions, as a sidebar would.
import json
import sys


def send(message):
    message["jsonrpc"] = "2.0"
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


waiting = {}
asked = heard = 0
while True:
    line = sys.stdin.readline()
    if not line:
        break
    m = json.loads(line)
    method = m.get("method")
    if method is None:
        caller = waiting.pop(m.get("id"), None)
        if caller is None:
            continue
        if "error" in m:
            send({"id": caller, "error": m["error"]})
        else:
            size = len(json.dumps(m.get("result")))
            send({"id": caller, "result": {"bytes": size, "heard": heard}})
    elif method == "guest.init":
        send({"id": m["id"], "result": None})
    elif method == "guest.on-event":
        heard += len(m["params"]["batch"])
    elif method == "guest.handle":
        asked += 1
        rid = f"r{asked}"
        waiting[rid] = m["id"]
        send({"id": rid, "method": "host.call", "params": {"method": "sessions.list", "args": {}}})
    elif "id" in m:
        send({"id": m["id"], "error": {"code": -32601, "message": f"no {method}"}})
"#;

const PYTHON_ONCE: &str = r#"# A bench one-shot: one request in, one answer out.
import json
import sys

request = json.loads(sys.stdin.readline())
answer = {"jsonrpc": "2.0", "id": 1, "result": {"method": request["params"]["method"]}}
sys.stdout.write(json.dumps(answer) + "\n")
"#;

const BASH_SERVICE: &str = r#"#!/usr/bin/env bash
# A bench service: answers each request at once, with no process per line.
request='^\{"jsonrpc":"2\.0","id":([0-9]+),"method":"([a-z.-]+)"'
while IFS= read -r line; do
  [[ $line =~ $request ]] || continue
  if [[ ${BASH_REMATCH[2]} == guest.init ]]; then
    printf '{"jsonrpc":"2.0","id":%s,"result":null}\n' "${BASH_REMATCH[1]}"
  else
    printf '{"jsonrpc":"2.0","id":%s,"result":{"ok":true}}\n' "${BASH_REMATCH[1]}"
  fi
done
"#;

const BASH_ONCE: &str = r#"#!/usr/bin/env bash
# A bench one-shot: one request in, one answer out; it is always asked with id 1.
read -r request
printf '{"jsonrpc":"2.0","id":1,"result":{"ok":true}}\n'
"#;

/// Links `exts` into the vornd `rpc` talks to.
pub fn link(rpc: &Rpc, exts: &[Ext]) -> Result<(), String> {
    for ext in exts {
        rpc.call("extension:link", json!({ "path": ext.dir }))
            .map_err(|e| format!("extension:link {}: {e:?}", ext.id))?;
    }
    Ok(())
}

/// One call to `ext`, in milliseconds.
pub fn call(rpc: &Rpc, ext: &Ext) -> Result<f64, String> {
    let t = Instant::now();
    rpc.call(
        "extension:call",
        json!({ "id": ext.id, "action": ext.action(), "args": {} }),
    )
    .map_err(|e| format!("{}: {e:?}", ext.id))?;
    Ok(t.elapsed().as_secs_f64() * 1000.0)
}

/// The services running now: the bench's and Vorn's own built-ins.
pub fn running(rpc: &Rpc) -> Result<(usize, usize), String> {
    let count = |method: &str| -> Result<usize, String> {
        let list = rpc
            .call(method, Value::Null)
            .map_err(|e| format!("{method}: {e:?}"))?;
        Ok(list
            .as_array()
            .map_or(0, |l| l.iter().filter(|p| p["running"] == true).count()))
    };
    Ok((count("extension:installed")?, count("extension:builtins")?))
}

/// What a driver's calls took, and the most services it saw running.
#[derive(Default)]
pub struct Calls {
    pub ms: Vec<f64>,
    pub errors: Vec<String>,
    pub peak_bench: usize,
    pub peak_builtin: usize,
}

/// Calls every extension in turn, each once per period, on a connection of
/// its own, until it is finished; once a period it counts what is running.
pub struct Driver {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<Calls>,
}

impl Driver {
    pub fn start(home: &Path, exts: Arc<Vec<Ext>>, period: Duration) -> Result<Driver, String> {
        let ep = Endpoint::find(home).map_err(|e| format!("{e:?}"))?;
        let (rpc, events) = Rpc::connect(&ep, Arc::new(|| {})).map_err(|e| format!("{e:?}"))?;
        // Its events are the app's to draw; this connection only calls.
        drop(events);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let gap = period / (exts.len() + 1) as u32;
        let thread = std::thread::spawn(move || {
            let mut calls = Calls::default();
            let mut next = Instant::now();
            for tick in 0.. {
                if stopped.load(Ordering::Relaxed) {
                    break;
                }
                match exts.get(tick % (exts.len() + 1)) {
                    Some(ext) => match call(&rpc, ext) {
                        Ok(ms) => calls.ms.push(ms),
                        Err(e) => calls.errors.push(e),
                    },
                    None => {
                        if let Ok((bench, builtin)) = running(&rpc) {
                            calls.peak_bench = calls.peak_bench.max(bench);
                            calls.peak_builtin = calls.peak_builtin.max(builtin);
                        }
                    }
                }
                next += gap;
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                } else {
                    next = now;
                }
            }
            calls
        });
        Ok(Driver { stop, thread })
    }

    pub fn finish(self) -> Calls {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.join().unwrap_or_default()
    }
}

/// The resident memory of the processes vornd started for extensions, in
/// MB, by language: everything under vornd outside its session holders.
/// Wasm components run inside vornd and count there.
pub fn programs_rss(vornd: u32, holders: &[u32]) -> BTreeMap<&'static str, (usize, f64)> {
    let Ok(out) = std::process::Command::new("ps")
        .args(["-A", "-o", "pid=,ppid=,rss=,command="])
        .output()
    else {
        return BTreeMap::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let procs: Vec<(u32, u32, f64, &str)> = text
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let pid = f.next()?.parse().ok()?;
            let ppid = f.next()?.parse().ok()?;
            let rss: f64 = f.next()?.parse().ok()?;
            let command = l.split_once("bench.").map_or("", |(_, rest)| rest);
            Some((pid, ppid, rss / 1024.0, command))
        })
        .collect();
    let skip: HashSet<u32> = holders.iter().copied().collect();
    let mut under = HashSet::from([vornd]);
    // Parents come before children in pid order only mostly; settle by rounds.
    loop {
        let before = under.len();
        for &(pid, ppid, _, _) in &procs {
            if under.contains(&ppid) && !skip.contains(&pid) {
                under.insert(pid);
            }
        }
        if under.len() == before {
            break;
        }
    }
    let mut by: BTreeMap<&'static str, (usize, f64)> = BTreeMap::new();
    for &(pid, _, mb, bench) in &procs {
        if pid == vornd || !under.contains(&pid) {
            continue;
        }
        let lang = [Lang::Node, Lang::Python, Lang::Bash, Lang::Rust]
            .into_iter()
            .find(|l| bench.starts_with(l.name()))
            .map_or("other", Lang::name);
        let e = by.entry(lang).or_default();
        e.0 += 1;
        e.1 += mb;
    }
    by
}

/// What vornd's log says the services did.
#[derive(Default, Debug, PartialEq)]
pub struct Lifecycle {
    /// Bench services started.
    pub starts: usize,
    /// Starts past the first for a service: after the cap, idle or a crash stopped it.
    pub restarts: usize,
    /// Services the cap stopped, the built-ins' included.
    pub evictions: usize,
    pub crashes: usize,
    pub idle_stops: usize,
}

pub fn read_log(log: &str) -> Lifecycle {
    let mut starts: HashMap<&str, usize> = HashMap::new();
    let mut seen = Lifecycle::default();
    for line in log.lines() {
        let Some((_, said)) = line.split_once("[extensions] ") else {
            continue;
        };
        if said.contains(" stops for the cap of ") {
            seen.evictions += 1;
        } else if said.ends_with(" ended; it starts again") {
            seen.crashes += 1;
        } else if said.ends_with(" stops after sitting idle") {
            seen.idle_stops += 1;
        } else if let Some(id) = said
            .strip_suffix(" started")
            .or_else(|| said.strip_suffix(" started as a component"))
            .filter(|id| id.starts_with("bench."))
        {
            *starts.entry(id).or_default() += 1;
        }
    }
    seen.starts = starts.values().sum();
    seen.restarts = starts.values().map(|n| n - 1).sum();
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_counts_restarts_past_each_services_first_start() {
        let log = "\
INFO [extensions] bench.node-svc-1 started
INFO [extensions] bench.rust-svc-1 started as a component
INFO [extensions] vorn.tasks started as a component
INFO [extensions] bench.node-svc-1 stops for the cap of 8 running
INFO [extensions] bench.node-svc-1 started
WARN [extensions] bench.rust-svc-1 ended; it starts again
INFO [extensions] bench.rust-svc-1 stops after sitting idle
";
        assert_eq!(
            read_log(log),
            Lifecycle {
                starts: 3,
                restarts: 1,
                evictions: 1,
                crashes: 1,
                idle_stops: 1,
            }
        );
    }
}
