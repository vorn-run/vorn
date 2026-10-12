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

pub mod act;
pub mod app;
pub mod canvas;
pub mod client;
pub mod editor;
pub mod file_icons;
pub mod gates;
pub mod git;
pub mod graph;
pub mod grid;
pub mod highlight;
pub mod layout;
pub mod library;
pub mod look;
pub mod markdown;
pub mod paint;
pub mod panes;
pub mod runs;
pub mod screen;
pub mod settings;
pub mod state;
pub mod system;
pub mod tasks;
pub mod templates;
pub mod ui;
pub mod vars;
pub mod view;
pub mod workflows;

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
