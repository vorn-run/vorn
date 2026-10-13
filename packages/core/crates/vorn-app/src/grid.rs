//! The terminal grid: one connection to vornd's grid endpoint, one pane per
//! session on screen. Panes are attached and detached as cards come and go;
//! an I/O thread keeps each pane's mirror, returns every frame's credit as
//! soon as it is applied, and marks the pane dirty for the next frame.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use vorn_grid_client::{Client, Got};
use vorn_term_proto::msg::{
    caps, Attach, AttachMode, ClientKind, ClientMsg, CopyFormat, GridPoint, Hello, InputEvent,
    KeyAction, KeyCode, SelectKind, ServerMsg, Size, PROTO_MAJOR, PROTO_MINOR,
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
    /// Recent pastes by input sequence, kept until vornd may ask to confirm one.
    pastes: VecDeque<(u64, String, String)>,
    /// Requests in flight, by request id: the session each is for.
    asks: Vec<(u32, String)>,
    next_req: u32,
    replies: Vec<Reply>,
}

/// What vornd answered or asked about a pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// The word around a point, if there is one.
    Selection {
        session: String,
        range: Option<(GridPoint, GridPoint)>,
    },
    /// A selection's text.
    Copied(String),
    /// A paste the program cannot tell from typing: run it only once the person says so.
    ConfirmPaste { session: String, text: String },
}

/// How many recent pastes are kept for a confirmation that may come.
const PASTES_KEPT: usize = 4;

impl State {
    fn pane(&self, session: &str) -> Option<&Pane> {
        self.panes.iter().find(|p| p.session == session)
    }

    fn sid(&self, session: &str) -> Option<u32> {
        self.pane(session)?.sid
    }

    /// The session request `req` was for, now that it is answered.
    fn answered(&mut self, req: u32) -> Option<String> {
        let i = self.asks.iter().position(|(r, _)| *r == req)?;
        Some(self.asks.remove(i).1)
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
                pastes: VecDeque::new(),
                asks: Vec::new(),
                next_req: 0,
                replies: Vec::new(),
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

    /// Bracketed paste of the person's clipboard. vornd asks first, as a
    /// [`Reply::ConfirmPaste`], when the program could take it for typed commands.
    pub fn paste_clipboard(&self, session: &str, utf8: &str) {
        let seq = self.input(
            session,
            InputEvent::Paste {
                utf8: utf8.to_owned(),
                confirmed: false,
            },
        );
        if let Some(seq) = seq {
            let mut st = self.lock();
            if st.pastes.len() == PASTES_KEPT {
                st.pastes.pop_front();
            }
            st.pastes
                .push_back((seq, session.to_owned(), utf8.to_owned()));
        }
    }

    /// Bracketed paste of `utf8`, already confirmed by the person or written by the app.
    pub fn paste(&self, session: &str, utf8: &str) {
        self.input(
            session,
            InputEvent::Paste {
                utf8: utf8.to_owned(),
                confirmed: true,
            },
        );
    }

    /// Asks for the word at `at`; the answer is a [`Reply::Selection`].
    pub fn select_word(&self, session: &str, at: GridPoint) {
        self.ask(session, |sid, req| ClientMsg::SelectAt {
            sid,
            req,
            at,
            kind: SelectKind::Word,
        });
    }

    /// Asks for the text from `from` to `to`; the answer is a [`Reply::Copied`].
    pub fn copy(&self, session: &str, from: GridPoint, to: GridPoint) {
        self.ask(session, |sid, req| ClientMsg::Copy {
            sid,
            req,
            from,
            to,
            rect: false,
            format: CopyFormat::Plain,
        });
    }

    /// What vornd answered or asked since the last call.
    pub fn take_replies(&self) -> Vec<Reply> {
        std::mem::take(&mut self.lock().replies)
    }

    fn ask(&self, session: &str, msg: impl FnOnce(u32, u32) -> ClientMsg) {
        {
            let mut st = self.lock();
            let Some(sid) = st.sid(session) else {
                return;
            };
            st.next_req = st.next_req.wrapping_add(1);
            let req = st.next_req;
            st.asks.push((req, session.to_owned()));
            st.client.send(&msg(sid, req));
        }
        self.flush();
    }

    /// The wheel over the terminal: `dy` in lines, at cell `(x, y)`.
    pub fn wheel(&self, session: &str, dy: f32, x: f32, y: f32) {
        self.input(
            session,
            InputEvent::Wheel {
                dx: 0.0,
                dy,
                mods: 0,
                x,
                y,
            },
        );
    }

    /// Locks the session's size to this client's viewport, or releases it.
    pub fn lock_size(&self, session: &str, locked: bool) {
        self.send(session, |sid| ClientMsg::LockSize { sid, locked });
    }

    /// Makes the session follow this client's viewport ("Fit to this device").
    pub fn take_size(&self, session: &str) {
        self.send(session, |sid| ClientMsg::TakeSize { sid });
    }

    fn send(&self, session: &str, msg: impl FnOnce(u32) -> ClientMsg) {
        {
            let mut st = self.lock();
            let Some(sid) = st.sid(session) else {
                return;
            };
            st.client.send(&msg(sid));
        }
        self.flush();
    }

    /// Sends `event`; its input sequence, once the pane is attached.
    fn input(&self, session: &str, event: InputEvent) -> Option<u64> {
        let input_seq = {
            let mut st = self.lock();
            let sid = st.sid(session)?;
            let input_seq = st.client.input_seq();
            st.client.send(&ClientMsg::Input {
                sid,
                input_seq,
                event,
            });
            input_seq
        };
        self.flush();
        Some(input_seq)
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
                    // An attach that beat the session's start: forget it, so the next paint asks again.
                    if let Some(session) = message.strip_prefix("no session ") {
                        st.panes.retain(|p| p.session != session || p.sid.is_some());
                        changed = true;
                    } else {
                        st.errors.push(format!("error {code}: {message}"));
                    }
                }
                Got::Message(ServerMsg::PasteConfirm { sid, input_seq }) => {
                    let mine = |p: &(u64, String, String)| {
                        p.0 == input_seq && st.pane(&p.1).is_some_and(|p| p.sid == Some(sid))
                    };
                    if let Some(i) = st.pastes.iter().position(mine) {
                        let (_, session, text) = st.pastes.remove(i).unwrap_or_default();
                        st.replies.push(Reply::ConfirmPaste { session, text });
                        changed = true;
                    }
                }
                Got::Message(ServerMsg::Selection { req, range, .. }) => {
                    if let Some(session) = st.answered(req) {
                        st.replies.push(Reply::Selection { session, range });
                        changed = true;
                    }
                }
                Got::Message(ServerMsg::Copied { req, text, .. }) => {
                    if st.answered(req).is_none() {
                        continue;
                    }
                    st.replies.push(Reply::Copied(text));
                    changed = true;
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
