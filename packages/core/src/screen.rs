//! The screen model `terminal-screen.ts` keeps in a headless xterm, kept in a
//! libghostty-vt terminal instead.
//!
//! Spike code for the Ghostty-vs-JS comparison. Differences from the xterm
//! model that matter for the comparison: the parse is synchronous, so there is
//! no queue to bound and no drain to wait for, and the serialized screen comes
//! from Ghostty's own VT formatter rather than `@xterm/addon-serialize`.

use libghostty_vt::fmt::{Format, Formatter, FormatterOptions};
use libghostty_vt::terminal::{Options, Terminal};
use memchr::memmem;
use napi::bindgen_prelude::Buffer;
use napi_derive::napi;

/// Vorn's own shell integration reports the cwd on this private OSC.
const OSC_CWD: &[u8] = b"\x1b]5522;cwd;";
const MAX_LABEL_BYTES: usize = 512;

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
    term: Terminal<'static, 'static>,
    cols: u32,
    rows: u32,
    cwd: String,
}

#[napi]
impl Screen {
    #[napi(constructor)]
    pub fn new(cols: u32, rows: u32) -> napi::Result<Self> {
        let term = Terminal::new(Options {
            cols: cols as u16,
            rows: rows as u16,
            // Same as the xterm model: the screen, not history.
            max_scrollback: 0,
        })
        .map_err(crate::to_napi)?;
        Ok(Self {
            term,
            cols,
            rows,
            cwd: String::new(),
        })
    }

    /// One flush of output, as the string node-pty produced.
    #[napi]
    pub fn feed(&mut self, data: String) {
        self.write(data.as_bytes());
    }

    /// The same, from bytes, for a caller that never decoded them.
    #[napi]
    pub fn feed_bytes(&mut self, data: Buffer) {
        self.write(&data);
    }

    #[napi]
    pub fn resize(&mut self, cols: u32, rows: u32) -> napi::Result<()> {
        self.term
            .resize(cols as u16, rows as u16, 1, 1)
            .map_err(crate::to_napi)?;
        self.cols = cols;
        self.rows = rows;
        Ok(())
    }

    #[napi]
    pub fn serialize(&self) -> napi::Result<ScreenSnapshot> {
        let opts = FormatterOptions::new()
            .with_format(Format::Vt)
            .with_modes(true)
            .with_scrolling_region(true)
            .with_cursor(true)
            .with_style(true)
            .with_hyperlink(true)
            .with_charsets(true);
        let mut f = Formatter::new(&self.term, opts).map_err(crate::to_napi)?;
        let bytes = f.format_alloc(None).map_err(crate::to_napi)?;
        Ok(ScreenSnapshot {
            screen: String::from_utf8_lossy(&bytes).into_owned(),
            cols: self.cols,
            rows: self.rows,
            title: self.title(),
            cwd: self.cwd(),
        })
    }

    #[napi(getter)]
    pub fn title(&self) -> String {
        self.term.title().map(clip).unwrap_or_default()
    }

    /// The last cwd from OSC 5522, else OSC 7.
    #[napi(getter)]
    pub fn cwd(&self) -> String {
        if !self.cwd.is_empty() {
            return self.cwd.clone();
        }
        self.term
            .pwd()
            .map(|p| {
                clip(
                    p.trim_start_matches("file://")
                        .trim_start_matches(|c| c != '/'),
                )
            })
            .unwrap_or_default()
    }
}

impl Screen {
    fn write(&mut self, bytes: &[u8]) {
        self.term.vt_write(bytes);
        if let Some(at) = memmem::rfind(bytes, OSC_CWD) {
            let rest = &bytes[at + OSC_CWD.len()..];
            let end = rest
                .iter()
                .position(|&b| b == 0x07 || b == 0x1b)
                .unwrap_or(rest.len());
            if end < rest.len() {
                let path = String::from_utf8_lossy(&rest[..end]);
                if path.starts_with('/') && !path.chars().any(|c| c.is_control()) {
                    self.cwd = clip(&path);
                }
            }
        }
    }
}

fn clip(s: &str) -> String {
    if s.len() <= MAX_LABEL_BYTES {
        return s.to_owned();
    }
    let mut end = MAX_LABEL_BYTES;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_owned()
}

/// Throughput of the libghostty-vt parse alone, without napi, fed per flush as
/// the server feeds it: `VORN_TRANSCRIPTS=<dir> cargo test --release
/// screen_throughput -- --ignored --nocapture`
#[cfg(test)]
mod throughput {
    use super::*;
    use std::time::Instant;

    #[test]
    #[ignore]
    fn screen_throughput() {
        let dir = std::env::var("VORN_TRANSCRIPTS").expect("VORN_TRANSCRIPTS");
        for name in ["agent", "spinner", "bulk"] {
            let raw = std::fs::read_to_string(format!("{dir}/{name}.json")).unwrap();
            let chunks: Vec<String> = serde_json::from_str(&raw).unwrap();
            let flushes: Vec<String> = chunks.chunks(100).map(|c| c.concat()).collect();
            let bytes: usize = chunks.iter().map(String::len).sum();
            let mut best = f64::MAX;
            for _ in 0..20 {
                // The terminal directly: `Screen::new` returns a napi error type,
                // which a test binary has no Node to link against.
                let mut term = Terminal::new(Options {
                    cols: 200,
                    rows: 50,
                    max_scrollback: 0,
                })
                .unwrap();
                let t = Instant::now();
                for f in &flushes {
                    term.vt_write(f.as_bytes());
                }
                best = best.min(t.elapsed().as_secs_f64());
            }
            println!(
                "{name}: {:.2} ms/MB ({:.0} MB/s)",
                best * 1000.0 / (bytes as f64 / 1048576.0),
                bytes as f64 / 1048576.0 / best
            );
        }
    }
}
