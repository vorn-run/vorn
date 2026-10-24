//! The UI layer, behind one module: every other module reaches vornui
//! through these names, so moving to another build of it is this file and
//! one line of `Cargo.toml`.

pub use vornui::accesskit::{Role, TreeUpdate};
pub use vornui::input::{from_ime, mods};
pub use vornui::text::{BOLD, ITALIC};
#[cfg(not(target_arch = "wasm32"))]
pub use vornui::tty;
pub use vornui::widgets::{
    area, code, field, item_id, split, text_input, textarea, tooltip, tooltip_at,
};
pub use vornui::window::{run as run_window, run_with, App as WindowApp, Waker, WindowOptions};
pub use vornui::winit::window::Window as NativeWindow;
pub use vornui::{backend, cells, icons, menu, palette, theme, widgets};
pub use vornui::{
    custom, decode_png, div, icon, image, rasterize_svg_rgba, shape, text, Action, Clipboard,
    Cursor, Dropped, El, Event, Id, Input, Laid, MemoryClipboard, MonoFamily, Offscreen, Place,
    Rect, RenderMode, Rgba, Sense, Ui, UiConfig, WheelPhase, WindowRequest, WHEEL_LINE,
};
pub use vornui::{use_bundled_fonts, TextStyle};
pub use vornui::{TextEdit, MOVE_SLOP};

/// The terminal cell at `scale`, in logical pixels, without a GPU.
pub fn cell_logical(cfg: &UiConfig) -> (f32, f32) {
    vornui::TextSystem::new(cfg.scale, cfg.ui_font, cfg.mono_font, cfg.term_size).cell_logical()
}
