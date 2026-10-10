//! A C ABI over vornd's grid mode for the native macOS app; `vorn_grid.h`
//! (apps/macos/Sources/CVornGrid/include) is its header.
//!
//! Every entry point treats a null pointer or text that is not UTF-8 as a
//! no-op and catches panics, so nothing unwinds into the app.

mod client;
mod view;

use std::ffi::{c_char, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

use client::{Grid, Waker};
use view::{PaneView, VgRun, VgTheme};
use vorn_term_proto::msg::Presence;

/// The opaque handle the app holds.
pub struct VgClient(Grid);

/// Mirrors `VgView` in `vorn_grid.h`.
#[repr(C)]
pub struct VgView {
    pub rev: u64,
    pub cols: u16,
    pub rows: u16,
    pub cursor_x: u16,
    pub cursor_y: u16,
    pub cursor_visible: u8,
    pub cursor_style: u8,
    pub _pad0: u8,
    pub _pad1: u8,
    pub fg: u32,
    pub bg: u32,
    pub cursor_color: u32,
    pub nruns: u32,
    pub runs: *const VgRun,
    pub text: *const u8,
    pub text_len: u32,
    /// The [`PaneView`] the pointers borrow from; freed by `vg_view_free`.
    pub owner: *mut c_void,
}

/// Runs `f`, answering `fallback` if it panics.
fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

/// # Safety
/// `c` is null or a pointer `vg_connect` returned and `vg_close` has not freed.
unsafe fn grid<'a>(c: *const VgClient) -> Option<&'a Grid> {
    // SAFETY: the caller's contract above.
    unsafe { c.as_ref() }.map(|c| &c.0)
}

/// # Safety
/// `p` is null or a NUL-terminated string.
unsafe fn text_arg<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    // SAFETY: the caller's contract above.
    unsafe { CStr::from_ptr(p) }.to_str().ok()
}

fn c_string(s: String) -> *mut c_char {
    CString::new(s.replace('\0', " ")).map_or(ptr::null_mut(), CString::into_raw)
}

/// # Safety
/// `socket_path` and `build` are null or NUL-terminated; `theme` is null or
/// points at a `VgTheme`.
#[no_mangle]
pub unsafe extern "C" fn vg_connect(
    socket_path: *const c_char,
    build: *const c_char,
    theme: *const VgTheme,
) -> *mut VgClient {
    guard(ptr::null_mut(), || {
        // SAFETY: the caller's contract above.
        let Some(path) = (unsafe { text_arg(socket_path) }) else {
            return ptr::null_mut();
        };
        // SAFETY: as above.
        let build = unsafe { text_arg(build) }.unwrap_or("");
        // SAFETY: as above.
        let theme = unsafe { theme.as_ref() }.copied().unwrap_or_default();
        match Grid::connect(path, build, theme) {
            Ok(g) => Box::into_raw(Box::new(VgClient(g))),
            Err(_) => ptr::null_mut(),
        }
    })
}

/// # Safety
/// `c` is null or a live handle; it is freed and must not be used again.
#[no_mangle]
pub unsafe extern "C" fn vg_close(c: *mut VgClient) {
    if c.is_null() {
        return;
    }
    guard((), || {
        // SAFETY: the caller hands back ownership of a handle from vg_connect.
        drop(unsafe { Box::from_raw(c) });
    });
}

/// # Safety
/// `c` is null or a live handle; `cb` may be called with `ctx` on any thread.
#[no_mangle]
pub unsafe extern "C" fn vg_set_waker(
    c: *mut VgClient,
    cb: Option<unsafe extern "C" fn(*mut c_void)>,
    ctx: *mut c_void,
) {
    guard((), || {
        // SAFETY: the caller's contract above.
        if let Some(g) = unsafe { grid(c) } {
            g.set_waker(cb.map(|cb| Waker { cb, ctx }));
        }
    });
}

/// # Safety
/// `c` is null or a live handle; `session_id` is null or NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn vg_attach(
    c: *mut VgClient,
    session_id: *const c_char,
    cols: u16,
    rows: u16,
) -> u32 {
    guard(0, || {
        // SAFETY: the caller's contract above.
        match unsafe { (grid(c), text_arg(session_id)) } {
            (Some(g), Some(s)) if !s.is_empty() => g.attach(s, cols, rows),
            _ => 0,
        }
    })
}

/// # Safety
/// `c` is null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn vg_detach(c: *mut VgClient, pane: u32) {
    guard((), || {
        // SAFETY: the caller's contract above.
        if let Some(g) = unsafe { grid(c) } {
            g.detach(pane);
        }
    });
}

/// # Safety
/// `c` is null or a live handle; `out` is null or has room for `cap` ids.
#[no_mangle]
pub unsafe extern "C" fn vg_take_dirty(c: *mut VgClient, out: *mut u32, cap: u32) -> u32 {
    guard(0, || {
        // SAFETY: the caller's contract above.
        let Some(g) = (unsafe { grid(c) }) else {
            return 0;
        };
        if out.is_null() || cap == 0 {
            return 0;
        }
        let ids = g.take_dirty(usize::try_from(cap).unwrap_or(usize::MAX));
        // SAFETY: `out` has room for `cap` ids and `ids.len() <= cap`.
        let out = unsafe { std::slice::from_raw_parts_mut(out, ids.len()) };
        out.copy_from_slice(&ids);
        // At most `cap`, a u32.
        ids.len() as u32
    })
}

/// # Safety
/// `c` is null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn vg_pane_state(c: *mut VgClient, pane: u32) -> i32 {
    guard(-1, || {
        // SAFETY: the caller's contract above.
        unsafe { grid(c) }
            .and_then(|g| g.pane_state(pane))
            .map_or(-1, |s| s as i32)
    })
}

/// # Safety
/// `c` is null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn vg_closed(c: *mut VgClient) -> bool {
    // SAFETY: the caller's contract above.
    guard(true, || unsafe { grid(c) }.is_none_or(Grid::closed))
}

/// # Safety
/// `c` is null or a live handle; `out` is null or points at a `VgView`.
#[no_mangle]
pub unsafe extern "C" fn vg_view(c: *mut VgClient, pane: u32, out: *mut VgView) -> bool {
    guard(false, || {
        if out.is_null() {
            return false;
        }
        // SAFETY: the caller's contract above.
        let Some(v) = (unsafe { grid(c) }).and_then(|g| g.view(pane)) else {
            return false;
        };
        let v = Box::new(v);
        let view = VgView {
            rev: v.rev,
            cols: v.cols,
            rows: v.rows,
            cursor_x: v.cursor_x,
            cursor_y: v.cursor_y,
            cursor_visible: u8::from(v.cursor_visible),
            cursor_style: v.cursor_style,
            _pad0: 0,
            _pad1: 0,
            fg: v.fg,
            bg: v.bg,
            cursor_color: v.cursor_color,
            // Both fit u32: a frame is at most 64 MiB.
            nruns: v.runs.len() as u32,
            runs: v.runs.as_ptr(),
            text: v.text.as_ptr(),
            text_len: v.text.len() as u32,
            owner: Box::into_raw(v).cast(),
        };
        // SAFETY: `out` points at a VgView the caller owns.
        unsafe { out.write(view) };
        true
    })
}

/// # Safety
/// `v` is null or a view `vg_view` filled (freeing it twice is a no-op).
#[no_mangle]
pub unsafe extern "C" fn vg_view_free(v: *mut VgView) {
    guard((), || {
        // SAFETY: the caller's contract above.
        let Some(v) = (unsafe { v.as_mut() }) else {
            return;
        };
        if !v.owner.is_null() {
            // SAFETY: `owner` came from Box::into_raw in vg_view.
            drop(unsafe { Box::from_raw(v.owner.cast::<PaneView>()) });
        }
        v.owner = ptr::null_mut();
        v.runs = ptr::null();
        v.text = ptr::null();
        v.nruns = 0;
        v.text_len = 0;
    });
}

/// # Safety
/// `c` is null or a live handle; `code` and `text` are null or NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn vg_key(
    c: *mut VgClient,
    pane: u32,
    code: *const c_char,
    mods: u16,
    text: *const c_char,
) {
    guard((), || {
        // SAFETY: the caller's contract above.
        if let (Some(g), Some(code)) = unsafe { (grid(c), text_arg(code)) } {
            // SAFETY: as above.
            g.key(pane, code, mods, unsafe { text_arg(text) });
        }
    });
}

/// # Safety
/// `c` is null or a live handle; `utf8` is null or NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn vg_text(c: *mut VgClient, pane: u32, utf8: *const c_char) {
    guard((), || {
        // SAFETY: the caller's contract above.
        if let (Some(g), Some(t)) = unsafe { (grid(c), text_arg(utf8)) } {
            g.text(pane, t);
        }
    });
}

/// # Safety
/// `c` is null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn vg_viewport(c: *mut VgClient, pane: u32, cols: u16, rows: u16) {
    guard((), || {
        // SAFETY: the caller's contract above.
        if let Some(g) = unsafe { grid(c) } {
            g.viewport(pane, cols, rows);
        }
    });
}

/// # Safety
/// `c` is null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn vg_take_size(c: *mut VgClient, pane: u32) {
    guard((), || {
        // SAFETY: the caller's contract above.
        if let Some(g) = unsafe { grid(c) } {
            g.take_size(pane);
        }
    });
}

/// # Safety
/// `c` is null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn vg_presence(c: *mut VgClient, pane: u32, state: u8) {
    let state = match state {
        0 => Presence::Active,
        1 => Presence::Watching,
        2 => Presence::Away,
        _ => return,
    };
    guard((), || {
        // SAFETY: the caller's contract above.
        if let Some(g) = unsafe { grid(c) } {
            g.presence(pane, state);
        }
    });
}

/// # Safety
/// `c` is null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn vg_read_text(c: *mut VgClient, pane: u32) -> *mut c_char {
    guard(ptr::null_mut(), || {
        // SAFETY: the caller's contract above.
        unsafe { grid(c) }
            .and_then(|g| g.read_text(pane))
            .map_or(ptr::null_mut(), c_string)
    })
}

/// # Safety
/// `c` is null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn vg_last_error(c: *mut VgClient) -> *mut c_char {
    guard(ptr::null_mut(), || {
        // SAFETY: the caller's contract above.
        unsafe { grid(c) }
            .and_then(Grid::last_error)
            .map_or(ptr::null_mut(), c_string)
    })
}

/// # Safety
/// `s` is null or a string this library returned, not yet freed.
#[no_mangle]
pub unsafe extern "C" fn vg_free_string(s: *mut c_char) {
    if !s.is_null() {
        guard((), || {
            // SAFETY: `s` came from CString::into_raw in this library.
            drop(unsafe { CString::from_raw(s) });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_handles_are_no_ops() {
        // SAFETY: null is allowed everywhere.
        unsafe {
            let c = ptr::null_mut();
            assert!(vg_connect(ptr::null(), ptr::null(), ptr::null()).is_null());
            assert!(
                vg_connect(c"/nonexistent/vorn.sock".as_ptr(), ptr::null(), ptr::null()).is_null()
            );
            assert_eq!(vg_attach(c, c"s".as_ptr(), 80, 24), 0);
            assert_eq!(vg_pane_state(c, 1), -1);
            assert!(vg_closed(c));
            let mut v = std::mem::zeroed::<VgView>();
            assert!(!vg_view(c, 1, &mut v));
            vg_view_free(&mut v);
            vg_view_free(ptr::null_mut());
            vg_key(c, 1, c"KeyA".as_ptr(), 0, ptr::null());
            assert!(vg_read_text(c, 1).is_null());
            assert!(vg_last_error(c).is_null());
            vg_free_string(ptr::null_mut());
            vg_close(c);
        }
    }

    #[test]
    fn view_layout_matches_the_header() {
        assert_eq!(std::mem::size_of::<VgRun>(), 24);
        assert_eq!(std::mem::offset_of!(VgView, fg), 20);
        assert_eq!(std::mem::offset_of!(VgView, runs), 40);
        assert_eq!(std::mem::size_of::<VgView>(), 72);
        assert_eq!(std::mem::size_of::<VgTheme>(), 76);
    }
}
