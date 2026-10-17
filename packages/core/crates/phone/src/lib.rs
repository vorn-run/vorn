//! Vorn on a phone: screens a host either draws whole or reads as native views around a drawn terminal.

pub mod ffi;
pub mod markdown;
pub mod model;
pub mod pages;
pub mod phone;
pub mod screens;

pub use model::{Link, Model, Target};
pub use phone::{Insets, Phone, Tab};

use vornui::theme::font::{WEB_MONO, WEB_UI};
use vornui::UiConfig;

/// The terminal's text size, logical pixels.
pub const TERM_SIZE: f32 = 12.0;

/// The faces the phone draws with, bundled: a phone has none of the desktop's.
pub fn config(scale: f32) -> UiConfig {
    vornui::use_bundled_fonts(vec![
        include_bytes!("../../vorn-web/static/fonts/Inter-Regular.ttf").to_vec(),
        include_bytes!("../../vorn-web/static/fonts/Inter-Medium.ttf").to_vec(),
        include_bytes!("../../vorn-web/static/fonts/JetBrainsMono-Regular.ttf").to_vec(),
        include_bytes!("../../vorn-web/static/fonts/JetBrainsMono-Bold.ttf").to_vec(),
    ]);
    UiConfig {
        scale,
        ui_font: WEB_UI,
        mono_font: WEB_MONO,
        term_size: TERM_SIZE,
    }
}
