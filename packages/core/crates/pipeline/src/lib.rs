//! A terminal's output, handled on a thread of its own.
//!
//! Every flush the server sends to its clients is also parsed into a screen
//! model, kept in a scrollback buffer and framed into the history log. Those
//! three used to run on the server's event loop, one after another, for every
//! flush of every terminal -- and at 64 KB a flush the parse alone holds the
//! loop for half a millisecond. Here they run on a thread the terminal owns,
//! fed in order through a channel, so the loop pays for one hand-off.
//!
//! Ordering is the whole contract. Everything is a message on one channel, so
//! a read -- a snapshot, the scrollback, a checkpoint's cut -- is answered after
//! every feed sent before it and before any sent after. That is what lets the
//! server keep its rule that a flush's sequence number and the state it reads
//! never disagree.
//!
//! Plain Rust with no Node in it: the napi adapter in `vorn-core` turns
//! `Event`s into calls on the event loop.

use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

pub mod checkpoint;
pub mod frame;
pub mod history;
pub mod ring;

pub use checkpoint::{Cursor, Meta};
pub use frame::Record;
pub use vorn_screen::Snapshot;

use ring::Scrollback;
use vorn_screen::Screen;

/// How many messages may wait before a sender blocks. A feed is at most one
/// 64 KB flush, so this bounds what a terminal can have queued at 16 MB. The
/// parse runs several times faster than any PTY produces, so reaching it
/// means the machine is starved, and waiting is then the honest answer.
const CAPACITY: usize = 256;

/// What the thread reports back, outside any request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A BEL the terminal acted on: not the one that ends an OSC title.
    Bell,
    /// The cwd an OSC 5522 moved to.
    Cwd(String),
    /// The screen model failed and was dropped. Scrollback and history carry on.
    ScreenFailed(String),
}

/// A checkpoint, cut at one point in the stream.
#[derive(Debug)]
pub struct Cut {
    /// The checkpoint file's body, or `None` when there is no screen model to
    /// take one from, because it failed.
    pub body: Option<Vec<u8>>,
    /// Every frame built before the cut and not yet taken. Frames for output
    /// fed after the cut stay behind, for the log that follows the checkpoint.
    pub frames: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The thread is gone: it panicked, or the pipeline was freed.
    Stopped,
    Screen(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Stopped => write!(f, "the terminal's core thread has stopped"),
            Error::Screen(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

enum Command {
    Feed {
        data: String,
        record: Option<Record>,
        /// Kept as scrollback too; false only for a replay into the screen.
        keep: bool,
    },
    Resize {
        cols: u32,
        rows: u32,
        record: Option<Record>,
    },
    RestoreLabels {
        title: Option<String>,
        cwd: Option<String>,
    },
    SeedScrollback(String),
    AppendScrollback(String),
    Serialize(mpsc::Sender<Result<Snapshot>>),
    Scrollback(mpsc::Sender<String>),
    Cut(Meta, Box<dyn FnOnce(Cut) + Send>),
    Stop,
}

pub struct Pipeline {
    tx: SyncSender<Command>,
    /// Frames built and not yet taken. Shared rather than requested, so the
    /// writer's tick takes what is ready without waiting for the thread.
    frames: Arc<Mutex<Vec<u8>>>,
    thread: Option<JoinHandle<()>>,
}

impl Pipeline {
    /// Start a terminal's thread, with a screen model at this size.
    ///
    /// `on_event` runs on that thread.
    pub fn spawn(cols: u32, rows: u32, on_event: impl Fn(Event) + Send + 'static) -> Result<Self> {
        let (tx, rx) = mpsc::sync_channel(CAPACITY);
        let frames = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::clone(&frames);
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("vorn-terminal".into())
            .spawn(move || {
                // The Ghostty terminal is made here and never leaves: it is not
                // `Send`, and it does not need to be.
                match Screen::new(cols, rows) {
                    Ok(screen) => {
                        let _ = ready_tx.send(Ok(()));
                        Worker {
                            screen: Some(screen),
                            scrollback: Scrollback::default(),
                            frames: shared,
                            on_event: Box::new(on_event),
                        }
                        .run(rx);
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(Error::Screen(e.to_string())));
                    }
                }
            })
            .map_err(|e| Error::Screen(format!("could not start a thread: {e}")))?;
        ready_rx.recv().map_err(|_| Error::Stopped)??;
        Ok(Self {
            tx,
            frames,
            thread: Some(thread),
        })
    }

    /// One flush of output, framed for the history when `record` is given.
    pub fn feed(&self, data: String, record: Option<Record>) -> Result<()> {
        self.send(Command::Feed {
            data,
            record,
            keep: true,
        })
    }

    /// The screen alone, as recovery replays a checkpoint and a log into it:
    /// not kept as scrollback, not framed.
    pub fn feed_screen(&self, data: String) -> Result<()> {
        self.send(Command::Feed {
            data,
            record: None,
            keep: false,
        })
    }

    pub fn resize(&self, cols: u32, rows: u32, record: Option<Record>) -> Result<()> {
        self.send(Command::Resize { cols, rows, record })
    }

    pub fn restore_labels(&self, title: Option<String>, cwd: Option<String>) -> Result<()> {
        self.send(Command::RestoreLabels { title, cwd })
    }

    pub fn seed_scrollback(&self, data: String) -> Result<()> {
        self.send(Command::SeedScrollback(data))
    }

    /// Scrollback alone: bytes the screen should not parse and the history
    /// should not record.
    pub fn append_scrollback(&self, data: String) -> Result<()> {
        self.send(Command::AppendScrollback(data))
    }

    /// The screen once everything fed so far has been parsed.
    pub fn serialize(&self) -> Result<Snapshot> {
        self.ask(Command::Serialize)?
    }

    pub fn scrollback(&self) -> Result<String> {
        self.ask(Command::Scrollback)
    }

    /// A checkpoint of the screen and scrollback, and the frames before it,
    /// all at one point in the stream.
    pub fn cut(&self, meta: Meta) -> Result<Cut> {
        let (tx, rx) = mpsc::channel();
        self.cut_then(meta, move |cut| {
            let _ = tx.send(cut);
        })?;
        rx.recv().map_err(|_| Error::Stopped)
    }

    /// The same, answered on the terminal's thread rather than waited for.
    /// Its place in the stream is fixed now; `then` runs once the thread
    /// reaches it. If the thread stops first, `then` is dropped uncalled.
    pub fn cut_then(&self, meta: Meta, then: impl FnOnce(Cut) + Send + 'static) -> Result<()> {
        self.send(Command::Cut(meta, Box::new(then)))
    }

    /// The frames built so far, without waiting for any still queued.
    pub fn take_frames(&self) -> Vec<u8> {
        std::mem::take(&mut *lock(&self.frames))
    }

    fn send(&self, command: Command) -> Result<()> {
        self.tx.send(command).map_err(|_| Error::Stopped)
    }

    fn ask<T>(&self, command: impl FnOnce(mpsc::Sender<T>) -> Command) -> Result<T> {
        let (tx, rx) = mpsc::channel();
        self.send(command(tx))?;
        rx.recv().map_err(|_| Error::Stopped)
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Poisoned only by a panic on the other side mid-append, which leaves the
/// bytes valid up to the frame it was building; the reader stops at that one.
fn lock(frames: &Mutex<Vec<u8>>) -> std::sync::MutexGuard<'_, Vec<u8>> {
    frames.lock().unwrap_or_else(|e| e.into_inner())
}

struct Worker {
    screen: Option<Screen>,
    scrollback: Scrollback,
    frames: Arc<Mutex<Vec<u8>>>,
    on_event: Box<dyn Fn(Event) + Send>,
}

impl Worker {
    fn run(mut self, rx: Receiver<Command>) {
        while let Ok(command) = rx.recv() {
            match command {
                Command::Feed { data, record, keep } => self.feed(&data, record, keep),
                Command::Resize { cols, rows, record } => self.resize(cols, rows, record),
                Command::RestoreLabels { title, cwd } => {
                    if let Some(screen) = self.screen.as_mut() {
                        screen.restore_labels(title.as_deref(), cwd.as_deref());
                    }
                }
                Command::SeedScrollback(data) => self.scrollback.seed(data.as_bytes()),
                Command::AppendScrollback(data) => self.scrollback.append(data.as_bytes()),
                Command::Serialize(reply) => {
                    let _ = reply.send(self.serialize());
                }
                Command::Scrollback(reply) => {
                    let _ = reply.send(self.scrollback.read());
                }
                Command::Cut(meta, reply) => {
                    let body = self.serialize().ok().map(|snapshot| {
                        checkpoint::encode(&snapshot, &self.scrollback.read(), meta)
                    });
                    reply(Cut {
                        body,
                        frames: std::mem::take(&mut *lock(&self.frames)),
                    });
                }
                Command::Stop => break,
            }
        }
    }

    fn feed(&mut self, data: &str, record: Option<Record>, keep: bool) {
        let bytes = data.as_bytes();
        // Framed first: the checksum is the one step here whose output is
        // waited on by something other than this thread.
        if let Some(at) = record {
            frame::data(&mut lock(&self.frames), at, bytes);
        }
        if keep {
            self.scrollback.append(bytes);
        }
        if let Some(screen) = self.screen.as_mut() {
            let fed = screen.feed(bytes);
            // A replay rang its bells when the output was live: ringing them
            // again would notify for every BEL in a restored session.
            if keep && fed.bells > 0 {
                (self.on_event)(Event::Bell);
            }
            if let Some(cwd) = fed.cwd {
                (self.on_event)(Event::Cwd(cwd));
            }
        }
    }

    fn resize(&mut self, cols: u32, rows: u32, record: Option<Record>) {
        if let Some(at) = record {
            frame::resize(&mut lock(&self.frames), at, cols as u16, rows as u16);
        }
        let Some(screen) = self.screen.as_mut() else {
            return;
        };
        if let Err(e) = screen.resize(cols, rows) {
            self.screen = None;
            (self.on_event)(Event::ScreenFailed(e.to_string()));
        }
    }

    fn serialize(&self) -> Result<Snapshot> {
        match self.screen.as_ref() {
            Some(screen) => screen.serialize().map_err(|e| Error::Screen(e.to_string())),
            None => Err(Error::Screen("the screen model failed earlier".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::Receiver;

    const META: Meta = Meta {
        generation: 2,
        resume: Cursor {
            epoch: 1,
            next_rseq: 1,
            next_offset: 6,
        },
        closed_cleanly: false,
    };

    fn pipeline() -> (Pipeline, Receiver<Event>) {
        let (tx, rx) = mpsc::channel();
        let p = Pipeline::spawn(40, 8, move |e| {
            let _ = tx.send(e);
        })
        .unwrap();
        (p, rx)
    }

    #[test]
    fn a_read_sees_every_feed_sent_before_it() {
        let (p, _) = pipeline();
        for i in 0..200 {
            p.feed(format!("line {i}\r\n"), None).unwrap();
        }
        let snap = p.serialize().unwrap();
        assert!(snap.screen.contains("line 199"));
        assert!(p.scrollback().unwrap().ends_with("line 199\r\n"));
    }

    #[test]
    fn frames_only_what_is_recorded_and_cuts_between_feeds() {
        let (p, _) = pipeline();
        p.feed(
            "before".into(),
            Some(Record {
                rseq: 0,
                start_offset: 0,
            }),
        )
        .unwrap();
        p.feed("unrecorded".into(), None).unwrap();
        let cut = p.cut(META).unwrap();
        p.feed(
            "after".into(),
            Some(Record {
                rseq: 1,
                start_offset: 6,
            }),
        )
        .unwrap();
        // A round trip, so the feed above has run.
        p.scrollback().unwrap();

        let body: serde_json::Value = serde_json::from_slice(&cut.body.unwrap()).unwrap();
        assert!(body["screen"]
            .as_str()
            .unwrap()
            .contains("beforeunrecorded"));
        assert_eq!(body["scrollback"], "beforeunrecorded");
        assert_eq!(body["generation"], 2);
        let mut expected = Vec::new();
        frame::data(
            &mut expected,
            Record {
                rseq: 0,
                start_offset: 0,
            },
            b"before",
        );
        assert_eq!(cut.frames, expected);

        let mut after = Vec::new();
        frame::data(
            &mut after,
            Record {
                rseq: 1,
                start_offset: 6,
            },
            b"after",
        );
        assert_eq!(p.take_frames(), after);
        assert!(p.take_frames().is_empty());
    }

    #[test]
    fn reports_a_bell_but_not_the_bel_ending_a_title() {
        let (p, events) = pipeline();
        p.feed("\x1b]0;title\x07".into(), None).unwrap();
        p.feed("ding\x07".into(), None).unwrap();
        p.serialize().unwrap();
        assert_eq!(events.try_iter().collect::<Vec<_>>(), vec![Event::Bell]);
    }

    #[test]
    fn a_replay_rings_no_bell_and_still_moves_the_cwd() {
        let (p, events) = pipeline();
        p.feed_screen("ding\x07\x1b]5522;cwd;/srv\x07".into())
            .unwrap();
        p.serialize().unwrap();
        let got: Vec<_> = events.try_iter().collect();
        assert!(!got.contains(&Event::Bell), "a replay rang: {got:?}");
        assert!(
            got.iter().any(|e| matches!(e, Event::Cwd(_))),
            "no cwd event in {got:?}"
        );
    }

    #[test]
    fn reports_the_cwd_vorn_shell_integration_sends() {
        let (p, events) = pipeline();
        p.feed("\x1b]5522;cwd;/srv\x07".into(), None).unwrap();
        p.serialize().unwrap();
        let got: Vec<_> = events.try_iter().collect();
        assert!(
            got.iter().any(|e| matches!(e, Event::Cwd(_))),
            "no cwd event in {got:?}"
        );
    }

    #[test]
    fn a_failed_resize_drops_the_screen_and_keeps_the_rest() {
        let (p, events) = pipeline();
        p.feed("kept".into(), None).unwrap();
        p.resize(
            0,
            8,
            Some(Record {
                rseq: 1,
                start_offset: 4,
            }),
        )
        .unwrap();
        assert!(p.serialize().is_err());
        let cut = p.cut(META).unwrap();
        assert!(cut.body.is_none());
        // The resize is still recorded: history carries on without a screen.
        assert!(!cut.frames.is_empty());
        assert_eq!(p.scrollback().unwrap(), "kept");
        assert!(matches!(
            events.try_iter().next(),
            Some(Event::ScreenFailed(_))
        ));
    }

    #[test]
    fn seeded_scrollback_comes_back() {
        let (p, _) = pipeline();
        p.seed_scrollback("restored\r\n".into()).unwrap();
        p.feed("live".into(), None).unwrap();
        p.append_scrollback(" and more".into()).unwrap();
        p.feed_screen(" screen only".into()).unwrap();
        assert_eq!(p.scrollback().unwrap(), "restored\r\nlive and more");
        assert!(p.serialize().unwrap().screen.contains("live"));
        assert!(!p.serialize().unwrap().screen.contains("and more"));
        assert!(p.serialize().unwrap().screen.contains("screen only"));
    }

    #[test]
    fn refuses_a_screen_it_cannot_make() {
        assert!(Pipeline::spawn(0, 0, |_| {}).is_err());
    }
}
