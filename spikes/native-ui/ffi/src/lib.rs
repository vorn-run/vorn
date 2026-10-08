//! A small C ABI over the spike's grid client, so the SwiftUI prototype
//! decodes the grid with exactly the Rust code the other two use.
//! `include/vorn_spike.h` is its header.
#![allow(clippy::missing_safety_doc)]

use std::ffi::{c_char, c_void, CStr};
use std::sync::{Arc, Mutex};

use spike_core::bench::{Bench, Config, Step};
use spike_core::{Grid, PaneView, RunC};

pub struct VsHandle {
    grid: Arc<Grid>,
    bench: Mutex<Bench>,
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

unsafe fn str_arg<'a>(p: *const c_char) -> Option<&'a str> {
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
    match Grid::connect(&socket, spike_core::env::sessions(), cols, rows) {
        Ok(grid) => Box::into_raw(Box::new(VsHandle {
            grid,
            bench: Mutex::new(Bench::new(Config::from_env())),
        })),
        Err(_) => std::ptr::null_mut(),
    }
}

/// `cb(ctx)` is called on the reader thread when panes changed.
#[no_mangle]
pub unsafe extern "C" fn vs_set_waker(
    h: *const VsHandle,
    cb: extern "C" fn(*mut c_void),
    ctx: *mut c_void,
) {
    let ctx = ctx as usize;
    (*h).grid.set_waker(move || cb(ctx as *mut c_void));
}

#[no_mangle]
pub unsafe extern "C" fn vs_panes(h: *const VsHandle) -> u32 {
    (*h).grid.panes() as u32
}

/// Writes up to `cap` changed pane indices to `out`; answers how many.
#[no_mangle]
pub unsafe extern "C" fn vs_take_dirty(h: *const VsHandle, out: *mut u32, cap: u32) -> u32 {
    let d = (*h).grid.take_dirty();
    let n = d.len().min(cap as usize);
    for (i, p) in d.iter().take(n).enumerate() {
        *out.add(i) = *p as u32;
    }
    n as u32
}

#[no_mangle]
pub unsafe extern "C" fn vs_all_snapshotted(h: *const VsHandle) -> bool {
    (*h).grid.all_snapshotted()
}

#[no_mangle]
pub unsafe extern "C" fn vs_closed(h: *const VsHandle) -> bool {
    (*h).grid.closed()
}

/// The pane now, or false if it has no screen yet.
#[no_mangle]
pub unsafe extern "C" fn vs_view(h: *const VsHandle, pane: u32, out: *mut VsView) -> bool {
    let Some(v) = (*h).grid.view(pane as usize) else {
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
        (*h).grid.key(pane as usize, code, mods, str_arg(text));
    }
}

#[no_mangle]
pub unsafe extern "C" fn vs_text(h: *const VsHandle, pane: u32, utf8: *const c_char) {
    if let Some(t) = str_arg(utf8) {
        (*h).grid.text(pane as usize, t);
    }
}

#[no_mangle]
pub unsafe extern "C" fn vs_resize(h: *const VsHandle, pane: u32, cols: u16, rows: u16) {
    (*h).grid.resize(pane as usize, cols, rows);
}

/// 0 interactive, 1 latency, 2 frames, 3 start.
#[no_mangle]
pub unsafe extern "C" fn vs_bench_mode(h: *const VsHandle) -> u32 {
    use spike_core::bench::Mode;
    match (*h).bench.lock().unwrap().cfg.mode {
        Mode::Interactive => 0,
        Mode::Latency => 1,
        Mode::Frames => 2,
        Mode::Start => 3,
    }
}

/// 0 idle, 1 type `*ch`, 2 press Enter, 3 done.
#[no_mangle]
pub unsafe extern "C" fn vs_bench_tick(h: *const VsHandle, ch: *mut u32) -> u32 {
    let step = (*h).bench.lock().unwrap().tick(&(*h).grid);
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
    (*h).bench.lock().unwrap().frame(at_ms, work_ms);
}

#[no_mangle]
pub unsafe extern "C" fn vs_bench_first_frame(h: *const VsHandle) {
    (*h).bench.lock().unwrap().first_frame();
}

#[no_mangle]
pub unsafe extern "C" fn vs_bench_latency(h: *const VsHandle, ms: f64) {
    (*h).bench.lock().unwrap().latency(ms);
}

#[no_mangle]
pub unsafe extern "C" fn vs_bench_set_period(h: *const VsHandle, ms: f64) {
    (*h).bench.lock().unwrap().period_ms = ms;
}

#[no_mangle]
pub unsafe extern "C" fn vs_bench_write(h: *const VsHandle) {
    let n = (*h).grid.panes();
    let errors = (*h).grid.errors();
    let errs = format!(
        "[{}]",
        errors
            .iter()
            .map(|e| format!("{:?}", e))
            .collect::<Vec<_>>()
            .join(",")
    );
    (*h).bench
        .lock()
        .unwrap()
        .write("swift", n, &[("errors", errs)]);
}
