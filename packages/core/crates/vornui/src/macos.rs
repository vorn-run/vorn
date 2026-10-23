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

/// Whether the window's edge is being dragged right now: AppKit's live
/// resize, which winit does not report.
pub(crate) fn in_live_resize(window: &Window) -> bool {
    // SAFETY: a property read on the main thread that owns the view.
    content_view(window).is_some_and(|v| unsafe { v.inLiveResize() })
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

/// The files on the system clipboard, as a file manager's copy leaves them.
pub(crate) fn clipboard_files() -> Vec<std::path::PathBuf> {
    // SAFETY: the general pasteboard is safe to read from any thread.
    let pb = unsafe { NSPasteboard::generalPasteboard() };
    if !has_files(&pb) {
        return Vec::new();
    }
    files(&pb).unwrap_or_default()
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
/// Accessibility › Display › Reduce transparency.
pub(crate) fn reduce_transparency() -> bool {
    let Some(cls) = objc2::runtime::AnyClass::get("NSWorkspace") else {
        return false;
    };
    // SAFETY: NSWorkspace's shared instance always exists, and the property
    // is a plain BOOL getter on it.
    unsafe {
        let ws: *mut AnyObject = objc2::msg_send![cls, sharedWorkspace];
        if ws.is_null() {
            return false;
        }
        let on: Bool = objc2::msg_send![ws, accessibilityDisplayShouldReduceTransparency];
        on.as_bool()
    }
}

pub(crate) fn flush() {
    if let Some(cls) = objc2::runtime::AnyClass::get("CATransaction") {
        // SAFETY: `+[CATransaction flush]` takes nothing and returns nothing, on the main thread.
        unsafe {
            let _: () = objc2::msg_send![cls, flush];
        }
    }
}

thread_local! {
    /// The material view behind this thread's window while it is on.
    static MATERIAL: std::cell::RefCell<Option<Retained<NSView>>> = const { std::cell::RefCell::new(None) };
}

/// Puts the system's sidebar material behind the window's content, in the
/// dark or light appearance, or takes it away and paints `solid` again.
/// `false` when AppKit has no material view to give.
pub(crate) fn set_material(window: &Window, dark: Option<bool>, solid: (f64, f64, f64)) -> bool {
    let Some(content) = content_view(window) else {
        return false;
    };
    let (Some(win), Some(frame)) = (content.window(), unsafe { content.superview() }) else {
        return false;
    };
    MATERIAL.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(old) = m.take() {
            // SAFETY: a view this module added, removed on the main thread.
            unsafe { old.removeFromSuperview() };
        }
        let Some(dark) = dark else {
            // SAFETY: plain setters on the live window, on the main thread.
            unsafe {
                win.setOpaque(true);
                let (r, g, b) = solid;
                let color = objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.0);
                win.setBackgroundColor(Some(&color));
            }
            return true;
        };
        let Some(cls) = objc2::runtime::AnyClass::get("NSVisualEffectView") else {
            return false;
        };
        // NSVisualEffectMaterialSidebar, NSVisualEffectBlendingModeBehindWindow,
        // NSVisualEffectStateFollowsWindowActiveState, width and height sizable.
        const SIDEBAR: isize = 7;
        const BEHIND_WINDOW: isize = 0;
        const FOLLOWS_WINDOW: isize = 0;
        const SIZABLE: usize = 2 | 16;
        // NSWindowBelow.
        const BELOW: isize = -1;
        // SAFETY: AppKit's own view class made, configured and inserted on
        // the main thread; the selectors and argument types are AppKit's.
        unsafe {
            let bounds = frame.bounds();
            let view: *mut AnyObject = objc2::msg_send![cls, alloc];
            let view: *mut AnyObject = objc2::msg_send![view, initWithFrame: bounds];
            let Some(view) = Retained::from_raw(view.cast::<NSView>()) else {
                return false;
            };
            let _: () = objc2::msg_send![&*view, setMaterial: SIDEBAR];
            let _: () = objc2::msg_send![&*view, setBlendingMode: BEHIND_WINDOW];
            let _: () = objc2::msg_send![&*view, setState: FOLLOWS_WINDOW];
            let _: () = objc2::msg_send![&*view, setAutoresizingMask: SIZABLE];
            if let Some(appearance) = objc2::runtime::AnyClass::get("NSAppearance") {
                let name = NSString::from_str(if dark {
                    "NSAppearanceNameDarkAqua"
                } else {
                    "NSAppearanceNameAqua"
                });
                let a: *mut AnyObject = objc2::msg_send![appearance, appearanceNamed: &*name];
                let _: () = objc2::msg_send![&*view, setAppearance: a];
            }
            let _: () = objc2::msg_send![
                &*frame,
                addSubview: &*view,
                positioned: BELOW,
                relativeTo: content
            ];
            win.setOpaque(false);
            win.setBackgroundColor(Some(&objc2_app_kit::NSColor::clearColor()));
            *m = Some(view);
        }
        true
    })
}

thread_local! {
    /// The glass views over this thread's window, one a box, reused frame to frame.
    static GLASS: std::cell::RefCell<Vec<Retained<NSView>>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Template images made from icon SVGs, by SVG text.
    static ICONS: std::cell::RefCell<std::collections::HashMap<std::rc::Rc<str>, Retained<AnyObject>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// How much black lies in the glass in the dark appearance.
const GLASS_DARK_TINT: f64 = 0.35;

extern "C" fn pass_through(_: &AnyObject, _: Sel, _: objc2_foundation::NSPoint) -> *mut AnyObject {
    std::ptr::null_mut()
}

/// NSGlassEffectView that lets every press through to the app under it,
/// which draws the button's states and answers it.
fn glass_class() -> Option<&'static objc2::runtime::AnyClass> {
    static CLASS: std::sync::OnceLock<Option<&'static objc2::runtime::AnyClass>> =
        std::sync::OnceLock::new();
    *CLASS.get_or_init(|| {
        let sup = objc2::runtime::AnyClass::get("NSGlassEffectView")?;
        if let Some(c) = objc2::runtime::AnyClass::get("VornGlassView") {
            return Some(c);
        }
        let mut b = objc2::runtime::ClassBuilder::new("VornGlassView", sup)?;
        // SAFETY: hitTest: takes an NSPoint and answers a view or nil.
        unsafe {
            b.add_method(sel!(hitTest:), pass_through as extern "C" fn(_, _, _) -> _);
        }
        Some(b.register())
    })
}

/// A template image of `svg`, so the glass tints it as its own label.
fn icon_image(svg: &std::rc::Rc<str>) -> Option<Retained<AnyObject>> {
    ICONS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(img) = cache.get(svg) {
            return Some(img.clone());
        }
        let cls = objc2::runtime::AnyClass::get("NSImage")?;
        let data = NSData::with_bytes(svg.as_bytes());
        // SAFETY: NSImage made from data on the main thread.
        let img: Option<Retained<AnyObject>> = unsafe {
            let img: *mut AnyObject = objc2::msg_send![cls, alloc];
            let img: *mut AnyObject = objc2::msg_send![img, initWithData: &*data];
            Retained::from_raw(img)
        };
        let img = img?;
        // SAFETY: a plain setter on the image just made.
        unsafe {
            let _: () = objc2::msg_send![&*img, setTemplate: true];
        }
        cache.insert(svg.clone(), img.clone());
        Some(img)
    })
}

/// Puts the system's glass (NSGlassEffectView, macOS 26) over each box,
/// given in device pixels with its corner radius, with the icons inside it
/// drawn by the system; none on older systems.
pub(crate) fn sync_glass(
    window: &Window,
    boxes: &[(crate::scene::Rect, f32)],
    icons: &[(crate::scene::Rect, std::rc::Rc<str>, crate::scene::Rgba)],
    labels: &[(
        crate::scene::Rect,
        std::rc::Rc<str>,
        f32,
        crate::scene::Rgba,
    )],
    scale: f64,
) {
    let Some(cls) = glass_class() else {
        return;
    };
    let Some(content) = content_view(window) else {
        return;
    };
    // SAFETY: the content view is winit's, live and used on the main thread.
    let Some(frame) = (unsafe { content.superview() }) else {
        return;
    };
    let height = content.frame().size.height;
    let s = scale.max(1.0);
    let pt = |r: &crate::scene::Rect| {
        (
            f64::from(r.x) / s,
            f64::from(r.y) / s,
            f64::from(r.w) / s,
            f64::from(r.h) / s,
        )
    };
    GLASS.with(|g| {
        let mut g = g.borrow_mut();
        while g.len() > boxes.len() {
            if let Some(v) = g.pop() {
                // SAFETY: a view this module added, removed on the main thread.
                unsafe { v.removeFromSuperview() };
            }
        }
        // NSWindowAbove: over the app's own drawing, so the glass bends it.
        const ABOVE: isize = 1;
        for (i, (r, radius)) in boxes.iter().enumerate() {
            let (x, y, w, h) = pt(r);
            let rect = objc2_foundation::NSRect::new(
                objc2_foundation::NSPoint::new(x, height - y - h),
                objc2_foundation::NSSize::new(w, h),
            );
            // SAFETY: AppKit's own view classes made and placed on the main
            // thread; the selectors and argument types are AppKit's.
            unsafe {
                if i == g.len() {
                    let view: *mut AnyObject = objc2::msg_send![cls, alloc];
                    let view: *mut AnyObject = objc2::msg_send![view, initWithFrame: rect];
                    let Some(view) = Retained::from_raw(view.cast::<NSView>()) else {
                        return;
                    };
                    let inner: *mut AnyObject = objc2::msg_send![objc2::class!(NSView), alloc];
                    let bounds = objc2_foundation::NSRect::new(
                        objc2_foundation::NSPoint::new(0.0, 0.0),
                        rect.size,
                    );
                    let inner: *mut AnyObject = objc2::msg_send![inner, initWithFrame: bounds];
                    let _: () = objc2::msg_send![&*view, setContentView: inner];
                    let _: () = objc2::msg_send![inner, release];
                    let _: () = objc2::msg_send![
                        &*frame,
                        addSubview: &*view,
                        positioned: ABOVE,
                        relativeTo: content
                    ];
                    g.push(view);
                }
                let v = &g[i];
                v.setFrame(rect);
                let _: () = objc2::msg_send![&**v, setCornerRadius: f64::from(*radius) / s];
                // A dark wash in the dark appearance, so the glass sits with the app.
                let tint = crate::palette::dark().then(|| {
                    objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(
                        0.0,
                        0.0,
                        0.0,
                        GLASS_DARK_TINT,
                    )
                });
                let _: () = objc2::msg_send![&**v, setTintColor: tint.as_deref()];
                let inner: *mut AnyObject = objc2::msg_send![&**v, contentView];
                let subs: *mut AnyObject = objc2::msg_send![inner, subviews];
                let old: *mut AnyObject = objc2::msg_send![subs, copy];
                let _: () =
                    objc2::msg_send![old, makeObjectsPerformSelector: sel!(removeFromSuperview)];
                let _: () = objc2::msg_send![old, release];
                for (ir, svg, color) in icons {
                    let (ix, iy, iw, ih) = pt(ir);
                    if ix < x || iy < y || ix + iw > x + w + 0.5 || iy + ih > y + h + 0.5 {
                        continue;
                    }
                    let Some(img) = icon_image(svg) else { continue };
                    let iv: *mut AnyObject =
                        objc2::msg_send![objc2::class!(NSImageView), imageViewWithImage: &*img];
                    let at = objc2_foundation::NSRect::new(
                        objc2_foundation::NSPoint::new(ix - x, h - (iy - y) - ih),
                        objc2_foundation::NSSize::new(iw, ih),
                    );
                    let _: () = objc2::msg_send![iv, setFrame: at];
                    let [cr, cg, cb, ca] = color.0;
                    let tint = objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(
                        f64::from(cr),
                        f64::from(cg),
                        f64::from(cb),
                        f64::from(ca),
                    );
                    let _: () = objc2::msg_send![iv, setContentTintColor: &*tint];
                    let _: () = objc2::msg_send![inner, addSubview: iv];
                }
                for (lr, text, size, color) in labels {
                    let (lx, ly, lw, lh) = pt(lr);
                    if lx < x || ly < y || lx + lw > x + w + 0.5 || ly + lh > y + h + 0.5 {
                        continue;
                    }
                    let t = NSString::from_str(text);
                    let tf: *mut AnyObject =
                        objc2::msg_send![objc2::class!(NSTextField), labelWithString: &*t];
                    let font: *mut AnyObject = objc2::msg_send![
                        objc2::class!(NSFont),
                        systemFontOfSize: f64::from(*size) / s,
                        weight: 0.23f64
                    ];
                    let _: () = objc2::msg_send![tf, setFont: font];
                    let [cr, cg, cb, ca] = color.0;
                    let ink = objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(
                        f64::from(cr),
                        f64::from(cg),
                        f64::from(cb),
                        f64::from(ca),
                    );
                    let _: () = objc2::msg_send![tf, setTextColor: &*ink];
                    let fit: objc2_foundation::NSSize = objc2::msg_send![tf, fittingSize];
                    let at = objc2_foundation::NSRect::new(
                        objc2_foundation::NSPoint::new(
                            lx - x,
                            h - (ly - y) - lh + (lh - fit.height) / 2.0,
                        ),
                        objc2_foundation::NSSize::new(lw.max(fit.width), fit.height),
                    );
                    let _: () = objc2::msg_send![tf, setFrame: at];
                    let _: () = objc2::msg_send![inner, addSubview: tf];
                }
            }
        }
    });
}
