//! The OS web views behind the browser pane (WKWebView, WebView2,
//! WebKitGTK through wry), seated as children of the app's window over the
//! boxes the screen leaves for them. The views live on the UI thread; what
//! they report (loads, titles, the page script's messages, script answers)
//! comes back over a channel and wakes the next frame.
//!
//! Without a window (an offscreen shot) there are no views: the screen
//! still draws the pane's chrome and leaves the page box empty.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use wry::dpi::{PhysicalPosition, PhysicalSize};
use wry::http::{header, Request, Response};
use wry::{NewWindowResponse, PageLoadEvent, WebView, WebViewBuilder};

use crate::client::rpc::Wake;
use crate::ui::{NativeWindow, Rect};

/// The script every page gets before its own (`window.__vorn`).
pub const PAGE_SCRIPT: &str = include_str!("web/page.js");

/// The page a device pane shows: the latest frame, refreshed. Taps need the
/// simulator companion, so the frame is view-only.
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

/// Answers `vorn://localhost/device/<session>[/frame.png]`.
fn serve(frames: &Frames, req: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let path = req.uri().path();
    let reply = |status: u16, kind: &str, body: Cow<'static, [u8]>| {
        Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, kind)
            .header(header::CACHE_CONTROL, "no-store")
            .body(body)
            .unwrap_or_else(|_| Response::new(Cow::Borrowed(&[][..])))
    };
    let Some(rest) = path.strip_prefix("/device/") else {
        return reply(404, "text/plain", Cow::Borrowed(b"not found"));
    };
    match rest.strip_suffix("/frame.png") {
        Some(session) => {
            let frames = frames.lock().unwrap_or_else(PoisonError::into_inner);
            match frames.get(session) {
                Some(png) => reply(200, "image/png", Cow::Owned(png.clone())),
                None => reply(404, "text/plain", Cow::Borrowed(b"no frame")),
            }
        }
        None => reply(200, "text/html", Cow::Borrowed(DEVICE_PAGE.as_bytes())),
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

struct View {
    wv: WebView,
    /// The profile's context, which must outlive the view (Windows, Linux).
    _context: Option<Box<wry::WebContext>>,
    /// The address last asked for; a slot asking for another loads it, and
    /// where the page wandered since does not count.
    asked: String,
    shown: bool,
    rect: Rect,
    zoom: f64,
    /// Bumped on each load, so a ref from the last page misses.
    generation: u32,
    history: Vec<String>,
    at: usize,
    /// A history step in flight, to the index it lands on.
    stepping: Option<usize>,
}

/// The web views of every open pane.
pub struct Host {
    window: Option<Arc<NativeWindow>>,
    /// Why there are no views here, said in the pane.
    unavailable: Option<String>,
    views: HashMap<String, View>,
    tx: Sender<WebEvent>,
    rx: Receiver<WebEvent>,
    wake: Wake,
    next_token: u64,
    /// Device frames the `vorn` scheme serves.
    pub frames: Frames,
    /// Where connection profiles keep their data, off macOS.
    profiles: Option<std::path::PathBuf>,
}

impl Host {
    pub fn new(wake: Wake) -> Host {
        let (tx, rx) = channel();
        Host {
            window: None,
            unavailable: None,
            views: HashMap::new(),
            tx,
            rx,
            wake,
            next_token: 0,
            frames: Frames::default(),
            profiles: None,
        }
    }

    /// Keeps connection profiles under `dir`.
    pub fn set_profiles(&mut self, dir: std::path::PathBuf) {
        self.profiles = Some(dir);
    }

    /// Makes `key`'s view if it is not there, hidden: a page the app works
    /// in rather than one anybody looks at.
    pub fn ensure(&mut self, key: &str, url: &str, profile: Option<&str>) -> Result<(), String> {
        if self.views.contains_key(key) {
            return Ok(());
        }
        if self.window.is_none() {
            return Err("No window to open the page in.".to_owned());
        }
        if let Some(why) = &self.unavailable {
            return Err(why.clone());
        }
        let slot = Slot {
            key: key.to_owned(),
            url: url.to_owned(),
            rect: Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
            zoom: 1.0,
            profile: profile.map(str::to_owned),
        };
        let mut view = self.build(&slot)?;
        let _ = view.wv.set_visible(false);
        view.shown = false;
        self.views.insert(key.to_owned(), view);
        Ok(())
    }

    /// Drops every view in `connection`'s profile and what the profile
    /// keeps on disk.
    pub fn forget(&mut self, connection: &str) {
        self.views
            .retain(|k, _| crate::signin::connection_of(k) != Some(connection));
        #[cfg(target_os = "macos")]
        {
            use wry::WebViewExtDarwin;
            WebView::remove_data_store(&crate::signin::profile_uuid(connection), |_| {});
        }
        #[cfg(not(target_os = "macos"))]
        if let Some(root) = &self.profiles {
            let _ = std::fs::remove_dir_all(root.join(crate::signin::profile_folder(connection)));
        }
    }

    /// Seats future views in `window`.
    pub fn attach(&mut self, window: &Arc<NativeWindow>) {
        self.unavailable = platform_ready(window).err();
        self.window = Some(Arc::clone(window));
    }

    /// Why pages cannot show, when they cannot.
    pub fn unavailable(&self) -> Option<&str> {
        self.unavailable.as_deref()
    }

    /// Puts each slot's view where it goes, creating and loading as
    /// needed, and hides every view not among them.
    pub fn show(&mut self, slots: &[Slot]) {
        if self.window.is_none() || self.unavailable.is_some() {
            return;
        }
        for slot in slots {
            if !self.views.contains_key(&slot.key) {
                match self.build(slot) {
                    Ok(view) => {
                        self.views.insert(slot.key.clone(), view);
                    }
                    Err(e) => {
                        self.unavailable = Some(e);
                        return;
                    }
                }
            }
            let Some(v) = self.views.get_mut(&slot.key) else {
                continue;
            };
            if v.asked != slot.url {
                v.asked.clone_from(&slot.url);
                v.stepping = None;
                let _ = v.wv.load_url(&slot.url);
            }
            if v.rect != slot.rect {
                v.rect = slot.rect;
                let _ = v.wv.set_bounds(bounds(slot.rect));
            }
            if v.zoom != slot.zoom {
                v.zoom = slot.zoom;
                let _ = v.wv.zoom(slot.zoom);
            }
            if !v.shown {
                v.shown = true;
                let _ = v.wv.set_visible(true);
            }
        }
        for (key, v) in &mut self.views {
            if v.shown && !slots.iter().any(|s| &s.key == key) {
                v.shown = false;
                let _ = v.wv.set_visible(false);
            }
        }
    }

    fn build(&self, slot: &Slot) -> Result<View, String> {
        let window = self.window.as_ref().ok_or("No window")?;
        let send = |make: fn(String, String) -> WebEvent| {
            let tx = self.tx.clone();
            let wake = Arc::clone(&self.wake);
            let key = slot.key.clone();
            move |value: String| {
                let _ = tx.send(make(key.clone(), value));
                wake();
            }
        };
        let page = {
            let tx = self.tx.clone();
            let wake = Arc::clone(&self.wake);
            let key = slot.key.clone();
            move |event: PageLoadEvent, url: String| {
                let finished = matches!(event, PageLoadEvent::Finished);
                let _ = tx.send(WebEvent::Page {
                    key: key.clone(),
                    url,
                    finished,
                });
                wake();
            }
        };
        let title = send(|key, title| WebEvent::Title { key, title });
        let message = send(|key, body| WebEvent::Message { key, body });
        let popup = send(|key, url| WebEvent::NewWindow { key, url });
        let frames = Arc::clone(&self.frames);
        let mut context = self.context_for(slot.profile.as_deref());
        let builder = match context.as_deref_mut() {
            Some(ctx) => WebViewBuilder::new_with_web_context(ctx),
            None => WebViewBuilder::new(),
        };
        let mut builder = builder
            .with_url(&slot.url)
            .with_bounds(bounds(slot.rect))
            .with_on_page_load_handler(page)
            .with_document_title_changed_handler(title)
            .with_ipc_handler(move |req| message(req.into_body()))
            .with_new_window_req_handler(move |url, _| {
                popup(url);
                NewWindowResponse::Deny
            });
        match &slot.profile {
            // A connection's pages are the service's own: no pane script, no `vorn` scheme.
            Some(_conn) => {
                #[cfg(target_os = "macos")]
                {
                    use wry::WebViewBuilderExtDarwin;
                    builder =
                        builder.with_data_store_identifier(crate::signin::profile_uuid(_conn));
                }
            }
            None => {
                builder = builder
                    .with_initialization_script(PAGE_SCRIPT)
                    .with_custom_protocol("vorn".to_owned(), move |_, req| serve(&frames, &req));
            }
        }
        let wv = builder
            .build_as_child(window.as_ref())
            .map_err(|e| format!("The page view did not open: {e}"))?;
        if slot.zoom != 1.0 {
            let _ = wv.zoom(slot.zoom);
        }
        Ok(View {
            wv,
            _context: context,
            asked: slot.url.clone(),
            shown: true,
            rect: slot.rect,
            zoom: slot.zoom,
            generation: 1,
            history: Vec::new(),
            at: 0,
            stepping: None,
        })
    }

    /// A profile's own context off macOS, where profiles are folders.
    fn context_for(&self, profile: Option<&str>) -> Option<Box<wry::WebContext>> {
        let conn = profile?;
        if cfg!(target_os = "macos") {
            return None;
        }
        let root = self.profiles.clone()?;
        Some(Box::new(wry::WebContext::new(Some(
            root.join(crate::signin::profile_folder(conn)),
        ))))
    }

    /// Drops the views whose key `gone` says goes.
    pub fn close(&mut self, gone: impl Fn(&str) -> bool) {
        self.views.retain(|k, _| !gone(k));
    }

    pub fn has(&self, key: &str) -> bool {
        self.views.contains_key(key)
    }

    /// What arrived since the last call, with each view's own bookkeeping
    /// (address, generation, history) already folded in.
    pub fn poll(&mut self) -> Vec<WebEvent> {
        let events: Vec<WebEvent> = self.rx.try_iter().collect();
        for e in &events {
            let WebEvent::Page { key, url, finished } = e else {
                continue;
            };
            let Some(v) = self.views.get_mut(key) else {
                continue;
            };
            if !finished {
                v.generation += 1;
                continue;
            }
            match v.stepping.take() {
                Some(i) if v.history.get(i) == Some(url) => v.at = i,
                _ if v.history.get(v.at) == Some(url) => {}
                _ => {
                    v.history.truncate(v.at + 1);
                    v.history.push(url.clone());
                    v.at = v.history.len() - 1;
                }
            }
        }
        events
    }

    /// The page's generation, for the refs a read hands out.
    pub fn generation(&self, key: &str) -> Option<u32> {
        self.views.get(key).map(|v| v.generation)
    }

    /// Whether the view can go back and forward.
    pub fn nav(&self, key: &str) -> (bool, bool) {
        self.views
            .get(key)
            .map_or((false, false), |v| (v.at > 0, v.at + 1 < v.history.len()))
    }

    /// Steps the view's history; the address it goes to, or why not.
    pub fn step(&mut self, key: &str, back: bool) -> Result<String, String> {
        let v = self.views.get_mut(key).ok_or(NO_PANE)?;
        let to = if back {
            v.at.checked_sub(1)
        } else {
            Some(v.at + 1).filter(|i| *i < v.history.len())
        };
        let Some(to) = to else {
            return Err(if back {
                "No page to go back to \u{2014} this is the first page in this tab."
            } else {
                "No page to go forward to \u{2014} this is the newest page in this tab."
            }
            .to_owned());
        };
        v.stepping = Some(to);
        let _ = if back {
            v.wv.go_back()
        } else {
            v.wv.go_forward()
        };
        Ok(v.history[to].clone())
    }

    /// Loads `url` even when it is the address last asked for.
    pub fn load(&mut self, key: &str, url: &str) {
        if let Some(v) = self.views.get_mut(key) {
            v.asked = url.to_owned();
            v.stepping = None;
            let _ = v.wv.load_url(url);
        }
    }

    pub fn reload(&self, key: &str) {
        if let Some(v) = self.views.get(key) {
            let _ = v.wv.reload();
        }
    }

    /// Runs `js` in the view, not waiting for an answer.
    pub fn run(&self, key: &str, js: &str) {
        if let Some(v) = self.views.get(key) {
            let _ = v.wv.evaluate_script(js);
        }
    }

    /// Runs `js` in the view; its JSON answer arrives as
    /// [`WebEvent::Answer`] with the token returned here.
    pub fn eval(&mut self, key: &str, js: &str) -> Result<u64, String> {
        let v = self.views.get(key).ok_or(NO_PANE)?;
        self.next_token += 1;
        let token = self.next_token;
        let tx = self.tx.clone();
        let wake = Arc::clone(&self.wake);
        v.wv.evaluate_script_with_callback(js, move |json| {
            let _ = tx.send(WebEvent::Answer { token, json });
            wake();
        })
        .map_err(|e| e.to_string())?;
        Ok(token)
    }

    /// Pumps GTK's loop, which WebKitGTK runs on; when to come back.
    pub fn tick(&mut self) -> Option<Instant> {
        #[cfg(target_os = "linux")]
        if !self.views.is_empty() {
            while gtk::events_pending() {
                gtk::main_iteration_do(false);
            }
            return Some(Instant::now() + std::time::Duration::from_millis(8));
        }
        None
    }
}

/// What a bridge call says when the session has no pane to drive.
pub const NO_PANE: &str =
    "This session has no browser pane open. Open one from the session card, then retry.";

fn bounds(r: Rect) -> wry::Rect {
    wry::Rect {
        position: PhysicalPosition::new(r.x.round() as i32, r.y.round() as i32).into(),
        size: PhysicalSize::new(r.w.max(1.0).round() as u32, r.h.max(1.0).round() as u32).into(),
    }
}

/// WebKitGTK seats a child view only in an X11 window, on GTK's loop.
#[cfg(target_os = "linux")]
fn platform_ready(window: &NativeWindow) -> Result<(), String> {
    use wry::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = window.window_handle().map_err(|e| e.to_string())?;
    if !matches!(
        handle.as_raw(),
        RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_)
    ) {
        return Err("Pages show under X11 only; on Wayland, open them in your browser.".to_owned());
    }
    gtk::gdk::set_allowed_backends("x11");
    gtk::init().map_err(|e| format!("GTK did not start: {e}"))
}

#[cfg(not(target_os = "linux"))]
fn platform_ready(_window: &NativeWindow) -> Result<(), String> {
    Ok(())
}
