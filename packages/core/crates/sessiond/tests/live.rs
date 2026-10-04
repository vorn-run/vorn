//! sessiond end to end: real processes, the real endpoint, a test vornd.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use vorn_sessiond::os;
use vorn_sessiond::server::{self, Config, Sessiond};
use vorn_sessiond::wire::*;
use vorn_term_proto::{Cursor, Entry, Record, Stream};

#[cfg(unix)]
type Conn = tokio::net::UnixStream;
#[cfg(windows)]
type Conn = tokio::net::windows::named_pipe::NamedPipeClient;

struct Vornd {
    rd: ReadHalf<Conn>,
    wr: WriteHalf<Conn>,
    frames: FrameReader,
}

impl Vornd {
    async fn connect(d: &Sessiond) -> Vornd {
        let s = os::connect(&d.endpoint()).await.expect("connect");
        let (rd, wr) = tokio::io::split(s);
        Vornd {
            rd,
            wr,
            frames: FrameReader::default(),
        }
    }

    async fn hello(d: &Sessiond) -> (Vornd, Welcome) {
        let mut v = Vornd::connect(d).await;
        v.send(ToSessiond::Hello(Hello {
            proto_min: 1,
            proto_max: 1,
            vornd_instance: 1,
            vornd_build: "test".into(),
        }))
        .await;
        match v.recv().await {
            Some(ToVornd::Welcome(w)) => (v, w),
            other => panic!("expected Welcome, got {other:?}"),
        }
    }

    async fn send(&mut self, m: ToSessiond) {
        self.wr.write_all(&m.encode()).await.expect("send");
    }

    /// The next message, or None when sessiond closed the connection.
    async fn recv(&mut self) -> Option<ToVornd> {
        let mut buf = vec![0u8; 64 << 10];
        loop {
            if let Some(m) = self.frames.read::<ToVornd>().expect("well-formed") {
                return Some(m);
            }
            let n = tokio::time::timeout(Duration::from_secs(20), self.rd.read(&mut buf))
                .await
                .expect("sessiond answers within 20 s")
                .unwrap_or(0);
            if n == 0 {
                return None;
            }
            self.frames.push(&buf[..n]);
        }
    }

    async fn spawn(&mut self, argv: &[&str], io: Io) -> String {
        self.send(ToSessiond::Spawn(Spawn {
            req: 1,
            spec: SpawnSpec {
                argv: argv.iter().map(|s| s.to_string()).collect(),
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                env: Vec::new(),
                io,
                ring_bytes: None,
            },
        }))
        .await;
        loop {
            match self.recv().await {
                Some(ToVornd::Spawned(s)) => return s.session,
                Some(ToVornd::Failed(f)) => panic!("spawn failed: {}", f.error),
                Some(_) => continue,
                None => panic!("closed"),
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

    /// Records for `session` until Exit, acking as they come.
    async fn until_exit(&mut self, session: &str) -> Vec<Entry> {
        let mut out = Vec::new();
        loop {
            match self.recv().await {
                Some(ToVornd::Entries(e)) if e.session == session => {
                    let last = e.entries.last().expect("non-empty").after();
                    let done = e
                        .entries
                        .iter()
                        .any(|x| matches!(x.rec, Record::Exit { .. }));
                    out.extend(e.entries);
                    self.send(ToSessiond::Ack(Ack {
                        session: session.into(),
                        delivered: last,
                    }))
                    .await;
                    if done {
                        return out;
                    }
                }
                Some(_) => {}
                None => panic!("closed before Exit"),
            }
        }
    }
}

async fn start(
    idle: Duration,
) -> (
    Arc<Sessiond>,
    tempfile::TempDir,
    tokio::task::JoinHandle<std::io::Result<()>>,
) {
    let home = tempfile::tempdir().unwrap();
    let d = Sessiond::new(Config {
        home: home.path().to_path_buf(),
        instance: rand_instance(),
        build: "test".into(),
        idle_exit: idle,
        spool_cap: 64 << 20,
    });
    let listener = server::bind(&d).unwrap();
    let task = tokio::spawn(server::serve(Arc::clone(&d), listener));
    (d, home, task)
}

fn rand_instance() -> u128 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    t ^ (u128::from(std::process::id()) << 64)
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

fn exit_code(entries: &[Entry]) -> Option<i32> {
    match entries.last().map(|e| &e.rec) {
        Some(Record::Exit { code, .. }) => *code,
        other => panic!("last record is not Exit: {other:?}"),
    }
}

/// Records numbered from 0 with no holes and offsets that add up.
fn assert_contiguous(entries: &[Entry]) {
    let mut at = Cursor::start(0);
    for e in entries {
        assert!(at.is_followed_by(&e.hdr), "{at:?} then {:?}", e.hdr);
        at = e.after();
    }
}

#[cfg(unix)]
fn loop_lines(n: u32) -> Vec<String> {
    vec![
        "sh".into(),
        "-c".into(),
        format!("i=0; while [ $i -lt {n} ]; do echo line$i; i=$((i+1)); done; exit 7"),
    ]
}

#[cfg(windows)]
fn loop_lines(n: u32) -> Vec<String> {
    vec![
        "cmd.exe".into(),
        "/c".into(),
        format!("(for /L %i in (0,1,{}) do @echo line%i) & exit /b 7", n - 1),
    ]
}

/// RC-T13: a piped agent's stdout and stderr arrive as their own streams,
/// stdin takes input and then EOF, and the exit code comes last.
#[cfg(unix)]
#[tokio::test]
async fn a_piped_agent_keeps_its_streams_and_exit_code() {
    let (d, _home, _t) = start(Duration::from_secs(60)).await;
    let (mut v, w) = Vornd::hello(&d).await;
    assert!(w.sessions.is_empty());
    let id = v
        .spawn(
            &[
                "sh",
                "-c",
                "printf out; printf err >&2; read x; printf got-$x; cat >/dev/null; exit 7",
            ],
            Io::Piped { stdin: Stdin::Pipe },
        )
        .await;
    v.attach(&id, AttachFrom::SessionStart).await;
    v.send(ToSessiond::Write(Write {
        session: id.clone(),
        input_seq: 1,
        bytes: b"hi\n".to_vec(),
    }))
    .await;
    v.send(ToSessiond::CloseStdin(SessionRef {
        session: id.clone(),
    }))
    .await;
    let mut written = None;
    let mut entries = Vec::new();
    loop {
        match v.recv().await.expect("open") {
            ToVornd::InputDone(i) => written = Some((i.input_seq, i.written)),
            ToVornd::Entries(e) => {
                let done = e
                    .entries
                    .iter()
                    .any(|x| matches!(x.rec, Record::Exit { .. }));
                entries.extend(e.entries);
                if done {
                    break;
                }
            }
            _ => {}
        }
    }
    assert_eq!(bytes(&entries, Some(Stream::Stdout)), b"outgot-hi");
    assert_eq!(bytes(&entries, Some(Stream::Stderr)), b"err");
    assert_eq!(exit_code(&entries), Some(7));
    assert_eq!(written, Some((1, 3)));
    assert_contiguous(&entries);
}

#[cfg(windows)]
#[tokio::test]
async fn a_piped_agent_keeps_its_streams_and_exit_code() {
    let (d, _home, _t) = start(Duration::from_secs(60)).await;
    let (mut v, _) = Vornd::hello(&d).await;
    let id = v
        .spawn(
            &["cmd.exe", "/c", "echo out& echo err 1>&2& exit /b 7"],
            Io::Piped { stdin: Stdin::Null },
        )
        .await;
    v.attach(&id, AttachFrom::SessionStart).await;
    let entries = v.until_exit(&id).await;
    assert!(String::from_utf8_lossy(&bytes(&entries, Some(Stream::Stdout))).contains("out"));
    assert!(String::from_utf8_lossy(&bytes(&entries, Some(Stream::Stderr))).contains("err"));
    assert_eq!(exit_code(&entries), Some(7));
    assert_contiguous(&entries);
}

/// RC-T9: everything the program printed, then Exit with its code, and
/// nothing after it.
#[tokio::test]
async fn output_ends_before_exit() {
    let (d, _home, _t) = start(Duration::from_secs(60)).await;
    let (mut v, _) = Vornd::hello(&d).await;
    let argv = loop_lines(3000);
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    let id = v.spawn(&argv, Io::Pty { cols: 80, rows: 24 }).await;
    v.attach(&id, AttachFrom::SessionStart).await;
    let entries = v.until_exit(&id).await;
    assert_eq!(exit_code(&entries), Some(7));
    let text = String::from_utf8_lossy(&bytes(&entries, None)).into_owned();
    assert!(
        text.contains("line2999"),
        "the last line arrived before Exit"
    );
    #[cfg(unix)]
    {
        let mut from = 0;
        for i in 0..3000 {
            let needle = format!("line{i}\r\n");
            let at = text[from..]
                .find(&needle)
                .unwrap_or_else(|| panic!("line{i} in order"));
            from += at + needle.len();
        }
    }
    assert_contiguous(&entries);
}

/// RC-T1 and RC-T10 in small: a vornd that goes away mid-output and a new
/// one that resumes from its cursor together receive every byte exactly
/// once, in order, with no Gap.
#[tokio::test]
async fn a_new_vornd_resumes_from_the_cursor_without_loss_or_repeats() {
    let (d, _home, _t) = start(Duration::from_secs(60)).await;
    let (mut a, _) = Vornd::hello(&d).await;
    let argv = loop_lines(20_000);
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    let id = a.spawn(&argv, Io::Pty { cols: 80, rows: 24 }).await;
    a.attach(&id, AttachFrom::SessionStart).await;
    let mut first = Vec::new();
    while first.len() < 20 {
        if let ToVornd::Entries(e) = a.recv().await.expect("open") {
            first.extend(e.entries);
        }
    }
    let cut = first.last().unwrap().after();
    drop(a);

    let (mut b, w) = Vornd::hello(&d).await;
    let info = w
        .sessions
        .iter()
        .find(|s| s.session == id)
        .expect("session survives");
    assert!(info.sent.next_rseq >= cut.next_rseq);
    b.attach(&id, AttachFrom::Cursor(cut)).await;
    let rest = b.until_exit(&id).await;
    assert_eq!(rest[0].hdr.rseq, cut.next_rseq);

    let mut joined = first.clone();
    joined.extend(rest);
    assert_contiguous(&joined);
    assert!(!joined.iter().any(|e| matches!(e.rec, Record::Gap { .. })));
    assert_eq!(exit_code(&joined), Some(7));

    // The whole log, read again from the start, is the same byte stream.
    let (mut c, _) = Vornd::hello(&d).await;
    c.attach(&id, AttachFrom::SessionStart).await;
    let all = c.until_exit(&id).await;
    assert_eq!(bytes(&all, None), bytes(&joined, None));
}

#[cfg(unix)]
#[tokio::test]
async fn a_resize_is_recorded_before_the_output_that_follows_it() {
    let (d, _home, _t) = start(Duration::from_secs(60)).await;
    let (mut v, _) = Vornd::hello(&d).await;
    let id = v
        .spawn(
            &["sh", "-c", "read x; stty size; exit 0"],
            Io::Pty { cols: 80, rows: 24 },
        )
        .await;
    v.attach(&id, AttachFrom::SessionStart).await;
    v.send(ToSessiond::Resize(Resize {
        session: id.clone(),
        req: 9,
        cols: 100,
        rows: 40,
        px_w: 0,
        px_h: 0,
    }))
    .await;
    v.send(ToSessiond::Write(Write {
        session: id.clone(),
        input_seq: 1,
        bytes: b"\r".to_vec(),
    }))
    .await;
    let entries = v.until_exit(&id).await;
    let at = entries
        .iter()
        .position(|e| {
            matches!(
                e.rec,
                Record::Resize {
                    cols: 100,
                    rows: 40,
                    req: Some(9),
                    ..
                }
            )
        })
        .expect("resize recorded");
    let after = String::from_utf8_lossy(&bytes(&entries[at..], None)).into_owned();
    assert!(after.contains("40 100"), "{after:?}");
}

#[tokio::test]
async fn a_second_hello_replaces_the_first_connection() {
    let (d, _home, _t) = start(Duration::from_secs(60)).await;
    let (mut a, _) = Vornd::hello(&d).await;
    let (mut b, _) = Vornd::hello(&d).await;
    a.send(ToSessiond::Ping(Nonce { nonce: 1 })).await;
    assert_eq!(a.recv().await, None);
    b.send(ToSessiond::Ping(Nonce { nonce: 2 })).await;
    assert_eq!(b.recv().await, Some(ToVornd::Pong(Nonce { nonce: 2 })));
}

#[tokio::test]
async fn unknown_sessions_are_refused_and_released_ones_forgotten() {
    let (d, _home, _t) = start(Duration::from_secs(60)).await;
    let (mut v, _) = Vornd::hello(&d).await;
    v.attach("nope", AttachFrom::SessionStart).await;
    assert!(matches!(
        v.recv().await,
        Some(ToVornd::Refused(Refused {
            why: AttachRefusal::NoSuchSession,
            ..
        }))
    ));
    let argv = loop_lines(1);
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    let id = v.spawn(&argv, Io::Pty { cols: 80, rows: 24 }).await;
    v.attach(&id, AttachFrom::SessionStart).await;
    v.until_exit(&id).await;
    v.send(ToSessiond::Release(SessionRef {
        session: id.clone(),
    }))
    .await;
    v.send(ToSessiond::Ping(Nonce { nonce: 3 })).await;
    while v.recv().await != Some(ToVornd::Pong(Nonce { nonce: 3 })) {}
    let (_, w) = Vornd::hello(&d).await;
    assert!(w.sessions.iter().all(|s| s.session != id));
}

#[tokio::test]
async fn a_frame_it_cannot_read_closes_only_the_connection() {
    let (d, _home, _t) = start(Duration::from_secs(60)).await;
    let mut v = Vornd::connect(&d).await;
    v.wr.write_all(&[5, 0, 0, 0, 0x7f, 1, 2, 3, 4])
        .await
        .unwrap();
    assert_eq!(v.recv().await, None);
    // A message before Hello closes it too.
    let mut v = Vornd::connect(&d).await;
    v.send(ToSessiond::Ping(Nonce { nonce: 1 })).await;
    assert_eq!(v.recv().await, None);
    let (mut v, _) = Vornd::hello(&d).await;
    v.send(ToSessiond::Ping(Nonce { nonce: 4 })).await;
    assert_eq!(v.recv().await, Some(ToVornd::Pong(Nonce { nonce: 4 })));
}

#[tokio::test]
async fn it_exits_when_idle_with_no_sessions() {
    let (_d, _home, task) = start(Duration::from_millis(300)).await;
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("exits by itself")
        .unwrap()
        .unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn the_endpoint_is_for_this_user_only() {
    use std::os::unix::fs::PermissionsExt;
    let (d, home, _t) = start(Duration::from_secs(60)).await;
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&home.path().join("run")), 0o700);
    assert_eq!(mode(std::path::Path::new(&d.endpoint())), 0o600);
}
