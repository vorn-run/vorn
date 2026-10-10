//! The UI layer, behind one module: every other module reaches vornui
//! through these names, so moving to another build of it is this file and
//! one line of `Cargo.toml`.

pub use vornui::accesskit::{Role, TreeUpdate};
pub use vornui::input::{from_ime, mods};
pub use vornui::text::{BOLD, ITALIC};
pub use vornui::window::{run as run_window, App as WindowApp, Waker};
pub use vornui::{
    custom, decode_png, div, icon, image, text, El, Input, Laid, Offscreen, Rect, RenderMode, Rgba,
    Ui, UiConfig,
};

/// The terminal cell at `scale`, in logical pixels, without a GPU.
pub fn cell_logical(cfg: &UiConfig) -> (f32, f32) {
    vornui::TextSystem::new(cfg.scale, cfg.ui_font, cfg.mono_font, cfg.term_size).cell_logical()
}
