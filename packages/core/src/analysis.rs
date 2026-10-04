//! The napi face of `vorn-analysis`: what `appendOutput` calls.

use napi::bindgen_prelude::Utf16String;
use napi_derive::napi;
use vorn_analysis::utf16::push_utf16;

#[napi]
pub struct Analyzer {
    inner: vorn_analysis::Analyzer,
    /// The batch as UTF-8, reused so a call allocates nothing once warm.
    scratch: String,
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[napi]
impl Analyzer {
    #[napi(constructor, catch_unwind)]
    pub fn new() -> Self {
        Self {
            inner: vorn_analysis::Analyzer::new(),
            scratch: String::new(),
        }
    }

    /// Feed a batch of raw output. `analyze` is false for hook-driven
    /// sessions, which only take status from bracketed paste. Returns one of
    /// the status codes: none, running, waiting, error.
    ///
    /// Taken as UTF-16, which V8 hands over as a copy, and converted here,
    /// which is faster than V8's own UTF-8 conversion on terminal output.
    #[napi(catch_unwind)]
    pub fn append(&mut self, data: Utf16String, analyze: bool) -> u32 {
        self.scratch.clear();
        push_utf16(&mut self.scratch, &data);
        let status = self.inner.append_str(&self.scratch, analyze);
        // A huge batch should not pin its buffer for the life of the session.
        if self.scratch.capacity() > 1 << 20 {
            self.scratch = String::new();
        }
        status
    }

    /// The last `lines` completed lines, oldest first; all of them when omitted or zero.
    #[napi(catch_unwind)]
    pub fn output(&self, lines: Option<u32>) -> Vec<String> {
        self.inner.output(lines)
    }

    /// The line in progress, stripped.
    #[napi(catch_unwind)]
    pub fn partial(&self) -> String {
        self.inner.partial()
    }

    /// Release the line ring now; the analyzer starts empty if fed again.
    #[napi(catch_unwind)]
    pub fn free(&mut self) {
        self.inner.free();
        self.scratch = String::new();
    }
}

/// Every chunk through one analyzer in a single call: the same work as
/// [`Analyzer::append`] per chunk without a napi crossing per chunk, so the
/// bench can show what the boundary costs. Returns the last status code.
#[napi(catch_unwind)]
pub fn analyze_batch(chunks: Vec<String>, analyze: bool) -> u32 {
    vorn_analysis::analyze_batch(&chunks, analyze)
}
