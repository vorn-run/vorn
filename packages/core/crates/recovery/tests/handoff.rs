//! A newer sessiond killed by the OS in the middle of adopting an older
//! one's sessions: the older one keeps every session, and every byte the
//! programs print reaches its log, in order, with no gap.
//!
//! Runs the real vorn-sessiond binary, which cargo builds for the tests of
//! vorn-sessiond: run `cargo build -p vorn-sessiond` with the same profile
//! first when testing this crate alone.
#![cfg(unix)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use vorn_recovery::emit;
use vorn_sessiond_wire::{
    Ack, Adopt, Attach, AttachFrom, Hello, Io, Message, Spawn, SpawnSpec, Stdin, ToSessiond,
    ToVornd, PROTO,
};
use vorn_term_proto::{Cursor, Entry, Record, Stream};

const EMIT: &str = env!("CARGO_BIN_EXE_recovery-emit");
const PATIENCE: Duration = Duration::from_secs(30);
/// About a second of output in 1 KiB pieces, so the handoff starts while
/// every program is still printing.
const BYTES: u64 = 256 << 10;
const CHUNK: &str = "1024";
const PAUSE_MS: &str = "4";

fn sessiond_bin() -> PathBuf {
    let dir = Path::new(EMIT)
        .parent()
        .expect("recovery-emit is in a directory");
    let bin = dir.join("vorn-sessiond");
    assert!(
        bin.exists(),
        "{} is missing: run `cargo build -p vorn-sessiond` with the same profile first",
        bin.display()
    );
    bin
}

/// A vorn-sessiond process under `home`, killed on drop.
struct Holder {
    child: Child,
    endpoint: String,
    stderr: mpsc::Receiver<String>,
}

impl Holder {
    fn start(home: &Path, fault: Option<&str>) -> Holder {
        let mut cmd = Command::new(sessiond_bin());
        cmd.arg("--home")
            .arg(home)
            .args(["--idle-exit", "600"])
            .env_remove("VORN_HOME")
            .env_remove("VORN_SESSIOND_HANDOFF_FAULT")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(f) = fault {
            cmd.env("VORN_SESSIOND_HANDOFF_FAULT", f);
        }
        let mut child = cmd.spawn().expect("start vorn-sessiond");
        let mut line = String::new();
        BufReader::new(child.stdout.take().expect("piped"))
            .read_line(&mut line)
            .expect("its first line");
        let endpoint = line
            .trim()
            .strip_prefix("listening ")
            .unwrap_or_else(|| panic!("expected `listening <endpoint>`, got {line:?}"))
            .to_owned();
        let (tx, stderr) = mpsc::channel();
        let err = child.stderr.take().expect("piped");
        std::thread::spawn(move || {
            for l in BufReader::new(err).lines().map_while(Result::ok) {
                let _ = tx.send(l);
            }
        });
        Holder {
            child,
            endpoint,
            stderr,
        }
    }

    fn wait_for_stderr(&self, text: &str) {
        let t = Instant::now();
        while t.elapsed() < PATIENCE {
            if let Ok(l) = self.stderr.recv_timeout(Duration::from_millis(100)) {
                if l.contains(text) {
                    return;
                }
            }
        }
        panic!("sessiond never said {text:?}");
    }
}

impl Drop for Holder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A blocking vornd: keeps each session's records and acks them.
struct Client {
    sock: UnixStream,
    frames: vorn_sessiond_wire::FrameReader,
    logs: HashMap<String, Vec<Entry>>,
}

impl Client {
    fn hello(endpoint: &str) -> Client {
        let sock = UnixStream::connect(endpoint).expect("connect");
        sock.set_read_timeout(Some(PATIENCE)).unwrap();
        let mut c = Client {
            sock,
            frames: Default::default(),
            logs: HashMap::new(),
        };
        c.send(&ToSessiond::Hello(Hello {
            proto_min: PROTO,
            proto_max: PROTO,
            vornd_instance: 7,
            vornd_build: "test".into(),
        }));
        match c.recv() {
            Some(ToVornd::Welcome(_)) => c,
            other => panic!("expected Welcome, got {other:?}"),
        }
    }

    fn send(&mut self, m: &ToSessiond) {
        self.sock.write_all(&m.encode()).expect("send");
    }

    /// The next message, or None once sessiond closed the connection.
    fn recv(&mut self) -> Option<ToVornd> {
        let mut buf = vec![0u8; 64 << 10];
        loop {
            if let Some(m) = self.frames.read::<ToVornd>().expect("well-formed") {
                if let ToVornd::Entries(e) = &m {
                    let delivered = e.entries.last().expect("non-empty").after();
                    self.logs
                        .entry(e.session.clone())
                        .or_default()
                        .extend(e.entries.iter().cloned());
                    self.send(&ToSessiond::Ack(Ack {
                        session: e.session.clone(),
                        delivered,
                    }));
                }
                return Some(m);
            }
            match self.sock.read(&mut buf) {
                Ok(0) => return None,
                Ok(n) => self.frames.push(&buf[..n]),
                Err(e) => panic!("sessiond did not answer: {e}"),
            }
        }
    }

    fn spawn(&mut self, seed: u64, io: Io) -> String {
        self.send(&ToSessiond::Spawn(Spawn {
            req: seed,
            spec: SpawnSpec {
                argv: vec![
                    EMIT.into(),
                    seed.to_string(),
                    BYTES.to_string(),
                    CHUNK.into(),
                    PAUSE_MS.into(),
                ],
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                env: Vec::new(),
                io,
                ring_bytes: None,
            },
        }));
        let id = loop {
            match self.recv().expect("open") {
                ToVornd::Spawned(s) => break s.session,
                ToVornd::Failed(f) => panic!("spawn failed: {}", f.error),
                _ => {}
            }
        };
        self.send(&ToSessiond::Attach(Attach {
            session: id.clone(),
            from: AttachFrom::SessionStart,
        }));
        id
    }

    /// What the program printed to its terminal or standard output.
    fn stdout(&self, session: &str) -> Vec<u8> {
        let mut out = Vec::new();
        for e in self.logs.get(session).into_iter().flatten() {
            if let Record::Data {
                stream: Stream::Stdout | Stream::Pty,
                bytes,
            } = &e.rec
            {
                out.extend(bytes);
            }
        }
        out
    }
}

/// `bytes` without carriage returns: a terminal's line discipline adds them
/// to newlines as it sees fit, so a terminal's output is compared without.
fn without_cr(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().copied().filter(|&b| b != b'\r').collect()
}

/// Numbered from 0 with no holes and no Gap.
fn assert_whole(id: &str, entries: &[Entry]) {
    let mut at = Cursor::start(0);
    for e in entries {
        assert!(at.is_followed_by(&e.hdr), "{id}: {at:?} then {:?}", e.hdr);
        assert!(
            !matches!(e.rec, Record::Gap { .. }),
            "{id}: a gap at {:?}",
            e.hdr
        );
        at = e.after();
    }
}

#[test]
fn a_sessiond_killed_while_adopting_loses_nothing() {
    for (n, step) in ["stage", "ready", "took"].into_iter().enumerate() {
        let home = tempfile::tempdir().unwrap();
        let old = Holder::start(home.path(), None);
        let mut v = Client::hello(&old.endpoint);
        let mut sessions = Vec::new();
        for k in 0..4u64 {
            let seed = 100 * n as u64 + k;
            let io = if k % 2 == 0 {
                Io::Pty { cols: 80, rows: 24 }
            } else {
                Io::Piped { stdin: Stdin::Pipe }
            };
            let pty = matches!(io, Io::Pty { .. });
            sessions.push((v.spawn(seed, io), seed, pty));
        }
        // Some output first, so the handoff starts in the middle of it.
        while sessions.iter().any(|(id, ..)| v.stdout(id).len() < 4096) {
            v.recv().expect("open");
        }

        let new = Holder::start(home.path(), Some(&format!("{step}:stall")));
        let mut adopter = Client::hello(&new.endpoint);
        adopter.send(&ToSessiond::Adopt(Adopt {
            req: 1,
            from: old.endpoint.clone(),
        }));
        new.wait_for_stderr(&format!("handoff stalled at {step}"));
        drop(new);
        assert!(adopter.recv().is_none(), "{step}: the adopter is gone");

        // The older sessiond's vornd reads on, and every program's output
        // arrives whole.
        let want: Vec<Vec<u8>> = sessions
            .iter()
            .map(|&(_, seed, pty)| {
                let out = emit::output(seed, BYTES);
                if pty {
                    without_cr(&out)
                } else {
                    out
                }
            })
            .collect();
        let t = Instant::now();
        let printed = |v: &Client, id: &str, pty: bool| {
            let out = v.stdout(id);
            if pty {
                without_cr(&out)
            } else {
                out
            }
        };
        while sessions
            .iter()
            .zip(&want)
            .any(|(&(ref id, _, pty), w)| printed(&v, id, pty).len() < w.len())
        {
            assert!(t.elapsed() < PATIENCE * 2, "{step}: output stopped");
            v.recv().expect("the older sessiond keeps its vornd");
        }
        for ((id, seed, pty), w) in sessions.iter().zip(&want) {
            assert_whole(id, &v.logs[id]);
            let got = printed(&v, id, *pty);
            let at = got
                .iter()
                .zip(w)
                .position(|(a, b)| a != b)
                .unwrap_or(got.len().min(w.len()));
            assert!(
                got == *w,
                "{step}: {id} (seed {seed}) recorded {} bytes, printed {}; they part at {at}: {:?} vs {:?}",
                got.len(),
                w.len(),
                String::from_utf8_lossy(&got[at.saturating_sub(20)..(at + 20).min(got.len())]),
                String::from_utf8_lossy(&w[at.saturating_sub(20)..(at + 20).min(w.len())]),
            );
        }
    }
}
