//! Handing every live session to a newer sessiond, on macOS and Linux.
//!
//! The newer sessiond (the adopter) connects to the older one (the donor)
//! and says [`ToSessiond::Handoff`]. The donor freezes every session, then
//! sends each one's manifest with its terminal master or pipe ends beside it
//! (`SCM_RIGHTS`), its records in memory and its checkpoints. The adopter
//! stages them all and says [`Ready`]; the donor says [`Commit`]; the
//! adopter answers [`Took`] and runs them from the same cursors. Up to
//! `Took` the donor still holds everything, and any failure, on either side,
//! leaves every session with it, thawed, as if nothing had happened.
//!
//! A program can only be reaped by the process that started it, so the
//! donor stays behind as a reaper: it holds nothing else, takes no
//! connections, writes each exit to `run/exits/<session>` for whichever
//! sessiond holds the session by then, and exits once the last of its
//! programs is reaped. Starting programs under a stable subreaper instead
//! works on Linux only, and a separate reaper process could itself never be
//! upgraded.

use std::collections::{HashSet, VecDeque};
use std::io;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::fdpass::{self, Receiver};
use crate::log::SessionLog;
use crate::session::{Handover, Session, Staged, Written};
use crate::wire::*;

use super::Sessiond;

/// How long either side waits for the other's next frame before it calls the
/// handoff off. The donor does not time out its wait for [`Took`]: past
/// [`Commit`] the adopter may already run the sessions.
const FRAME_WAIT: Duration = Duration::from_secs(10);
/// Output bytes per [`RingChunk`]: well under the frame cap.
const CHUNK_BYTES: u64 = 1 << 20;
/// How long the adopter gives a donor that hung up before the commit to
/// show it is gone, in which case nobody else holds the sessions.
const DONOR_GONE_WAIT: Duration = Duration::from_secs(1);

/// A step in the handoff where a test makes it fail or hang.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Donor: freezing the sessions.
    Freeze,
    /// Donor: after the offer went out.
    Offer,
    /// Donor: after the first session went out.
    Send,
    /// Donor: before the commit.
    Commit,
    /// Adopter: before it says hello.
    Hello,
    /// Adopter: after it staged the first session.
    Stage,
    /// Adopter: before it says it is ready.
    Ready,
    /// Adopter: before it says it took the sessions.
    Took,
}

impl Step {
    const NAMES: [(Step, &'static str); 8] = [
        (Step::Freeze, "freeze"),
        (Step::Offer, "offer"),
        (Step::Send, "send"),
        (Step::Commit, "commit"),
        (Step::Hello, "hello"),
        (Step::Stage, "stage"),
        (Step::Ready, "ready"),
        (Step::Took, "took"),
    ];

    fn name(self) -> &'static str {
        Step::NAMES
            .iter()
            .find(|(s, _)| *s == self)
            .map_or("?", |(_, n)| n)
    }
}

/// A failure a test injects into the handoff: at `step`, fail, or hang for
/// good so the test can kill the process there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fault {
    pub step: Step,
    pub stall: bool,
}

impl std::str::FromStr for Fault {
    type Err = String;

    /// `<step>:fail` or `<step>:stall`, as `VORN_SESSIOND_HANDOFF_FAULT` says.
    fn from_str(s: &str) -> Result<Fault, String> {
        let (step, act) = s.split_once(':').ok_or("expected <step>:<fail|stall>")?;
        let step = Step::NAMES
            .iter()
            .find(|(_, n)| *n == step)
            .map(|(s, _)| *s)
            .ok_or_else(|| format!("no handoff step {step}"))?;
        let stall = match act {
            "fail" => false,
            "stall" => true,
            other => return Err(format!("no fault {other}")),
        };
        Ok(Fault { step, stall })
    }
}

impl Sessiond {
    fn at(&self, step: Step) -> io::Result<()> {
        match *self.fault.lock().unwrap_or_else(|e| e.into_inner()) {
            Some(f) if f.step == step && f.stall => {
                eprintln!("handoff stalled at {}", step.name());
                loop {
                    std::thread::park();
                }
            }
            Some(f) if f.step == step => Err(io::Error::other(format!(
                "injected failure at {}",
                step.name()
            ))),
            _ => Ok(()),
        }
    }

    /// Where the sessiond that started a handed-off program writes its exit,
    /// for whichever one holds its session.
    fn exit_file(&self, session: &str) -> PathBuf {
        self.cfg.home.join("run").join("exits").join(session)
    }

    /// Give every session to the adopter on `sock`, which said `hello`.
    /// Returns once the adopter took them all, or every one is back here.
    pub(super) fn donate(self: &Arc<Self>, mut sock: UnixStream, hello: HandoffHello) {
        let refuse = |sock: &mut UnixStream, why: String| {
            let _ = fdpass::send(sock, &ToAdopter::Refuse(Refuse { why }), &[]);
        };
        if hello.version_min > HANDOFF || hello.version_max < HANDOFF {
            return refuse(&mut sock, format!("handoff version {HANDOFF} only"));
        }
        let sessions = {
            // Under the map lock, so no session is added past this point.
            let map = self.sessions();
            if self.handing.swap(true, Ordering::SeqCst) {
                drop(map);
                return refuse(&mut sock, "already handing over".into());
            }
            map.values().cloned().collect::<Vec<_>>()
        };
        let mut frozen = Thaw {
            d: self,
            sessions: Vec::new(),
        };
        let handovers = match self.freeze_all(&sessions, &mut frozen) {
            Ok(h) => h,
            Err(why) => return refuse(&mut sock, why),
        };
        let mut rx = Receiver::new(sock);
        if let Err(e) = self.offer(&mut rx, &handovers) {
            return refuse(rx.socket(), e.to_string());
        }
        // Past Commit only the adopter's Took, or the end of the connection
        // without it, says who has the sessions.
        let _ = rx.socket().set_read_timeout(None);
        if !matches!(rx.recv::<ToDonor>(), Ok(ToDonor::Took(Took))) {
            return;
        }
        self.handed.store(true, Ordering::SeqCst);
        let sessions = std::mem::take(&mut frozen.sessions);
        let mut map = self.sessions();
        for s in &sessions {
            s.hand_off(self.exit_file(&s.id));
            map.remove(&s.id);
        }
        drop(map);
        self.reaping
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(sessions);
        // Every vornd connection closes: the sessions are the adopter's.
        self.generation.send_modify(|n| *n += 1);
        self.stop.notify_waiters();
    }

    fn freeze_all(
        &self,
        sessions: &[Arc<Session>],
        frozen: &mut Thaw<'_>,
    ) -> Result<Vec<Handover>, String> {
        self.at(Step::Freeze).map_err(|e| e.to_string())?;
        let mut out = Vec::with_capacity(sessions.len());
        for s in sessions {
            out.push(s.freeze()?);
            frozen.sessions.push(Arc::clone(s));
        }
        Ok(out)
    }

    /// Send the offer and every session, wait for Ready, then commit.
    fn offer(&self, rx: &mut Receiver, handovers: &[Handover]) -> io::Result<()> {
        rx.socket().set_read_timeout(Some(FRAME_WAIT))?;
        let offer = Offer {
            version: HANDOFF,
            instance: self.cfg.instance,
            pid: std::process::id(),
            sessions: handovers.len() as u32,
        };
        fdpass::send(rx.socket(), &ToAdopter::Offer(offer), &[])?;
        self.at(Step::Offer)?;
        for (i, h) in handovers.iter().enumerate() {
            send_session(rx.socket(), h)?;
            if i == 0 {
                self.at(Step::Send)?;
            }
        }
        fdpass::send(rx.socket(), &ToAdopter::Done(Done), &[])?;
        match rx.recv::<ToDonor>()? {
            ToDonor::Ready(Ready) => {}
            other => return Err(io::Error::other(format!("expected Ready, got {other:?}"))),
        }
        self.at(Step::Commit)?;
        fdpass::send(rx.socket(), &ToAdopter::Commit(Commit), &[])
    }

    /// Take every session the donor at `from` holds, or none, and return
    /// their ids.
    pub(super) fn adopt(
        &self,
        from: &str,
        on_written: impl Fn(&str, Written) + Send + Clone + 'static,
    ) -> io::Result<Vec<SessionId>> {
        if from == self.endpoint() {
            return Err(io::Error::other("a sessiond cannot adopt its own sessions"));
        }
        if self.handing.load(Ordering::SeqCst) || self.handed.load(Ordering::SeqCst) {
            return Err(io::Error::other("handing its own sessions over"));
        }
        self.at(Step::Hello)?;
        let mut sock = UnixStream::connect(from)?;
        sock.set_read_timeout(Some(FRAME_WAIT))?;
        let hello = HandoffHello {
            version_min: HANDOFF,
            version_max: HANDOFF,
            instance: self.cfg.instance,
            build: self.cfg.build.clone(),
        };
        fdpass::send(&mut sock, &ToSessiond::Handoff(hello), &[])?;
        let mut rx = Receiver::new(sock);
        let offer = match rx.recv::<ToAdopter>()? {
            ToAdopter::Offer(o) if o.version == HANDOFF => o,
            ToAdopter::Refuse(r) => return Err(io::Error::other(r.why)),
            other => return Err(io::Error::other(format!("expected Offer, got {other:?}"))),
        };
        let staged = self.stage_all(&mut rx, &offer)?;
        self.at(Step::Ready)?;
        fdpass::send(rx.socket(), &ToDonor::Ready(Ready), &[])?;
        match rx.recv::<ToAdopter>() {
            Ok(ToAdopter::Commit(Commit)) => {
                self.at(Step::Took)?;
                fdpass::send(rx.socket(), &ToDonor::Took(Took), &[])?;
            }
            // A donor that died after sending everything holds nothing any
            // more: the descriptors here are all that keeps the sessions.
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof && donor_gone(offer.pid) => {}
            Ok(other) => return Err(io::Error::other(format!("expected Commit, got {other:?}"))),
            Err(e) => return Err(e),
        }
        let mut map = self.sessions();
        let ids = staged
            .into_iter()
            .map(|s| {
                let id = s.id().to_owned();
                map.insert(id.clone(), s.start(self.exit_file(&id), on_written.clone()));
                id
            })
            .collect();
        Ok(ids)
    }

    fn stage_all(&self, rx: &mut Receiver, offer: &Offer) -> io::Result<Vec<Staged>> {
        let mut staged = Vec::new();
        let mut seen = HashSet::new();
        for _ in 0..offer.sessions {
            let m = match rx.recv::<ToAdopter>()? {
                ToAdopter::Manifest(m) => *m,
                other => return Err(unexpected("Manifest", &other)),
            };
            let fds = rx.take_fds(m.fds.len())?;
            if !seen.insert(m.session.clone()) || self.holds(&m.session) {
                return Err(io::Error::other(format!("{} is held already", m.session)));
            }
            let mut ring = VecDeque::new();
            while (ring.len() as u64) < m.ring_entries {
                match rx.recv::<ToAdopter>()? {
                    ToAdopter::Ring(c) if c.session == m.session && !c.entries.is_empty() => {
                        ring.extend(c.entries)
                    }
                    other => return Err(unexpected("Ring", &other)),
                }
            }
            let newest = m.newest.then(|| checkpoint(rx, true)).transpose()?;
            let fallback = m.fallback.then(|| checkpoint(rx, false)).transpose()?;
            let spool = self.spool_dir().join(format!("{}.log", m.session));
            let log = SessionLog::adopt(&m, ring, newest, fallback, spool, self.pool.clone())?;
            staged.push(Session::stage(&m, fds, log)?);
            if staged.len() == 1 {
                self.at(Step::Stage)?;
            }
        }
        match rx.recv::<ToAdopter>()? {
            ToAdopter::Done(Done) if rx.unclaimed() == 0 => Ok(staged),
            ToAdopter::Done(Done) => Err(io::Error::other("descriptors no session claimed")),
            other => Err(unexpected("Done", &other)),
        }
    }
}

fn unexpected(want: &str, got: &ToAdopter) -> io::Error {
    io::Error::other(format!("expected {want}, got {:#04x}", got.kind()))
}

/// Sessions frozen for a handoff, thawed if it is called off.
struct Thaw<'a> {
    d: &'a Sessiond,
    sessions: Vec<Arc<Session>>,
}

impl Drop for Thaw<'_> {
    fn drop(&mut self) {
        for s in &self.sessions {
            s.thaw();
        }
        self.d.handing.store(false, Ordering::SeqCst);
    }
}

fn send_session(sock: &mut UnixStream, h: &Handover) -> io::Result<()> {
    fdpass::send(sock, &ToAdopter::Manifest(Box::new(h.manifest.clone())), &h.fds)?;
    let ring = |entries: &[vorn_term_proto::Entry]| {
        ToAdopter::Ring(RingChunk {
            session: h.manifest.session.clone(),
            entries: entries.to_vec(),
        })
    };
    let mut start = 0;
    let mut bytes = 0;
    for (i, e) in h.ring.iter().enumerate() {
        bytes += e.rec.len();
        if bytes >= CHUNK_BYTES {
            fdpass::send(sock, &ring(&h.ring[start..=i]), &[])?;
            start = i + 1;
            bytes = 0;
        }
    }
    if start < h.ring.len() {
        fdpass::send(sock, &ring(&h.ring[start..]), &[])?;
    }
    if let Some(cp) = &h.newest {
        fdpass::send(sock, &ToAdopter::Newest(cp.clone()), &[])?;
    }
    if let Some(cp) = &h.fallback {
        fdpass::send(sock, &ToAdopter::Fallback(cp.clone()), &[])?;
    }
    Ok(())
}

fn checkpoint(rx: &mut Receiver, newest: bool) -> io::Result<Checkpoint> {
    match (rx.recv::<ToAdopter>()?, newest) {
        (ToAdopter::Newest(cp), true) | (ToAdopter::Fallback(cp), false) => Ok(cp),
        (other, _) => Err(unexpected("a checkpoint", &other)),
    }
}

/// Whether the donor process is gone, given a moment to finish dying.
fn donor_gone(pid: u32) -> bool {
    let deadline = Instant::now() + DONOR_GONE_WAIT;
    while crate::launch::alive(pid) {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    true
}

/// Read the rest of a first frame that is [`ToSessiond::Handoff`], given its
/// first five bytes, then run the donor on the socket.
pub(super) async fn serve(d: Arc<Sessiond>, mut stream: tokio::net::UnixStream, head: [u8; 5]) {
    use tokio::io::AsyncReadExt;
    let len = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
    if len == 0 || len > MAX_FRAME {
        return;
    }
    let mut body = vec![0u8; len - 1];
    let read = tokio::time::timeout(FRAME_WAIT, stream.read_exact(&mut body)).await;
    if !matches!(read, Ok(Ok(_))) {
        return;
    }
    let Ok(ToSessiond::Handoff(hello)) = ToSessiond::read(head[4], &body) else {
        return;
    };
    let Ok(sock) = stream.into_std() else {
        return;
    };
    if sock.set_nonblocking(false).is_ok() {
        let _ = tokio::task::spawn_blocking(move || d.donate(sock, hello)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faults_read_as_the_tests_write_them() {
        for (step, name) in Step::NAMES {
            let f: Fault = format!("{name}:stall").parse().unwrap();
            assert_eq!(f, Fault { step, stall: true });
            assert_eq!(step.name(), name);
        }
        assert_eq!(
            "send:fail".parse::<Fault>(),
            Ok(Fault {
                step: Step::Send,
                stall: false
            })
        );
        assert!("send".parse::<Fault>().is_err());
        assert!("later:fail".parse::<Fault>().is_err());
        assert!("send:crash".parse::<Fault>().is_err());
    }
}
