//! The terminal grid: one connection to vornd's grid endpoint, one pane per
//! session on screen. Panes are attached and detached as cards come and go;
//! an I/O thread keeps each pane's mirror, returns every frame's credit as
//! soon as it is applied, and marks the pane dirty for the next frame.

use std::sync::{Arc, Mutex, MutexGuard};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use vorn_grid_client::{Client, Got};
use vorn_term_proto::msg::{
    caps, Attach, AttachMode, ClientKind, ClientMsg, Hello, InputEvent, KeyAction, KeyCode,
    ServerMsg, Size, PROTO_MAJOR, PROTO_MINOR,
};

use crate::client::rpc::Wake;
use crate::view::{self, PaneView};

struct Pane {
    session: String,
    /// Known once vornd says the attach took.
    sid: Option<u32>,
    dirty: bool,
    size: (u16, u16),
}

struct State {
    client: Client,
    panes: Vec<Pane>,
    errors: Vec<String>,
    closed: bool,
}

impl State {
    fn pane(&self, session: &str) -> Option<&Pane> {
        self.panes.iter().find(|p| p.session == session)
    }

    fn sid(&self, session: &str) -> Option<u32> {
        self.pane(session)?.sid
    }
}

/// The grid connection. Shared between the UI and its I/O thread.
pub struct Grid {
    state: Mutex<State>,
    out: mpsc::UnboundedSender<Vec<u8>>,
    wake: Wake,
}

impl Grid {
    /// Connects to `endpoint` (a Unix socket path, or a named pipe on
    /// Windows) and says hello; `wake` runs whenever a pane changes.
    pub fn connect(endpoint: &str, wake: Wake) -> std::io::Result<Arc<Grid>> {
        let mut client = Client::new();
        client.send(&ClientMsg::Hello(Hello {
            proto_major: PROTO_MAJOR,
            proto_minor: PROTO_MINOR,
            caps: caps::ALL,
            client: ClientKind::Native,
            build: concat!("vorn-app ", env!("CARGO_PKG_VERSION")).into(),
        }));
        let (tx, rx) = mpsc::unbounded_channel();
        let grid = Arc::new(Grid {
            state: Mutex::new(State {
                client,
                panes: Vec::new(),
                errors: Vec::new(),
                closed: false,
            }),
            out: tx,
            wake,
        });
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()?;
        let conn = rt.block_on(open(endpoint))?;
        grid.flush();
        let g = Arc::clone(&grid);
        std::thread::Builder::new()
            .name("grid-io".into())
            .spawn(move || rt.block_on(g.io(conn, rx)))?;
        Ok(grid)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A poisoned lock only means a UI thread panicked; the state is still whole.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Attaches `session` at `cols`x`rows`, unless it is already.
    pub fn attach(&self, session: &str, cols: u16, rows: u16) {
        {
            let mut st = self.lock();
            if st.pane(session).is_some() {
                return;
            }
            let (cols, rows) = (cols.max(2), rows.max(1));
            st.client.attach(Attach {
                session: session.to_owned(),
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
            st.panes.push(Pane {
                session: session.to_owned(),
                sid: None,
                dirty: false,
                size: (cols, rows),
            });
        }
        self.flush();
    }

    /// Detaches `session`; its screen is forgotten.
    pub fn detach(&self, session: &str) {
        {
            let mut st = self.lock();
            let Some(i) = st.panes.iter().position(|p| p.session == session) else {
                return;
            };
            let pane = st.panes.remove(i);
            if let Some(sid) = pane.sid {
                st.client.send(&ClientMsg::Detach { sid });
                st.client.drop_pane(sid);
            }
        }
        self.flush();
    }

    /// The sessions attached, in the order they were.
    pub fn attached(&self) -> Vec<String> {
        self.lock()
            .panes
            .iter()
            .map(|p| p.session.clone())
            .collect()
    }

    /// The sessions whose screens changed since the last call.
    pub fn take_dirty(&self) -> Vec<String> {
        let mut st = self.lock();
        st.panes
            .iter_mut()
            .filter_map(|p| std::mem::take(&mut p.dirty).then(|| p.session.clone()))
            .collect()
    }

    /// Whether the connection has ended.
    pub fn closed(&self) -> bool {
        self.lock().closed
    }

    /// What went wrong on the connection, oldest first.
    pub fn errors(&self) -> Vec<String> {
        self.lock().errors.clone()
    }

    /// `session`'s screen as it is now, once it has one.
    pub fn view(&self, session: &str) -> Option<PaneView> {
        let st = self.lock();
        let m = st.client.pane(st.sid(session)?)?.mirror()?;
        Some(view::build(m))
    }

    /// A key press: `code` is the W3C `code` name ("KeyA", "Enter"), `text`
    /// what it types, if anything.
    pub fn key(&self, session: &str, code: &str, mods: u16, text: Option<&str>) {
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
            session,
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

    /// Committed text: IME output, or a paste.
    pub fn text(&self, session: &str, utf8: &str) {
        self.input(
            session,
            InputEvent::Text {
                utf8: utf8.to_owned(),
            },
        );
    }

    fn input(&self, session: &str, event: InputEvent) {
        {
            let mut st = self.lock();
            let Some(sid) = st.sid(session) else {
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

    /// The card's terminal is now `cols`x`rows`: says so and takes the
    /// size, so the session follows the window. Nothing is sent when the
    /// size is what it was.
    pub fn resize(&self, session: &str, cols: u16, rows: u16) {
        let (cols, rows) = (cols.max(2), rows.max(1));
        {
            let mut st = self.lock();
            let Some(pane) = st.panes.iter_mut().find(|p| p.session == session) else {
                return;
            };
            if pane.size == (cols, rows) {
                return;
            }
            pane.size = (cols, rows);
            let Some(sid) = pane.sid else {
                return;
            };
            Self::send_size(&mut st.client, sid, cols, rows);
        }
        self.flush();
    }

    fn send_size(client: &mut Client, sid: u32, cols: u16, rows: u16) {
        client.send(&ClientMsg::Viewport {
            sid,
            size: Size {
                cols,
                rows,
                px_w: 0,
                px_h: 0,
            },
        });
        client.send(&ClientMsg::TakeSize { sid });
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
                bytes = rx.recv() => {
                    let Some(bytes) = bytes else { break };
                    if wr.write_all(&bytes).await.is_err() {
                        break;
                    }
                }
            }
        }
        self.lock().closed = true;
        (self.wake)();
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
                    let st = &mut *st;
                    let pane = st
                        .panes
                        .iter_mut()
                        .find(|p| p.session == a.session && p.sid.is_none());
                    match pane {
                        Some(p) => {
                            p.sid = Some(a.sid);
                            // The card may have changed size while the attach was on its way.
                            let (cols, rows) = p.size;
                            Self::send_size(&mut st.client, a.sid, cols, rows);
                        }
                        // Detached before vornd answered.
                        None => {
                            st.client.send(&ClientMsg::Detach { sid: a.sid });
                            st.client.drop_pane(a.sid);
                        }
                    }
                }
                Got::Frame { sid, rev, .. } => {
                    st.client.ack(sid, rev);
                    if let Some(p) = st.panes.iter_mut().find(|p| p.sid == Some(sid)) {
                        p.dirty = true;
                        changed = true;
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
        drop(st);
        if !out.is_empty() {
            let _ = self.out.send(out);
        }
        if changed {
            (self.wake)();
        }
        true
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
    use std::time::{Duration, Instant};
    use tokio::net::windows::named_pipe::ClientOptions;
    const ERROR_PIPE_BUSY: i32 = 231;
    let started = Instant::now();
    loop {
        match ClientOptions::new().open(endpoint) {
            Ok(c) => return Ok(Box::new(c)),
            Err(e)
                if e.raw_os_error() == Some(ERROR_PIPE_BUSY)
                    && started.elapsed() < Duration::from_secs(5) =>
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(e) => return Err(e),
        }
    }
}
