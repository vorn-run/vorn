//! A grid client on vornd's grid endpoint, as the native app is: attaches
//! to sessions, types into them and follows their frames, acknowledging
//! each so credit never runs out.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use vorn_grid_client::{Client, Got, Pane};
use vorn_term_proto::msg::{Attach, AttachMode, ClientMsg, InputEvent, ServerMsg, Size};

use crate::error::Error;

#[derive(Debug)]
pub struct GridClient {
    sock: UnixStream,
    client: Client,
    buf: Vec<u8>,
    sids: HashMap<String, u32>,
    /// Detached sids, whose stragglers are not acknowledged.
    gone: HashSet<u32>,
}

impl GridClient {
    pub async fn connect(endpoint: &str, within: Duration) -> Result<GridClient, Error> {
        let mut g = GridClient {
            sock: UnixStream::connect(endpoint).await?,
            client: Client::hello("scale-bench"),
            buf: vec![0u8; 1 << 20],
            sids: HashMap::new(),
            gone: HashSet::new(),
        };
        let deadline = Instant::now() + within;
        while g.client.welcome().is_none() {
            g.pump(deadline, "Welcome").await?;
        }
        Ok(g)
    }

    /// Writes what the client has to say, then applies one read's worth of
    /// what vornd sent, acknowledging its frames.
    pub async fn pump(&mut self, deadline: Instant, what: &str) -> Result<Vec<Got>, Error> {
        self.flush().await?;
        let left = deadline.saturating_duration_since(Instant::now());
        let n = tokio::time::timeout(left, self.sock.read(&mut self.buf))
            .await
            .map_err(|_| Error::Timeout(what.to_owned()))??;
        if n == 0 {
            return Err(Error::Closed("vornd's grid endpoint"));
        }
        let got = self
            .client
            .receive(&self.buf[..n])
            .map_err(|e| Error::Protocol(format!("{e:?}")))?;
        for g in &got {
            match g {
                Got::Frame { sid, rev, .. } if !self.gone.contains(sid) => {
                    self.client.ack(*sid, *rev)
                }
                Got::Attached(a) => {
                    self.sids.insert(a.session.clone(), a.sid);
                }
                // Not fatal: a straggler for a detached sid draws one.
                Got::Message(ServerMsg::Error { code, message }) => {
                    eprintln!("vorn-scale-bench: grid error {code}: {message}");
                }
                _ => {}
            }
        }
        Ok(got)
    }

    async fn flush(&mut self) -> Result<(), Error> {
        let out = self.client.take_out();
        if !out.is_empty() {
            self.sock.write_all(&out).await?;
        }
        Ok(())
    }

    /// Attaches to `session` and waits for its first snapshot; answers its
    /// sid and how long that took.
    pub async fn attach(
        &mut self,
        session: &str,
        within: Duration,
    ) -> Result<(u32, Duration), Error> {
        let t = Instant::now();
        self.client.attach(Attach {
            session: session.to_owned(),
            mode: AttachMode::Grid,
            view: Size {
                cols: 80,
                rows: 24,
                px_w: 0,
                px_h: 0,
            },
            visible: true,
            resume: None,
            history_tail: 0,
        });
        let deadline = t + within;
        loop {
            let got = self.pump(deadline, "a snapshot").await?;
            let sid = self.sids.get(session).copied();
            let snapped = got
                .iter()
                .any(|g| matches!(g, Got::Frame { sid: s, snapshot: true, .. } if Some(*s) == sid));
            if let (true, Some(sid)) = (snapped, sid) {
                return Ok((sid, t.elapsed()));
            }
        }
    }

    pub fn detach(&mut self, sid: u32) {
        self.client.send(&ClientMsg::Detach { sid });
        self.gone.insert(sid);
        if let Some(p) = self.client.drop_pane(sid) {
            self.sids.remove(&p.session);
        }
    }

    /// Sends `bytes` as typed and waits for the frame that shows them.
    pub async fn type_and_wait(
        &mut self,
        sid: u32,
        bytes: &[u8],
        within: Duration,
    ) -> Result<Duration, Error> {
        let input_seq = self.client.input_seq();
        self.client.send(&ClientMsg::Input {
            sid,
            input_seq,
            event: InputEvent::Raw {
                bytes: bytes.to_vec(),
            },
        });
        let t = Instant::now();
        let deadline = t + within;
        loop {
            let got = self.pump(deadline, "the echo").await?;
            if got
                .iter()
                .any(|g| matches!(g, Got::Frame { sid: s, .. } if *s == sid))
            {
                return Ok(t.elapsed());
            }
        }
    }

    /// Reads and applies whatever arrives for `quiet`, so later waits see
    /// only what they caused.
    pub async fn drain(&mut self, quiet: Duration) -> Result<(), Error> {
        loop {
            match self.pump(Instant::now() + quiet, "").await {
                Ok(_) => {}
                Err(Error::Timeout(_)) => return Ok(()),
                Err(e) => return Err(e),
            }
        }
    }

    pub fn pane(&self, sid: u32) -> Option<&Pane> {
        self.client.pane(sid)
    }

    /// The pane's screen as text.
    pub fn text(&self, sid: u32) -> String {
        self.pane(sid)
            .and_then(Pane::mirror)
            .map(|m| m.text().join("\n"))
            .unwrap_or_default()
    }
}
