//! The OS web views behind the browser pane (WKWebView, WebView2,
//! WebKitGTK through wry), seated as children of the app's window over the
//! boxes the screen leaves for them. The views live on the UI thread; what
//! they report (loads, titles, the page script's messages, script answers)
//! comes back over a channel and wakes the next frame.
//!
//! Without a window (an offscreen shot) there are no views: the screen
//! still draws the pane's chrome and leaves the page box empty.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::ui::Rect;

pub mod address;

#[cfg(all(not(target_arch = "wasm32"), not(feature = "webview")))]
mod off;
#[cfg(all(not(target_arch = "wasm32"), not(feature = "webview")))]
pub use off::Host;
#[cfg(all(not(target_arch = "wasm32"), feature = "webview"))]
mod views;
#[cfg(all(not(target_arch = "wasm32"), feature = "webview"))]
pub use views::Host;
#[cfg(target_arch = "wasm32")]
mod tabs;
#[cfg(target_arch = "wasm32")]
pub use tabs::Host;

/// The script every page gets before its own (`window.__vorn`).
pub const PAGE_SCRIPT: &str = include_str!("web/page.js");

/// The page a device pane shows: the latest frame, refreshed. Taps need the
/// simulator companion, so the frame is view-only.
#[cfg(all(not(target_arch = "wasm32"), feature = "webview"))]
const DEVICE_PAGE: &str = include_str!("web/device.html");

/// The latest screen of each session's device, as PNG bytes.
pub type Frames = Arc<Mutex<HashMap<String, Vec<u8>>>>;

/// The address of `session`'s device page, served by the views themselves.
pub fn device_url(session: &str) -> String {
    // WebView2 and Android serve custom schemes over http on `<scheme>.localhost`.
    if cfg!(windows) {
        format!("http://vorn.localhost/device/{session}")
    } else {
        format!("vorn://localhost/device/{session}")
    }
}

/// What a web view reported, by its key.
#[derive(Debug, Clone, PartialEq)]
pub enum WebEvent {
    /// A load started (`finished` false) or finished at `url`.
    Page {
        key: String,
        url: String,
        finished: bool,
    },
    Title {
        key: String,
        title: String,
    },
    /// A message the page script posted.
    Message {
        key: String,
        body: String,
    },
    /// The JSON answer to [`Host::eval`]'s script.
    Answer {
        token: u64,
        json: String,
    },
    /// The page asked for a new window; the pane opens it as a tab.
    NewWindow {
        key: String,
        url: String,
    },
    /// A load ended before the page committed (no such host, offline).
    Failed {
        key: String,
        url: String,
    },
}

/// Where one web view should be this frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Slot {
    pub key: String,
    pub url: String,
    /// Physical pixels in the window.
    pub rect: Rect,
    pub zoom: f64,
    /// A connection whose signed-in profile the view keeps its cookies in.
    pub profile: Option<String>,
}

/// What a bridge call says when the session has no pane to drive.
pub const NO_PANE: &str =
    "This session has no browser pane open. Open one from the session card, then retry.";
