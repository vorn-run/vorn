//! The C API the iOS host calls, all on its main thread. Strings in are
//! UTF-8 and borrowed; strings out are freed with [`vp_free_string`].

use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::Arc;

use serde_json::{json, Value};
use vornui::backend::Backend;
use vornui::embed::Layer;
use vornui::native::Native;
use vornui::{Id, Ui};
use web_time::Instant;

use crate::model::{Model, Target};
use crate::phone::{Insets, Phone};
use crate::screens;

/// What the host is told when vornd sends something: call `vp_frame` or pull trees again.
pub type WakeFn = extern "C" fn(ctx: *mut c_void);

struct Ctx(*mut c_void);
// SAFETY: the host's context is a pointer it promises is safe to pass to its wake from any thread.
unsafe impl Send for Ctx {}
unsafe impl Sync for Ctx {}

#[derive(Default)]
struct Metrics {
    first_frame_ms: Option<f64>,
    /// CPU time to build, lay out, draw and present, milliseconds.
    frame_ms: Vec<f32>,
    /// Time between presented frames while something moved.
    interval_ms: Vec<f32>,
    keystroke_ms: Vec<f32>,
    last_present: Option<Instant>,
}

const KEEP: usize = 2000;

fn push(v: &mut Vec<f32>, x: f32) {
    if v.len() == KEEP {
        v.remove(0);
    }
    v.push(x);
}

fn stats(v: &[f32]) -> Value {
    if v.is_empty() {
        return json!({ "n": 0 });
    }
    let mut s = v.to_vec();
    s.sort_by(f32::total_cmp);
    let at = |q: f32| s[((s.len() - 1) as f32 * q).round() as usize];
    json!({ "n": s.len(), "p50": at(0.5), "p95": at(0.95), "max": s[s.len() - 1] })
}

/// One phone app: its state, the layer it draws into, and the UIs its native screens are read from.
pub struct Handle {
    phone: Phone,
    layer: Option<Layer>,
    natives: HashMap<String, Ui>,
    started: Instant,
    metrics: Metrics,
    typed: Option<(Instant, u64)>,
}

impl Handle {
    fn ui(&mut self) -> Option<&mut Ui> {
        self.layer.as_mut().map(|l| &mut l.ui)
    }

    fn typed(&mut self) {
        self.typed = Some((Instant::now(), self.phone.model.grid_updates));
    }

    /// Draws a frame if one is due; whether it drew.
    fn frame(&mut self) -> bool {
        let Some(layer) = &mut self.layer else {
            self.phone.model.pump();
            return false;
        };
        if !self.phone.due(&layer.ui) {
            self.metrics.last_present = None;
            return false;
        }
        let t0 = Instant::now();
        let drew = matches!(layer.frame(&mut self.phone), Ok(Some(_)));
        if !drew {
            return false;
        }
        let now = Instant::now();
        let m = &mut self.metrics;
        push(&mut m.frame_ms, ms(now - t0));
        if let Some(prev) = m.last_present {
            push(&mut m.interval_ms, ms(now - prev));
        }
        m.last_present = Some(now);
        if m.first_frame_ms.is_none() && self.phone.model.link == crate::Link::Online {
            m.first_frame_ms = Some(f64::from(ms(now - self.started)));
        }
        if let Some((at, n)) = self.typed {
            if self.phone.model.grid_updates > n {
                push(&mut m.keystroke_ms, ms(now - at));
                self.typed = None;
            }
        }
        true
    }

    fn tree(&mut self, screen: &str) -> String {
        let (w, h) = self.phone.size;
        let scale = self.layer.as_ref().map_or(3.0, |l| l.ui.scale());
        let ui = self
            .natives
            .entry(screen.to_owned())
            .or_insert_with(|| Ui::native(&crate::config(scale)));
        self.phone.model.pump();
        let el = screens::native(&self.phone, ui, screen);
        ui.begin(vornui::Rgba::TRANSPARENT);
        ui.layout(el, (w, h));
        let mut out = Native::default();
        let _ = out.present(ui);
        out.json
    }

    fn act(&mut self, screen: &str, a: &Value) {
        let Some(id) = a["id"].as_str().and_then(|s| s.parse().ok()).map(Id) else {
            return;
        };
        // The template's bindings are the last screen read's; this screen's first.
        let _ = self.tree(screen);
        if a["confirmed"].as_bool() == Some(true) {
            self.phone.model.points.asking = Some(id.0);
            let _ = self.tree(screen);
            self.phone.model.click(id.child("ok"));
            return;
        }
        if let Some(v) = a["value"].as_str() {
            let Some(ui) = self.natives.get_mut(screen) else {
                return;
            };
            ui.set_edit_text(id, v);
            if a["submit"].as_bool() == Some(true) {
                self.phone.model.submit(ui, id);
            }
            return;
        }
        self.phone.model.click(id);
    }
}

fn ms(d: std::time::Duration) -> f32 {
    d.as_secs_f32() * 1000.0
}

fn str_in<'a>(s: *const c_char) -> &'a str {
    if s.is_null() {
        return "";
    }
    // SAFETY: the host passes NUL-terminated UTF-8 that lives through the call.
    unsafe { CStr::from_ptr(s) }.to_str().unwrap_or("")
}

fn str_out(s: String) -> *mut c_char {
    CString::new(s).map_or(std::ptr::null_mut(), CString::into_raw)
}

fn with<T: Default>(h: *mut Handle, f: impl FnOnce(&mut Handle) -> T) -> T {
    // SAFETY: `h` came from `vp_new` and is used on the host's main thread only.
    match unsafe { h.as_mut() } {
        Some(h) => f(h),
        None => T::default(),
    }
}

/// A phone app connecting to vornd at `host` (`ip:port`) with device token `token`.
#[no_mangle]
pub extern "C" fn vp_new(
    host: *const c_char,
    token: *const c_char,
    wake: WakeFn,
    ctx: *mut c_void,
) -> *mut Handle {
    let started = Instant::now();
    let ctx = Arc::new(Ctx(ctx));
    let model = Model::new(Arc::new(move || wake(ctx.0)));
    let mut phone = Phone::new(model);
    let (host, token) = (str_in(host), str_in(token));
    if !host.is_empty() {
        phone.connect(Target {
            host: host.to_owned(),
            token: token.to_owned(),
        });
    }
    Box::into_raw(Box::new(Handle {
        phone,
        layer: None,
        natives: HashMap::new(),
        started,
        metrics: Metrics::default(),
        typed: None,
    }))
}

/// # Safety
/// `h` came from [`vp_new`] and is not used again.
#[no_mangle]
pub unsafe extern "C" fn vp_free(h: *mut Handle) {
    if !h.is_null() {
        // SAFETY: per the contract above.
        drop(unsafe { Box::from_raw(h) });
    }
}

/// Draws into `layer`, a CAMetalLayer, `w`×`h` points at `scale`; only the
/// open session's terminal when `terminal_only`. Answers whether it could.
///
/// # Safety
/// `layer` outlives `h`'s use of it (until `vp_free`).
#[no_mangle]
pub unsafe extern "C" fn vp_attach_layer(
    h: *mut Handle,
    layer: *mut c_void,
    w: f32,
    hgt: f32,
    scale: f32,
    terminal_only: bool,
) -> bool {
    with(h, |h| {
        let px = ((w * scale) as u32, (hgt * scale) as u32);
        // SAFETY: the host keeps the layer alive, per the contract above.
        match unsafe { Layer::core_animation(layer, px, &crate::config(scale)) } {
            Ok(l) => {
                h.layer = Some(l);
                h.phone.terminal_only = terminal_only;
                h.phone.size = (w, hgt);
                h.phone.dirty = true;
                true
            }
            Err(e) => {
                eprintln!("vorn-phone: {e}");
                false
            }
        }
    })
}

/// The view is `w`×`h` points at `scale`, inside `insets`, with the keyboard `keyboard` points tall.
#[no_mangle]
pub extern "C" fn vp_resize(
    h: *mut Handle,
    w: f32,
    hgt: f32,
    scale: f32,
    top: f32,
    bottom: f32,
    left: f32,
    right: f32,
    keyboard: f32,
) {
    with(h, |h| {
        let p = &mut h.phone;
        p.size = (w, hgt);
        p.insets = Insets {
            top,
            bottom,
            left,
            right,
        };
        p.keyboard = keyboard;
        p.dirty = true;
        if let Some(l) = &mut h.layer {
            l.resize(((w * scale) as u32, (hgt * scale) as u32), scale);
        }
    });
}

/// Draws a frame if one is due; whether it drew.
#[no_mangle]
pub extern "C" fn vp_frame(h: *mut Handle) -> bool {
    with(h, Handle::frame)
}

/// Folds in what vornd sent; whether anything changed (pull trees again).
#[no_mangle]
pub extern "C" fn vp_pump(h: *mut Handle) -> bool {
    with(h, |h| {
        let changed = h.phone.model.pump();
        h.phone.dirty |= changed;
        changed
    })
}

/// A touch at (`x`, `y`) points: 0 began, 1 moved, 2 ended, 3 cancelled.
#[no_mangle]
pub extern "C" fn vp_touch(h: *mut Handle, phase: i32, x: f32, y: f32) {
    with(h, |h| {
        let Some(l) = &mut h.layer else { return };
        let (p, ui) = (&mut h.phone, &mut l.ui);
        match phase {
            0 => p.touch_down(ui, x, y),
            1 => p.touch_move(ui, x, y),
            2 => p.touch_up(ui, x, y),
            _ => p.touch_cancel(ui),
        }
    });
}

/// Text the system keyboard typed or dictated.
#[no_mangle]
pub extern "C" fn vp_text(h: *mut Handle, s: *const c_char) {
    with(h, |h| {
        h.typed();
        let Some(l) = &mut h.layer else { return };
        h.phone.text(&mut l.ui, str_in(s));
    });
}

/// The IME's marked text, empty when composition ends.
#[no_mangle]
pub extern "C" fn vp_preedit(h: *mut Handle, s: *const c_char) {
    with(h, |h| {
        let Some(l) = &mut h.layer else { return };
        h.phone.preedit(&mut l.ui, str_in(s));
    });
}

/// A key by DOM code (`Backspace`, `ArrowUp`) with modifier bits (shift 1, alt 2, ctrl 4).
#[no_mangle]
pub extern "C" fn vp_key(h: *mut Handle, code: *const c_char, mods: u16) {
    with(h, |h| {
        h.typed();
        let Some(l) = &mut h.layer else { return };
        h.phone.key(&mut l.ui, str_in(code), mods);
    });
}

/// Sends `text` and Return to the open session: the composer.
#[no_mangle]
pub extern "C" fn vp_send(h: *mut Handle, text: *const c_char) {
    with(h, |h| {
        h.typed();
        h.phone.send(str_in(text));
    });
}

/// Whether the system keyboard should be up.
#[no_mangle]
pub extern "C" fn vp_wants_keyboard(h: *mut Handle) -> bool {
    with(h, |h| {
        let typing = h.phone.typing;
        h.ui().map_or(typing, |ui| ui.focus().is_some()) || (typing && h.phone.session.is_some())
    })
}

/// The keyboard went down by the system's hand.
#[no_mangle]
pub extern "C" fn vp_keyboard_hidden(h: *mut Handle) {
    with(h, |h| {
        h.phone.typing = false;
        if let Some(ui) = h.ui() {
            ui.set_focus(None);
        }
        h.phone.dirty = true;
    });
}

/// Opens the session `id`'s terminal; null or empty closes it.
#[no_mangle]
pub extern "C" fn vp_open_session(h: *mut Handle, id: *const c_char) {
    with(h, |h| {
        let id = str_in(id);
        h.phone.open((!id.is_empty()).then(|| id.to_owned()));
    });
}

/// The semantic tree of `screen` (see [`screens::native`]) as JSON.
#[no_mangle]
pub extern "C" fn vp_native_tree(h: *mut Handle, screen: *const c_char) -> *mut c_char {
    with(h, |h| str_out(h.tree(str_in(screen))))
}

/// A native control acted on `screen`: `{"id", "value"?, "submit"?, "confirmed"?}`.
#[no_mangle]
pub extern "C" fn vp_native_action(h: *mut Handle, screen: *const c_char, action: *const c_char) {
    with(h, |h| {
        if let Ok(a) = serde_json::from_str::<Value>(str_in(action)) {
            h.act(str_in(screen), &a);
        }
    });
}

/// How the app has run: first frame, frame times, keystroke latency, as JSON.
#[no_mangle]
pub extern "C" fn vp_metrics(h: *mut Handle) -> *mut c_char {
    with(h, |h| {
        let m = &h.metrics;
        str_out(
            json!({
                "firstFrameMs": m.first_frame_ms,
                "frameMs": stats(&m.frame_ms),
                "intervalMs": stats(&m.interval_ms),
                "keystrokeMs": stats(&m.keystroke_ms),
                "link": format!("{:?}", h.phone.model.link),
            })
            .to_string(),
        )
    })
}

/// Clears the frame and keystroke samples, to measure one gesture.
#[no_mangle]
pub extern "C" fn vp_metrics_reset(h: *mut Handle) {
    with(h, |h| {
        let first = h.metrics.first_frame_ms;
        h.metrics = Metrics {
            first_frame_ms: first,
            ..Metrics::default()
        };
    });
}

/// # Safety
/// `s` came from this library and is not used again.
#[no_mangle]
pub unsafe extern "C" fn vp_free_string(s: *mut c_char) {
    if !s.is_null() {
        // SAFETY: per the contract above.
        drop(unsafe { CString::from_raw(s) });
    }
}
