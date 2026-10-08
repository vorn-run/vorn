//! A client of the session holder's socket, doing what vornd does: says
//! Hello, starts and attaches sessions, and acknowledges every record so
//! the holder never holds back output on the bench's account.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use vorn_sessiond_wire::{
    Ack, Attach, AttachFrom, FrameReader, Hello, Message, ToSessiond, ToVornd, PROTO,
};
use vorn_term_proto::Record;

use crate::error::Error;

/// One connection to a holder.
#[derive(Debug)]
pub struct HolderClient {
    out: mpsc::UnboundedSender<Vec<u8>>,
    got: mpsc::UnboundedReceiver<ToVornd>,
    watched: Arc<Mutex<HashSet<String>>>,
    received: Arc<AtomicU64>,
}

impl HolderClient {
    pub async fn connect(endpoint: &str) -> Result<HolderClient, Error> {
        let (mut rd, mut wr) = UnixStream::connect(endpoint).await?.into_split();
        // Unbounded both ways: the reader must never wait on the bench, or
        // it would be the bench, not the holder, that slows the output.
        let (out, mut outgoing) = mpsc::unbounded_channel::<Vec<u8>>();
        let (deliver, got) = mpsc::unbounded_channel();
        let watched = Arc::new(Mutex::new(HashSet::new()));
        let received = Arc::new(AtomicU64::new(0));
        tokio::spawn(async move {
            while let Some(frame) = outgoing.recv().await {
                if wr.write_all(&frame).await.is_err() {
                    return;
                }
            }
        });
        let (acks, seen, count) = (out.clone(), Arc::clone(&watched), Arc::clone(&received));
        tokio::spawn(async move {
            let mut frames = FrameReader::default();
            let mut buf = vec![0u8; 256 << 10];
            loop {
                match rd.read(&mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => frames.push(&buf[..n]),
                }
                loop {
                    let m = match frames.read::<ToVornd>() {
                        Ok(Some(m)) => m,
                        Ok(None) => break,
                        Err(_) => return,
                    };
                    if let ToVornd::Entries(e) = &m {
                        let bytes: usize = e.entries.iter().map(|e| data_len(&e.rec)).sum();
                        count.fetch_add(bytes as u64, Ordering::Relaxed);
                        if let Some(last) = e.entries.last() {
                            let ack = ToSessiond::Ack(Ack {
                                session: e.session.clone(),
                                delivered: last.after(),
                            });
                            let _ = acks.send(ack.encode());
                        }
                        let watching = seen.lock().unwrap_or_else(|p| p.into_inner());
                        if !watching.contains(&e.session) {
                            continue;
                        }
                    }
                    if deliver.send(m).is_err() {
                        return;
                    }
                }
            }
        });
        let mut c = HolderClient {
            out,
            got,
            watched,
            received,
        };
        c.send(ToSessiond::Hello(Hello {
            proto_min: PROTO,
            proto_max: PROTO,
            vornd_instance: u128::from(std::process::id()),
            vornd_build: "scale-bench".into(),
        }))?;
        match c.next(Duration::from_secs(60)).await? {
            ToVornd::Welcome(_) => Ok(c),
            other => Err(Error::Protocol(format!("expected Welcome, got {other:?}"))),
        }
    }

    pub fn send(&self, m: ToSessiond) -> Result<(), Error> {
        self.out
            .send(m.encode())
            .map_err(|_| Error::Closed("the holder"))
    }

    /// The next message: anything but records, and the records of the
    /// sessions being watched.
    pub async fn next(&mut self, within: Duration) -> Result<ToVornd, Error> {
        tokio::time::timeout(within, self.got.recv())
            .await
            .map_err(|_| Error::Timeout(format!("the holder, {within:?}")))?
            .ok_or(Error::Closed("the holder"))
    }

    /// Drops what has arrived and not been read.
    pub fn drain(&mut self) {
        while self.got.try_recv().is_ok() {}
    }

    /// Attaches to `session` from its start; its records are acknowledged
    /// from then on, and handed to [`HolderClient::next`] when `watch`.
    pub fn attach(&self, session: &str, watch: bool) -> Result<(), Error> {
        if watch {
            self.watched
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(session.to_owned());
        }
        self.send(ToSessiond::Attach(Attach {
            session: session.to_owned(),
            from: AttachFrom::SessionStart,
        }))
    }

    /// Output bytes received so far, from every attached session.
    pub fn received(&self) -> u64 {
        self.received.load(Ordering::Relaxed)
    }
}

pub fn data_len(r: &Record) -> usize {
    match r {
        Record::Data { bytes, .. } => bytes.len(),
        _ => 0,
    }
}
