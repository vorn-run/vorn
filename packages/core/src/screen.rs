//! The napi face of `vorn-screen`: what `terminal-screen.ts` calls.

use napi::bindgen_prelude::Buffer;
use napi_derive::napi;

#[napi(object)]
pub struct ScreenSnapshot {
    pub screen: String,
    pub cols: u32,
    pub rows: u32,
    pub title: String,
    pub cwd: String,
}

#[napi]
pub struct Screen {
    /// `None` once freed: the Ghostty terminal's memory is invisible to V8, so
    /// it is released when the session ends rather than whenever V8 collects.
    inner: Option<vorn_screen::Screen>,
}

fn to_napi(err: vorn_screen::Error) -> napi::Error {
    napi::Error::from_reason(err.to_string())
}

#[napi]
impl Screen {
    #[napi(constructor, catch_unwind)]
    pub fn new(cols: u32, rows: u32) -> napi::Result<Self> {
        Ok(Self {
            inner: Some(vorn_screen::Screen::new(cols, rows).map_err(to_napi)?),
        })
    }

    /// One flush of output, as the string node-pty produced. Returns the cwd an
    /// OSC 5522 in it moved to, for the server to record.
    #[napi(catch_unwind)]
    pub fn feed(&mut self, data: String) -> Option<String> {
        self.inner.as_mut()?.feed(data.as_bytes())
    }

    /// The same, from bytes, for a caller that never decoded them.
    #[napi(catch_unwind)]
    pub fn feed_bytes(&mut self, data: Buffer) -> Option<String> {
        self.inner.as_mut()?.feed(&data)
    }

    #[napi(catch_unwind)]
    pub fn restore_labels(&mut self, title: Option<String>, cwd: Option<String>) {
        if let Some(s) = self.inner.as_mut() {
            s.restore_labels(title.as_deref(), cwd.as_deref());
        }
    }

    #[napi(catch_unwind)]
    pub fn resize(&mut self, cols: u32, rows: u32) -> napi::Result<()> {
        match self.inner.as_mut() {
            Some(s) => s.resize(cols, rows).map_err(to_napi),
            None => Ok(()),
        }
    }

    #[napi(catch_unwind)]
    pub fn serialize(&self) -> napi::Result<ScreenSnapshot> {
        let s = self
            .inner
            .as_ref()
            .ok_or_else(|| napi::Error::from_reason("screen was freed"))?;
        let snap = s.serialize().map_err(to_napi)?;
        Ok(ScreenSnapshot {
            screen: snap.screen,
            cols: snap.cols,
            rows: snap.rows,
            title: snap.title,
            cwd: snap.cwd,
        })
    }

    #[napi(getter, catch_unwind)]
    pub fn title(&self) -> String {
        self.inner
            .as_ref()
            .map_or_else(String::new, |s| s.title().to_owned())
    }

    #[napi(getter, catch_unwind)]
    pub fn cwd(&self) -> String {
        self.inner
            .as_ref()
            .map_or_else(String::new, |s| s.cwd().to_owned())
    }

    /// Release the terminal now. Every later call is a no-op or an error.
    #[napi(catch_unwind)]
    pub fn free(&mut self) {
        self.inner = None;
    }
}
