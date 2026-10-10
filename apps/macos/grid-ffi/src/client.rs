//! One grid connection: a [`vorn_grid_client::Client`] behind a mutex, fed by
//! a reader thread that applies and acknowledges every frame as it arrives
//! and wakes the app when a pane changed.
//!
//! Panes have local ids the app keeps; vornd's `sid` for a pane changes when
//! it is re-attached after a refused frame, so the app never sees one.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::c_void;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;

use vorn_grid_client::{Client, Got};
use vorn_term_proto::msg::{
    caps, Attach, AttachMode, ClientKind, ClientMsg, Hello, InputEvent, KeyAction, KeyCode,
    Presence, ServerMsg, Size, PROTO_MAJOR, PROTO_MINOR,
};

use crate::view::{self, PaneView, VgTheme};

/// Where a pane is, as `vg_pane_state` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneState {
    Attaching = 0,
    Live = 1,
    Failed = 2,
    Closed = 3,
}

/// The app's wake-up: a C function and its context.
#[derive(Clone, Copy)]
pub struct Waker {
    pub cb: unsafe extern "C" fn(*mut c_void),
    pub ctx: *mut c_void,
}

// SAFETY: the app hands over a callback that may be called from any thread,
// and the context is only passed back to it.
unsafe impl Send for Waker {}

#[derive(Debug)]
struct Pane {
    session: String,
    sid: Option<u32>,
    state: PaneState,
    /// What fits on the app's side; sent again after a re-attach.
    view: Size,
    /// The viewport vornd last heard for this pane.
    sent_view: Size,
    presence: Option<Presence>,
}

#[derive(Debug)]
struct State {
    client: Client,
    panes: BTreeMap<u32, Pane>,
    /// Attaches awaiting Attached or Error, oldest first, by local id and
    /// session; a pane detached meanwhile stays here until its reply comes.
    pending: VecDeque<(u32, String)>,
    dirty: BTreeSet<u32>,
    next_pane: u32,
    closed: bool,
    last_error: Option<String>,
}

impl State {
    fn pane_of(&self, sid: u32) -> Option<u32> {
        self.panes
            .iter()
            .find(|(_, p)| p.sid == Some(sid))
            .map(|(id, _)| *id)
    }

    fn live_sid(&self, pane: u32) -> Option<u32> {
        self.panes.get(&pane)?.sid
    }

    fn send_attach(&mut self, pane: u32) {
        let Some(p) = self.panes.get_mut(&pane) else {
            return;
        };
        p.sid = None;
        p.state = PaneState::Attaching;
        p.sent_view = p.view;
        let a = Attach {
            session: p.session.clone(),
            mode: AttachMode::Grid,
            view: p.view,
            visible: true,
            resume: None,
            history_tail: 0,
        };
        self.pending.push_back((pane, a.session.clone()));
        self.client.attach(a);
    }

    /// The oldest pending attach the reply is for.
    fn take_pending(&mut self, matches: impl Fn(&str) -> bool) -> Option<u32> {
        let i = self.pending.iter().position(|(_, s)| matches(s))?;
        self.pending.remove(i).map(|(pane, _)| pane)
    }

    /// Applies what one read brought; true when a pane changed.
    fn on(&mut self, got: Got) -> bool {
        match got {
            Got::Attached(a) => {
                let Some(pane) = self.take_pending(|s| s == a.session) else {
                    return false;
                };
                let Some(p) = self.panes.get_mut(&pane) else {
                    // Detached before vornd answered.
                    self.client.send(&ClientMsg::Detach { sid: a.sid });
                    self.client.drop_pane(a.sid);
                    return false;
                };
                p.sid = Some(a.sid);
                // The viewport moved while vornd was attaching it.
                let moved = (p.view != p.sent_view).then_some(p.view);
                p.sent_view = p.view;
                let presence = p.presence;
                if let Some(size) = moved {
                    self.client.send(&ClientMsg::Viewport { sid: a.sid, size });
                }
                if let Some(state) = presence {
                    self.client.send(&ClientMsg::Presence { sid: a.sid, state });
                }
                self.dirty.insert(pane);
                true
            }
            Got::Frame { sid, rev, .. } => {
                let Some(pane) = self.pane_of(sid) else {
                    // A frame in flight for a detached attachment.
                    self.client.drop_pane(sid);
                    return false;
                };
                self.client.ack(sid, rev);
                if let Some(p) = self.panes.get_mut(&pane) {
                    p.state = PaneState::Live;
                }
                self.dirty.insert(pane);
                true
            }
            Got::Refused { sid, why } => {
                self.client.drop_pane(sid);
                let Some(pane) = self.pane_of(sid) else {
                    return false;
                };
                // There is no resync request: start over with a fresh attach.
                self.last_error = Some(format!("frame refused ({why:?}); re-attaching"));
                self.client.send(&ClientMsg::Detach { sid });
                self.send_attach(pane);
                self.dirty.insert(pane);
                true
            }
            Got::Message(ServerMsg::Error { code, message }) => {
                // Errors carry no sid: an attach's is told by the session it names.
                let failed = if matches!(code, 404 | 501) {
                    self.take_pending(|s| message == format!("no session {s}"))
                        .or_else(|| self.take_pending(|s| message.contains(s)))
                } else {
                    None
                };
                self.last_error = Some(format!("{code}: {message}"));
                match failed.and_then(|pane| self.panes.get_mut(&pane).map(|p| (pane, p))) {
                    Some((pane, p)) => {
                        p.state = PaneState::Failed;
                        self.dirty.insert(pane);
                        true
                    }
                    None => false,
                }
            }
            Got::Welcome(_) | Got::Message(_) => false,
        }
    }
}

/// Locks, carrying on past a panic that poisoned the lock: the state stays
/// consistent per message, and the app must keep running.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct Shared {
    state: Mutex<State>,
    out: Mutex<UnixStream>,
    theme: VgTheme,
    waker: Mutex<Option<Waker>>,
    wake_pending: AtomicBool,
}

impl Shared {
    /// Writes what the client has to say. The write lock is taken first so
    /// messages go out in the order they were queued.
    fn flush(&self) {
        let mut w = lock(&self.out);
        let out = lock(&self.state).client.take_out();
        if !out.is_empty() {
            // A failed write shows up as the reader's end of stream.
            let _ = w.write_all(&out);
        }
    }

    fn wake(&self) {
        if self.wake_pending.swap(true, Ordering::SeqCst) {
            return;
        }
        // Called with no lock held, so the callback may call back in.
        let w = *lock(&self.waker);
        if let Some(w) = w {
            // SAFETY: the app registered `cb` to be called with `ctx`.
            unsafe { (w.cb)(w.ctx) };
        }
    }

    fn read_loop(&self, mut s: UnixStream) {
        let mut buf = vec![0u8; 256 << 10];
        loop {
            let n = match s.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    lock(&self.state).last_error = Some(format!("read: {e}"));
                    break;
                }
            };
            let (changed, ok) = {
                let mut st = lock(&self.state);
                match st.client.receive(&buf[..n]) {
                    Ok(got) => (got.into_iter().fold(false, |c, g| st.on(g) | c), true),
                    Err(e) => {
                        st.last_error = Some(format!("decode: {e}"));
                        (false, false)
                    }
                }
            };
            self.flush();
            if changed {
                self.wake();
            }
            if !ok {
                break;
            }
        }
        {
            let mut st = lock(&self.state);
            st.closed = true;
            let ids: Vec<u32> = st.panes.keys().copied().collect();
            st.dirty.extend(ids);
        }
        self.wake();
    }
}

/// A connection to vornd's grid endpoint.
pub struct Grid {
    shared: Arc<Shared>,
    /// A handle on the socket outside every lock, to unblock the reader.
    ctl: UnixStream,
    reader: Option<JoinHandle<()>>,
}

impl Grid {
    /// Connects to `socket` and says Hello as the native app.
    pub fn connect(socket: &str, build: &str, theme: VgTheme) -> std::io::Result<Grid> {
        let stream = UnixStream::connect(socket)?;
        let reader = stream.try_clone()?;
        let ctl = stream.try_clone()?;
        let mut client = Client::new();
        client.send(&ClientMsg::Hello(Hello {
            proto_major: PROTO_MAJOR,
            proto_minor: PROTO_MINOR,
            caps: caps::ALL,
            client: ClientKind::Native,
            build: build.to_owned(),
        }));
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                client,
                panes: BTreeMap::new(),
                pending: VecDeque::new(),
                dirty: BTreeSet::new(),
                next_pane: 0,
                closed: false,
                last_error: None,
            }),
            out: Mutex::new(stream),
            theme,
            waker: Mutex::new(None),
            wake_pending: AtomicBool::new(false),
        });
        shared.flush();
        let s = Arc::clone(&shared);
        let reader = std::thread::Builder::new()
            .name("vorn-grid-reader".into())
            .spawn(move || s.read_loop(reader))?;
        Ok(Grid {
            shared,
            ctl,
            reader: Some(reader),
        })
    }

    /// Sets or clears the waker; calls it once now if changes are waiting.
    pub fn set_waker(&self, w: Option<Waker>) {
        *lock(&self.shared.waker) = w;
        let waiting = {
            let st = lock(&self.shared.state);
            !st.dirty.is_empty() || st.closed
        };
        self.shared.wake_pending.store(false, Ordering::SeqCst);
        if waiting {
            self.shared.wake();
        }
    }

    pub fn attach(&self, session: &str, cols: u16, rows: u16) -> u32 {
        let id = {
            let mut st = lock(&self.shared.state);
            if st.closed {
                return 0;
            }
            st.next_pane += 1;
            let id = st.next_pane;
            let view = viewport(cols, rows);
            st.panes.insert(
                id,
                Pane {
                    session: session.to_owned(),
                    sid: None,
                    state: PaneState::Attaching,
                    view,
                    sent_view: view,
                    presence: None,
                },
            );
            st.send_attach(id);
            id
        };
        self.shared.flush();
        id
    }

    pub fn detach(&self, pane: u32) {
        {
            let mut st = lock(&self.shared.state);
            let Some(p) = st.panes.remove(&pane) else {
                return;
            };
            st.dirty.remove(&pane);
            if let Some(sid) = p.sid {
                st.client.send(&ClientMsg::Detach { sid });
                st.client.drop_pane(sid);
            }
        }
        self.shared.flush();
    }

    /// Panes changed since the last call, at most `cap`; re-arms the waker.
    pub fn take_dirty(&self, cap: usize) -> Vec<u32> {
        self.shared.wake_pending.store(false, Ordering::SeqCst);
        let mut st = lock(&self.shared.state);
        let out: Vec<u32> = st.dirty.iter().take(cap).copied().collect();
        for id in &out {
            st.dirty.remove(id);
        }
        out
    }

    pub fn pane_state(&self, pane: u32) -> Option<PaneState> {
        let st = lock(&self.shared.state);
        let p = st.panes.get(&pane)?;
        Some(if st.closed {
            PaneState::Closed
        } else {
            p.state
        })
    }

    pub fn closed(&self) -> bool {
        lock(&self.shared.state).closed
    }

    pub fn view(&self, pane: u32) -> Option<PaneView> {
        let st = lock(&self.shared.state);
        let m = st.client.pane(st.live_sid(pane)?)?.mirror()?;
        Some(view::build(m, &self.shared.theme))
    }

    /// The viewport's text, rows joined by newlines, trailing blanks trimmed.
    pub fn read_text(&self, pane: u32) -> Option<String> {
        let st = lock(&self.shared.state);
        let m = st.client.pane(st.live_sid(pane)?)?.mirror()?;
        Some(m.text().join("\n"))
    }

    pub fn last_error(&self) -> Option<String> {
        lock(&self.shared.state).last_error.clone()
    }

    /// A key press: `code` is the W3C `code` name ("KeyA", "Enter"...),
    /// `text` what it types, if anything.
    pub fn key(&self, pane: u32, code: &str, mods: u16, text: Option<&str>) {
        self.input(pane, key_event(code, mods, text));
    }

    /// Committed text: IME output or a paste.
    pub fn text(&self, pane: u32, utf8: &str) {
        if utf8.is_empty() {
            return;
        }
        self.input(
            pane,
            InputEvent::Text {
                utf8: utf8.to_owned(),
            },
        );
    }

    fn input(&self, pane: u32, event: InputEvent) {
        self.send(pane, |st, sid| {
            let input_seq = st.client.input_seq();
            Some(ClientMsg::Input {
                sid,
                input_seq,
                event,
            })
        });
    }

    pub fn viewport(&self, pane: u32, cols: u16, rows: u16) {
        let size = viewport(cols, rows);
        {
            let mut st = lock(&self.shared.state);
            let Some(p) = st.panes.get_mut(&pane) else {
                return;
            };
            p.view = size;
            let Some(sid) = p.sid.filter(|_| p.sent_view != size) else {
                return;
            };
            p.sent_view = size;
            st.client.send(&ClientMsg::Viewport { sid, size });
        }
        self.shared.flush();
    }

    pub fn take_size(&self, pane: u32) {
        self.send(pane, |_, sid| Some(ClientMsg::TakeSize { sid }));
    }

    pub fn presence(&self, pane: u32, state: Presence) {
        if let Some(p) = lock(&self.shared.state).panes.get_mut(&pane) {
            p.presence = Some(state);
        }
        self.send(pane, |_, sid| Some(ClientMsg::Presence { sid, state }));
    }

    /// Queues what `msg` builds for the pane's attachment, if it has one.
    fn send(&self, pane: u32, msg: impl FnOnce(&mut State, u32) -> Option<ClientMsg>) {
        {
            let mut st = lock(&self.shared.state);
            let Some(sid) = st.live_sid(pane) else {
                return;
            };
            let Some(m) = msg(&mut st, sid) else { return };
            st.client.send(&m);
        }
        self.shared.flush();
    }
}

impl Drop for Grid {
    fn drop(&mut self) {
        *lock(&self.shared.waker) = None;
        // Unblocks the reader wherever it waits; then it can be joined.
        let _ = self.ctl.shutdown(Shutdown::Both);
        if let Some(h) = self.reader.take() {
            // Closing from the waker, on the reader itself, must not join.
            if h.thread().id() != std::thread::current().id() {
                let _ = h.join();
            }
        }
    }
}

/// vornd takes at least two columns and one row.
fn viewport(cols: u16, rows: u16) -> Size {
    Size {
        cols: cols.max(2),
        rows: rows.max(1),
        px_w: 0,
        px_h: 0,
    }
}

fn key_event(code: &str, mods: u16, text: Option<&str>) -> InputEvent {
    // W3C "KeyA" is the wire's "A"; every other name is the same.
    let name = code
        .strip_prefix("Key")
        .filter(|l| l.len() == 1)
        .unwrap_or(code);
    let key = KeyCode::from_name(name).unwrap_or_default();
    let text = text.filter(|t| !t.is_empty() && !t.chars().any(char::is_control));
    let unshifted = text
        .and_then(|t| t.chars().next())
        .map(|c| c.to_ascii_lowercase());
    InputEvent::Key {
        action: KeyAction::Press,
        key,
        mods,
        consumed_mods: 0,
        text: text.map(str::to_owned),
        unshifted,
        composing: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::tests::{line, snapshot, style};
    use std::os::unix::net::UnixListener;
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};
    use vorn_term_proto::msg::{Attached, FrameReader, Welcome};
    use vorn_term_proto::screen::Color;

    #[test]
    fn keys_map_to_wire_codes() {
        let InputEvent::Key {
            key,
            text,
            unshifted,
            ..
        } = key_event("KeyA", 1, Some("A"))
        else {
            panic!("a key event");
        };
        assert_eq!(key.name(), "A");
        assert_eq!(text.as_deref(), Some("A"));
        assert_eq!(unshifted, Some('a'));
        let InputEvent::Key { key, text, .. } = key_event("Enter", 0, Some("\r")) else {
            panic!("a key event");
        };
        assert_eq!((key.name(), text), ("Enter", None));
        let InputEvent::Key { key, .. } = key_event("Nonsense", 0, None) else {
            panic!("a key event");
        };
        assert_eq!(key.name(), "Unidentified");
    }

    /// A fake vornd: reads client messages, answers with `reply`.
    struct Fake {
        conn: UnixStream,
        reader: FrameReader,
    }

    impl Fake {
        fn next(&mut self) -> ClientMsg {
            let mut buf = [0u8; 4096];
            loop {
                if let Some((k, p)) = self.reader.next_frame().expect("frame") {
                    return ClientMsg::decode(k, p).expect("decode").expect("known");
                }
                let n = self.conn.read(&mut buf).expect("read");
                assert!(n > 0, "client hung up");
                self.reader.push(&buf[..n]);
            }
        }

        fn reply(&mut self, m: &ServerMsg) {
            let mut out = Vec::new();
            m.encode(&mut out);
            self.conn.write_all(&out).expect("write");
        }
    }

    fn wait_for(what: &str, f: impl Fn() -> bool) {
        let until = Instant::now() + Duration::from_secs(5);
        while !f() {
            assert!(Instant::now() < until, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    static WAKES: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "C" fn count_wake(_: *mut c_void) {
        WAKES.fetch_add(1, Ordering::SeqCst);
    }

    #[test]
    fn attaches_applies_acks_fails_and_closes() {
        let dir = std::env::temp_dir().join(format!("vorn-grid-ffi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("grid.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).expect("bind");

        let grid = Grid::connect(path.to_str().expect("utf-8"), "test", VgTheme::default())
            .expect("connect");
        grid.set_waker(Some(Waker {
            cb: count_wake,
            ctx: std::ptr::null_mut(),
        }));
        let (conn, _) = listener.accept().expect("accept");
        let mut fake = Fake {
            conn,
            reader: FrameReader::new(),
        };
        assert!(matches!(fake.next(), ClientMsg::Hello(h) if h.client == ClientKind::Native));
        fake.reply(&ServerMsg::Welcome(Welcome::default()));

        let a = grid.attach("s1", 10, 3);
        let b = grid.attach("gone", 10, 3);
        let c = grid.attach("s1", 10, 3);
        assert_eq!((a, b, c), (1, 2, 3));
        for want in ["s1", "gone", "s1"] {
            assert!(matches!(fake.next(), ClientMsg::Attach(x) if x.session == want));
        }
        fake.reply(&ServerMsg::Error {
            code: 404,
            message: "no session gone".into(),
        });
        for sid in [7, 8] {
            fake.reply(&ServerMsg::Attached(Attached {
                sid,
                session: "s1".into(),
                epoch: 0,
                owner: true,
                fidelity: vorn_term_proto::msg::Fidelity::Exact,
            }));
        }
        let snap = snapshot(
            5,
            vec![style(0, Color::Default, Color::Default, 0)],
            vec![line(0, &[(0, "$ ls")]), line(1, &[(0, "a  b")])],
        );
        fake.reply(&ServerMsg::Snapshot {
            sid: 8,
            snap: Box::new(snap),
        });
        assert_eq!(fake.next(), ClientMsg::Ack { sid: 8, rev: 5 });

        wait_for("pane 3 live", || {
            grid.pane_state(c) == Some(PaneState::Live)
        });
        assert_eq!(grid.pane_state(a), Some(PaneState::Attaching));
        assert_eq!(grid.pane_state(b), Some(PaneState::Failed));
        assert!(WAKES.load(Ordering::SeqCst) >= 1);
        let mut dirty = grid.take_dirty(16);
        dirty.sort_unstable();
        assert_eq!(dirty, vec![1, 2, 3]);
        assert_eq!(grid.read_text(c).as_deref(), Some("$ ls\na  b\n"));
        let v = grid.view(c).expect("a screen");
        assert_eq!((v.rev, v.cols, v.rows, v.runs.len()), (5, 10, 3, 2));
        assert!(grid.view(a).is_none());

        grid.key(c, "KeyX", 0, Some("x"));
        assert!(matches!(
            fake.next(),
            ClientMsg::Input {
                sid: 8,
                input_seq: 1,
                ..
            }
        ));
        grid.viewport(c, 80, 24);
        assert!(matches!(
            fake.next(),
            ClientMsg::Viewport { sid: 8, size } if (size.cols, size.rows) == (80, 24)
        ));
        grid.take_size(c);
        assert_eq!(fake.next(), ClientMsg::TakeSize { sid: 8 });
        grid.detach(a);
        assert_eq!(fake.next(), ClientMsg::Detach { sid: 7 });

        // A delta the mirror cannot apply: detach and attach afresh.
        fake.reply(&ServerMsg::Delta {
            sid: 8,
            delta: Box::new(vorn_term_proto::screen::Delta {
                state_gen: 2,
                base_rev: 5,
                rev: 6,
                ..Default::default()
            }),
        });
        assert_eq!(fake.next(), ClientMsg::Detach { sid: 8 });
        assert!(
            matches!(fake.next(), ClientMsg::Attach(x) if x.session == "s1" && x.view.cols == 80)
        );
        wait_for("pane 3 attaching", || {
            grid.pane_state(c) == Some(PaneState::Attaching)
        });

        drop(fake);
        wait_for("closed", || grid.closed());
        assert_eq!(grid.pane_state(c), Some(PaneState::Closed));
        drop(grid);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn drop_unblocks_a_waiting_reader() {
        let dir = std::env::temp_dir().join(format!("vorn-grid-ffi-drop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("grid.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).expect("bind");
        let grid = Grid::connect(path.to_str().expect("utf-8"), "test", VgTheme::default())
            .expect("connect");
        let (_conn, _) = listener.accept().expect("accept");
        drop(grid);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
