//! Handing live sessions from one sessiond to a newer one in the same home:
//! the sessions carry on byte-exact, and any failure leaves them all where
//! they were.
#![cfg(unix)]

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use vorn_sessiond::server::{self, Config, Fault, Sessiond, Step};
use vorn_sessiond::wire::*;
use vorn_term_proto::{Cursor, Entry, Record, Stream};

/// A test vornd: keeps every session's records as they come, acking them.
struct Client {
    sock: UnixStream,
    frames: FrameReader,
    logs: HashMap<String, Vec<Entry>>,
    written: Vec<u64>,
}

impl Client {
    async fn hello(d: &Sessiond) -> (Client, Welcome) {
        let sock = UnixStream::connect(d.endpoint()).await.expect("connect");
        let mut c = Client {
            sock,
            frames: FrameReader::default(),
            logs: HashMap::new(),
            written: Vec::new(),
        };
        c.send(ToSessiond::Hello(Hello {
            proto_min: PROTO,
            proto_max: PROTO,
            vornd_instance: 1,
            vornd_build: "test".into(),
        }))
        .await;
        match c.recv().await {
            Some(ToVornd::Welcome(w)) => (c, w),
            other => panic!("expected Welcome, got {other:?}"),
        }
    }

    async fn send(&mut self, m: ToSessiond) {
        self.sock.write_all(&m.encode()).await.expect("send");
    }

    /// The next message, or None once sessiond closed the connection.
    async fn recv(&mut self) -> Option<ToVornd> {
        let mut buf = vec![0u8; 64 << 10];
        loop {
            if let Some(m) = self.frames.read::<ToVornd>().expect("well-formed") {
                match &m {
                    ToVornd::Entries(e) => {
                        let last = e.entries.last().expect("non-empty").after();
                        self.logs
                            .entry(e.session.clone())
                            .or_default()
                            .extend(e.entries.iter().cloned());
                        let ack = ToSessiond::Ack(Ack {
                            session: e.session.clone(),
                            delivered: last,
                        });
                        self.send(ack).await;
                    }
                    ToVornd::InputDone(i) => self.written.push(i.input_seq),
                    _ => {}
                }
                return Some(m);
            }
            let n = tokio::time::timeout(Duration::from_secs(30), self.sock.read(&mut buf))
                .await
                .expect("sessiond answers within 30 s")
                .unwrap_or(0);
            if n == 0 {
                return None;
            }
            self.frames.push(&buf[..n]);
        }
    }

    async fn spawn(&mut self, argv: &str, io: Io) -> String {
        self.send(ToSessiond::Spawn(Spawn {
            req: 1,
            spec: SpawnSpec {
                argv: vec!["sh".into(), "-c".into(), argv.into()],
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                env: Vec::new(),
                io,
                ring_bytes: None,
            },
        }))
        .await;
        loop {
            match self.recv().await.expect("open") {
                ToVornd::Spawned(s) => return s.session,
                ToVornd::Failed(f) => panic!("spawn failed: {}", f.error),
                _ => {}
            }
        }
    }

    async fn attach(&mut self, session: &str, from: AttachFrom) {
        self.send(ToSessiond::Attach(Attach {
            session: session.into(),
            from,
        }))
        .await;
    }

    /// Write `bytes` and wait until the kernel took them.
    async fn write(&mut self, session: &str, seq: u64, bytes: &[u8]) {
        self.send(ToSessiond::Write(Write {
            session: session.into(),
            input_seq: seq,
            bytes: bytes.to_vec(),
        }))
        .await;
        while !self.written.contains(&seq) {
            self.recv().await.expect("open");
        }
    }

    /// Read until `session`'s output so far contains `text`.
    async fn until_text(&mut self, session: &str, text: &str) {
        while !String::from_utf8_lossy(&bytes(self.log(session), None)).contains(text) {
            self.recv().await.expect("open");
        }
    }

    async fn until_exit(&mut self, session: &str) {
        while exit_of(self.log(session)).is_none() {
            self.recv().await.expect("open");
        }
    }

    async fn adopt(&mut self, from: &str) -> Result<Vec<String>, String> {
        self.send(ToSessiond::Adopt(Adopt {
            req: 9,
            from: from.into(),
        }))
        .await;
        loop {
            match self.recv().await.expect("open") {
                ToVornd::Adopted(a) => return Ok(a.sessions),
                ToVornd::Failed(f) if f.req == 9 => return Err(f.error),
                _ => {}
            }
        }
    }

    fn log(&self, session: &str) -> &[Entry] {
        self.logs.get(session).map_or(&[], Vec::as_slice)
    }
}

fn start_in(home: &Path) -> (Arc<Sessiond>, tokio::task::JoinHandle<std::io::Result<()>>) {
    let d = Sessiond::new(Config {
        home: home.to_path_buf(),
        instance: instance(),
        build: "test".into(),
        idle_exit: Duration::from_secs(60),
        spool_cap: 64 << 20,
    });
    let listener = server::bind(&d).unwrap();
    let task = tokio::spawn(server::serve(Arc::clone(&d), listener));
    (d, task)
}

fn instance() -> u128 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    t ^ u128::from(NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst))
}

fn bytes(entries: &[Entry], stream: Option<Stream>) -> Vec<u8> {
    let mut out = Vec::new();
    for e in entries {
        if let Record::Data { stream: s, bytes } = &e.rec {
            if stream.is_none_or(|w| w == *s) {
                out.extend(bytes);
            }
        }
    }
    out
}

fn exit_of(entries: &[Entry]) -> Option<(Option<i32>, Option<i32>)> {
    entries.iter().find_map(|e| match e.rec {
        Record::Exit { code, signal } => Some((code, signal)),
        _ => None,
    })
}

/// Numbered from 0 with no holes, no Gap, and nothing after Exit.
fn assert_whole(entries: &[Entry]) {
    let mut at = Cursor::start(0);
    for e in entries {
        assert!(at.is_followed_by(&e.hdr), "{at:?} then {:?}", e.hdr);
        assert!(!matches!(e.rec, Record::Gap { .. }), "a gap at {:?}", e.hdr);
        at = e.after();
    }
    assert!(matches!(
        entries.last().map(|e| &e.rec),
        Some(Record::Exit { .. })
    ));
}

const SHELL: &str = r#"while read l; do echo "got:$l"; done; exit 3"#;
const AGENT: &str = r#"while read l; do echo "out:$l"; echo "err:$l" >&2; done; exit 5"#;

/// Three sessions on `a`: a terminal and a piped agent that each answered
/// one line, and an agent that already exited.
async fn three_sessions(v: &mut Client) -> (String, String, String) {
    let pty = v.spawn(SHELL, Io::Pty { cols: 80, rows: 24 }).await;
    let piped = v.spawn(AGENT, Io::Piped { stdin: Stdin::Pipe }).await;
    let done = v
        .spawn("printf bye; exit 9", Io::Piped { stdin: Stdin::Null })
        .await;
    for id in [&pty, &piped, &done] {
        v.attach(id, AttachFrom::SessionStart).await;
    }
    v.write(&pty, 1, b"one\n").await;
    v.write(&piped, 2, b"one\n").await;
    v.until_text(&pty, "got:one").await;
    v.until_text(&piped, "err:one").await;
    v.until_text(&piped, "out:one").await;
    v.until_exit(&done).await;
    (pty, piped, done)
}

/// Line two, then the end of input, for both live sessions on `v`.
async fn finish(v: &mut Client, pty: &str, piped: &str) {
    v.write(pty, 11, b"two\n").await;
    v.write(piped, 12, b"two\n").await;
    v.until_text(pty, "got:two").await;
    v.write(pty, 13, b"\x04").await;
    v.send(ToSessiond::CloseStdin(SessionRef {
        session: piped.into(),
    }))
    .await;
    v.until_exit(pty).await;
    v.until_exit(piped).await;
}

#[tokio::test]
async fn live_sessions_move_to_the_newer_sessiond_and_carry_on_byte_exact() {
    let home = tempfile::tempdir().unwrap();
    let (a, a_task) = start_in(home.path());
    let (mut va, _) = Client::hello(&a).await;
    let (pty, piped, done) = three_sessions(&mut va).await;

    let (b, _b_task) = start_in(home.path());
    let (mut vb, _) = Client::hello(&b).await;
    let mut took = vb.adopt(&a.endpoint()).await.expect("adopted");
    took.sort();
    let mut want = vec![pty.clone(), piped.clone(), done.clone()];
    want.sort();
    assert_eq!(took, want);
    // The donor marks itself handed off just after the adopter's Took reaches it.
    tokio::time::timeout(Duration::from_secs(5), async {
        while !a.handed_off() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("handed off");
    assert!(!a.holds(&pty) && b.holds(&pty));

    // The older sessiond closed its vornd connection and its endpoint.
    while va.recv().await.is_some() {}
    assert!(UnixStream::connect(a.endpoint()).await.is_err());

    // The newer one carries on from the cursor vornd reached, with no
    // snapshot in between.
    let (mut vc, w) = Client::hello(&b).await;
    assert_eq!(w.sessions.len(), 3);
    for id in [&pty, &piped] {
        let at = va.log(id).last().unwrap().after();
        vc.attach(id, AttachFrom::Cursor(at)).await;
    }
    finish(&mut vc, &pty, &piped).await;

    for (id, code) in [(&pty, 3), (&piped, 5)] {
        let mut joined = va.log(id).to_vec();
        joined.extend_from_slice(vc.log(id));
        assert_whole(&joined);
        // The program's own exit status, reaped by the older sessiond.
        assert_eq!(exit_of(&joined), Some((Some(code), None)));

        // Read again from the start, the newer sessiond's log is the one the
        // older sessiond started, record for record.
        let (mut vd, _) = Client::hello(&b).await;
        vd.attach(id, AttachFrom::SessionStart).await;
        vd.until_exit(id).await;
        assert_eq!(vd.log(id), joined.as_slice());
        drop(vd);
    }
    let agent = [va.log(&piped), vc.log(&piped)].concat();
    assert_eq!(bytes(&agent, Some(Stream::Stdout)), b"out:one\nout:two\n");
    assert_eq!(bytes(&agent, Some(Stream::Stderr)), b"err:one\nerr:two\n");

    // The session that had ended before is there too, as it was.
    let (mut ve, _) = Client::hello(&b).await;
    ve.attach(&done, AttachFrom::SessionStart).await;
    ve.until_exit(&done).await;
    assert_eq!(ve.log(&done), va.log(&done));

    // Once its programs are reaped the older sessiond is gone, and every
    // exit it wrote down was taken.
    tokio::time::timeout(Duration::from_secs(10), a_task)
        .await
        .expect("the older sessiond exits")
        .unwrap()
        .unwrap();
    let exits = home.path().join("run").join("exits");
    assert_eq!(std::fs::read_dir(&exits).map_or(0, |d| d.count()), 0);
}

#[tokio::test]
async fn a_failure_at_any_step_leaves_every_session_with_the_older_sessiond() {
    let donor = [Step::Freeze, Step::Offer, Step::Send, Step::Commit];
    let adopter = [Step::Hello, Step::Stage, Step::Ready, Step::Took];
    let faults = donor
        .iter()
        .map(|&s| (s, true))
        .chain(adopter.iter().map(|&s| (s, false)));
    for (step, on_donor) in faults {
        let home = tempfile::tempdir().unwrap();
        let (a, _a_task) = start_in(home.path());
        let (mut va, _) = Client::hello(&a).await;
        let (pty, piped, done) = three_sessions(&mut va).await;

        let (b, _b_task) = start_in(home.path());
        let fault = Fault { step, stall: false };
        if on_donor {
            a.inject(fault);
        } else {
            b.inject(fault);
        }
        let (mut vb, _) = Client::hello(&b).await;
        let err = vb.adopt(&a.endpoint()).await.expect_err("refused");
        assert!(!err.is_empty(), "{step:?}");
        assert!(!a.handed_off(), "{step:?}");
        for id in [&pty, &piped, &done] {
            assert!(a.holds(id) && !b.holds(id), "{step:?}: {id}");
        }

        // The older sessiond's vornd carries on as if nothing happened.
        finish(&mut va, &pty, &piped).await;
        for (id, code) in [(&pty, 3), (&piped, 5)] {
            assert_whole(va.log(id));
            assert_eq!(exit_of(va.log(id)), Some((Some(code), None)), "{step:?}");
        }
    }
}

#[tokio::test]
async fn a_handoff_in_another_version_is_refused_and_changes_nothing() {
    let home = tempfile::tempdir().unwrap();
    let (a, _a_task) = start_in(home.path());
    let (mut va, _) = Client::hello(&a).await;
    let pty = va.spawn(SHELL, Io::Pty { cols: 80, rows: 24 }).await;
    va.attach(&pty, AttachFrom::SessionStart).await;

    let mut s = UnixStream::connect(a.endpoint()).await.unwrap();
    let hello = ToSessiond::Handoff(HandoffHello {
        version_min: HANDOFF + 1,
        version_max: HANDOFF + 1,
        instance: 1,
        build: "later".into(),
    });
    s.write_all(&hello.encode()).await.unwrap();
    let mut frames = FrameReader::default();
    let mut buf = vec![0u8; 4096];
    let reply = loop {
        if let Some(m) = frames.read::<ToAdopter>().unwrap() {
            break m;
        }
        let n = s.read(&mut buf).await.unwrap();
        assert_ne!(n, 0, "closed without an answer");
        frames.push(&buf[..n]);
    };
    assert!(matches!(reply, ToAdopter::Refuse(_)), "{reply:?}");
    assert!(!a.handed_off() && a.holds(&pty));

    va.write(&pty, 1, b"one\n").await;
    va.until_text(&pty, "got:one").await;
}

#[tokio::test]
async fn a_sessiond_does_not_adopt_from_itself() {
    let home = tempfile::tempdir().unwrap();
    let (a, _a_task) = start_in(home.path());
    let (mut va, _) = Client::hello(&a).await;
    let err = va.adopt(&a.endpoint()).await.expect_err("refused");
    assert!(err.contains("own"), "{err}");
    assert!(va.adopt("/nonexistent/sessiond.sock").await.is_err());
}
