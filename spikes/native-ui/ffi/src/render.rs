//! The GPU pane renderer's C API (`include/vorn_term.h`): an app handle that
//! owns the device and an action queue, and one surface per pane that the
//! host attaches to its own view. Render threads never call into the host
//! directly: they queue tagged actions and call the wakeup callback, and
//! the host drains the queue on its main thread with `vt_app_tick`.

use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use spike_core::PaneView;
use term_render::{Host, Presented, Renderer, Surface};

use crate::{str_arg, Shared, VsHandle, GPU};

/// Action tags.
pub const CONTENT_CHANGED: u32 = 1;
pub const FIRST_FRAME: u32 = 2;

type Wakeup = extern "C" fn(*mut c_void);
type Action = extern "C" fn(*mut c_void, u32, u32);

struct Queue {
    items: Mutex<Vec<(u32, u32, Option<Arc<AtomicBool>>)>>,
    wakeup: Wakeup,
    ud: usize,
}

impl Queue {
    fn post(&self, pane: u32, tag: u32, clear: Option<Arc<AtomicBool>>) {
        let was_empty = {
            let mut q = self.items.lock().unwrap();
            q.push((pane, tag, clear));
            q.len() == 1
        };
        if was_empty {
            (self.wakeup)(self.ud as *mut c_void);
        }
    }
}

pub struct VtApp {
    s: Arc<Shared>,
    r: Renderer,
    q: Arc<Queue>,
    action: Action,
    ud: usize,
}

pub struct VtSurface {
    s: Arc<Shared>,
    pane: usize,
    surf: Surface,
    size: (u16, u16),
}

struct PaneHost {
    s: Arc<Shared>,
    pane: usize,
    q: Arc<Queue>,
    first: AtomicBool,
    /// A content-changed action is queued and not yet delivered.
    changed: Arc<AtomicBool>,
}

impl Host for PaneHost {
    fn view(&self) -> Option<PaneView> {
        self.s.grid.view(self.pane)
    }

    fn presented(&self, p: Presented) {
        self.s.gpu_frame(p.at, p.work_ms);
        if p.probe_hit {
            self.s.hit(p.at);
        }
        if p.had_view {
            self.s.shown(self.pane);
            if !self.first.swap(true, Ordering::Relaxed) {
                self.q.post(self.pane as u32, FIRST_FRAME, None);
            }
            if !self.changed.swap(true, Ordering::Relaxed) {
                self.q.post(self.pane as u32, CONTENT_CHANGED, Some(Arc::clone(&self.changed)));
            }
        }
    }
}

/// The app handle over an open grid client. `wakeup(ud)` is called from
/// any thread when actions are queued; `action(ud, pane, tag)` is called
/// from `vt_app_tick` on the thread that calls it.
#[no_mangle]
pub unsafe extern "C" fn vt_app_new(
    h: *const VsHandle,
    wakeup: Wakeup,
    action: Action,
    ud: *mut c_void,
) -> *mut VtApp {
    match Renderer::new() {
        Ok(r) => Box::into_raw(Box::new(VtApp {
            s: Arc::clone(&(&*h).s),
            r,
            q: Arc::new(Queue {
                items: Mutex::new(Vec::new()),
                wakeup,
                ud: ud as usize,
            }),
            action,
            ud: ud as usize,
        })),
        Err(e) => {
            eprintln!("term-render: {e}");
            std::ptr::null_mut()
        }
    }
}

/// Delivers queued actions.
#[no_mangle]
pub unsafe extern "C" fn vt_app_tick(app: *const VtApp) {
    let app = &*app;
    let items = std::mem::take(&mut *app.q.items.lock().unwrap());
    for (pane, tag, clear) in items {
        if let Some(c) = clear {
            c.store(false, Ordering::Relaxed);
        }
        (app.action)(app.ud as *mut c_void, pane, tag);
    }
}

#[no_mangle]
pub unsafe extern "C" fn vt_app_free(app: *mut VtApp) {
    drop(Box::from_raw(app));
}

/// Attaches a renderer for `pane` to `view` (an NSView or UIView); main
/// thread only. The pane is drawn by it until `vt_surface_free`.
#[no_mangle]
pub unsafe extern "C" fn vt_surface_new(
    app: *const VtApp,
    pane: u32,
    view: *mut c_void,
    scale: f64,
    font_pt: f64,
) -> *mut VtSurface {
    let app = &*app;
    let pane = pane as usize;
    let host = Arc::new(PaneHost {
        s: Arc::clone(&app.s),
        pane,
        q: Arc::clone(&app.q),
        first: AtomicBool::new(false),
        changed: Arc::new(AtomicBool::new(false)),
    });
    let surf = match Surface::new(&app.r, view, host, font_pt, scale) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("term-render: {e}");
            return std::ptr::null_mut();
        }
    };
    let waker = surf.waker();
    app.s.owner[pane].store(GPU, Ordering::Relaxed);
    app.s.gpu_wakers.lock().unwrap()[pane] = Some(Box::new(move || waker.wake()));
    Box::into_raw(Box::new(VtSurface {
        s: Arc::clone(&app.s),
        pane,
        surf,
        size: (0, 0),
    }))
}

#[no_mangle]
pub unsafe extern "C" fn vt_surface_free(s: *mut VtSurface) {
    let s = Box::from_raw(s);
    s.s.gpu_wakers.lock().unwrap()[s.pane] = None;
    drop(s);
}

/// The host view is `w`x`h` points; resizes the pane's grid to fit.
#[no_mangle]
pub unsafe extern "C" fn vt_surface_set_size(s: *mut VtSurface, w: f64, h: f64) {
    let s = &mut *s;
    let size = s.surf.set_size(w, h);
    if size != s.size && w > 0.0 {
        s.size = size;
        s.s.grid.resize(s.pane, size.0, size.1);
    }
}

#[no_mangle]
pub unsafe extern "C" fn vt_surface_set_scale(s: *const VtSurface, scale: f64) {
    (&*s).surf.set_scale(scale);
}

#[no_mangle]
pub unsafe extern "C" fn vt_surface_set_focus(s: *const VtSurface, focused: bool) {
    (&*s).surf.set_focus(focused);
}

#[no_mangle]
pub unsafe extern "C" fn vt_surface_key(
    s: *const VtSurface,
    code: *const c_char,
    mods: u16,
    text: *const c_char,
) {
    if let Some(code) = str_arg(code) {
        (&*s).s.grid.key((&*s).pane, code, mods, str_arg(text));
    }
}

#[no_mangle]
pub unsafe extern "C" fn vt_surface_text(s: *const VtSurface, utf8: *const c_char) {
    if let Some(t) = str_arg(utf8) {
        (&*s).s.grid.text((&*s).pane, t);
    }
}

/// The IME's composition (null or empty clears it), drawn at the cursor.
#[no_mangle]
pub unsafe extern "C" fn vt_surface_preedit(s: *const VtSurface, utf8: *const c_char) {
    (&*s).surf.set_preedit(str_arg(utf8).unwrap_or(""));
}

/// Pointer input at (`x`, `y`) points. The spike has no selection, so it
/// is accepted and ignored; the host handles focus.
#[no_mangle]
pub unsafe extern "C" fn vt_surface_mouse(_s: *const VtSurface, _x: f64, _y: f64, _button: i32, _action: i32) {}

#[repr(C)]
pub struct VtMetrics {
    pub cell_w: f64,
    pub cell_h: f64,
    pub pad_x: f64,
    pub pad_y: f64,
    pub cursor_x: u16,
    pub cursor_y: u16,
    pub cols: u16,
    pub rows: u16,
}

/// Cell geometry in points and the cursor's cell, for IME placement.
#[no_mangle]
pub unsafe extern "C" fn vt_surface_metrics(s: *const VtSurface, out: *mut VtMetrics) {
    let s = &*s;
    let m = s.surf.metrics;
    let (cx, cy, cols, rows) = s
        .s
        .grid
        .view(s.pane)
        .map_or((0, 0, 0, 0), |v| (v.cursor_x, v.cursor_y, v.cols, v.rows));
    *out = VtMetrics {
        cell_w: m.cell_w,
        cell_h: m.cell_h,
        pad_x: m.pad_x,
        pad_y: m.pad_y,
        cursor_x: cx,
        cursor_y: cy,
        cols,
        rows,
    };
}

/// The pane's text for accessibility; free with `vs_free_text`.
#[no_mangle]
pub unsafe extern "C" fn vt_surface_read_text(s: *const VtSurface) -> *mut c_char {
    let v = (&*s).s.grid.view((&*s).pane);
    let t = v.map(|v| crate::screen_text(&v)).unwrap_or_default();
    std::ffi::CString::new(t.replace('\0', " ")).map_or(std::ptr::null_mut(), |c| c.into_raw())
}
