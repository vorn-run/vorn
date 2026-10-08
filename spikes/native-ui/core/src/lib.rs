//! The grid client every prototype links. It connects to vornd's grid
//! endpoint, attaches one pane per session, keeps each pane's
//! [`vorn_term_mirror::Mirror`] through [`vorn_grid_client::Client`] on a
//! reader thread, returns every frame's credit as soon as it is applied, and
//! tells the UI when a pane changed. The UI pulls a [`view::PaneView`] of each
//! changed pane and draws it; everything above that line is the same code in
//! all three prototypes.

pub mod bench;
pub mod view;

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use vorn_grid_client::{Client, Got};
use vorn_term_proto::msg::{
    caps, Attach, AttachMode, ClientKind, ClientMsg, Hello, InputEvent, KeyAction, KeyCode,
    ServerMsg, Size, PROTO_MAJOR, PROTO_MINOR,
};

pub use view::{PaneView, RunC};

type Waker = Box<dyn Fn() + Send + Sync>;

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
}

impl State {
    fn pane_of(&self, sid: u32) -> Option<usize> {
        self.sids.iter().position(|s| *s == Some(sid))
    }
}

pub struct Grid {
    state: Mutex<State>,
    out: Mutex<UnixStream>,
    waker: Mutex<Option<Waker>>,
    wake_pending: AtomicBool,
}

impl Grid {
    /// Connects to `socket`, says hello and attaches one pane per session
    /// at `cols`x`rows`.
    pub fn connect(
        socket: &str,
        sessions: Vec<String>,
        cols: u16,
        rows: u16,
    ) -> std::io::Result<Arc<Grid>> {
        let stream = UnixStream::connect(socket)?;
        let reader = stream.try_clone()?;
        let mut client = Client::new();
        client.send(&ClientMsg::Hello(Hello {
            proto_major: PROTO_MAJOR,
            proto_minor: PROTO_MINOR,
            caps: caps::ALL,
            client: ClientKind::Native,
            build: "native-ui-spike".into(),
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
            }),
            out: Mutex::new(stream),
            waker: Mutex::new(None),
            wake_pending: AtomicBool::new(false),
        });
        grid.flush();
        let g = Arc::clone(&grid);
        std::thread::Builder::new()
            .name("grid-reader".into())
            .spawn(move || g.read_loop(reader))?;
        Ok(grid)
    }

    /// Called from the reader thread when a pane changed and the UI has not
    /// taken the changes since the last call.
    pub fn set_waker(&self, f: impl Fn() + Send + Sync + 'static) {
        self.wake_pending.store(true, Ordering::SeqCst);
        // Frames that came before the waker are already pending: say so.
        f();
        *self.waker.lock().unwrap() = Some(Box::new(f));
    }

    pub fn panes(&self) -> usize {
        self.state.lock().unwrap().sessions.len()
    }

    /// The panes that changed since the last call; re-arms the waker.
    pub fn take_dirty(&self) -> Vec<usize> {
        self.wake_pending.store(false, Ordering::SeqCst);
        let mut st = self.state.lock().unwrap();
        let out: Vec<usize> = (0..st.dirty.len()).filter(|i| st.dirty[*i]).collect();
        for i in &out {
            st.dirty[*i] = false;
        }
        out
    }

    /// Whether every pane has had its first snapshot.
    pub fn all_snapshotted(&self) -> bool {
        self.state.lock().unwrap().snapshotted.iter().all(|s| *s)
    }

    pub fn closed(&self) -> bool {
        self.state.lock().unwrap().closed
    }

    pub fn errors(&self) -> Vec<String> {
        self.state.lock().unwrap().errors.clone()
    }

    /// The pane as it is now, if it has a screen yet. Taking a view in which
    /// the armed probe's glyph shows disarms the probe and sets `probe_hit`.
    pub fn view(&self, pane: usize) -> Option<PaneView> {
        let mut st = self.state.lock().unwrap();
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

    /// Arms the latency probe on `pane`: the next view in which `ch` shows
    /// at the cursor's current cell is a hit.
    pub fn probe_arm(&self, pane: usize, ch: char) {
        let mut st = self.state.lock().unwrap();
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
        matches!(self.state.lock().unwrap().probes.get(pane), Some(Some(_)))
    }

    pub fn probe_cancel(&self, pane: usize) {
        if let Some(p) = self.state.lock().unwrap().probes.get_mut(pane) {
            *p = None;
        }
    }

    /// A key press: `code` is the W3C `code` name ("KeyA", "Enter"...) or
    /// the wire's own name ("A"); `text` what it types, if anything.
    pub fn key(&self, pane: usize, code: &str, mods: u16, text: Option<&str>) {
        // W3C "KeyA" is the wire's "A"; every other name is the same.
        let name = code
            .strip_prefix("Key")
            .filter(|l| l.len() == 1)
            .unwrap_or(code);
        let key = KeyCode::from_name(name).unwrap_or_default();
        let text = text.filter(|t| !t.is_empty() && !t.chars().any(char::is_control));
        let unshifted = text.and_then(|t| t.chars().next()).map(|c| c.to_ascii_lowercase());
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
            let mut st = self.state.lock().unwrap();
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
            let mut st = self.state.lock().unwrap();
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
        let out = self.state.lock().unwrap().client.take_out();
        if !out.is_empty() {
            let _ = self.out.lock().unwrap().write_all(&out);
        }
    }

    fn read_loop(self: Arc<Self>, mut s: UnixStream) {
        let mut buf = vec![0u8; 256 << 10];
        loop {
            let n = match s.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let woke = {
                let mut st = self.state.lock().unwrap();
                let got = match st.client.receive(&buf[..n]) {
                    Ok(g) => g,
                    Err(e) => {
                        st.errors.push(format!("decode: {e}"));
                        break;
                    }
                };
                let mut changed = false;
                for g in got {
                    match g {
                        Got::Attached(a) => {
                            if let Some(i) = st
                                .sessions
                                .iter()
                                .enumerate()
                                .position(|(i, s)| *s == a.session && st.sids[i].is_none())
                            {
                                st.sids[i] = Some(a.sid);
                            }
                        }
                        Got::Frame { sid, rev, snapshot } => {
                            st.client.ack(sid, rev);
                            if let Some(i) = st.pane_of(sid) {
                                st.dirty[i] = true;
                                if snapshot {
                                    st.snapshotted[i] = true;
                                }
                                changed = true;
                                check_probe(&mut st, i, sid, rev);
                            }
                        }
                        Got::Refused { sid, why } => {
                            st.errors.push(format!("sid {sid} refused a frame: {why:?}"));
                        }
                        Got::Message(ServerMsg::Error { code, message }) => {
                            st.errors.push(format!("error {code}: {message}"));
                        }
                        _ => {}
                    }
                }
                changed
            };
            self.flush();
            if woke && !self.wake_pending.swap(true, Ordering::SeqCst) {
                if let Some(w) = self.waker.lock().unwrap().as_ref() {
                    w();
                }
            }
        }
        self.state.lock().unwrap().closed = true;
        if let Some(w) = self.waker.lock().unwrap().as_ref() {
            w();
        }
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
    let (x, y, ch) = (p.x, p.y, p.ch.clone());
    if view::cell_at(m, x, y).as_deref() == Some(ch.as_str()) {
        if let Some(p) = st.probes[pane].as_mut() {
            p.seen_rev = Some(rev);
        }
    }
}

/// Where the prototypes read their configuration: the harness sets these.
pub mod env {
    /// The grid endpoint.
    pub const GRID: &str = "VORN_SPIKE_GRID";
    /// Comma-separated session ids, one pane each.
    pub const SESSIONS: &str = "VORN_SPIKE_SESSIONS";

    pub fn grid() -> Option<String> {
        std::env::var(GRID).ok()
    }

    pub fn sessions() -> Vec<String> {
        std::env::var(SESSIONS)
            .unwrap_or_default()
            .split(',')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect()
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
        assert_eq!(super::layout(16), (6, 3));
        assert_eq!(super::layout(32), (8, 4));
    }
}
