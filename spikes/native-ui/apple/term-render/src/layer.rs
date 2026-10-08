//! The renderer's own CAMetalLayer, added as a sublayer of the host view's
//! layer (an NSView on macOS, a UIView on iOS). Only the main thread
//! touches the layer's geometry; the render thread only draws into it.

use std::ffi::c_void;

use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{class, msg_send};

#[repr(C)]
#[derive(Clone, Copy)]
struct Rect(f64, f64, f64, f64);

// SAFETY: matches CGRect's layout and encoding on 64-bit Apple platforms.
unsafe impl Encode for Rect {
    const ENCODING: Encoding = Encoding::Struct(
        "CGRect",
        &[
            Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]),
            Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]),
        ],
    );
}
unsafe impl RefEncode for Rect {
    const ENCODING_REF: Encoding = Encoding::Pointer(&<Self as Encode>::ENCODING);
}

#[repr(transparent)]
struct ColorSpace(*const c_void);
// SAFETY: a CGColorSpaceRef.
unsafe impl Encode for ColorSpace {
    const ENCODING: Encoding = Encoding::Pointer(&Encoding::Struct("CGColorSpace", &[]));
}

pub struct Layer(Retained<AnyObject>);

// SAFETY: the render thread only reads the pointer to hand it to wgpu,
// which is how Metal layers are meant to be drawn from another thread.
unsafe impl Send for Layer {}
unsafe impl Sync for Layer {}

fn nsstring(s: &std::ffi::CStr) -> *mut AnyObject {
    // SAFETY: an autoreleased NSString from a C string, on the main thread.
    unsafe { msg_send![class!(NSString), stringWithUTF8String: s.as_ptr()] }
}

fn no_actions<R>(f: impl FnOnce() -> R) -> R {
    let ca = AnyClass::get(c"CATransaction").expect("QuartzCore");
    // SAFETY: CATransaction class methods, on the main thread.
    unsafe {
        let _: () = msg_send![ca, begin];
        let _: () = msg_send![ca, setDisableActions: true];
        let r = f();
        let _: () = msg_send![ca, commit];
        r
    }
}

impl Layer {
    /// Creates the layer and attaches it to `view` (an NSView or UIView).
    ///
    /// # Safety
    /// `view` is a live view, and this runs on the main thread.
    pub unsafe fn attach(view: *mut c_void, scale: f64) -> Layer {
        let view = view.cast::<AnyObject>();
        #[cfg(target_os = "macos")]
        let _: () = msg_send![view, setWantsLayer: true];
        let host: *mut AnyObject = msg_send![view, layer];
        let layer: Retained<AnyObject> = msg_send![class!(CAMetalLayer), new];
        let _: () = msg_send![&*layer, setOpaque: true];
        // While a resize waits for the next frame the old one stays pinned
        // to the top-left corner instead of stretching.
        #[cfg(target_os = "macos")]
        let gravity = nsstring(c"topLeft");
        #[cfg(not(target_os = "macos"))]
        let gravity = nsstring(c"bottomLeft");
        let _: () = msg_send![&*layer, setContentsGravity: gravity];
        // Colours are sRGB, as option A's CGColors are: let the system match
        // them to the display instead of showing them raw.
        let _: () = msg_send![&*layer, setColorspace: ColorSpace(crate::glyphs::srgb())];
        let _: () = msg_send![&*layer, setContentsScale: scale];
        no_actions(|| {
            let _: () = msg_send![host, addSublayer: &*layer];
        });
        Layer(layer)
    }

    pub fn ptr(&self) -> *mut c_void {
        Retained::as_ptr(&self.0) as *mut c_void
    }

    /// Main thread: the layer covers the host's bounds (`w`x`h` points).
    pub fn set_frame(&self, w: f64, h: f64, scale: f64) {
        no_actions(|| {
            // SAFETY: a live layer, on the main thread.
            unsafe {
                let _: () = msg_send![&*self.0, setFrame: Rect(0.0, 0.0, w, h)];
                let _: () = msg_send![&*self.0, setContentsScale: scale];
            }
        });
    }

    /// Main thread: takes the layer off its host.
    pub fn detach(&self) {
        // SAFETY: a live layer, on the main thread.
        no_actions(|| unsafe {
            let _: () = msg_send![&*self.0, removeFromSuperlayer];
        });
    }
}
