//! What winit does not reach on macOS: where the traffic lights sit when
//! the app draws its own top bar under them, drops of images that are not files and where a
//! drop landed, and whether the clipboard holds an image.

use std::sync::Mutex;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, Imp, ProtocolObject, Sel};
use objc2::{sel, ClassType};
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSDragOperation, NSDraggingInfo,
    NSFilenamesPboardType, NSImage, NSPasteboard, NSPasteboardType, NSPasteboardTypePNG,
    NSPasteboardTypeTIFF, NSView, NSWindowButton,
};
use objc2_foundation::{NSArray, NSCopying, NSData, NSDictionary, NSString};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use crate::host::{Dropped, Waker};

fn content_view(window: &Window) -> Option<&NSView> {
    let RawWindowHandle::AppKit(h) = window.window_handle().ok()?.as_raw() else {
        return None;
    };
    // SAFETY: winit's handle is its live content view, and winit calls us on
    // the main thread that owns it; the reference lives only in the caller.
    Some(unsafe { h.ns_view.cast::<NSView>().as_ref() })
}

/// Puts the close, minimize and zoom buttons at `(x, y)` logical points
/// from the window's top left, centred in a title area `2y` taller than
/// them.
pub(crate) fn place_traffic_lights(window: &Window, (x, y): (f32, f32)) {
    let Some(win) = content_view(window).and_then(NSView::window) else {
        return;
    };
    let buttons = [
        NSWindowButton::NSWindowCloseButton,
        NSWindowButton::NSWindowMiniaturizeButton,
        NSWindowButton::NSWindowZoomButton,
    ]
    .map(|b| win.standardWindowButton(b));
    let [Some(close), Some(mini), Some(_)] = &buttons else {
        return;
    };
    let space = mini.frame().origin.x - close.frame().origin.x;
    let button_h = close.frame().size.height;
    // SAFETY: AppKit views read and moved on the main thread, as above.
    unsafe {
        let Some(container) = close.superview().and_then(|v| v.superview()) else {
            return;
        };
        let mut bar = container.frame();
        bar.size.height = button_h + 2.0 * f64::from(y);
        bar.origin.y = win.frame().size.height - bar.size.height;
        container.setFrame(bar);
        for (i, b) in buttons.iter().flatten().enumerate() {
            let mut f = b.frame();
            f.origin.x = f64::from(x) + i as f64 * space;
            f.origin.y = (bar.size.height - button_h) / 2.0;
            b.setFrameOrigin(f.origin);
        }
    }
}

type Landed = (Dropped, Option<(f32, f32)>);
type OnDrag<R> = extern "C" fn(&AnyObject, Sel, &ProtocolObject<dyn NSDraggingInfo>) -> R;

/// Drops read by [`take_drops`]' handlers, with where they landed in
/// logical points, until the event loop hands them to the app.
static DROPS: Mutex<Vec<Landed>> = Mutex::new(Vec::new());
static WAKE: Mutex<Option<Waker>> = Mutex::new(None);

/// The drops since the last call.
pub(crate) fn drops() -> Vec<Landed> {
    std::mem::take(&mut *DROPS.lock().unwrap_or_else(|e| e.into_inner()))
}

/// winit's window takes only files and drops them without a place: this
/// lets it take images too (dragged from a web page, with no file), and
/// reads what was dropped, and where, itself.
pub(crate) fn take_drops(window: &Window, waker: Waker) {
    let Some(win) = content_view(window).and_then(NSView::window) else {
        return;
    };
    // SAFETY: the delegate is winit's, alive as long as the window.
    let Some(delegate) = (unsafe { win.delegate() }) else {
        return;
    };
    // SAFETY: every protocol object is an object.
    let class = unsafe { &*Retained::as_ptr(&delegate).cast::<AnyObject>() }.class();
    let (Some(entered), Some(perform)) = (
        class.instance_method(sel!(draggingEntered:)),
        class.instance_method(sel!(performDragOperation:)),
    ) else {
        return;
    };
    *WAKE.lock().unwrap_or_else(|e| e.into_inner()) = Some(waker);
    // SAFETY: both replace winit's methods of the same selectors with
    // functions of the same signatures; the casts only erase the arguments.
    unsafe {
        entered.set_implementation(std::mem::transmute::<OnDrag<NSDragOperation>, Imp>(
            dragging_entered,
        ));
        perform.set_implementation(std::mem::transmute::<OnDrag<Bool>, Imp>(perform_drag));
    }
    // SAFETY: plain AppKit calls on the main thread.
    let types = unsafe {
        let mut t: Vec<Retained<NSPasteboardType>> = NSImage::imageTypes().to_vec_retained();
        t.push(NSFilenamesPboardType.copy());
        t
    };
    win.registerForDraggedTypes(&NSArray::from_vec(types));
}

extern "C" fn dragging_entered(
    _: &AnyObject,
    _: Sel,
    sender: &ProtocolObject<dyn NSDraggingInfo>,
) -> NSDragOperation {
    // SAFETY: AppKit calls this on the main thread with a live sender.
    let pb = unsafe { sender.draggingPasteboard() };
    if has_files(&pb) || has_image_in(&pb) {
        NSDragOperation::Copy
    } else {
        NSDragOperation::None
    }
}

extern "C" fn perform_drag(
    _: &AnyObject,
    _: Sel,
    sender: &ProtocolObject<dyn NSDraggingInfo>,
) -> Bool {
    // SAFETY: AppKit calls this on the main thread with a live sender.
    let (pb, at) = unsafe { (sender.draggingPasteboard(), drop_point(sender)) };
    let what = match files(&pb) {
        Some(f) if !f.is_empty() => Dropped::Files(f),
        _ => match png_in(&pb) {
            Some(bytes) => Dropped::Image {
                name: "image.png".into(),
                bytes,
            },
            None => return Bool::NO,
        },
    };
    DROPS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((what, at));
    if let Some(w) = &*WAKE.lock().unwrap_or_else(|e| e.into_inner()) {
        w.wake();
    }
    Bool::YES
}

/// Where the drop landed in the content view, in logical points from its
/// top left.
///
/// # Safety
/// On the main thread, during the drag.
unsafe fn drop_point(sender: &ProtocolObject<dyn NSDraggingInfo>) -> Option<(f32, f32)> {
    let view = sender.draggingDestinationWindow()?.contentView()?;
    let p = view.convertPoint_fromView(sender.draggingLocation(), None);
    let y = if view.isFlipped() {
        p.y
    } else {
        view.bounds().size.height - p.y
    };
    Some((p.x as f32, y as f32))
}

fn has_files(pb: &NSPasteboard) -> bool {
    // SAFETY: a pasteboard read on the main thread.
    unsafe {
        pb.availableTypeFromArray(&NSArray::from_vec(vec![NSFilenamesPboardType.copy()]))
            .is_some()
    }
}

fn files(pb: &NSPasteboard) -> Option<Vec<std::path::PathBuf>> {
    // SAFETY: the filenames type's property list is an array of strings.
    let list = unsafe { pb.propertyListForType(NSFilenamesPboardType) }?;
    let list: Retained<NSArray<NSString>> = unsafe { Retained::cast(list) };
    Some(list.iter().map(|s| s.to_string().into()).collect())
}

fn image_types() -> Retained<NSArray<NSPasteboardType>> {
    // SAFETY: AppKit's constant strings.
    unsafe {
        NSArray::from_vec(vec![
            NSPasteboardTypePNG.copy(),
            NSPasteboardTypeTIFF.copy(),
        ])
    }
}

fn has_image_in(pb: &NSPasteboard) -> bool {
    // SAFETY: a pasteboard read on the main thread.
    unsafe {
        pb.availableTypeFromArray(&image_types()).is_some() || NSImage::canInitWithPasteboard(pb)
    }
}

/// The pasteboard's image as PNG: as it is when it is one, else drawn from
/// whatever image type it holds.
fn png_in(pb: &NSPasteboard) -> Option<Vec<u8>> {
    // SAFETY: pasteboard and image reads on the main thread.
    unsafe {
        if let Some(d) = pb.dataForType(NSPasteboardTypePNG) {
            return Some(d.bytes().to_vec());
        }
        let tiff: Retained<NSData> = match pb.dataForType(NSPasteboardTypeTIFF) {
            Some(t) => t,
            None => NSImage::initWithPasteboard(NSImage::alloc(), pb)?.TIFFRepresentation()?,
        };
        let rep = NSBitmapImageRep::imageRepWithData(&tiff)?;
        let png = rep
            .representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())?;
        Some(png.bytes().to_vec())
    }
}

/// Whether the system clipboard holds an image.
pub(crate) fn clipboard_has_image() -> bool {
    // SAFETY: the general pasteboard is safe to read from any thread.
    unsafe {
        NSPasteboard::generalPasteboard()
            .availableTypeFromArray(&image_types())
            .is_some()
    }
}

/// Paints the window's own background `rgb` (0 to 1), so a window shown
/// before its first frame is the app's colour rather than the system's.
pub(crate) fn set_background(window: &Window, (r, g, b): (f64, f64, f64)) {
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else {
        return;
    };
    // SAFETY: winit's handle is its live content view, on the main thread that owns it.
    let view: &NSView = unsafe { h.ns_view.cast::<NSView>().as_ref() };
    if let Some(win) = view.window() {
        // SAFETY: an sRGB colour from plain numbers, set on the main thread.
        unsafe {
            let color = objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.0);
            win.setBackgroundColor(Some(&color));
        }
    }
}

/// Puts what the window shows on screen now rather than when the run loop
/// next turns: a window shown before slow setup appears before it.
pub(crate) fn flush() {
    if let Some(cls) = objc2::runtime::AnyClass::get("CATransaction") {
        // SAFETY: `+[CATransaction flush]` takes nothing and returns nothing, on the main thread.
        unsafe {
            let _: () = objc2::msg_send![cls, flush];
        }
    }
}
