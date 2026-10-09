//! RC-T1 and RC-T2 end to end, on every OS the app ships for: the real
//! vornd and vorn-sessiond binaries, eight sessions of seeded output from
//! `recovery-emit` (four in a PTY, four piped), and vornd killed by the OS
//! (SIGKILL, TerminateProcess) at seeded random moments while they print.
//! Each kill is followed by a new vornd, which finds the running sessiond
//! and recovers every session from its restore base (a checkpoint, or the
//! session start) and the records after it.
//!
//! At least two kills must land while output is still arriving (vornd had
//! applied less than the whole of it), and every vornd must find the one
//! sessiond the first started. Once every record is applied:
//!
//! - RC-T1, the recovered byte stream: per session, vornd's disk history
//!   holds every record once, rseq contiguous from 0 and offsets chaining
//!   with no Gap, up to the head sessiond reports, which is where vornd's
//!   cursor stands. A piped session's stdout in it is the emitter's output
//!   byte for byte, by hash. When sessiond still retains a session from its
//!   start, the records it hands an attach from there are the history's.
//! - RC-T2, the recovered state: every session reports exact fidelity, and
//!   its terminal's state digest (`/vornd/sessions?digest=1`) is the digest
//!   of a terminal that never died: a fresh session engine terminal with
//!   vornd's configuration, at the spawn size, fed the same records.
//!
//! On Windows the bundle sessiond is installed from holds the sideloaded
//! ConPTY, taken from `VORN_CONPTY_DIR`, and the test checks the console
//! host in use is that one (`OpenConsole.exe`), not the system's.
//!
//! The binaries are found beside vornd: build them first with
//! `cargo build --release --locked -p vorn-sessiond -p vorn-recovery --bins`.

#![cfg(feature = "engine")]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use vorn_engine::{Cadence, Config, Session, PIPED_SIZE};
use vorn_pipeline::history::{self, Frame};
use vorn_recovery::{emit, Digest, Rng};
use vorn_sessiond::launch;
use vorn_sessiond_wire::{
    Attach, AttachFrom, FrameReader, Hello, Message, SessionInfo, ToSessiond, ToVornd, PROTO,
};
use vorn_term_proto::{Cursor, Entry, Record, RecordHeader, Stream};

const VORND: &str = env!("CARGO_BIN_EXE_vornd");

/// Long enough for a slow CI machine to start a process or two.
const PATIENCE: Duration = Duration::from_secs(30);

/// How long a PTY session's cursor must stand still to count as done: its
/// emitter pauses milliseconds between pieces, and a console host renders
/// within a frame or two.
const STILL: Duration = Duration::from_secs(3);

/// The size the PTY sessions are spawned at.
const PTY_SIZE: (u16, u16) = (100, 30);

/// One run: how much each session prints and how often vornd dies.
#[derive(Debug, Clone, Copy)]
struct Plan {
    seed: u64,
    /// Output per session, before a PTY adds to it.
    bytes: u64,
    /// The emitter's pieces and the pause between them.
    chunk: usize,
    pause_ms: u64,
    kills: usize,
    /// Each kill comes this many milliseconds after the vornd it kills
    /// started, uniformly.
    gap_ms: (u64, u64),
    /// How long every session may take to finish once the kills are done.
    finish: Duration,
}

/// RC-T1 and RC-T2 at a size CI can afford: 2.5 MiB per session, past the
/// engine's checkpoint cadence twice so later vornds restore from
/// checkpoints cut mid-stream, in 8 KiB pieces 10 ms apart, so output runs
/// for a few seconds; and five kills.
#[test]
fn vornd_killed_mid_burst_recovers_every_session() {
    gate(Plan {
        seed: 0x6a7e,
        bytes: 5 << 19,
        chunk: 8 << 10,
        pause_ms: 10,
        kills: 5,
        gap_ms: (150, 700),
        finish: Duration::from_secs(120),
    });
}

/// The full size: 50 MB over the eight sessions and twenty kills.
#[test]
#[ignore = "50 MB through eight sessions and twenty kills; run with --ignored"]
fn vornd_killed_mid_burst_fifty_megabytes() {
    gate(Plan {
        seed: 0x50,
        bytes: 50_000_000 / 8,
        chunk: 8 << 10,
        pause_ms: 10,
        kills: 20,
        gap_ms: (150, 700),
        finish: Duration::from_secs(600),
    });
}

/// A session the test started, and what its emitter prints.
#[derive(Debug)]
struct Spawned {
    id: String,
    pid: u32,
    epoch: u32,
    pty: bool,
    seed: u64,
    /// What the emitter prints; a piped session records exactly this.
    expected: Vec<u8>,
}

impl Spawned {
    fn size(&self) -> (u16, u16) {
        if self.pty {
            PTY_SIZE
        } else {
            PIPED_SIZE
        }
    }
}

fn gate(plan: Plan) {
    // The home is the temporary directory itself: macOS caps a socket path
    // at 104 bytes, and the sockets live under it.
    let home_dir = tempfile::tempdir().unwrap();
    let home = home_dir.path().to_path_buf();
    let bundle_dir = tempfile::tempdir().unwrap();
    let sessiond = bundle(bundle_dir.path());
    let emitter = beside_vornd("recovery-emit");
    let mut rig = Rig {
        home: home.clone(),
        sessiond,
        vornd: None,
        pids: Vec::new(),
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let first = rig.start_vornd();
    let holder = first.wait_connected();
    let sessiond_pid = pid_of(&holder["current"]);
    rig.vornd = Some(first);

    let sessions = rt.block_on(spawn_all(&rig, &emitter, plan));
    rig.pids.extend(sessions.iter().map(|s| s.pid));
    let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
    let vornd = rig.vornd.as_ref().expect("a vornd");

    // ConPTY holds its output until its start-up cursor query is answered,
    // which only a live vornd does: the kills start once every session is
    // printing.
    let started = Instant::now();
    loop {
        let applied = applied(&vornd.get("/vornd/sessions"));
        if ids
            .iter()
            .all(|id| applied.get(*id).is_some_and(|c| c.next_offset >= 4096))
        {
            break;
        }
        assert!(
            started.elapsed() < PATIENCE,
            "the sessions did not start printing: {applied:?}\nvornd log:\n{}",
            vornd.log_text()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    #[cfg(windows)]
    sideloaded_conpty_in_use(&home);

    // The kills, each at a seeded moment after its vornd started. What
    // vornd had applied in total just before each says whether output was
    // still arriving.
    let mut rng = Rng::new(plan.seed);
    let mut before_kill = Vec::with_capacity(plan.kills);
    for _ in 0..plan.kills {
        let gap = rng.range(plan.gap_ms.0, plan.gap_ms.1);
        std::thread::sleep(Duration::from_millis(gap));
        let mut v = rig.vornd.take().expect("a vornd");
        before_kill.push(total(&applied(&v.get("/vornd/sessions"))));
        v.kill();
        rig.vornd = Some(rig.start_vornd());
    }
    let vornd = rig.vornd.as_ref().expect("a vornd");
    let done = wait_done(vornd, &sessions, plan.finish);
    let end = total(&done);
    let mid = before_kill.iter().filter(|&&b| b < end).count();
    assert!(
        mid >= 2.min(plan.kills),
        "only {mid} of {} kills landed while output was arriving: {before_kill:?}, end {end}",
        plan.kills
    );

    // RC-T2 from vornd's side: exact, with a digest to compare.
    let report = vornd.get("/vornd/sessions?digest=1");
    assert_eq!(applied(&report), done, "nothing more arrived: {report}");
    let mut digests = HashMap::new();
    for s in report["sessions"].as_array().expect("sessions") {
        let id = s["session"].as_str().expect("an id").to_owned();
        assert_eq!(s["fidelity"], "exact", "{s}");
        assert_eq!(s["state"], "live", "{s}");
        let digest = s["digest"]
            .as_str()
            .and_then(|d| u64::from_str_radix(d, 16).ok())
            .unwrap_or_else(|| panic!("no digest: {s}"));
        digests.insert(id, digest);
    }
    let health = vornd.get("/vornd/health");
    assert_eq!(
        pid_of(&health["sessiond"]["current"]),
        sessiond_pid,
        "every vornd found the sessiond the first one started"
    );

    // A clean stop leaves sessiond to itself, to be asked directly.
    rig.vornd.take().expect("a vornd").stop();
    let held = rt.block_on(from_sessiond(&home, &sessions));
    let instances = launch::running(&home);
    assert_eq!(
        instances.iter().map(|i| i.pid).collect::<Vec<_>>(),
        [sessiond_pid],
        "one sessiond throughout"
    );

    let history_dir = home.join("vornd").join("history");
    for s in &sessions {
        let (info, retained) = held
            .get(&s.pid)
            .unwrap_or_else(|| panic!("sessiond does not hold {}", s.id));
        let frames = read_history(&history_dir, &s.id, s.epoch);
        // RC-T1: each record once, in order, nothing lost, up to the head.
        let mut next = Cursor::start(s.epoch);
        let mut stdout = Vec::new();
        for f in &frames {
            assert_eq!(
                (f.rseq, f.start_offset),
                (next.next_rseq, next.next_offset),
                "{}: the history does not follow on at rseq {}",
                s.id,
                f.rseq
            );
            assert!(
                !matches!(f.record, Record::Gap { .. }),
                "{}: a Gap at rseq {}",
                s.id,
                f.rseq
            );
            if let Record::Data { stream, bytes } = &f.record {
                assert_eq!(
                    *stream,
                    if s.pty { Stream::Pty } else { Stream::Stdout },
                    "{}",
                    s.id
                );
                stdout.extend_from_slice(bytes);
            }
            next.next_rseq += 1;
            next.next_offset += f.record.len();
        }
        assert_eq!(
            next, info.head,
            "{}: the history ends at sessiond's head",
            s.id
        );
        assert_eq!(
            done[&s.id], info.head,
            "{}: vornd's cursor is sessiond's head",
            s.id
        );
        if !s.pty {
            assert_same_bytes(&s.id, &stdout, &s.expected);
        }
        if let Some(log) = retained {
            assert_eq!(log.len(), frames.len(), "{}: sessiond's records", s.id);
            for (e, f) in log.iter().zip(&frames) {
                assert!(
                    (e.hdr.rseq, e.hdr.start_offset) == (f.rseq, f.start_offset)
                        && same_record(&e.rec, &f.record),
                    "{}: sessiond's record {:?} is {:?} in the history",
                    s.id,
                    e.hdr,
                    f.record
                );
            }
        }

        // RC-T2: the terminal that never died, fed the same records.
        let want = reference_digest(s, &frames);
        assert_eq!(
            digests.get(&s.id).copied(),
            Some(want),
            "{}: the recovered terminal is not the one that never died (seed {})",
            s.id,
            s.seed
        );
    }
}

/// Whether the history holds `rec` as sessiond recorded it. The history
/// keeps a resize's cells only, not its pixels or the request it answered.
fn same_record(rec: &Record, kept: &Record) -> bool {
    match (rec, kept) {
        (
            Record::Resize { cols, rows, .. },
            Record::Resize {
                cols: c, rows: r, ..
            },
        ) => (cols, rows) == (c, r),
        _ => rec == kept,
    }
}

/// The recovered stream is the generated one: by hash, and where they part
/// when they do.
fn assert_same_bytes(id: &str, got: &[u8], want: &[u8]) {
    if Digest::of(got) == Digest::of(want) && got == want {
        return;
    }
    let at = got
        .iter()
        .zip(want)
        .position(|(a, b)| a != b)
        .unwrap_or(got.len().min(want.len()));
    panic!(
        "{id}: recovered {} bytes, generated {}; they part at byte {at}",
        got.len(),
        want.len()
    );
}

/// The state digest of a terminal that never died: a fresh session engine
/// terminal as vornd configures one (its default scrollback and colours),
/// at the session's spawn size, fed `frames` in order. It cuts no
/// checkpoints, so nothing but the records shapes it.
fn reference_digest(s: &Spawned, frames: &[Frame]) -> u64 {
    let cfg = Arc::new(Config {
        colors: Some(vornd::engine::DEFAULT_COLORS),
        cadence: Cadence {
            bytes: u64::MAX,
            quiet_bytes: u64::MAX,
            ..Cadence::default()
        },
        ..Config::default()
    });
    let entries: Vec<Entry> = frames
        .iter()
        .map(|f| Entry {
            hdr: RecordHeader {
                epoch: s.epoch,
                rseq: f.rseq,
                start_offset: f.start_offset,
            },
            at_ns: 0,
            rec: f.record.clone(),
        })
        .collect();
    let mut term = Session::fresh(&s.id, cfg, s.size(), Cursor::start(s.epoch))
        .expect("a terminal at the spawn size");
    term.apply_all(&entries, Instant::now(), &mut Vec::new());
    term.summary().digest.expect("a running terminal")
}

/// Session `id`'s disk history, older segment first. Each segment must
/// start where the one before ended, the first at the session start, and
/// hold nothing torn.
fn read_history(dir: &Path, id: &str, epoch: u32) -> Vec<Frame> {
    let mut all: Vec<Frame> = Vec::new();
    let mut expect_start = Cursor::start(epoch);
    for name in [format!("{id}.log.1"), format!("{id}.log")] {
        let path = dir.join(&name);
        let Ok(bytes) = std::fs::read(&path) else {
            assert!(name.ends_with(".1"), "{} is missing", path.display());
            continue;
        };
        let (_, start, frames, good) =
            history::parse(&bytes).unwrap_or_else(|| panic!("{name} is not a history log"));
        assert_eq!(good, bytes.len(), "{name} ends in a torn frame");
        assert_eq!(start, expect_start, "{name} starts somewhere else");
        if let Some(f) = frames.last() {
            expect_start = Cursor {
                epoch,
                next_rseq: f.rseq + 1,
                next_offset: f.start_offset + f.record.len(),
            };
        }
        all.extend(frames);
    }
    all
}

/// Waits until every session's records are all applied: a piped session's
/// cursor at the end of what its emitter prints, a PTY session's standing
/// still for [`STILL`]. Answers each session's cursor then.
fn wait_done(v: &Vornd, sessions: &[Spawned], within: Duration) -> HashMap<String, Cursor> {
    let t = Instant::now();
    let mut last: HashMap<String, (Cursor, Instant)> = HashMap::new();
    loop {
        let report = v.get("/vornd/sessions");
        let now = Instant::now();
        let cursors = applied(&report);
        let live: Vec<&str> = report["sessions"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter(|s| s["state"] == "live")
                    .filter_map(|s| s["session"].as_str())
                    .collect()
            })
            .unwrap_or_default();
        let mut waiting = Vec::new();
        for s in sessions {
            let Some(&c) = cursors.get(&s.id) else {
                waiting.push(format!("{}: not reported", s.id));
                continue;
            };
            let since = match last.get(&s.id) {
                Some(&(prev, since)) if prev == c => since,
                _ => now,
            };
            last.insert(s.id.clone(), (c, since));
            let done = live.contains(&s.id.as_str())
                && if s.pty {
                    now.duration_since(since) >= STILL
                } else {
                    c.next_offset == s.expected.len() as u64
                };
            if !done {
                waiting.push(format!("{}: {c:?}", s.id));
            }
        }
        if waiting.is_empty() {
            return cursors;
        }
        assert!(
            t.elapsed() < within,
            "not done within {within:?}: {waiting:?}\nreport {report}\nvornd log:\n{}",
            v.log_text()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Each reported session's cursor.
fn applied(report: &Value) -> HashMap<String, Cursor> {
    let mut out = HashMap::new();
    for s in report["sessions"].as_array().into_iter().flatten() {
        let c = &s["cursor"];
        let (Some(id), Some(epoch), Some(rseq), Some(offset)) = (
            s["session"].as_str(),
            c["epoch"].as_u64().and_then(|e| u32::try_from(e).ok()),
            c["nextRseq"].as_u64(),
            c["nextOffset"].as_u64(),
        ) else {
            continue;
        };
        out.insert(
            id.to_owned(),
            Cursor {
                epoch,
                next_rseq: rseq,
                next_offset: offset,
            },
        );
    }
    out
}

fn total(cursors: &HashMap<String, Cursor>) -> u64 {
    cursors.values().map(|c| c.next_offset).sum()
}

fn pid_of(v: &Value) -> u32 {
    v["pid"]
        .as_u64()
        .and_then(|p| u32::try_from(p).ok())
        .unwrap_or_else(|| panic!("no pid in {v}"))
}

/// A binary built next to vornd. Cargo only builds the binaries of the
/// package under test, so the others have to be built first.
fn beside_vornd(name: &str) -> PathBuf {
    let dir = Path::new(VORND).parent().expect("vornd is in a directory");
    let bin = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    assert!(
        bin.exists(),
        "{} is missing: run `cargo build --release --locked -p vorn-sessiond -p vorn-recovery --bins` (drop --release for a debug test run) first",
        bin.display()
    );
    bin
}

/// An app bundle's sessiond: a copy of vorn-sessiond in `dir`, with the
/// ConPTY sideload beside it on Windows. Answers the binary.
fn bundle(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let bin = dir.join(format!("vorn-sessiond{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(beside_vornd("vorn-sessiond"), &bin).expect("copy vorn-sessiond");
    #[cfg(windows)]
    {
        let from = std::env::var_os("VORN_CONPTY_DIR")
            .map(PathBuf::from)
            .expect(
            "VORN_CONPTY_DIR must name a directory holding conpty.dll and x64\\OpenConsole.exe: \
             unpack the Microsoft.Windows.Console.ConPTY NuGet package and copy \
             runtimes/win-x64/native/conpty.dll and build/native/runtimes/x64/OpenConsole.exe \
             (arm64 likewise) into it",
        );
        let arch = if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "x64"
        };
        for (rel, required) in [
            ("conpty.dll", true),
            ("x64\\OpenConsole.exe", arch == "x64"),
            ("arm64\\OpenConsole.exe", arch == "arm64"),
        ] {
            let src = from.join(rel);
            if !src.is_file() {
                assert!(
                    !required,
                    "{} is missing from VORN_CONPTY_DIR",
                    src.display()
                );
                continue;
            }
            let to = dir.join(rel);
            std::fs::create_dir_all(to.parent().expect("a directory")).unwrap();
            std::fs::copy(&src, &to).unwrap_or_else(|e| panic!("copy {}: {e}", src.display()));
        }
    }
    bin
}

/// The ConPTY that sessiond uses is the one it was installed with: the DLL
/// sits beside the installed binary, and its console host is running.
#[cfg(windows)]
fn sideloaded_conpty_in_use(home: &Path) {
    let version = Command::new(beside_vornd("vorn-sessiond"))
        .arg("--version")
        .output()
        .expect("vorn-sessiond --version");
    let version = String::from_utf8_lossy(&version.stdout).trim().to_owned();
    let installed = launch::installed_path(home, &version);
    let dir = installed.parent().expect("an install directory");
    assert!(
        dir.join("conpty.dll").is_file(),
        "conpty.dll was not installed beside {}",
        installed.display()
    );
    let out = Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq OpenConsole.exe", "/FO", "CSV", "/NH"])
        .output()
        .expect("run tasklist");
    let text = String::from_utf8_lossy(&out.stdout);
    let hosts = text
        .lines()
        .filter(|l| {
            l.trim_start()
                .to_ascii_lowercase()
                .starts_with("\"openconsole.exe\"")
        })
        .count();
    assert!(
        hosts > 0,
        "no OpenConsole.exe is running, so the PTY sessions use the system's console host: {text}"
    );
}

/// Starts the eight sessions on the app's channel: four PTYs, four piped.
async fn spawn_all(rig: &Rig, emitter: &Path, plan: Plan) -> Vec<Spawned> {
    let vornd = rig.vornd.as_ref().expect("a vornd");
    let mut app = App::connect(&rig.home, vornd.child.id(), &vornd.log).await;
    let hello = app.call("vornd:hello", json!({})).await.expect("hello");
    assert!(hello["protocol"].as_u64().is_some(), "{hello}");
    // Room for everything, and for what a terminal adds to it.
    let ring = u32::try_from(plan.bytes.saturating_mul(4) + (1 << 20)).unwrap_or(u32::MAX);
    let mut out = Vec::new();
    for i in 0..8u64 {
        let pty = i % 2 == 0;
        let seed = Rng::derive(plan.seed, i).next_u64();
        let id = format!("gate-{}-{i}", if pty { "pty" } else { "piped" });
        let mut params = json!({
            "name": id,
            "argv": [
                emitter.to_string_lossy(),
                seed.to_string(),
                plan.bytes.to_string(),
                plan.chunk.to_string(),
                plan.pause_ms.to_string(),
            ],
            "cwd": rig.home.to_string_lossy(),
            "ringBytes": ring,
        });
        if pty {
            params["cols"] = json!(PTY_SIZE.0);
            params["rows"] = json!(PTY_SIZE.1);
        } else {
            params["piped"] = json!(true);
        }
        let r = app
            .call("vornd:spawn", params)
            .await
            .unwrap_or_else(|e| panic!("spawn {id}: {e}"));
        assert_eq!(r["id"], id.as_str(), "{r}");
        out.push(Spawned {
            id,
            pid: pid_of(&r),
            epoch: r["epoch"]
                .as_u64()
                .and_then(|e| u32::try_from(e).ok())
                .expect("an epoch"),
            pty,
            seed,
            expected: emit::output(seed, plan.bytes),
        });
    }
    out
}

/// The app's channel to one vornd ([`vornd::control`]): length-prefixed
/// frames of JSON-RPC.
struct App {
    s: Box<dyn Duplex>,
    frames: vornd::control::Frames,
    next: u64,
}

trait Duplex: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> Duplex for T {}

impl App {
    /// Connects to the endpoint vornd `pid` announced under `home`.
    async fn connect(home: &Path, pid: u32, log: &Path) -> App {
        let file = home.join("run").join(vornd::control::ANNOUNCEMENT);
        let t = Instant::now();
        loop {
            let named = std::fs::read_to_string(&file)
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .filter(|v| v["pid"].as_u64() == Some(u64::from(pid)))
                .and_then(|v| v["endpoint"].as_str().map(str::to_owned));
            if let Some(endpoint) = named {
                match vorn_sessiond::os::connect(&endpoint).await {
                    Ok(s) => {
                        return App {
                            s: Box::new(s),
                            frames: Default::default(),
                            next: 0,
                        }
                    }
                    Err(e) if t.elapsed() > PATIENCE => panic!("connect to {endpoint}: {e}"),
                    Err(_) => {}
                }
            }
            assert!(
                t.elapsed() < PATIENCE,
                "vornd {pid} announced no app endpoint in {}; its log:\n{}",
                file.display(),
                std::fs::read_to_string(log).unwrap_or_default()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// One call, answered with its result or its error's message.
    async fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next += 1;
        let id = self.next;
        let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let payload = body.to_string();
        let len = u32::try_from(payload.len() + 1).expect("a small frame");
        let mut frame = len.to_le_bytes().to_vec();
        frame.push(vornd::control::KIND_TEXT);
        frame.extend_from_slice(payload.as_bytes());
        self.s.write_all(&frame).await.expect("send to vornd");
        let mut buf = vec![0u8; 64 << 10];
        let deadline = tokio::time::Instant::now() + PATIENCE;
        loop {
            while let Some((kind, payload)) = self.frames.take().expect("well-formed frames") {
                if kind != vornd::control::KIND_TEXT {
                    continue;
                }
                let v: Value = serde_json::from_slice(&payload).expect("JSON");
                if v["id"].as_u64() != Some(id) {
                    continue;
                }
                return match v.get("error") {
                    Some(e) => Err(e["message"].as_str().unwrap_or("an error").to_owned()),
                    None => Ok(v["result"].clone()),
                };
            }
            let n = tokio::time::timeout_at(deadline, self.s.read(&mut buf))
                .await
                .unwrap_or_else(|_| panic!("no answer to {method} within {PATIENCE:?}"))
                .expect("read from vornd");
            assert!(n > 0, "vornd closed the app's channel during {method}");
            self.frames.push(&buf[..n]);
        }
    }
}

/// What sessiond holds of each session, by its program's pid: where its
/// log ends, and its records from the session start when it still retains
/// them. Asked directly, as a vornd would, once no vornd is connected.
async fn from_sessiond(
    home: &Path,
    sessions: &[Spawned],
) -> HashMap<u32, (SessionInfo, Option<Vec<Entry>>)> {
    let instance = launch::running(home)
        .into_iter()
        .next()
        .expect("a running sessiond");
    let t = Instant::now();
    let (mut s, welcome) = loop {
        match hello(&instance.endpoint).await {
            Ok(x) => break x,
            Err(e) if t.elapsed() > PATIENCE => panic!("hello to sessiond: {e}"),
            Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    };
    let mut held: HashMap<u32, (SessionInfo, Option<Vec<Entry>>)> = HashMap::new();
    let mut ids = HashMap::new();
    for info in welcome {
        if sessions.iter().any(|x| x.pid == info.pid) {
            ids.insert(info.session.clone(), info.pid);
            s.send(ToSessiond::Attach(Attach {
                session: info.session.clone(),
                from: AttachFrom::SessionStart,
            }))
            .await;
            held.insert(info.pid, (info, Some(Vec::new())));
        }
    }
    let complete = |held: &HashMap<u32, (SessionInfo, Option<Vec<Entry>>)>| {
        held.values().all(|(info, log)| {
            log.as_ref().is_none_or(|l| {
                l.last().map_or(Cursor::start(info.epoch), Entry::after) == info.head
            })
        })
    };
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while !complete(&held) {
        let m = tokio::time::timeout_at(deadline, s.recv())
            .await
            .expect("sessiond answers every attach in time")
            .expect("sessiond stays connected");
        match m {
            ToVornd::Entries(e) => {
                let Some(pid) = ids.get(&e.session) else {
                    continue;
                };
                if let Some((_, Some(log))) = held.get_mut(pid) {
                    for entry in e.entries {
                        // From rseq 0 on, each once, should an answer
                        // repeat itself.
                        let next = log.last().map_or(0, |l| l.hdr.rseq + 1);
                        if entry.hdr.rseq == next {
                            log.push(entry);
                        }
                    }
                }
            }
            // No longer retained from the start: the history alone is
            // checked.
            ToVornd::Refused(r) => {
                if let Some((_, log)) = ids.get(&r.session).and_then(|p| held.get_mut(p)) {
                    *log = None;
                }
            }
            _ => {}
        }
    }
    held
}

/// A bare connection to sessiond, as vornd opens one.
struct Wire {
    s: Box<dyn Duplex>,
    frames: FrameReader,
    buf: Vec<u8>,
}

impl Wire {
    async fn send(&mut self, m: ToSessiond) {
        self.s
            .write_all(&m.encode())
            .await
            .expect("send to sessiond");
    }

    async fn recv(&mut self) -> Option<ToVornd> {
        loop {
            if let Some(m) = self.frames.read::<ToVornd>().expect("well-formed") {
                return Some(m);
            }
            let n = self.s.read(&mut self.buf).await.unwrap_or(0);
            if n == 0 {
                return None;
            }
            self.frames.push(&self.buf[..n]);
        }
    }
}

async fn hello(endpoint: &str) -> std::io::Result<(Wire, Vec<SessionInfo>)> {
    let s = vorn_sessiond::os::connect(endpoint).await?;
    let mut w = Wire {
        s: Box::new(s),
        frames: FrameReader::default(),
        buf: vec![0u8; 256 << 10],
    };
    w.send(ToSessiond::Hello(Hello {
        proto_min: 1,
        proto_max: PROTO,
        vornd_instance: 0x6a7e_7e57,
        vornd_build: "gate".into(),
    }))
    .await;
    match tokio::time::timeout(PATIENCE, w.recv()).await {
        Ok(Some(ToVornd::Welcome(welcome))) => Ok((w, welcome.sessions)),
        Ok(other) => Err(std::io::Error::other(format!("not welcomed: {other:?}"))),
        Err(_) => Err(std::io::Error::other("no welcome")),
    }
}

/// What the test started under one home, ended on drop however the test
/// ends: the vornd, then every sessiond (so no vornd starts another), then
/// the sessions' programs, which a piped session's would outlive.
struct Rig {
    home: PathBuf,
    sessiond: PathBuf,
    vornd: Option<Vornd>,
    pids: Vec<u32>,
}

impl Rig {
    fn start_vornd(&self) -> Vornd {
        Vornd::start(&self.home, &self.sessiond)
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        drop(self.vornd.take());
        let mut pids: Vec<u32> = launch::running(&self.home).iter().map(|i| i.pid).collect();
        pids.extend(&self.pids);
        for &pid in &pids {
            if launch::alive(pid) {
                let _ = launch::kill(pid);
            }
        }
        // Gone before the temporary directory is removed: Windows keeps a
        // running binary's file, and a dying process's, from being deleted.
        let t = Instant::now();
        while pids.iter().any(|&p| launch::alive(p)) && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// A running vornd with a session holder and the engine under `home`.
/// Killed on drop.
struct Vornd {
    child: Child,
    port: u16,
    log: PathBuf,
}

impl Vornd {
    fn start(home: &Path, sessiond: &Path) -> Vornd {
        let log = home.join("vornd.log");
        let mut cmd = Command::new(VORND);
        // Nothing listens on the discard port: the holder does not need the
        // Node server.
        cmd.args([
            "--upstream",
            "127.0.0.1:9",
            "--exit-with-stdin",
            "--sessiond",
        ])
        .arg(sessiond)
        .arg("--home")
        .arg(home)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .arg("--log-file")
        .arg(&log)
        .env_remove("VORND_GROUPS")
        .env_remove("VORN_SESSIOND_IDLE_EXIT")
        .env("VORND_LOG", "info")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
        let mut child = cmd.spawn().expect("start vornd");
        let stdout = child.stdout.take().expect("piped stdout");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let _ = BufReader::new(stdout).read_line(&mut line);
            let _ = tx.send(line);
        });
        let mut v = Vornd {
            child,
            port: 0,
            log,
        };
        let line = rx
            .recv_timeout(PATIENCE)
            .unwrap_or_else(|_| panic!("vornd printed no port: {}", v.log_text()));
        let ready: Value = serde_json::from_str(line.trim())
            .unwrap_or_else(|e| panic!("port line {line:?}: {e}; {}", v.log_text()));
        v.port = ready["port"]
            .as_u64()
            .and_then(|p| u16::try_from(p).ok())
            .expect("a port");
        v
    }

    fn log_text(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// The JSON body vornd answers a GET of `path` with.
    fn get(&self, path: &str) -> Value {
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).expect("connect to vornd");
        s.set_read_timeout(Some(PATIENCE)).unwrap();
        let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
        s.write_all(req.as_bytes()).expect("send");
        let mut res = String::new();
        s.read_to_string(&mut res).expect("read the answer");
        let (_, body) = res.split_once("\r\n\r\n").expect("a body");
        serde_json::from_str(body).unwrap_or_else(|e| panic!("{path} body {body:?}: {e}"))
    }

    /// Waits until the engine is connected to a current sessiond; answers
    /// the holder's part of the health check then.
    fn wait_connected(&self) -> Value {
        let t = Instant::now();
        loop {
            let health = self.get("/vornd/health");
            let sessions = self.get("/vornd/sessions");
            if health["sessiond"]["current"].is_object() && sessions["connected"] == true {
                return health["sessiond"].clone();
            }
            assert!(
                t.elapsed() < PATIENCE,
                "not connected: {health} {sessions}\nvornd log:\n{}",
                self.log_text()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Ends it with the OS: SIGKILL, or TerminateProcess.
    fn kill(&mut self) {
        self.child.kill().expect("kill vornd");
        self.child.wait().expect("reap vornd");
    }

    /// Stops it the way the app does, by closing its stdin.
    fn stop(mut self) {
        drop(self.child.stdin.take());
        let t = Instant::now();
        while t.elapsed() < PATIENCE {
            if self.child.try_wait().expect("wait for vornd").is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("vornd did not stop: {}", self.log_text());
    }
}

impl Drop for Vornd {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
