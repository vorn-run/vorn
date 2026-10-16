//! Vorn's composed components: the pieces of today's screens that know
//! what a session, an agent or an approval is, built only from vornui's
//! primitives and tokens. The app composes screens from these; vornui
//! stays free of anything Vorn-specific.
//!
//! Components take plain data and the [`Id`](vornui::Id)s their controls
//! report clicks on; what a click does is the caller's to decide.

pub mod agent;
pub mod board;
pub mod card;
pub mod dock;
pub mod glyph;
pub mod graph;
pub mod sidebar;
pub mod status;
pub mod template;
pub mod tree;

pub use agent::{agent_icon, status_icon, AgentMarks};
pub use status::{status_dot, Status};

use vornui::{icon, El, Rgba};

/// Lucide icon `name` at `size` with lines `stroke` wide, in `color`.
pub fn lucide(name: &str, size: f32, stroke: f32, color: Rgba) -> El {
    icon(
        vornui::icons::lucide(name, stroke).unwrap_or_default(),
        size,
        color,
    )
}
