//! vornd's session engine against a sessiond that stops reading while it
//! writes, as the real one does when a full outbox holds up its reader. vornd
//! must keep reading whatever it is writing, or both ends wait on each other
//! for ever; and a peer that never reads again must cost vornd the
//! connection, not a hang.

#![cfg(all(feature = "engine", unix))]

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use vorn_engine::{Cadence, Config};
use vorn_sessiond_wire::{
    Entries, FrameReader, Kind, Message, SessionInfo, ToSessiond, ToVornd, Welcome, PROTO,
};
use vorn_term_proto::{Cursor, Entry, Record, RecordHeader, Stream};
use vornd::engine::Engine;
use vornd::holder::{self, Holder};

const SESSIONS: usize = 4;
/// Output per session, in records of [`RECORD`] bytes.
const OUTPUT: usize = 256 << 10;
const RECORD: usize = 16 << 10;
/// Between rounds of records.
const PACE: Duration = Duration::from_millis(100);

/// Sessions that keep their history in checkpoints, cut often, so vornd has
/// far more to send than a socket buffer holds.
fn config() -> Config {
    Config {
        scrollback: 512 << 10,
        cadence: Cadence {
            bytes: 32 << 10,
            ..Cadence::default()
        },
        build: "test".into(),
        ..Config::default()
    }
}

fn info(n: usize) -> SessionInfo {
    let start = Cursor::start(0);
    SessionInfo {
        session: format!("s{n}"),
        kind: Kind::Pty,
        pid: 1,
        epoch: 0,
        oldest: start,
        head: start,
        newest_cp: None,
        retain_from: start,
        sent: start,
        cols: 100,
        rows: 30,
        exited: None,
        spooled_bytes: 0,
    }
}

/// One frame from vornd.
async fn read_one(s: &mut UnixStream, frames: &mut FrameReader) -> Option<ToSessiond> {
    let mut buf = vec![0u8; 64 << 10];
    loop {
        if let Ok(Some(m)) = frames.read::<ToSessiond>() {
            return Some(m);
        }
        match s.read(&mut buf).await {
            Ok(0) | Err(_) => return None,
            Ok(n) => frames.push(&buf[..n]),
        }
    }
}

/// Accepts vornd, welcomes it with [`SESSIONS`] sessions and writes every
/// session's output without reading a byte. Answers the stream, its frame
/// reader, and where each session's output ends.
async fn burst(listener: &UnixListener) -> (UnixStream, FrameReader, HashMap<String, Cursor>) {
    let (mut s, _) = listener.accept().await.unwrap();
    let mut frames = FrameReader::default();
    let hello = read_one(&mut s, &mut frames).await;
    assert!(matches!(hello, Some(ToSessiond::Hello(_))), "{hello:?}");
    let welcome = ToVornd::Welcome(Welcome {
        proto: PROTO,
        sessiond_instance: 1,
        sessiond_build: "test".into(),
        sessions: (0..SESSIONS).map(info).collect(),
    });
    s.write_all(&welcome.encode()).await.unwrap();

    let mut ends = HashMap::new();
    let mut at = vec![Cursor::start(0); SESSIONS];
    let line = "0123456789abcdefghijklmnopqrstuvwxyz ".repeat(2);
    'burst: for round in 0..OUTPUT / RECORD {
        for (n, cursor) in at.iter_mut().enumerate() {
            let mut bytes = Vec::with_capacity(RECORD + 128);
            let mut i = 0;
            while bytes.len() < RECORD {
                bytes.extend_from_slice(format!("s{n} r{round} l{i} {line}\r\n").as_bytes());
                i += 1;
            }
            let rec = Record::Data {
                stream: Stream::Pty,
                bytes,
            };
            let entry = Entry {
                hdr: RecordHeader {
                    epoch: 0,
                    rseq: cursor.next_rseq,
                    start_offset: cursor.next_offset,
                },
                at_ns: 0,
                rec,
            };
            *cursor = entry.after();
            let msg = ToVornd::Entries(Entries {
                session: format!("s{n}"),
                entries: vec![entry],
            });
            // Writing stalls only while vornd is not reading: with vornd
            // stuck on a write of its own, it stalls for good. A vornd that
            // hung up ends the burst; the callers tell whether it should
            // have.
            let written = tokio::time::timeout(Duration::from_secs(20), s.write_all(&msg.encode()))
                .await
                .expect("vornd stopped reading while sessiond was writing");
            if written.is_err() {
                break 'burst;
            }
        }
        // Output keeps coming while vornd cuts checkpoints, as a busy
        // session's does.
        tokio::time::sleep(PACE).await;
    }
    for (n, c) in at.into_iter().enumerate() {
        ends.insert(format!("s{n}"), c);
    }
    (s, frames, ends)
}

/// Many sessions streaming at once and big checkpoints going back: vornd
/// keeps reading through its own writes, and every session's records are
/// acknowledged once the peer reads again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn vornd_reads_while_it_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sessiond.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let engine = Engine::new(config());
    let holder = Holder::with_engine(engine);
    let endpoint = path.to_string_lossy().into_owned();
    let vornd = tokio::spawn(async move { holder::connect(&endpoint, &holder).await });

    let (mut s, mut frames, ends) = burst(&listener).await;
    let mut acked: HashMap<String, Cursor> = HashMap::new();
    let mut checkpoint_bytes = 0usize;
    let deadline = Instant::now() + Duration::from_secs(60);
    while acked.len() < SESSIONS || acked.iter().any(|(id, c)| ends[id] != *c) {
        let left = deadline.saturating_duration_since(Instant::now());
        let m = tokio::time::timeout(left, read_one(&mut s, &mut frames))
            .await
            .expect("every session acknowledged")
            .expect("vornd stayed connected");
        match m {
            ToSessiond::Ack(a) => {
                acked.insert(a.session, a.delivered);
            }
            ToSessiond::PutCheckpoint(cp) => checkpoint_bytes += cp.blob.len(),
            _ => {}
        }
    }
    // The point of the test: vornd had more to send than a socket buffer
    // takes, while the peer was not reading.
    assert!(
        checkpoint_bytes > 1 << 20,
        "{checkpoint_bytes} B of checkpoints"
    );
    vornd.abort();
}

/// A peer that never reads again: vornd gives the connection up after its
/// write timeout, so the holder can start over.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_peer_that_never_reads_costs_the_connection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sessiond.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let engine = Engine::with_write_timeout(config(), Duration::from_secs(1));
    let holder = Holder::with_engine(engine);
    let endpoint = path.to_string_lossy().into_owned();
    let vornd = tokio::spawn(async move { holder::connect(&endpoint, &holder).await });

    let (_s, _frames, _ends) = burst(&listener).await;
    let why = tokio::time::timeout(Duration::from_secs(30), vornd)
        .await
        .expect("vornd gave the connection up")
        .unwrap()
        .unwrap();
    assert!(why.contains("timed out"), "{why}");
}
