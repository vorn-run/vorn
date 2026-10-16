//! Vorn on a phone. Two hosts share these screens: one draws the whole app
//! into a Metal layer, the other reads each screen as a semantic tree for
//! the platform's own views and draws only the terminal.

pub mod ffi;
pub mod model;
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
