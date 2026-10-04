//! Real process kills: a [`Target`] that runs the engine in a child process
//! and kills it with the OS (SIGKILL on Unix, TerminateProcess on Windows).
//!
//! The child is `recovery-subject`, this crate's binary, which runs
//! [`ReferenceEngine`] behind [`serve`]. The two speak a small framed
//! protocol over the child's stdin and stdout: each frame is a little-endian
//! `u32` length and that many bytes of postcard.
//!
//! - driver → child: [`ToSubject::Start`] (size, config, and the checkpoint to
//!   restore, if any), then [`ToSubject::Entry`] per record, then
//!   [`ToSubject::Finish`]; [`ToSubject::Sync`] asks for
//!   [`FromSubject::Synced`] once every record before it is applied.
//! - child → driver: [`FromSubject::Started`] once the engine is up, a
//!   [`FromSubject::Checkpoint`] whenever it cuts one, and
//!   [`FromSubject::State`] after Finish. [`FromSubject::Failed`] instead of
//!   Started when the checkpoint would not restore.
//!
//! By default the driver does not wait for records to be applied: the pipe
//! buffers them, so a kill lands wherever the child had got to, which may be
//! no record at all when the OS is slow to start it. [`ChildProcess::exact`]
//! syncs before each kill instead, so the kill lands right after the record
//! the plan names. Checkpoints the child
//! wrote before it died are kept (a reader thread drains the pipe to its
//! end); a frame cut off by the kill is dropped, as sessiond drops a
//! checkpoint it did not receive whole.

use std::borrow::Cow;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};

use serde::{Deserialize, Serialize};
use vorn_term_proto::Entry;

use crate::compare::TermState;
use crate::driver::Target;
use crate::engine::{Checkpoint, Engine, ReferenceConfig, ReferenceEngine, Restore, Resume, Store};
use crate::log::Size;
use crate::Error;

/// The longest frame either side accepts: a checkpoint of a large screen
/// with its scrollback, with room to spare.
const MAX_FRAME: usize = 64 << 20;

/// Borrows the entry it sends, so a record is not copied to be framed.
#[derive(Debug, Serialize, Deserialize)]
pub enum ToSubject<'a> {
    Start {
        size: Size,
        config: ReferenceConfig,
        from: Option<Checkpoint>,
    },
    Entry(Cow<'a, Entry>),
    /// Answered with [`FromSubject::Synced`] after every earlier record.
    Sync,
    Finish,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum FromSubject {
    Started,
    Failed(String),
    Checkpoint(Checkpoint),
    Synced,
    State(Box<TermState>),
}

/// Writes one frame.
pub fn write_frame<T: Serialize>(w: &mut impl Write, msg: &T) -> Result<(), Error> {
    let body = postcard::to_stdvec(msg)?;
    let len = u32::try_from(body.len())
        .map_err(|_| Error::Subject(format!("frame of {} bytes", body.len())))?;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(&body)?;
    Ok(())
}

/// Reads one frame; `None` at a clean end of stream, an error for a frame
/// the stream ended inside of.
pub fn read_frame<T: for<'de> Deserialize<'de>>(
    r: &mut impl Read,
    buf: &mut Vec<u8>,
) -> Result<Option<T>, Error> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(Error::Subject(format!("frame of {len} bytes")));
    }
    buf.resize(len, 0);
    r.read_exact(buf)?;
    Ok(Some(postcard::from_bytes(buf)?))
}

/// The subject's side: runs a [`ReferenceEngine`] for the frames on `input`
/// and answers on `output`. `recovery-subject` is this over stdin and stdout.
pub fn serve(input: impl Read, output: impl Write) -> Result<(), Error> {
    let mut input = BufReader::new(input);
    let mut out = BufWriter::new(output);
    let mut buf = Vec::new();
    let Some(ToSubject::Start { size, config, from }) =
        read_frame::<ToSubject<'static>>(&mut input, &mut buf)?
    else {
        return Err(Error::Subject("expected Start".into()));
    };
    let engine = match &from {
        None => ReferenceEngine::start(&config, size),
        Some(cp) => ReferenceEngine::restore(&config, cp),
    };
    let mut engine = match engine {
        Ok(e) => e,
        Err(e) => {
            write_frame(&mut out, &FromSubject::Failed(e.to_string()))?;
            out.flush()?;
            return Ok(());
        }
    };
    write_frame(&mut out, &FromSubject::Started)?;
    out.flush()?;
    loop {
        match read_frame::<ToSubject<'static>>(&mut input, &mut buf)? {
            Some(ToSubject::Entry(entry)) => {
                if let Some(cp) = engine.apply(&entry)? {
                    write_frame(&mut out, &FromSubject::Checkpoint(cp))?;
                    out.flush()?;
                }
            }
            Some(ToSubject::Sync) => {
                write_frame(&mut out, &FromSubject::Synced)?;
                out.flush()?;
            }
            Some(ToSubject::Finish) => {
                write_frame(&mut out, &FromSubject::State(Box::new(engine.finish()?)))?;
                out.flush()?;
                return Ok(());
            }
            Some(ToSubject::Start { .. }) => return Err(Error::Subject("Start twice".into())),
            None => return Err(Error::Subject("input ended before Finish".into())),
        }
    }
}

/// One incarnation of the subject.
struct Running {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    frames: Receiver<FromSubject>,
    reader: JoinHandle<()>,
}

/// The engine in a child process; see the module docs.
pub struct ChildProcess {
    program: PathBuf,
    config: ReferenceConfig,
    restore: Restore,
    exact: bool,
    size: Option<Size>,
    running: Option<Running>,
    store: Store,
}

impl std::fmt::Debug for ChildProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChildProcess")
            .field("program", &self.program)
            .field("config", &self.config)
            .field("restore", &self.restore)
            .field("exact", &self.exact)
            .field("running", &self.running.as_ref().map(|r| r.child.id()))
            .finish_non_exhaustive()
    }
}

impl ChildProcess {
    /// `program` speaks the subject protocol; in this crate's tests it is
    /// `env!("CARGO_BIN_EXE_recovery-subject")`.
    pub fn new(program: impl Into<PathBuf>, config: ReferenceConfig, restore: Restore) -> Self {
        Self {
            program: program.into(),
            config,
            restore,
            exact: false,
            size: None,
            running: None,
            store: Store::new(),
        }
    }

    /// Kills only once the child has applied every record delivered so far,
    /// so each kill lands exactly after the record the plan names rather
    /// than wherever the child had got to.
    pub fn exact(mut self) -> Self {
        self.exact = true;
        self
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Spawns an incarnation and waits for it to start; `Ok(None)` when it
    /// could not restore `from`.
    fn spawn(&mut self, from: Option<&Checkpoint>) -> Result<Option<Running>, Error> {
        let size = self.size.ok_or(Error::Dead)?;
        let mut child = Command::new(&self.program)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Subject("no stdout".into()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Subject("no stdin".into()))?;
        let (tx, frames) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            let mut buf = Vec::new();
            // Ends at end of stream, or at a frame the kill cut off.
            while let Ok(Some(frame)) = read_frame::<FromSubject>(&mut r, &mut buf) {
                if tx.send(frame).is_err() {
                    break;
                }
            }
        });
        let mut running = Running {
            child,
            stdin: BufWriter::new(stdin),
            frames,
            reader,
        };
        write_frame(
            &mut running.stdin,
            &ToSubject::Start {
                size,
                config: self.config,
                from: from.cloned(),
            },
        )?;
        running.stdin.flush()?;
        match running.frames.recv() {
            Ok(FromSubject::Started) => Ok(Some(running)),
            Ok(FromSubject::Failed(_)) => {
                reap(running)?;
                Ok(None)
            }
            Ok(other) => Err(Error::Subject(format!("expected Started, got {other:?}"))),
            Err(_) => {
                let status = reap(running)?;
                Err(Error::Subject(format!("exited before starting: {status}")))
            }
        }
    }

    /// Stores every checkpoint that has arrived so far.
    fn drain(&mut self) {
        if let Some(r) = &self.running {
            while let Ok(frame) = r.frames.try_recv() {
                if let FromSubject::Checkpoint(cp) = frame {
                    self.store.put(cp);
                }
            }
        }
    }
}

/// Waits for an incarnation that is ending, and its reader.
fn reap(mut r: Running) -> Result<std::process::ExitStatus, Error> {
    drop(r.stdin);
    let status = r.child.wait()?;
    r.reader
        .join()
        .map_err(|_| Error::Subject("reader thread panicked".into()))?;
    Ok(status)
}

impl Target for ChildProcess {
    fn start(&mut self, size: Size) -> Result<(), Error> {
        self.size = Some(size);
        self.running = self.spawn(None)?;
        self.running
            .as_ref()
            .map(|_| ())
            .ok_or_else(|| Error::Subject("could not start a new session".into()))
    }

    fn deliver(&mut self, entry: &Entry) -> Result<(), Error> {
        self.drain();
        let r = self.running.as_mut().ok_or(Error::Dead)?;
        write_frame(&mut r.stdin, &ToSubject::Entry(Cow::Borrowed(entry)))?;
        // Flushed per record so the child sees records as they come, as a
        // vornd sees reads; the pipe, not this buffer, holds the backlog.
        r.stdin.flush()?;
        Ok(())
    }

    fn kill(&mut self) -> Result<(), Error> {
        let Some(mut r) = self.running.take() else {
            return Err(Error::Dead);
        };
        // The subject only exits when told to: one that is already gone
        // crashed, and a kill must not hide that.
        if let Some(status) = r.child.try_wait()? {
            return Err(Error::Subject(format!("exited on its own: {status}")));
        }
        if self.exact {
            write_frame(&mut r.stdin, &ToSubject::Sync)?;
            r.stdin.flush()?;
            loop {
                match r.frames.recv() {
                    Ok(FromSubject::Synced) => break,
                    Ok(FromSubject::Checkpoint(cp)) => self.store.put(cp),
                    Ok(other) => {
                        return Err(Error::Subject(format!("expected Synced, got {other:?}")))
                    }
                    Err(_) => return Err(Error::Subject("exited before Synced".into())),
                }
            }
        }
        // It can still exit between the check and the kill; the kill then
        // fails, and the wait below reaps it either way.
        let _ = r.child.kill();
        drop(r.stdin);
        r.child.wait()?;
        r.reader
            .join()
            .map_err(|_| Error::Subject("reader thread panicked".into()))?;
        // The reader saw every whole frame the child wrote before it died.
        while let Ok(frame) = r.frames.try_recv() {
            if let FromSubject::Checkpoint(cp) = frame {
                self.store.put(cp);
            }
        }
        Ok(())
    }

    fn recover(&mut self) -> Result<Resume, Error> {
        if self.restore == Restore::Checkpoint {
            let candidates: Vec<Checkpoint> = self.store.candidates().cloned().collect();
            for cp in &candidates {
                if let Some(r) = self.spawn(Some(cp))? {
                    self.running = Some(r);
                    return Ok(Resume::From(cp.resume));
                }
            }
        }
        self.running = self.spawn(None)?;
        if self.running.is_none() {
            return Err(Error::Subject("could not start a new session".into()));
        }
        Ok(Resume::SessionStart)
    }

    fn finish(mut self) -> Result<TermState, Error> {
        let mut r = self.running.take().ok_or(Error::Dead)?;
        write_frame(&mut r.stdin, &ToSubject::Finish)?;
        r.stdin.flush()?;
        let mut state = None;
        while let Ok(frame) = r.frames.recv() {
            match frame {
                FromSubject::Checkpoint(cp) => self.store.put(cp),
                FromSubject::State(s) => {
                    state = Some(*s);
                    break;
                }
                other => return Err(Error::Subject(format!("expected State, got {other:?}"))),
            }
        }
        let status = reap(r)?;
        state.ok_or_else(|| Error::Subject(format!("no state before exit: {status}")))
    }
}

impl Drop for ChildProcess {
    fn drop(&mut self) {
        // A run that failed part way leaves no process behind.
        if let Some(mut r) = self.running.take() {
            let _ = r.child.kill();
            let _ = r.child.wait();
        }
    }
}
