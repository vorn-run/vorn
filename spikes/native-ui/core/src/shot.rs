//! A screenshot of this process's own window, taken in-process: macOS lets an
//! app capture its own windows without the Screen Recording permission, so
//! every prototype calls this for the look test instead of an outside tool.
//! The window is the largest normal-layer window this process has on screen.

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{c_void, CString};

    type CFRef = *const c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGRect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFArrayGetCount(a: CFRef) -> isize;
        fn CFArrayGetValueAtIndex(a: CFRef, i: isize) -> CFRef;
        fn CFDictionaryGetValue(d: CFRef, k: CFRef) -> CFRef;
        fn CFNumberGetValue(n: CFRef, ty: isize, out: *mut c_void) -> bool;
        fn CFRelease(r: CFRef);
        fn CFURLCreateFromFileSystemRepresentation(
            alloc: CFRef,
            buf: *const u8,
            len: isize,
            dir: bool,
        ) -> CFRef;
        fn CFStringCreateWithCString(alloc: CFRef, s: *const i8, enc: u32) -> CFRef;
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, rel: u32) -> CFRef;
        fn CGRectMakeWithDictionaryRepresentation(d: CFRef, r: *mut CGRect) -> bool;
        fn CGImageGetWidth(i: CFRef) -> usize;
        static kCGWindowOwnerPID: CFRef;
        static kCGWindowNumber: CFRef;
        static kCGWindowLayer: CFRef;
        static kCGWindowBounds: CFRef;
    }

    #[link(name = "ImageIO", kind = "framework")]
    extern "C" {
        fn CGImageDestinationCreateWithURL(url: CFRef, ty: CFRef, n: usize, o: CFRef) -> CFRef;
        fn CGImageDestinationAddImage(d: CFRef, img: CFRef, props: CFRef);
        fn CGImageDestinationFinalize(d: CFRef) -> bool;
    }

    extern "C" {
        fn dlopen(path: *const i8, mode: i32) -> *mut c_void;
        fn dlsym(h: *mut c_void, name: *const i8) -> *mut c_void;
    }

    const K_CF_NUMBER_SINT64: isize = 4;
    const ON_SCREEN_ONLY: u32 = 1;
    const INCLUDING_WINDOW: u32 = 1 << 3;
    const BOUNDS_IGNORE_FRAMING: u32 = 1;
    const BEST_RESOLUTION: u32 = 1 << 3;

    unsafe fn num(d: CFRef, k: CFRef) -> Option<i64> {
        let v = CFDictionaryGetValue(d, k);
        let mut out = 0i64;
        (!v.is_null() && CFNumberGetValue(v, K_CF_NUMBER_SINT64, (&mut out as *mut i64).cast()))
            .then_some(out)
    }

    /// The id of this process's largest normal window.
    pub(super) unsafe fn my_window(on_screen_only: bool) -> Option<u32> {
        let pid = i64::from(std::process::id());
        let list = CGWindowListCopyWindowInfo(if on_screen_only { ON_SCREEN_ONLY } else { 0 }, 0);
        if list.is_null() {
            return None;
        }
        let mut best: Option<(f64, u32)> = None;
        for i in 0..CFArrayGetCount(list) {
            let d = CFArrayGetValueAtIndex(list, i);
            if num(d, kCGWindowOwnerPID) != Some(pid) {
                continue;
            }
            let layer = num(d, kCGWindowLayer).unwrap_or(-1);
            let mut r = CGRect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
            let b = CFDictionaryGetValue(d, kCGWindowBounds);
            if b.is_null() || !CGRectMakeWithDictionaryRepresentation(b, &mut r) {
                continue;
            }
            // Normal windows are layer 0; a floating bench window is 3, a
            // pop-up one 101.
            let area = r.w * r.h;
            if (0..1000).contains(&layer) && best.map_or(true, |(a, _)| area > a) {
                best = Some((area, num(d, kCGWindowNumber).unwrap_or(0) as u32));
            }
        }
        CFRelease(list);
        best.map(|(_, id)| id)
    }

    pub fn capture(path: &str) -> Result<(), String> {
        // SAFETY: CoreFoundation calls on values this function owns or
        // borrows from a dictionary it keeps alive until it releases it.
        unsafe {
            // A window on another Space (the user in a full-screen app)
            // is still captured, but its content may be stale: warn.
            let id = match my_window(true) {
                Some(id) => id,
                None => {
                    eprintln!("shot: the window is not on screen (another Space?)");
                    my_window(false).ok_or("no window of this process")?
                }
            };
            // The capture call is gone from the headers of newer SDKs but
            // still in the library; look it up at run time.
            let cg = dlopen(
                c"/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics".as_ptr(),
                1,
            );
            let f = dlsym(cg, c"CGWindowListCreateImage".as_ptr());
            if f.is_null() {
                return Err("no window capture call".into());
            }
            let create: extern "C" fn(CGRect, u32, u32, u32) -> CFRef = std::mem::transmute(f);
            let null = CGRect { x: f64::INFINITY, y: f64::INFINITY, w: 0.0, h: 0.0 };
            let img = create(null, INCLUDING_WINDOW, id, BOUNDS_IGNORE_FRAMING | BEST_RESOLUTION);
            if img.is_null() || CGImageGetWidth(img) == 0 {
                return Err("the capture came back empty".into());
            }
            let url = CFURLCreateFromFileSystemRepresentation(
                std::ptr::null(),
                path.as_ptr(),
                path.len() as isize,
                false,
            );
            let ty = CString::new("public.png").unwrap();
            let uti = CFStringCreateWithCString(std::ptr::null(), ty.as_ptr(), 0x0800_0100);
            let dest = CGImageDestinationCreateWithURL(url, uti, 1, std::ptr::null());
            if dest.is_null() {
                return Err(format!("cannot write {path}"));
            }
            CGImageDestinationAddImage(dest, img, std::ptr::null());
            let ok = CGImageDestinationFinalize(dest);
            for r in [dest, uti, url, img] {
                CFRelease(r);
            }
            ok.then_some(()).ok_or_else(|| format!("cannot write {path}"))
        }
    }
}

/// Writes this process's window to `path` as a PNG at the display's scale.
#[cfg(target_os = "macos")]
pub fn capture(path: &str) -> Result<(), String> {
    mac::capture(path)
}

/// Whether this process has a window on screen (not on another Space):
/// a hidden window may not draw, so its numbers are not comparable.
#[cfg(target_os = "macos")]
pub fn on_screen() -> bool {
    // SAFETY: see `mac::capture`.
    unsafe { mac::my_window(true).is_some() }
}

#[cfg(not(target_os = "macos"))]
pub fn on_screen() -> bool {
    true
}

#[cfg(not(target_os = "macos"))]
pub fn capture(_path: &str) -> Result<(), String> {
    Err("window capture is macOS-only in this spike".into())
}
