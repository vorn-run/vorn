//! Vorn's desktop app for Windows and Linux (it runs on macOS too, for
//! development): today's main screen on vornui, over a running vornd.
//!
//! - [`client`] speaks JSON-RPC to vornd and keeps its models current.
//! - [`grid`] holds the terminals' screens over the grid endpoint.
//! - [`screen`] turns that state into elements; [`paint`] draws terminals.
//! - [`app`] ties them together and routes keys; `main` gives it a window
//!   or an offscreen target.
//!
//! The UI layer is reached only through [`ui`], so it can be repointed.

pub mod app;
pub mod client;
pub mod grid;
pub mod layout;
pub mod look;
pub mod paint;
pub mod screen;
pub mod ui;
pub mod view;

pub use app::App;

/// The UI config at `scale`: the app's fonts and terminal size.
pub fn ui_config(scale: f32) -> ui::UiConfig {
    ui::UiConfig {
        scale,
        ui_font: look::UI_FONT,
        mono_font: look::MONO_FONT,
        term_size: look::TERM_FONT,
    }
}
