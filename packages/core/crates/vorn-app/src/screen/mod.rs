//! Today's main screen as elements: the top bar over either the composer
//! (no sessions yet) or the terminal cards. Each part is a function of what
//! the app knows, rebuilt every frame; terminals are custom boxes the app
//! paints after layout.

mod cards;
mod composer;
mod top_bar;

use crate::client::Store;
use crate::look::{self, Rgb};
use crate::ui::{div, icon, text, El, Rgba};

pub use cards::{card_index, CARD_ID_BASE};

/// The three views the top bar's pills switch between.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MainView {
    #[default]
    Sessions,
    Tasks,
    Workflows,
}

/// The composer's state: what is typed and for which project.
#[derive(Debug, Clone, Default)]
pub struct Composer {
    pub text: String,
    /// The selected project's name.
    pub project: Option<String>,
    /// Why the last launch failed.
    pub error: Option<String>,
    /// A launch is on its way.
    pub launching: bool,
}

/// What one frame of the screen shows.
pub struct Screen<'a> {
    pub store: &'a Store,
    pub view: MainView,
    pub selected: Option<&'a str>,
    pub composer: &'a Composer,
    /// Shown over the cards, as the new-session dialog is.
    pub composer_open: bool,
    /// The logo's image id, once registered.
    pub logo: Option<u32>,
    pub size: (f32, f32),
}

impl Screen<'_> {
    /// The whole window.
    pub fn build(&self) -> El {
        let body = match self.view {
            MainView::Sessions if self.store.sessions.is_empty() || self.composer_open => {
                composer::empty_state(self)
            }
            MainView::Sessions => cards::grid(self),
            MainView::Tasks => list(
                "Tasks",
                self.store.tasks.iter().map(|t| t.title.as_str()),
                "No tasks yet",
            ),
            MainView::Workflows => list(
                "Workflows",
                self.store.workflows.iter().map(|w| w.name.as_str()),
                "No workflows yet",
            ),
        };
        div()
            .col()
            .w_full()
            .h_full()
            .bg(c(look::SURFACE_BASE))
            .child(top_bar::bar(self))
            .child(body)
    }

    /// The grid's size: the window under the top bar.
    pub fn grid_size(&self) -> (f32, f32) {
        (self.size.0, (self.size.1 - look::TOP_BAR_H).max(0.0))
    }
}

/// The Tasks and Workflows views are not ported yet; until they are, their
/// pill shows the live list by name so the data is visible.
fn list<'a>(label: &str, names: impl Iterator<Item = &'a str>, empty: &str) -> El {
    let mut col = div()
        .col()
        .gap(8.0)
        .px(24.0)
        .py(24.0)
        .grow()
        .role(crate::ui::Role::List, label);
    let mut any = false;
    for name in names {
        any = true;
        col = col.child(text(name, 13.0, c(look::GRAY_300)).line_height(18.0));
    }
    if !any {
        col = col.child(text(empty, 13.0, c(look::GRAY_600)).line_height(18.0));
    }
    col
}

pub(crate) fn c(rgb: Rgb) -> Rgba {
    Rgba::hex(rgb)
}

/// Tailwind's `white/[a]`.
pub(crate) fn white(a: f32) -> Rgba {
    Rgba::hexa(look::WHITE, a)
}

/// A lucide icon at `size` with `stroke`, tinted.
pub(crate) fn ico(name: &str, size: f32, stroke: f32, color: Rgba) -> El {
    icon(look::icon(name, stroke).unwrap_or_default(), size, color)
}

/// The modifier the renderer's shortcuts use: ⌘ on macOS, Ctrl elsewhere.
pub(crate) const MOD_LABEL: &str = if cfg!(target_os = "macos") {
    "\u{2318}"
} else {
    "Ctrl+"
};

/// `s` cut to `max` characters with an ellipsis: text is not clipped, so
/// what would overflow its box is cut instead.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((at, _)) => format!("{}\u{2026}", s[..at].trim_end()),
        None => s.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_counts_characters() {
        assert_eq!(truncate("shell", 10), "shell");
        assert_eq!(truncate("日本語のテキスト", 3), "日本語\u{2026}");
        assert_eq!(truncate("ab cd", 3), "ab\u{2026}");
    }
}
