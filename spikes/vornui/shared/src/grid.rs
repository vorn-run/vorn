//! The grid client both prototypes link. It connects to vornd's grid
//! endpoint, attaches one pane per session, keeps each pane's mirror through
//! [`vorn_grid_client::Client`] on an I/O thread, returns every frame's credit
//! as soon as it is applied, and flags the panes that changed. The UI pulls a
//! [`PaneView`] of each changed pane and draws it; everything above that line
//! is the same code in both prototypes.

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Instant;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use vorn_grid_client::{Client, Got};
use vorn_term_proto::msg::{
    caps, Attach, AttachMode, ClientKind, ClientMsg, Hello, InputEvent, KeyAction, KeyCode,
    ServerMsg, Size, PROTO_MAJOR, PROTO_MINOR,
};

use crate::view::{self, PaneView};

struct Probe {
    x: u16,
    y: u16,
    ch: String,
    seen_rev: Option<u64>,
}

struct State {
    client: Client,
    sessions: Vec<String>,
    sids: Vec<Option<u32>>,
    dirty: Vec<bool>,
    snapshotted: Vec<bool>,
    probes: Vec<Option<Probe>>,
    errors: Vec<String>,
    closed: bool,
    /// Bumped on every change; the UI waits on it.
    changes: u64,
}

impl State {
    fn pane_of(&self, sid: u32) -> Option<usize> {
        self.sids.iter().position(|s| *s == Some(sid))
    }
}

pub struct Grid {
    state: Mutex<State>,
    changed: Condvar,
    out: mpsc::UnboundedSender<Vec<u8>>,
}

impl Grid {
    /// Connects to `endpoint` (a Unix socket path, or a named pipe on
    /// Windows), says hello and attaches one pane per session at
    /// `cols`x`rows`.
    pub fn connect(
        endpoint: &str,
        sessions: Vec<String>,
        cols: u16,
        rows: u16,
    ) -> std::io::Result<Arc<Grid>> {
        let mut client = Client::new();
        client.send(&ClientMsg::Hello(Hello {
            proto_major: PROTO_MAJOR,
            proto_minor: PROTO_MINOR,
            caps: caps::ALL,
            client: ClientKind::Native,
            build: "vornui-spike".into(),
        }));
        for s in &sessions {
            client.attach(Attach {
                session: s.clone(),
                mode: AttachMode::Grid,
                view: Size {
                    cols,
                    rows,
                    px_w: 0,
                    px_h: 0,
                },
                visible: true,
                resume: None,
                history_tail: 0,
            });
        }
        let n = sessions.len();
        let (tx, rx) = mpsc::unbounded_channel();
        let grid = Arc::new(Grid {
            state: Mutex::new(State {
                client,
                sessions,
                sids: vec![None; n],
                dirty: vec![false; n],
                snapshotted: vec![false; n],
                probes: (0..n).map(|_| None).collect(),
                errors: Vec::new(),
                closed: false,
                changes: 0,
            }),
            changed: Condvar::new(),
            out: tx,
        });
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()?;
        let conn = rt.block_on(open(endpoint))?;
        grid.flush();
        let g = Arc::clone(&grid);
        std::thread::Builder::new()
            .name("grid-io".into())
            .spawn(move || {
                rt.block_on(g.io(conn, rx));
            })?;
        Ok(grid)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A poisoned lock only means a UI thread panicked; the state is still whole.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn panes(&self) -> usize {
        self.lock().sessions.len()
    }

    /// Blocks until something changed after `seen` or `deadline` passes;
    /// returns the change count to pass next time.
    pub fn wait(&self, seen: u64, deadline: Instant) -> u64 {
        let mut st = self.lock();
        while st.changes == seen && !st.closed {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            st = self
                .changed
                .wait_timeout(st, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        st.changes
    }

    /// The panes that changed since the last call.
    pub fn take_dirty(&self) -> Vec<usize> {
        let mut st = self.lock();
        let out: Vec<usize> = (0..st.dirty.len()).filter(|i| st.dirty[*i]).collect();
        for i in &out {
            st.dirty[*i] = false;
        }
        out
    }

    /// Whether every pane has had its first snapshot.
    pub fn all_snapshotted(&self) -> bool {
        self.lock().snapshotted.iter().all(|s| *s)
    }

    pub fn closed(&self) -> bool {
        self.lock().closed
    }

    pub fn errors(&self) -> Vec<String> {
        self.lock().errors.clone()
    }

    /// The pane as it is now, if it has a screen yet. Taking a view in which
    /// the armed probe's glyph shows disarms the probe and sets `probe_hit`.
    pub fn view(&self, pane: usize) -> Option<PaneView> {
        let mut st = self.lock();
        let sid = (*st.sids.get(pane)?)?;
        let m = st.client.pane(sid)?.mirror()?;
        let mut v = view::build(m, m.rev());
        if let Some(Some(p)) = st.probes.get(pane) {
            if p.seen_rev.is_some_and(|r| r <= v.rev) {
                v.probe_hit = true;
                st.probes[pane] = None;
            }
        }
        Some(v)
    }

    /// The text of `pane`'s screen, one line per row: for checks, not drawing.
    pub fn screen_text(&self, pane: usize) -> String {
        let Some(v) = self.view(pane) else {
            return String::new();
        };
        let mut rows = vec![String::new(); v.rows as usize];
        let mut cols = vec![0u16; v.rows as usize];
        for r in &v.runs {
            if let (Some(line), Some(at)) =
                (rows.get_mut(r.row as usize), cols.get_mut(r.row as usize))
            {
                // Runs skip blank cells; pad to the run's column.
                line.extend(std::iter::repeat_n(' ', r.col.saturating_sub(*at) as usize));
                line.push_str(v.run_text(r));
                *at = r.col + r.ncols;
            }
        }
        rows.join("\n")
    }

    /// Arms the latency probe on `pane`: the next view in which `ch` shows
    /// at the cursor's current cell is a hit.
    pub fn probe_arm(&self, pane: usize, ch: char) {
        let mut st = self.lock();
        let Some(Some(sid)) = st.sids.get(pane).copied() else {
            return;
        };
        let Some(m) = st.client.pane(sid).and_then(|p| p.mirror()) else {
            return;
        };
        let c = m.term().cursor;
        st.probes[pane] = Some(Probe {
            x: c.x,
            y: c.y,
            ch: ch.to_string(),
            seen_rev: None,
        });
    }

    /// Whether a probe is still armed on `pane`.
    pub fn probe_pending(&self, pane: usize) -> bool {
        matches!(self.lock().probes.get(pane), Some(Some(_)))
    }

    pub fn probe_cancel(&self, pane: usize) {
        if let Some(p) = self.lock().probes.get_mut(pane) {
            *p = None;
        }
    }

    /// A key press: `code` is the W3C `code` name ("KeyA", "Enter") or the
    /// wire's own name ("A"); `text` what it types, if anything.
    pub fn key(&self, pane: usize, code: &str, mods: u16, text: Option<&str>) {
        let name = code
            .strip_prefix("Key")
            .filter(|l| l.len() == 1)
            .unwrap_or(code);
        let key = KeyCode::from_name(name).unwrap_or_default();
        let text = text.filter(|t| !t.is_empty() && !t.chars().any(char::is_control));
        let unshifted = text
            .and_then(|t| t.chars().next())
            .map(|c| c.to_ascii_lowercase());
        self.input(
            pane,
            InputEvent::Key {
                action: KeyAction::Press,
                key,
                mods,
                consumed_mods: 0,
                text: text.map(str::to_owned),
                unshifted,
                composing: false,
            },
        );
    }

    /// Committed text: IME output, or anything typed with no key behind it.
    pub fn text(&self, pane: usize, utf8: &str) {
        self.input(
            pane,
            InputEvent::Text {
                utf8: utf8.to_owned(),
            },
        );
    }

    fn input(&self, pane: usize, event: InputEvent) {
        {
            let mut st = self.lock();
            let Some(Some(sid)) = st.sids.get(pane).copied() else {
                return;
            };
            let input_seq = st.client.input_seq();
            st.client.send(&ClientMsg::Input {
                sid,
                input_seq,
                event,
            });
        }
        self.flush();
    }

    /// The pane's viewport changed: say so and take the size, so the
    /// session follows the window.
    pub fn resize(&self, pane: usize, cols: u16, rows: u16) {
        {
            let mut st = self.lock();
            let Some(Some(sid)) = st.sids.get(pane).copied() else {
                return;
            };
            st.client.send(&ClientMsg::Viewport {
                sid,
                size: Size {
                    cols: cols.max(2),
                    rows: rows.max(1),
                    px_w: 0,
                    px_h: 0,
                },
            });
            st.client.send(&ClientMsg::TakeSize { sid });
        }
        self.flush();
    }

    fn flush(&self) {
        let out = self.lock().client.take_out();
        if !out.is_empty() {
            let _ = self.out.send(out);
        }
    }

    /// Reads and writes on one task: a synchronous Windows pipe handle
    /// would serialize a blocking read against every write.
    async fn io(self: Arc<Self>, conn: Conn, mut rx: mpsc::UnboundedReceiver<Vec<u8>>) {
        let (mut rd, mut wr) = tokio::io::split(conn);
        let mut buf = vec![0u8; 256 << 10];
        loop {
            tokio::select! {
                r = rd.read(&mut buf) => match r {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if !self.receive(&buf[..n]) {
                            break;
                        }
                    }
                },
                Some(bytes) = rx.recv() => {
                    if wr.write_all(&bytes).await.is_err() {
                        break;
                    }
                }
            }
        }
        let mut st = self.lock();
        st.closed = true;
        st.changes += 1;
        self.changed.notify_all();
    }

    /// Applies what came in; false once the stream cannot be decoded.
    fn receive(&self, bytes: &[u8]) -> bool {
        let mut st = self.lock();
        let got = match st.client.receive(bytes) {
            Ok(g) => g,
            Err(e) => {
                st.errors.push(format!("decode: {e}"));
                return false;
            }
        };
        let mut changed = false;
        for g in got {
            match g {
                Got::Attached(a) => {
                    let free = (0..st.sessions.len())
                        .find(|&i| st.sessions[i] == a.session && st.sids[i].is_none());
                    if let Some(i) = free {
                        st.sids[i] = Some(a.sid);
                    }
                }
                Got::Frame { sid, rev, snapshot } => {
                    st.client.ack(sid, rev);
                    if let Some(i) = st.pane_of(sid) {
                        st.dirty[i] = true;
                        st.snapshotted[i] |= snapshot;
                        changed = true;
                        check_probe(&mut st, i, sid, rev);
                    }
                }
                Got::Refused { sid, why } => {
                    st.errors
                        .push(format!("sid {sid} refused a frame: {why:?}"));
                }
                Got::Message(ServerMsg::Error { code, message }) => {
                    st.errors.push(format!("error {code}: {message}"));
                }
                _ => {}
            }
        }
        let out = st.client.take_out();
        if changed {
            st.changes += 1;
            self.changed.notify_all();
        }
        drop(st);
        if !out.is_empty() {
            let _ = self.out.send(out);
        }
        true
    }
}

fn check_probe(st: &mut State, pane: usize, sid: u32, rev: u64) {
    let Some(p) = st.probes[pane].as_ref() else {
        return;
    };
    if p.seen_rev.is_some() {
        return;
    }
    let Some(m) = st.client.pane(sid).and_then(|p| p.mirror()) else {
        return;
    };
    if view::cell_at(m, p.x, p.y).as_deref() == Some(p.ch.as_str()) {
        if let Some(p) = st.probes[pane].as_mut() {
            p.seen_rev = Some(rev);
        }
    }
}

trait Duplex: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Duplex for T {}
type Conn = Box<dyn Duplex>;

#[cfg(unix)]
async fn open(endpoint: &str) -> std::io::Result<Conn> {
    Ok(Box::new(tokio::net::UnixStream::connect(endpoint).await?))
}

#[cfg(windows)]
async fn open(endpoint: &str) -> std::io::Result<Conn> {
    use tokio::net::windows::named_pipe::ClientOptions;
    const ERROR_PIPE_BUSY: i32 = 231;
    let started = Instant::now();
    loop {
        match ClientOptions::new().open(endpoint) {
            Ok(c) => return Ok(Box::new(c)),
            Err(e)
                if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && started.elapsed().as_secs() < 5 =>
            {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(e) => return Err(e),
        }
    }
}

/// Columns and rows of an `n`-pane grid in a landscape window.
pub fn layout(n: usize) -> (usize, usize) {
    let n = n.max(1);
    let cols = ((n as f64 * 2.0).sqrt().ceil() as usize).min(n);
    let rows = n.div_ceil(cols);
    (cols, rows)
}

#[cfg(test)]
mod tests {
    #[test]
    fn layouts() {
        assert_eq!(super::layout(1), (1, 1));
        assert_eq!(super::layout(8), (4, 2));
        assert_eq!(super::layout(32), (8, 4));
    }
}
