//! Today's main screen as elements: the top bar over either the composer
//! (no sessions yet) or the terminal cards. Each part is a function of what
//! the app knows, rebuilt every frame; terminals are custom boxes the app
//! paints after layout.

mod cards;
mod chrome;
mod composer;
mod dialogs;
pub mod gates;
pub mod git;
pub mod menus;
pub mod panes;
pub mod runs;
pub mod settings;
mod sidebar;
pub mod tasks;
pub mod workflows;

use std::collections::HashMap;

use crate::act::Acts;
use crate::client::Store;
use crate::look::{self, Rgb};
use crate::state::{LayoutMode, State};
use crate::ui::{div, icon, text, El, Place, Rgba, Ui};
use crate::view::PaneView;

pub use cards::{card_index, intent_id, rename_id, term_id, CARD_ID_BASE};
pub use dialogs::{PROJECT_NAME, PROJECT_PATH};
pub(crate) use sidebar::project_icon as sidebar_project_icon;

/// The three views the top bar's pills switch between.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum MainView {
    #[default]
    Sessions,
    Tasks,
    Workflows,
}

/// The composer's state: what is typed and for which project.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
    pub ui: &'a Ui,
    pub acts: &'a Acts,
    pub store: &'a Store,
    pub st: &'a State,
    pub panes: &'a HashMap<String, PaneView>,
    /// The logo's image id, once registered.
    pub logo: Option<u32>,
    /// The agents' marks, by agent.
    pub agents: &'a HashMap<&'static str, u32>,
    pub size: (f32, f32),
    /// The app draws its own window controls (no system title bar).
    pub chrome: bool,
    /// macOS: the traffic lights sit over the top bar's left end.
    pub traffic: bool,
}

impl Screen<'_> {
    /// The whole window.
    pub fn build(&self) -> El {
        let tabs = self.st.layout == LayoutMode::Tabs
            && self.st.view == MainView::Sessions
            && self.st.focused.is_none();
        let mut main = div().col().grow().min_w0();
        main = if tabs {
            main.child(cards::tabs(self))
        } else if self.st.view == MainView::Sessions && self.st.focused.is_some() {
            main.child(cards::focused(self))
        } else if self.st.view == MainView::Workflows && self.st.wf.editor.is_some() {
            main.child(workflows::editor(self))
        } else {
            main.child(chrome::top_bar(self))
                .child(self.beside(self.body()))
        };
        // Without a top bar the panel takes the window's height.
        if self.st.view == MainView::Sessions && (tabs || self.st.focused.is_some()) {
            if let Some(panel) = git::right_panel(self) {
                main = div()
                    .id(git::changes_row_id())
                    .row()
                    .grow()
                    .min_w0()
                    .min_h(0.0)
                    .child(main)
                    .child(panel);
            }
        }
        let mut row = div().row().grow().min_h(0.0);
        if self.st.sidebar {
            row = row.child(sidebar::sidebar(self));
        }
        let mut root = div()
            .col()
            .w_full()
            .h_full()
            .bg(c(look::SURFACE_BASE))
            .child(row.child(main));
        if self.st.view == MainView::Tasks {
            root = root.child_if(tasks::ghost(self));
        }
        if self.st.settings.open {
            root = root.child(
                div()
                    .id(crate::ui::Id::new("settings-page"))
                    .sense(crate::ui::Sense::HOVER)
                    .absolute(0.0, 0.0)
                    .size(self.size.0, self.size.1)
                    .layered()
                    .child(settings::page(self)),
            );
        }
        root = root.child_if(dialogs::dialog(self));
        if let Some(menu) = &self.st.menu {
            let w = menus::width(&menu.kind);
            let panel = match menu.kind {
                crate::state::MenuKind::Recent => chrome::recent_panel(self),
                crate::state::MenuKind::Headless => chrome::headless_panel(self),
                crate::state::MenuKind::WfIcon => workflows::icon_picker(self),
                crate::state::MenuKind::SetShell => settings::shell_panel(self),
                crate::state::MenuKind::Branch(_) => git::branch_panel(self),
                crate::state::MenuKind::OpenIn(ref sid) => git::open_in_panel(self, sid),
                _ => menus::panel(
                    self.ui,
                    menus::MENU,
                    w,
                    "Menu",
                    menus::items(self, &menu.kind),
                    menu.sub,
                ),
            };
            let place = if menu.kind.is_launch() {
                Place::AboveStart
            } else {
                Place::BelowStart
            };
            let anchor = div()
                .absolute(menu.at.0, menu.at.1)
                .size(0.0, 0.0)
                .overlay(panel, place, 0.0);
            root = root.child(anchor);
        }
        if self.st.composer_overlay(self.store) {
            root = root.child_if(composer::overlay(self));
        }
        root = root.child_if(tasks::overlay(self));
        root.child_if(chrome::toasts(self))
    }

    /// Under the top bar: the sessions view's grid, focused stage or
    /// composer, or the other views.
    fn body(&self) -> El {
        let st = self.st;
        let body = match st.view {
            MainView::Sessions if self.store.sessions.is_empty() => composer::empty_state(self),
            MainView::Sessions => cards::grid(self),
            MainView::Tasks => tasks::view(self),
            MainView::Workflows => workflows::landing(self),
        };
        let mut col = div().col().grow().min_h(0.0);
        if st.view == MainView::Sessions {
            col = col.child_if(chrome::banner(self));
        }
        col.child(body)
    }

    /// The left padding of a bar at the window's top left: room for the
    /// traffic lights on macOS (`TRAFFIC_LIGHT_PAD_PX`) while the sidebar,
    /// which otherwise holds them, is closed.
    pub fn lead(&self) -> f32 {
        if self.traffic && !self.st.sidebar {
            look::TRAFFIC_LIGHT_PAD
        } else {
            12.0
        }
    }

    /// `content` with the Changes panel beside it, under the top bar.
    fn beside(&self, content: El) -> El {
        match git::right_panel(self).filter(|_| self.st.view == MainView::Sessions) {
            Some(panel) => div()
                .id(git::changes_row_id())
                .sense(crate::ui::Sense::HOVER)
                .row()
                .grow()
                .min_h(0.0)
                .child(div().col().grow().min_w0().min_h(0.0).child(content))
                .child(panel),
            None => content,
        }
    }

    /// The grid's size: the window under the top bar, beside the sidebar.
    pub fn grid_size(&self) -> (f32, f32) {
        let side = if self.st.sidebar {
            look::SIDEBAR_W
        } else {
            0.0
        };
        let panel = match &self.st.git.panel {
            Some(_) if self.st.view == MainView::Sessions && self.st.git.maximized => {
                (self.size.0 - 48.0).max(look::MIN_CARD.0)
            }
            Some(_) if self.st.view == MainView::Sessions => self.st.git.width(),
            _ => 0.0,
        };
        (
            (self.size.0 - side - panel).max(0.0),
            (self.size.1 - look::TOP_BAR_H).max(0.0),
        )
    }
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

/// `AgentIcon`: an agent's mark, or a shell's terminal.
pub(crate) fn agent_icon(s: &Screen<'_>, agent: &str, size: f32) -> El {
    match s.agents.get(agent) {
        Some(id) => crate::ui::image(*id, size, size),
        None => ico("terminal", size, 1.5, c(look::GRAY_400)),
    }
}

/// The Alt key's label in shortcut hints.
pub(crate) const ALT_LABEL: &str = if cfg!(target_os = "macos") {
    "\u{2325}"
} else {
    "Alt+"
};

/// `border-t white/[0.06]` with `margin` above and below.
pub(crate) fn rule(margin: f32) -> El {
    div()
        .py(margin)
        .w_full()
        .child(div().h(1.0).w_full().bg(white(0.06)))
}

/// A `<kbd>`: mono text on `white/[0.04]` in a `white/[0.06]` outline.
pub(crate) fn kbd(label: &str, size: f32, color: Rgb) -> El {
    div()
        .px(6.0)
        .py(2.0)
        .rounded(4.0)
        .bg(white(0.04))
        .border(1.0, white(0.06))
        .shrink0()
        .child(
            text(label, size, c(color))
                .mono()
                .line_height(size + 2.0)
                .silent(),
        )
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

/// A dashed outline over a `w`×`h` box.
pub(crate) fn dashed(w: f32, h: f32, r: f32, color: Rgba) -> El {
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\"><rect x=\"0.5\" y=\"0.5\" width=\"{}\" height=\"{}\" rx=\"{r}\" fill=\"none\" stroke=\"#ffffff\" stroke-width=\"1\" stroke-dasharray=\"3 3\"/></svg>",
        w - 1.0,
        h - 1.0
    );
    crate::ui::shape(svg, w, h, color).absolute(0.0, 0.0)
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
