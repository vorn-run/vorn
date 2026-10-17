//! The screen a panic leaves behind: what went wrong, with
//! a way to start the app over and a way to copy the details.

use std::panic::{self, AssertUnwindSafe};
use std::sync::Mutex;

use crate::look;
use crate::ui::{div, text, Action, El, Event, Id, Rgba, Role, Ui};

/// Reload: drop the app's state and connect to vornd again.
pub const RELOAD: Id = Id(0x6372_6173_6872_6c64);
/// Copy error: the message and where it came from, to the clipboard.
pub const COPY: Id = Id(0x6372_6173_6863_7079);
const DETAILS: Id = Id(0x6372_6173_6864_7473);

/// The last panic's location and backtrace, kept by the hook [`install`] sets.
static LAST: Mutex<Option<String>> = Mutex::new(None);

/// Keeps each panic's location and backtrace for the crash screen, then
/// reports it as the default hook does.
pub fn install() {
    let default = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let at = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();
        let trace = std::backtrace::Backtrace::force_capture();
        if let Ok(mut last) = LAST.lock() {
            *last = Some(format!("at {at}\n\n{trace}"));
        }
        default(info);
    }));
}

/// What a caught panic reads as: its message, then where it happened.
fn describe(payload: &(dyn std::any::Any + Send)) -> String {
    let message = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "Unknown error".to_owned());
    let detail = LAST.lock().ok().and_then(|mut l| l.take());
    match detail {
        Some(d) => format!("{message}\n\n{d}"),
        None => message,
    }
}

/// Runs `f`, turning a panic into the error it shows.
pub fn guard<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    panic::catch_unwind(AssertUnwindSafe(f)).map_err(|p| describe(p.as_ref()))
}

/// What a press on the crash screen asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashAct {
    Reload,
}

/// Handles a UI event on the crash screen: copying happens here, reloading
/// is the caller's.
pub fn event(ui: &mut Ui, event: &Event, error: &str) -> Option<CrashAct> {
    match event {
        Event::Click(id) if *id == RELOAD => Some(CrashAct::Reload),
        Event::Click(id) if *id == COPY => {
            ui.clipboard().set(error);
            None
        }
        _ => None,
    }
}

/// The whole window: the heading, the explanation, the details and the two
/// buttons, in a 560-px column centred on the overlay surface.
pub fn view(ui: &Ui, error: &str) -> El {
    let button = |id: Id, label: &str, bg: Rgba, border: Rgba, fg: Rgba| {
        let hot = ui.hover(id);
        div()
            .id(id)
            .on(Action::Click)
            .px(16.0)
            .py(6.0)
            .rounded(6.0)
            .bg(bg.mix(Rgba::white(0.12), hot * 0.3))
            .border(1.0, border)
            .role(Role::Button, label)
            .child(text(label, 13.0, fg).line_height(18.0).silent())
    };
    let column = div()
        .col()
        .w_full()
        .max_w(560.0)
        .child(
            text("Something went wrong", 18.0, Rgba::hex(look::RED_400))
                .weight(600)
                .line_height(24.0)
                .mb(8.0)
                .role(Role::Heading, "Something went wrong"),
        )
        .child(
            text(
                "The app encountered an unexpected error. You can try reloading or check the details below.",
                13.0,
                Rgba::hex(look::GRAY_400),
            )
            .line_height(18.0)
            .wrapping()
            .mb(16.0),
        )
        .child(
            div()
                .id(DETAILS)
                .max_h(300.0)
                .scroll_y()
                .p(16.0)
                .mb(16.0)
                .rounded(8.0)
                .bg(Rgba::hex(look::SURFACE_SUNKEN))
                .border(1.0, Rgba::white(0.06))
                .child(
                    text(error, 12.0, Rgba::hex(look::GRAY_300))
                        .mono()
                        .line_height(18.0)
                        .wrapping(),
                ),
        )
        .child(
            div()
                .row()
                .gap(8.0)
                .child(button(
                    RELOAD,
                    "Reload",
                    Rgba::white(0.08),
                    Rgba::white(0.1),
                    Rgba::hex(0xe5e5e5),
                ))
                .child(button(
                    COPY,
                    "Copy error",
                    Rgba::TRANSPARENT,
                    Rgba::white(0.06),
                    Rgba::hex(look::GRAY_400),
                )),
        );
    div()
        .col()
        .w_full()
        .h_full()
        .center()
        .p(32.0)
        .bg(Rgba::hex(look::SURFACE_OVERLAY))
        .child(column)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_caught_panic_reads_as_its_message() {
        let out: Result<(), String> = guard(|| panic!("boom {}", 7));
        let error = out.expect_err("the panic is caught");
        assert!(error.starts_with("boom 7"), "{error}");
        assert_eq!(guard(|| 3), Ok(3));
    }
}
