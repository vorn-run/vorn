//! What winit does not reach on macOS: where the traffic lights sit when
//! the app draws its own top bar under them (Electron's
//! `trafficLightPosition`).

use objc2_app_kit::{NSView, NSWindowButton};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

/// Puts the close, minimize and zoom buttons at `(x, y)` logical points
/// from the window's top left, centred in a title area `2y` taller than
/// them, as Electron does.
pub(crate) fn place_traffic_lights(window: &Window, (x, y): (f32, f32)) {
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else {
        return;
    };
    // SAFETY: winit's handle is its live content view, and winit calls us on
    // the main thread that owns it; the reference lives only in this call.
    let view: &NSView = unsafe { h.ns_view.cast::<NSView>().as_ref() };
    let Some(win) = view.window() else {
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
