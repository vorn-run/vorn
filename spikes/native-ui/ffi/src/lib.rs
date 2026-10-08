//! A small C ABI over the spike's grid client, so the Swift apps decode the
//! grid with exactly the Rust code the other prototypes use.
//! `include/vorn_spike.h` is its header. With the `render` feature it also
//! exports the GPU pane renderer's surface API (`render.rs`,
//! `include/vorn_term.h`).
#![allow(clippy::missing_safety_doc)]

use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

use spike_core::bench::{Bench, Config, Step};
use spike_core::{Grid, PaneView, RunC};

#[cfg(feature = "render")]
mod render;

type Cb = Box<dyn Fn() + Send + Sync>;

/// Which renderer draws a pane: the host's own (Swift) or the GPU one.
pub const SWIFT: u8 = 0;
pub const GPU: u8 = 1;

pub struct VsHandle {
    s: Arc<Shared>,
}

pub(crate) struct Shared {
    pub grid: Arc<Grid>,
    pub bench: Mutex<Bench>,
    started: Instant,
    owner: Vec<AtomicU8>,
    /// Panes the host draws that changed since `vs_take_dirty`.
    host_dirty: Mutex<Vec<bool>>,
    host_waker: Mutex<Option<Cb>>,
    host_wake_pending: AtomicBool,
    /// Per pane, the GPU surface's waker while one is attached.
    pub gpu_wakers: Mutex<Vec<Option<Cb>>>,
    typed: Mutex<Option<Instant>>,
    shown: Vec<AtomicBool>,
    first_sent: AtomicBool,
    frames: Mutex<Agg>,
}

/// Presents from all GPU surfaces folded into one entry per display period,
/// so frame counts compare with a single display-linked loop.
#[derive(Default)]
struct Agg {
    slot: Option<i64>,
    at_ms: f64,
    work_ms: f64,
}

impl Shared {
    pub fn ms(&self, t: Instant) -> f64 {
        t.duration_since(self.started).as_secs_f64() * 1000.0
    }

    /// Routes changed panes to whoever draws them.
    fn route(&self) {
        let dirty = self.grid.take_dirty();
        let mut host = false;
        {
            let gpu = self.gpu_wakers.lock().unwrap();
            let mut hd = self.host_dirty.lock().unwrap();
            for p in dirty {
                match gpu.get(p).and_then(|w| w.as_ref()) {
                    Some(w) if self.owner[p].load(Ordering::Relaxed) == GPU => w(),
                    _ => {
                        hd[p] = true;
                        host = true;
                    }
                }
            }
        }
        if host && !self.host_wake_pending.swap(true, Ordering::SeqCst) {
            if let Some(w) = self.host_waker.lock().unwrap().as_ref() {
                w();
            }
        }
    }

    /// A probe glyph reached the screen: the latency since it was typed.
    pub fn hit(&self, at: Instant) {
        if let Some(t) = self.typed.lock().unwrap().take() {
            self.bench.lock().unwrap().latency(at.duration_since(t).as_secs_f64() * 1000.0);
        }
    }

    /// A pane has drawn a screen; the first frame is when all have.
    pub fn shown(&self, pane: usize) {
        if let Some(s) = self.shown.get(pane) {
            s.store(true, Ordering::Relaxed);
        }
        if !self.first_sent.load(Ordering::Relaxed)
            && self.shown.iter().all(|s| s.load(Ordering::Relaxed))
            && self.grid.all_snapshotted()
            && !self.first_sent.swap(true, Ordering::SeqCst)
        {
            self.bench.lock().unwrap().first_frame();
        }
    }

    /// A GPU surface presented at `at` after `work_ms` of drawing.
    pub fn gpu_frame(&self, at: Instant, work_ms: f64) {
        let ms = self.ms(at);
        let period = self.bench.lock().unwrap().period_ms.max(1.0);
        let slot = (ms / period).floor() as i64;
        let flush = {
            let mut a = self.frames.lock().unwrap();
            if a.slot == Some(slot) {
                a.work_ms += work_ms;
                None
            } else {
                let prev = a.slot.map(|_| (a.at_ms, a.work_ms));
                *a = Agg {
                    slot: Some(slot),
                    at_ms: ms,
                    work_ms,
                };
                prev
            }
        };
        if let Some((at, work)) = flush {
            self.bench.lock().unwrap().frame(at, work);
        }
    }
}

#[repr(C)]
pub struct VsView {
    pub rev: u64,
    pub cols: u16,
    pub rows: u16,
    pub cursor_x: u16,
    pub cursor_y: u16,
    pub cursor_visible: u8,
    pub cursor_style: u8,
    pub probe_hit: u8,
    pub _pad: u8,
    pub fg: u32,
    pub bg: u32,
    pub cursor_color: u32,
    pub nruns: u32,
    pub runs: *const RunC,
    pub text: *const u8,
    pub text_len: u32,
    /// Owned by Rust; freed by `vs_view_free`.
    pub owner: *mut c_void,
}

unsafe fn sh<'a>(h: *const VsHandle) -> &'a Shared {
    &(&*h).s
}

pub(crate) unsafe fn str_arg<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        None
    } else {
        CStr::from_ptr(p).to_str().ok()
    }
}

/// Connects to the grid endpoint and sessions named in the environment.
/// Null if they are missing or vornd is not there.
#[no_mangle]
pub extern "C" fn vs_open(cols: u16, rows: u16) -> *mut VsHandle {
    let Some(socket) = spike_core::env::grid() else {
        return std::ptr::null_mut();
    };
    let Ok(grid) = Grid::connect(&socket, spike_core::env::sessions(), cols, rows) else {
        return std::ptr::null_mut();
    };
    let n = grid.panes();
    // VORN_SPIKE_RENDERER: "swift" (default), "gpu", or one letter per pane
    // ("a" Swift, "b" GPU), e.g. "abab".
    let spec = std::env::var("VORN_SPIKE_RENDERER").unwrap_or_default();
    let owner = (0..n)
        .map(|i| {
            let gpu = match spec.as_str() {
                "gpu" | "rust" | "b" => true,
                s if s.len() > 1 => s.as_bytes().get(i).is_some_and(|c| *c == b'b'),
                _ => false,
            };
            AtomicU8::new(if gpu { GPU } else { SWIFT })
        })
        .collect();
    let s = Arc::new(Shared {
        grid: Arc::clone(&grid),
        bench: Mutex::new(Bench::new(Config::from_env())),
        started: Instant::now(),
        owner,
        host_dirty: Mutex::new(vec![false; n]),
        host_waker: Mutex::new(None),
        host_wake_pending: AtomicBool::new(false),
        gpu_wakers: Mutex::new((0..n).map(|_| None).collect()),
        typed: Mutex::new(None),
        shown: (0..n).map(|_| AtomicBool::new(false)).collect(),
        first_sent: AtomicBool::new(false),
        frames: Mutex::new(Agg::default()),
    });
    let weak: Weak<Shared> = Arc::downgrade(&s);
    grid.set_waker(move || {
        if let Some(s) = weak.upgrade() {
            s.route();
        }
    });
    Box::into_raw(Box::new(VsHandle { s }))
}

/// `cb(ctx)` is called on the reader thread when panes the host draws
/// changed and it has not taken them since.
#[no_mangle]
pub unsafe extern "C" fn vs_set_waker(
    h: *const VsHandle,
    cb: extern "C" fn(*mut c_void),
    ctx: *mut c_void,
) {
    let ctx = ctx as usize;
    let s = sh(h);
    *s.host_waker.lock().unwrap() = Some(Box::new(move || cb(ctx as *mut c_void)));
    s.host_wake_pending.store(true, Ordering::SeqCst);
    cb(ctx as *mut c_void);
}

#[no_mangle]
pub unsafe extern "C" fn vs_panes(h: *const VsHandle) -> u32 {
    sh(h).grid.panes() as u32
}

/// Writes up to `cap` changed host-drawn pane indices to `out`; answers how
/// many, and re-arms the waker.
#[no_mangle]
pub unsafe extern "C" fn vs_take_dirty(h: *const VsHandle, out: *mut u32, cap: u32) -> u32 {
    let s = sh(h);
    s.host_wake_pending.store(false, Ordering::SeqCst);
    let mut hd = s.host_dirty.lock().unwrap();
    let mut n = 0;
    for (p, d) in hd.iter_mut().enumerate() {
        if *d && n < cap as usize {
            *out.add(n) = p as u32;
            *d = false;
            n += 1;
        }
    }
    n as u32
}

/// 0: the host draws `pane` (option A); 1: the GPU renderer does (B).
#[no_mangle]
pub unsafe extern "C" fn vs_pane_renderer(h: *const VsHandle, pane: u32) -> u32 {
    sh(h).owner.get(pane as usize).map_or(0, |o| o.load(Ordering::Relaxed) as u32)
}

/// Switches who draws `pane`; the host re-creates the pane's view.
#[no_mangle]
pub unsafe extern "C" fn vs_set_pane_renderer(h: *const VsHandle, pane: u32, r: u32) {
    let s = sh(h);
    if let Some(o) = s.owner.get(pane as usize) {
        o.store(if r == 1 { GPU } else { SWIFT }, Ordering::Relaxed);
    }
    s.host_dirty.lock().unwrap()[pane as usize] = true;
}

/// The pane's screen as text, rows joined by newlines, trailing blanks
/// trimmed: what accessibility reads. Free with `vs_free_text`.
#[no_mangle]
pub unsafe extern "C" fn vs_read_text(h: *const VsHandle, pane: u32) -> *mut c_char {
    let text = sh(h).grid.view(pane as usize).map(|v| screen_text(&v)).unwrap_or_default();
    CString::new(text.replace('\0', " ")).map_or(std::ptr::null_mut(), CString::into_raw)
}

#[no_mangle]
pub unsafe extern "C" fn vs_free_text(t: *mut c_char) {
    if !t.is_null() {
        drop(CString::from_raw(t));
    }
}

pub(crate) fn screen_text(v: &PaneView) -> String {
    let mut rows: Vec<Vec<String>> = vec![vec![" ".to_owned(); v.cols as usize]; v.rows as usize];
    for r in &v.runs {
        let Some(row) = rows.get_mut(r.row as usize) else {
            continue;
        };
        let t = v.run_text(r);
        if r.flags & spike_core::view::flags::CLUSTER != 0 {
            if let Some(c) = row.get_mut(r.col as usize) {
                *c = t.to_owned();
                // The second column of a wide cell holds nothing.
                if r.ncols == 2 {
                    if let Some(c) = row.get_mut(r.col as usize + 1) {
                        c.clear();
                    }
                }
            }
        } else {
            for (i, ch) in t.chars().enumerate() {
                if let Some(c) = row.get_mut(r.col as usize + i) {
                    *c = ch.to_string();
                }
            }
        }
    }
    rows.iter()
        .map(|r| r.concat().trim_end().to_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

#[no_mangle]
pub unsafe extern "C" fn vs_all_snapshotted(h: *const VsHandle) -> bool {
    sh(h).grid.all_snapshotted()
}

#[no_mangle]
pub unsafe extern "C" fn vs_closed(h: *const VsHandle) -> bool {
    sh(h).grid.closed()
}

/// The pane now, or false if it has no screen yet.
#[no_mangle]
pub unsafe extern "C" fn vs_view(h: *const VsHandle, pane: u32, out: *mut VsView) -> bool {
    let Some(v) = sh(h).grid.view(pane as usize) else {
        return false;
    };
    let v: Box<PaneView> = Box::new(v);
    *out = VsView {
        rev: v.rev,
        cols: v.cols,
        rows: v.rows,
        cursor_x: v.cursor_x,
        cursor_y: v.cursor_y,
        cursor_visible: v.cursor_visible as u8,
        cursor_style: v.cursor_style,
        probe_hit: v.probe_hit as u8,
        _pad: 0,
        fg: v.fg,
        bg: v.bg,
        cursor_color: v.cursor_color,
        nruns: v.runs.len() as u32,
        runs: v.runs.as_ptr(),
        text: v.text.as_ptr(),
        text_len: v.text.len() as u32,
        owner: Box::into_raw(v).cast(),
    };
    true
}

#[no_mangle]
pub unsafe extern "C" fn vs_view_free(v: *mut VsView) {
    if !(*v).owner.is_null() {
        drop(Box::from_raw((*v).owner.cast::<PaneView>()));
        (*v).owner = std::ptr::null_mut();
    }
}

/// A key press: `code` a W3C code name ("KeyA", "Enter"), `text` what it
/// types (may be null).
#[no_mangle]
pub unsafe extern "C" fn vs_key(
    h: *const VsHandle,
    pane: u32,
    code: *const c_char,
    mods: u16,
    text: *const c_char,
) {
    if let Some(code) = str_arg(code) {
        sh(h).grid.key(pane as usize, code, mods, str_arg(text));
    }
}

#[no_mangle]
pub unsafe extern "C" fn vs_text(h: *const VsHandle, pane: u32, utf8: *const c_char) {
    if let Some(t) = str_arg(utf8) {
        sh(h).grid.text(pane as usize, t);
    }
}

#[no_mangle]
pub unsafe extern "C" fn vs_resize(h: *const VsHandle, pane: u32, cols: u16, rows: u16) {
    sh(h).grid.resize(pane as usize, cols, rows);
}

/// 0 interactive, 1 latency, 2 frames, 3 start.
#[no_mangle]
pub unsafe extern "C" fn vs_bench_mode(h: *const VsHandle) -> u32 {
    use spike_core::bench::Mode;
    match sh(h).bench.lock().unwrap().cfg.mode {
        Mode::Interactive => 0,
        Mode::Latency => 1,
        Mode::Frames => 2,
        Mode::Start => 3,
    }
}

/// 0 idle, 1 type `*ch`, 2 press Enter, 3 done.
#[no_mangle]
pub unsafe extern "C" fn vs_bench_tick(h: *const VsHandle, ch: *mut u32) -> u32 {
    let step = sh(h).bench.lock().unwrap().tick(&sh(h).grid);
    match step {
        Step::Idle => 0,
        Step::Type(c) => {
            *ch = c as u32;
            1
        }
        Step::Enter => 2,
        Step::Done => 3,
    }
}

#[no_mangle]
pub unsafe extern "C" fn vs_bench_frame(h: *const VsHandle, at_ms: f64, work_ms: f64) {
    sh(h).bench.lock().unwrap().frame(at_ms, work_ms);
}

#[no_mangle]
pub unsafe extern "C" fn vs_bench_first_frame(h: *const VsHandle) {
    sh(h).bench.lock().unwrap().first_frame();
}

#[no_mangle]
pub unsafe extern "C" fn vs_bench_latency(h: *const VsHandle, ms: f64) {
    sh(h).bench.lock().unwrap().latency(ms);
}

#[no_mangle]
pub unsafe extern "C" fn vs_bench_set_period(h: *const VsHandle, ms: f64) {
    sh(h).bench.lock().unwrap().period_ms = ms;
}

/// Takes the look test's screenshot of this process's window, if asked for.
#[no_mangle]
pub unsafe extern "C" fn vs_bench_shoot(h: *const VsHandle) {
    sh(h).bench.lock().unwrap().cfg.shoot();
}

/// The latency probe's key was just typed.
#[no_mangle]
pub unsafe extern "C" fn vs_bench_typed(h: *const VsHandle) {
    *sh(h).typed.lock().unwrap() = Some(Instant::now());
}

/// The host just put a view with `probe_hit` on screen.
#[no_mangle]
pub unsafe extern "C" fn vs_bench_hit(h: *const VsHandle) {
    sh(h).hit(Instant::now());
}

/// The host drew `pane` with a screen in it.
#[no_mangle]
pub unsafe extern "C" fn vs_pane_shown(h: *const VsHandle, pane: u32) {
    sh(h).shown(pane as usize);
}

/// Milliseconds on the clock GPU frames are recorded with, for host frames
/// in the same run.
#[no_mangle]
pub unsafe extern "C" fn vs_now_ms(h: *const VsHandle) -> f64 {
    sh(h).ms(Instant::now())
}

#[no_mangle]
pub unsafe extern "C" fn vs_bench_write(h: *const VsHandle) {
    vs_bench_write_as(h, c"swift".as_ptr());
}

/// Writes the results under the client name `client`.
#[no_mangle]
pub unsafe extern "C" fn vs_bench_write_as(h: *const VsHandle, client: *const c_char) {
    let client = str_arg(client).unwrap_or("swift");
    let n = sh(h).grid.panes();
    let errors = sh(h).grid.errors();
    let errs = format!(
        "[{}]",
        errors
            .iter()
            .map(|e| format!("{:?}", e))
            .collect::<Vec<_>>()
            .join(",")
    );
    sh(h).bench
        .lock()
        .unwrap()
        .write(client, n, &[("errors", errs)]);
}
