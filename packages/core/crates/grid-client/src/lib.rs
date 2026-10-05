//! A headless grid client: [`vorn_term_mirror::Mirror`] behind grid mode's
//! framing (Terminal State Protocol §7 and §8), for tests and tools.
//!
//! It does no I/O. Bytes read from vornd go in through
//! [`Client::receive`], which decodes each frame and applies it to the
//! attachment's mirror; what the client says goes out through
//! [`Client::take_out`]. When to acknowledge a frame is the caller's choice,
//! so a test can hold acks back the way a slow client would.
//!
//! Like the native app, it links no terminal: everything it shows is a frame
//! vornd sent.

use std::collections::BTreeMap;

use vorn_term_mirror::{Mirror, Refused};
use vorn_term_proto::msg::{
    caps, Attach, Attached, ClientKind, ClientMsg, EventId, EventKind, FrameReader, GridResume,
    Hello, MsgError, ServerMsg, Welcome, PROTO_MAJOR, PROTO_MINOR,
};

/// One attachment as the client sees it.
#[derive(Default)]
pub struct Pane {
    pub session: String,
    pub attached: Option<Attached>,
    mirror: Option<Mirror>,
    /// Tables were reset; nothing applies until the next snapshot.
    stale: bool,
    /// Snapshots and deltas applied.
    pub frames: u64,
    pub snapshots: u64,
    /// Payload bytes of the frames applied.
    pub frame_bytes: u64,
    /// Frames the mirror refused: each would be a resync in an app.
    pub refused: Vec<Refused>,
    pub events: Vec<(EventId, EventKind)>,
    /// Resyncs and table resets announced.
    pub resyncs: u64,
    pub resets: u64,
}

impl std::fmt::Debug for Pane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pane")
            .field("session", &self.session)
            .field("rev", &self.mirror.as_ref().map(Mirror::rev))
            .field("frames", &self.frames)
            .field("refused", &self.refused)
            .finish()
    }
}

impl Pane {
    /// The mirror, unless its tables were reset and no snapshot came yet.
    pub fn mirror(&self) -> Option<&Mirror> {
        self.mirror.as_ref().filter(|_| !self.stale)
    }

    /// What to resume from after a reconnect.
    pub fn resume(&self) -> Option<GridResume> {
        let m = self.mirror()?;
        Some(GridResume {
            state_gen: m.state_gen(),
            rev: m.rev(),
            table_gen: m.table_gen(),
            style_mark: m.style_mark(),
            link_mark: m.link_mark(),
        })
    }
}

/// What one message from vornd did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Got {
    Welcome(Welcome),
    Attached(Attached),
    /// A snapshot or delta was applied; acknowledge it to return the credit.
    Frame {
        sid: u32,
        rev: u64,
        snapshot: bool,
    },
    /// A frame the mirror would not apply.
    Refused {
        sid: u32,
        why: Refused,
    },
    /// Anything else: replies, events, errors.
    Message(ServerMsg),
}

/// One connection's worth of attachments, with no socket of its own.
#[derive(Debug, Default)]
pub struct Client {
    reader: FrameReader,
    out: Vec<u8>,
    welcome: Option<Welcome>,
    panes: BTreeMap<u32, Pane>,
    next_input: u64,
}

impl Client {
    pub fn new() -> Client {
        Client::default()
    }

    /// A client that has said hello as the native app does.
    pub fn hello(build: &str) -> Client {
        let mut c = Client::new();
        c.send(&ClientMsg::Hello(Hello {
            proto_major: PROTO_MAJOR,
            proto_minor: PROTO_MINOR,
            caps: caps::ALL,
            client: ClientKind::Cli,
            build: build.to_owned(),
        }));
        c
    }

    pub fn welcome(&self) -> Option<&Welcome> {
        self.welcome.as_ref()
    }

    /// Queues a message for [`Client::take_out`].
    pub fn send(&mut self, m: &ClientMsg) {
        m.encode(&mut self.out);
    }

    pub fn attach(&mut self, a: Attach) {
        self.send(&ClientMsg::Attach(a));
    }

    /// Returns a frame's credit.
    pub fn ack(&mut self, sid: u32, rev: u64) {
        self.send(&ClientMsg::Ack { sid, rev });
    }

    /// The next input sequence number.
    pub fn input_seq(&mut self) -> u64 {
        self.next_input += 1;
        self.next_input
    }

    /// What the client has to say, as bytes for the socket.
    pub fn take_out(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }

    pub fn pane(&self, sid: u32) -> Option<&Pane> {
        self.panes.get(&sid)
    }

    pub fn panes(&self) -> impl Iterator<Item = (u32, &Pane)> {
        self.panes.iter().map(|(s, p)| (*s, p))
    }

    /// Forgets an attachment, as after a detach.
    pub fn drop_pane(&mut self, sid: u32) -> Option<Pane> {
        self.panes.remove(&sid)
    }

    /// Bytes read from vornd: every whole frame in them, applied.
    pub fn receive(&mut self, bytes: &[u8]) -> Result<Vec<Got>, MsgError> {
        self.reader.push(bytes);
        let mut got = Vec::new();
        loop {
            let Some((kind, payload)) = self.reader.next_frame()? else {
                return Ok(got);
            };
            let len = payload.len() as u64;
            if let Some(m) = ServerMsg::decode(kind, payload)? {
                got.push(self.apply(m, len));
            }
        }
    }

    /// One message, already decoded. `len` is its payload size, counted
    /// against the pane it is for.
    pub fn apply(&mut self, m: ServerMsg, len: u64) -> Got {
        match m {
            ServerMsg::Welcome(w) => {
                self.welcome = Some(w.clone());
                Got::Welcome(w)
            }
            ServerMsg::Attached(a) => {
                let pane = self.panes.entry(a.sid).or_default();
                pane.session = a.session.clone();
                pane.attached = Some(a.clone());
                Got::Attached(a)
            }
            ServerMsg::Snapshot { sid, snap } => {
                let pane = self.panes.entry(sid).or_default();
                let rev = snap.rev;
                let applied = if let Some(m) = &mut pane.mirror {
                    m.resnapshot(*snap)
                } else {
                    Mirror::from_snapshot(*snap).map(|m| {
                        pane.mirror = Some(m);
                    })
                };
                pane.stale = false;
                match applied {
                    Ok(()) => {
                        pane.frames += 1;
                        pane.snapshots += 1;
                        pane.frame_bytes += len;
                        Got::Frame {
                            sid,
                            rev,
                            snapshot: true,
                        }
                    }
                    Err(why) => {
                        pane.refused.push(why);
                        pane.mirror = None;
                        Got::Refused { sid, why }
                    }
                }
            }
            ServerMsg::Delta { sid, delta } => {
                let pane = self.panes.entry(sid).or_default();
                let rev = delta.rev;
                let applied = match (&mut pane.mirror, pane.stale) {
                    (Some(m), false) => m.apply(*delta),
                    _ => Err(Refused::StateGen),
                };
                match applied {
                    Ok(()) => {
                        pane.frames += 1;
                        pane.frame_bytes += len;
                        Got::Frame {
                            sid,
                            rev,
                            snapshot: false,
                        }
                    }
                    Err(why) => {
                        pane.refused.push(why);
                        Got::Refused { sid, why }
                    }
                }
            }
            ServerMsg::ResetTables { sid, .. } => {
                let pane = self.panes.entry(sid).or_default();
                pane.stale = true;
                pane.resets += 1;
                Got::Message(m)
            }
            ServerMsg::Resync { sid, .. } => {
                self.panes.entry(sid).or_default().resyncs += 1;
                Got::Message(m)
            }
            ServerMsg::History {
                sid,
                sb_epoch,
                ref rows,
                ref styles,
                ref links,
                ..
            } => {
                if let Some(pane) = self.panes.get_mut(&sid) {
                    if let (Some(mirror), false) = (&mut pane.mirror, pane.stale) {
                        if let Err(why) = mirror.apply_history(
                            sb_epoch,
                            rows.clone(),
                            styles.clone(),
                            links.clone(),
                        ) {
                            pane.refused.push(why);
                        }
                    }
                }
                Got::Message(m)
            }
            ServerMsg::Event { sid, id, ref kind } => {
                if let Some(pane) = self.panes.get_mut(&sid) {
                    pane.events.push((id, kind.clone()));
                }
                Got::Message(m)
            }
            other => Got::Message(other),
        }
    }
}

#[cfg(test)]
mod tests {
    /// TP-T1's client half: the client, the mirror and the wire types link
    /// no terminal, so nothing on the app's side of the socket can parse VT.
    #[test]
    fn links_no_terminal() {
        for manifest in [
            include_str!("../Cargo.toml"),
            include_str!("../../term-mirror/Cargo.toml"),
            include_str!("../../term-proto/Cargo.toml"),
        ] {
            let deps = manifest.split("[dependencies]").nth(1).unwrap_or("");
            assert!(!deps.contains("ghostty"), "{manifest}");
            assert!(!deps.contains("vorn-screen"), "{manifest}");
        }
    }
}
