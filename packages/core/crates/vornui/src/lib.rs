//! vornui: Vorn's thin UI layer for Windows and Linux. It owns only the
//! glue: winit gives windows and input, wgpu (or the CPU renderer, where
//! the only adapter rasterizes in software) draws, cosmic-text and swash
//! shape and rasterize text, taffy lays out, resvg draws icons and AccessKit
//! speaks to screen readers.
//!
//! An app builds an [`El`] tree each frame from its own state, hands it to
//! [`Ui::layout`], and reads back [`Event`]s with [`Ui::take_events`]. The
//! [`Ui`] keeps what the tree does not: hover, focus, open menus, scroll
//! offsets, split ratios and text being edited. The [`widgets`] build the
//! app's controls with its look (`src/renderer/theme.css`) and behaviour.

mod a11y;
pub mod atlas;
mod cpu;
pub mod edit;
pub mod element;
pub mod gpu;
pub mod input;
mod interact;
pub mod pace;
mod render;
pub mod scene;
pub mod text;
pub mod theme;
mod ui;
pub mod widgets;
pub mod window;

pub use {accesskit, wgpu, winit};

pub use edit::{Clipboard, MemoryClipboard, TextEdit};
pub use element::{
    custom, div, icon, image, text, Action, Content, Cursor, El, Id, Overlay, Place, Sense,
};
pub use gpu::Gpu;
pub use input::Input;
pub use interact::{Event, BLINK, MENU_IN, MIN_SPLIT, TOOLTIP_DELAY};
pub use pace::Pacer;
pub use render::{Offscreen, RenderMode, OFFSCREEN_FORMAT};
pub use scene::{Rect, Rgba, Scene};
pub use text::{TextStyle, TextSystem};
pub use ui::{decode_png, rasterize_svg, write_png, CustomA11y, Laid, Ui, UiConfig, ROOT_NODE};
pub use window::{run, App, UserEvent, Waker};
