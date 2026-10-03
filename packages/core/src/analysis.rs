//! The napi face of `vorn-analysis`: what `appendOutput` calls.

use napi_derive::napi;

#[napi]
pub struct Analyzer {
    inner: vorn_analysis::Analyzer,
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
        }
    }

    /// Feed one raw chunk. `analyze` is false for hook-driven sessions, which
    /// only take status from bracketed paste. Returns one of the status codes:
    /// none, running, waiting, error.
    #[napi(catch_unwind)]
    pub fn append(&mut self, data: String, analyze: bool) -> u32 {
        self.inner.append_str(&data, analyze)
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
        self.inner.free()
    }
}

/// Every chunk through one analyzer in a single call: the same work as
/// [`Analyzer::append`] per chunk without a napi crossing per chunk, so the
/// bench can show what the boundary costs. Returns the last status code.
#[napi(catch_unwind)]
pub fn analyze_batch(chunks: Vec<String>, analyze: bool) -> u32 {
    vorn_analysis::analyze_batch(&chunks, analyze)
}
