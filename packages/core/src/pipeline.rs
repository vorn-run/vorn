//! The napi face of `vorn-pipeline`: one terminal's output on a thread of its own.

use napi::bindgen_prelude::{Buffer, Object};
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi::Status;
use napi::{Env, JsDeferred};
use napi_derive::napi;

use crate::screen::ScreenSnapshot;

/// Where a record sits in the session's stream. Numbers rather than bigints:
/// both stay far below 2^53 for any session that will ever run.
#[napi(object)]
pub struct RecordHeader {
    pub rseq: f64,
    pub start_offset: f64,
}

/// Something the terminal's thread noticed, delivered on the event loop.
#[napi(object)]
pub struct PipelineEvent {
    /// `bell`, `cwd` or `screen-failed`.
    pub kind: String,
    pub cwd: Option<String>,
    pub error: Option<String>,
}

/// The Session Recovery Contract's cursor.
#[napi(object)]
pub struct CheckpointCursor {
    pub epoch: u32,
    pub next_rseq: f64,
    pub next_offset: f64,
}

/// What the history writer knows about a checkpoint that the stream does not.
#[napi(object)]
pub struct CheckpointMeta {
    pub generation: u32,
    pub resume: CheckpointCursor,
    pub closed_cleanly: Option<bool>,
}

#[napi(object)]
pub struct PipelineCut {
    /// The checkpoint file's JSON body; absent when the screen model has failed.
    pub body: Option<Buffer>,
    /// History frames built before the cut, ready to append.
    pub frames: Buffer,
}

type Resolve = Box<dyn FnOnce(Env) -> napi::Result<PipelineCut>>;

/// A cut's promise, rejected if the thread stops before answering it rather
/// than left pending for ever.
struct Settle(Option<JsDeferred<PipelineCut, Resolve>>);

impl Drop for Settle {
    fn drop(&mut self) {
        if let Some(deferred) = self.0.take() {
            deferred.reject(napi::Error::from_reason(
                "the terminal's core thread stopped before the checkpoint",
            ));
        }
    }
}

/// Weak, so a terminal's thread is never what keeps the server alive.
type OnEvent = ThreadsafeFunction<PipelineEvent, (), PipelineEvent, Status, false, true>;

#[napi]
pub struct TerminalPipeline {
    /// `None` once freed, which stops and joins the thread.
    inner: Option<vorn_pipeline::Pipeline>,
}

fn to_napi(err: vorn_pipeline::Error) -> napi::Error {
    napi::Error::from_reason(err.to_string())
}

fn record(at: Option<RecordHeader>) -> Option<vorn_pipeline::Record> {
    at.map(|at| vorn_pipeline::Record {
        rseq: at.rseq as u64,
        start_offset: at.start_offset as u64,
    })
}

fn snapshot(s: vorn_pipeline::Snapshot) -> ScreenSnapshot {
    ScreenSnapshot {
        screen: s.screen,
        cols: s.cols,
        rows: s.rows,
        title: s.title,
        cwd: s.cwd,
    }
}

#[napi]
impl TerminalPipeline {
    #[napi(constructor, catch_unwind)]
    pub fn new(cols: u32, rows: u32, on_event: OnEvent) -> napi::Result<Self> {
        let inner = vorn_pipeline::Pipeline::spawn(cols, rows, move |event| {
            let event = match event {
                vorn_pipeline::Event::Bell => PipelineEvent {
                    kind: "bell".into(),
                    cwd: None,
                    error: None,
                },
                vorn_pipeline::Event::Cwd(cwd) => PipelineEvent {
                    kind: "cwd".into(),
                    cwd: Some(cwd),
                    error: None,
                },
                vorn_pipeline::Event::ScreenFailed(error) => PipelineEvent {
                    kind: "screen-failed".into(),
                    cwd: None,
                    error: Some(error),
                },
            };
            on_event.call(event, ThreadsafeFunctionCallMode::NonBlocking);
        })
        .map_err(to_napi)?;
        Ok(Self { inner: Some(inner) })
    }

    fn pipeline(&self) -> napi::Result<&vorn_pipeline::Pipeline> {
        self.inner
            .as_ref()
            .ok_or_else(|| napi::Error::from_reason("terminal pipeline was freed"))
    }

    /// One flush of output, framed for the history when `record` is given.
    /// Returns before it is parsed.
    #[napi(catch_unwind)]
    pub fn feed(&self, data: String, record_at: Option<RecordHeader>) -> napi::Result<()> {
        self.pipeline()?
            .feed(data, record(record_at))
            .map_err(to_napi)
    }

    /// The screen alone: not kept as scrollback, not framed. For a replay.
    #[napi(catch_unwind)]
    pub fn feed_screen(&self, data: String) -> napi::Result<()> {
        self.pipeline()?.feed_screen(data).map_err(to_napi)
    }

    #[napi(catch_unwind)]
    pub fn resize(
        &self,
        cols: u32,
        rows: u32,
        record_at: Option<RecordHeader>,
    ) -> napi::Result<()> {
        self.pipeline()?
            .resize(cols, rows, record(record_at))
            .map_err(to_napi)
    }

    #[napi(catch_unwind)]
    pub fn restore_labels(&self, title: Option<String>, cwd: Option<String>) -> napi::Result<()> {
        self.pipeline()?.restore_labels(title, cwd).map_err(to_napi)
    }

    #[napi(catch_unwind)]
    pub fn seed_scrollback(&self, data: String) -> napi::Result<()> {
        self.pipeline()?.seed_scrollback(data).map_err(to_napi)
    }

    #[napi(catch_unwind)]
    pub fn append_scrollback(&self, data: String) -> napi::Result<()> {
        self.pipeline()?.append_scrollback(data).map_err(to_napi)
    }

    /// The screen once everything fed so far is parsed. Waits for the thread.
    #[napi(catch_unwind)]
    pub fn serialize(&self) -> napi::Result<ScreenSnapshot> {
        self.pipeline()?.serialize().map(snapshot).map_err(to_napi)
    }

    /// The scrollback once everything fed so far is in it. Waits for the thread.
    #[napi(catch_unwind)]
    pub fn scrollback(&self) -> napi::Result<String> {
        self.pipeline()?.scrollback().map_err(to_napi)
    }

    /// A checkpoint's body and the frames before it, at one point in the
    /// stream: the point this is called at. Resolves once the thread gets
    /// there, so the event loop never waits on a parse.
    #[napi(catch_unwind, ts_return_type = "Promise<PipelineCut>")]
    pub fn cut<'env>(&self, env: &'env Env, meta: CheckpointMeta) -> napi::Result<Object<'env>> {
        let meta = vorn_pipeline::Meta {
            generation: meta.generation,
            resume: vorn_pipeline::Cursor {
                epoch: meta.resume.epoch,
                next_rseq: meta.resume.next_rseq as u64,
                next_offset: meta.resume.next_offset as u64,
            },
            closed_cleanly: meta.closed_cleanly.unwrap_or(false),
        };
        let pipeline = self.pipeline()?;
        let (deferred, promise) = env.create_deferred::<PipelineCut, Resolve>()?;
        let mut settle = Settle(Some(deferred));
        // A thread that has stopped drops this uncalled, and `Settle` rejects
        // the promise: one way to fail, which the caller already awaits.
        let _ = pipeline.cut_then(meta, move |cut| {
            if let Some(deferred) = settle.0.take() {
                deferred.resolve(Box::new(move |_| {
                    Ok(PipelineCut {
                        body: cut.body.map(Into::into),
                        frames: cut.frames.into(),
                    })
                }));
            }
        });
        Ok(promise)
    }

    /// Frames built so far; does not wait for output still queued.
    #[napi(catch_unwind)]
    pub fn take_frames(&self) -> napi::Result<Buffer> {
        Ok(self.pipeline()?.take_frames().into())
    }

    /// Stop the thread and release the terminal now.
    #[napi(catch_unwind)]
    pub fn free(&mut self) {
        self.inner = None;
    }
}
