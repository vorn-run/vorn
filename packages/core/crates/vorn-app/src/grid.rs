//! The terminal grid: one connection to vornd's grid endpoint, one pane per
//! session on screen. Panes are attached and detached as cards come and go;
//! an I/O thread keeps each pane's mirror, returns every frame's credit as
//! soon as it is applied, and marks the pane dirty for the next frame.
//!
//! A card's size, with what it measures in pixels, goes to vornd whenever it
//! changes; vornd's size rule coalesces a drag's sizes into a few resizes.
//! The card draws its last screen clipped or padded to the box meanwhile.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::mpsc;
use vorn_grid_client::{Client, Got};
use vorn_term_proto::msg::{
    caps, Attach, AttachMode, ClientKind, ClientMsg, CopyFormat, GridPoint, Hello, InputEvent,
    KeyAction, KeyCode, SelectKind, ServerMsg, Size, PROTO_MAJOR, PROTO_MINOR,
};
use vorn_term_proto::screen::Screen;

use vorn_term_proto::screen::{row_flags, Row, Screen as TermScreen};

use crate::client::link::{self, Frame, Inbound, Transport};
use crate::client::rpc::Wake;
use crate::scroll::Route;
use crate::view::{self, PaneView};

/// History lines a shell attaches with, for its earlier command blocks.
pub const SHELL_HISTORY: u16 = 1000;

/// Where a terminal's lines are, for cutting it into command blocks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lines {
    /// Line numbers are only comparable within one scrollback epoch.
    pub epoch: u32,
    /// Absolute lines where a prompt starts, oldest first, up to the cursor.
    pub prompts: Vec<u64>,
    /// The absolute line of screen row 0.
    pub top: u64,
    pub cursor_y: u16,
    pub rows: u16,
    /// A full-screen program has the alternate screen.
    pub alt: bool,
}

struct Pane {
    session: String,
    /// Known once vornd says the attach took.
    sid: Option<u32>,
    dirty: bool,
    /// The card's box, in cells.
    size: (u16, u16),
    /// One cell in device pixels, 0 when unknown.
    cell: (u32, u32),
    /// The size and cell last sent to vornd.
    sent: ((u16, u16), (u32, u32)),
    /// History lines asked for on attach and after each scrollback epoch.
    history: u16,
    /// The scrollback epoch history was last asked for in.
    epoch: Option<u32>,
    /// The absolute line the view starts at while scrolled back.
    top: Option<u64>,
    /// The history fetch in flight, as `(sb_epoch, from_line)`.
    fetching: Option<(u32, u64)>,
}

struct State {
    client: Client,
    panes: Vec<Pane>,
    /// The id of the next history fetch.
    req: u32,
    errors: Vec<String>,
    /// Why vornd will not serve this client: dialing it again cannot help.
    refused: Option<String>,
    closed: bool,
    /// Recent pastes by input sequence, kept until vornd may ask to confirm one.
    pastes: VecDeque<(u64, String, String)>,
    /// Requests in flight, by request id: the session each is for.
    asks: Vec<(u32, String)>,
    next_req: u32,
    replies: Vec<Reply>,
}

/// `cols`x`rows` of `cell`-sized cells, with their size in pixels.
fn measured(cols: u16, rows: u16, (w, h): (u32, u32)) -> Size {
    Size {
        cols,
        rows,
        px_w: u32::from(cols).saturating_mul(w),
        px_h: u32::from(rows).saturating_mul(h),
    }
}

/// vornd's error for a grid protocol it does not serve.
const UPGRADE: u16 = 426;

/// History fetched ahead of a scrolled view, above it.
const AHEAD: u64 = 200;
/// The most rows vornd sends for one fetch.
const MAX_FETCH: u64 = 1000;

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

    /// Asks for the first history a scrolled view lacks, from a few pages
    /// above it to its last row; one fetch at a time.
    fn fetch_history(&mut self, i: usize) {
        let p = &self.panes[i];
        let (Some(sid), Some(top), None) = (p.sid, p.top, p.fetching) else {
            return;
        };
        let Some(m) = self.client.pane(sid).and_then(|x| x.mirror()) else {
            return;
        };
        let t = m.term();
        let rows = u64::from(t.rows);
        let oldest = t.top_line.saturating_sub(t.history_lines);
        let from = top.saturating_sub(AHEAD.max(rows * 4)).max(oldest);
        let to = (top + rows + 1).min(t.top_line);
        // Nearest the view first: the rows it shows before the ones above.
        let Some((from_line, count)) = m
            .gaps(from, to)
            .last()
            .map(|&(a, n)| (a + n.saturating_sub(MAX_FETCH), n.min(MAX_FETCH)))
        else {
            return;
        };
        let sb_epoch = t.sb_epoch;
        self.req = self.req.wrapping_add(1);
        let req = self.req;
        self.panes[i].fetching = Some((sb_epoch, from_line));
        self.client.send(&ClientMsg::FetchHistory {
            sid,
            req,
            sb_epoch,
            from_line,
            count: count as u16,
        });
    }
}

/// The grid connection. Shared between the UI and its I/O thread.
pub struct Grid {
    state: Mutex<State>,
    out: mpsc::UnboundedSender<Frame>,
    wake: Wake,
}

impl Grid {
    /// Connects to `endpoint` (a Unix socket path, or a named pipe on
    /// Windows; in a browser, the page's `/grid` WebSocket admitting `token`)
    /// and says hello; `wake` runs whenever a pane changes.
    pub fn connect(endpoint: &str, token: &str, wake: Wake) -> std::io::Result<Arc<Grid>> {
        #[cfg(not(target_arch = "wasm32"))]
        let link = {
            let _ = token;
            link::LocalSocket(endpoint.to_owned())
        };
        #[cfg(target_arch = "wasm32")]
        let link = link::PageSocket {
            url: endpoint.to_owned(),
            token: token.to_owned(),
        };
        Grid::over(Box::new(link), wake)
    }

    /// Speaks the grid protocol over `link`.
    pub fn over(link: Box<dyn Transport>, wake: Wake) -> std::io::Result<Arc<Grid>> {
        let mut client = Client::new();
        client.send(&ClientMsg::Hello(Hello {
            proto_major: PROTO_MAJOR,
            proto_minor: PROTO_MINOR,
            caps: caps::ALL,
            client: if cfg!(target_arch = "wasm32") {
                ClientKind::Web
            } else {
                ClientKind::Native
            },
            build: concat!("vorn-app ", env!("CARGO_PKG_VERSION")).into(),
        }));
        let (tx, rx) = mpsc::unbounded_channel();
        let grid = Arc::new(Grid {
            state: Mutex::new(State {
                client,
                panes: Vec::new(),
                req: 0,
                errors: Vec::new(),
                refused: None,
                closed: false,
                pastes: VecDeque::new(),
                asks: Vec::new(),
                next_req: 0,
                replies: Vec::new(),
            }),
            out: tx,
            wake,
        });
        grid.flush();
        let (rd, end) = (Arc::clone(&grid), Arc::clone(&grid));
        link.start(
            rx,
            Inbound {
                frame: Box::new(move |f| match f {
                    Frame::Binary(bytes) => rd.receive(&bytes),
                    Frame::Text(_) | Frame::Json(_) => true,
                }),
                ended: Box::new(move |_| {
                    end.lock().closed = true;
                    (end.wake)();
                }),
            },
        )
        .map_err(std::io::Error::other)?;
        Ok(grid)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A poisoned lock only means a UI thread panicked; the state is still whole.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Attaches `session` at `cols`x`rows` of `cell`-sized cells (device
    /// pixels, 0 when unknown) with up to `history` lines of scrollback,
    /// unless it is already.
    pub fn attach(&self, session: &str, cols: u16, rows: u16, cell: (u32, u32), history: u16) {
        {
            let mut st = self.lock();
            if st.pane(session).is_some() {
                return;
            }
            let (cols, rows) = (cols.max(2), rows.max(1));
            st.client.attach(Attach {
                session: session.to_owned(),
                mode: AttachMode::Grid,
                view: measured(cols, rows, cell),
                visible: true,
                resume: None,
                history_tail: history,
            });
            st.panes.push(Pane {
                session: session.to_owned(),
                sid: None,
                dirty: false,
                size: (cols, rows),
                cell,
                sent: ((cols, rows), cell),
                history,
                epoch: None,
                top: None,
                fetching: None,
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

    /// Why vornd refused this client, when it did.
    pub fn refused(&self) -> Option<String> {
        self.lock().refused.clone()
    }

    /// What went wrong on the connection, oldest first.
    pub fn errors(&self) -> Vec<String> {
        self.lock().errors.clone()
    }

    /// `session`'s screen as it is now, once it has one, scrolled back as
    /// [`scroll_to`](Self::scroll_to) left it.
    pub fn view(&self, session: &str) -> Option<PaneView> {
        let st = self.lock();
        let p = st.pane(session)?;
        let m = st.client.pane(p.sid?)?.mirror()?;
        Some(view::build(m, p.top))
    }

    /// Starts the view `back` rows above the live screen, fetching the
    /// history around it; 0 is the live screen. Returns whether it moved.
    pub fn scroll_to(&self, session: &str, back: u64) -> bool {
        {
            let mut st = self.lock();
            let Some(i) = st.panes.iter().position(|p| p.session == session) else {
                return false;
            };
            let Some(m) = st.panes[i]
                .sid
                .and_then(|sid| st.client.pane(sid))
                .and_then(|x| x.mirror())
            else {
                return false;
            };
            let t = m.term();
            let top = (back > 0 && t.screen == Screen::Primary)
                .then(|| t.top_line - back.min(t.history_lines));
            if st.panes[i].top == top {
                return false;
            }
            st.panes[i].top = top;
            st.fetch_history(i);
        }
        self.flush();
        true
    }

    /// `rows` wheel rows (negative up) for a program: wheel reports when
    /// it tracks the mouse, else arrow keys, at cell `(x, y)`.
    pub fn wheel_rows(&self, session: &str, rows: i32, route: Route, x: f32, y: f32) {
        let dy = if rows < 0 { -1.0 } else { 1.0 };
        for _ in 0..rows.unsigned_abs().min(100) {
            match route {
                Route::Program => self.wheel(session, dy, x, y),
                Route::Arrows => {
                    let code = if rows < 0 { "ArrowUp" } else { "ArrowDown" };
                    self.key(session, code, 0, None);
                }
                Route::History => return,
            }
        }
    }

    /// Where `session`'s lines are: its prompts and cursor in absolute
    /// lines, across the history held and the screen.
    pub fn lines(&self, session: &str) -> Option<Lines> {
        let st = self.lock();
        let m = st.client.pane(st.sid(session)?)?.mirror()?;
        let t = m.term();
        let prompt = |r: &&Row| {
            r.flags & row_flags::PROMPT != 0 && r.flags & row_flags::PROMPT_CONTINUATION == 0
        };
        let alt = t.screen == TermScreen::Alternate;
        let mut prompts: Vec<u64> = m.history().iter().filter(prompt).map(|r| r.line).collect();
        if !alt {
            let screen = m.rows().iter().take(usize::from(t.cursor.y) + 1);
            prompts.extend(screen.filter(prompt).map(|r| r.line));
        }
        Some(Lines {
            epoch: t.sb_epoch,
            prompts,
            top: t.top_line,
            cursor_y: t.cursor.y,
            rows: t.rows,
            alt,
        })
    }

    /// `session`'s lines `from..to` as a view of their own, blank lines at
    /// either end left out.
    pub fn lines_view(&self, session: &str, from: u64, to: u64) -> Option<PaneView> {
        let st = self.lock();
        let m = st.client.pane(st.sid(session)?)?.mirror()?;
        let top = m.term().top_line;
        let mut rows: Vec<&Row> = (from..to.min(top))
            .filter_map(|l| m.history_line(l))
            .collect();
        // History still on its way: not cut yet, rather than cut short.
        if rows.len() as u64 != to.min(top).saturating_sub(from) {
            return None;
        }
        rows.extend(
            m.rows()
                .iter()
                .filter(|r| r.line >= from.max(top) && r.line < to),
        );
        let blank = |r: &&Row| vorn_term_mirror::row_text(r).trim().is_empty();
        let first = rows.iter().position(|r| !blank(r)).unwrap_or(rows.len());
        let last = rows
            .iter()
            .rposition(|r| !blank(r))
            .map_or(first, |i| i + 1);
        Some(view::rows(m, rows[first..last].iter().copied()))
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

    /// The card's terminal is now `cols`x`rows` of `cell`-sized cells: says
    /// so and takes the size, so the session follows the window. Nothing is
    /// sent when that is what was sent last; vornd's size rule paces a drag.
    pub fn resize(&self, session: &str, cols: u16, rows: u16, cell: (u32, u32)) {
        let want = (cols.max(2), rows.max(1));
        {
            let mut st = self.lock();
            let Some(pane) = st.panes.iter_mut().find(|p| p.session == session) else {
                return;
            };
            pane.size = want;
            pane.cell = cell;
            if pane.sent == (want, cell) {
                return;
            }
            let Some(sid) = pane.sid else { return };
            pane.sent = (want, cell);
            Self::send_size(&mut st.client, sid, want, cell);
        }
        self.flush();
    }

    fn send_size(client: &mut Client, sid: u32, (cols, rows): (u16, u16), cell: (u32, u32)) {
        client.send(&ClientMsg::Viewport {
            sid,
            size: measured(cols, rows, cell),
        });
        client.send(&ClientMsg::TakeSize { sid });
    }

    fn flush(&self) {
        let out = self.lock().client.take_out();
        if !out.is_empty() {
            let _ = self.out.send(Frame::Binary(out));
        }
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
                            p.sent = (p.size, p.cell);
                            Self::send_size(&mut st.client, a.sid, p.size, p.cell);
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
                    let st = &mut *st;
                    if let Some(p) = st.panes.iter_mut().find(|p| p.sid == Some(sid)) {
                        p.dirty = true;
                        changed = true;
                        let term = st.client.pane(sid).and_then(|c| c.mirror()).map(|m| {
                            let t = m.term();
                            (t.sb_epoch, t.top_line, t.history_lines)
                        });
                        // A new epoch dropped the history; ask again for what the attach had.
                        if let Some((epoch, top, held)) = term {
                            let was = p.epoch.replace(epoch);
                            let count = u64::from(p.history).min(held).min(top);
                            if was.is_some_and(|e| e != epoch) && count > 0 {
                                st.req = st.req.wrapping_add(1);
                                st.client.send(&ClientMsg::FetchHistory {
                                    sid,
                                    req: st.req,
                                    sb_epoch: epoch,
                                    from_line: top - count,
                                    count: count as u16,
                                });
                            }
                        }
                    }
                }
                Got::Message(ServerMsg::History {
                    sid,
                    sb_epoch,
                    from_line,
                    ..
                }) => {
                    let Some(i) = st.panes.iter().position(|p| p.sid == Some(sid)) else {
                        continue;
                    };
                    let asked = st.panes[i].fetching.take() == Some((sb_epoch, from_line));
                    let filled = st
                        .client
                        .pane(sid)
                        .and_then(|x| x.mirror())
                        .is_some_and(|m| m.gaps(from_line, from_line + 1).is_empty());
                    // A reply that filled nothing is not asked for again.
                    if !asked || filled {
                        st.fetch_history(i);
                    }
                    st.panes[i].dirty = true;
                    changed = true;
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
                        if code == UPGRADE {
                            st.refused = Some(message.clone());
                        }
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
            let _ = self.out.send(Frame::Binary(out));
        }
        if changed {
            (self.wake)();
        }
        true
    }
}
