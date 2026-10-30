//! The main screen as elements: the top bar over either the composer
//! (no sessions yet) or the terminal cards. Each part is a function of what
//! the app knows, rebuilt every frame; terminals are custom boxes the app
//! paints after layout.

mod cards;
pub use cards::{flex_divider_id, grid_id};
pub mod card_buttons;
mod chrome;
pub mod code;
pub use chrome::{DOCK, VIEWS};
mod composer;
pub mod customize;
mod dialogs;
pub mod gates;
pub mod git;
pub mod menus;
mod network;
mod onboarding;
mod palette;
mod panes;
pub(crate) mod plugins;
pub mod points;
pub mod runs;
pub mod settings;
pub mod side;
pub mod sidebar;
mod signin;
pub mod tasks;
mod video;
pub mod workflows;

use std::collections::HashMap;

use crate::act::{Act, Acts};
use crate::client::Store;
use crate::look::{self, Color};
use crate::points::{Point, Points};
use crate::state::{LayoutMode, State};
use crate::ui::{div, icon, El, Place, Rgba, Ui};
use crate::view::PaneView;

pub use cards::{
    block_of, card_box_id, card_index, intent_id, log_id, rename_id, term_id, CARD_ID_BASE, SETTLE,
};
pub use cards::{scrollbar_id, thumb_id};
pub use composer::COMPOSER_INPUT;
pub use dialogs::{PROJECT_COLOR, PROJECT_ICON, PROJECT_NAME, PROJECT_PATH};
pub use palette::{PALETTE_INPUT, PALETTE_LIST};
pub(crate) use sidebar::{picker_project_icon, project_icon as sidebar_project_icon};

/// The views the top bar's pills switch between: Sessions, then what
/// extensions add under `[[contributes.views]]`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum MainView {
    #[default]
    Sessions,
    /// Tasks' board, which the app draws over its tree.
    Tasks,
    /// Workflows' runs and editor, which the app draws over its tree.
    Workflows,
    /// Any other extension's view, drawn from its tree; `State::ext_view` names it.
    Extension,
}

impl MainView {
    /// The view top-bar point `p` opens.
    pub fn of(p: &Point) -> MainView {
        if p.extension == tasks::board::TASKS && p.id == "board" {
            MainView::Tasks
        } else if p.extension == crate::workflows::ID && p.id == "workflows" {
            MainView::Workflows
        } else {
            MainView::Extension
        }
    }

    /// What choosing top-bar point `p` does.
    pub fn act(p: &Point) -> Act {
        match MainView::of(p) {
            MainView::Extension => Act::ExtensionView(p.key()),
            v => Act::View(v),
        }
    }
}

/// The view point `view` shows, with a button or not; `None` for Sessions,
/// or one switched off.
pub fn view_point<'a>(points: &'a Points, view: MainView, ext: Option<&str>) -> Option<&'a Point> {
    points.views().into_iter().find(|p| {
        MainView::of(p) == view && (view != MainView::Extension || ext == Some(p.key().as_str()))
    })
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

/// How the terminal UI draws, for its Settings row: the desktop has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermDrawing {
    /// The terminal shows images, so the UI can be drawn as pixels.
    pub supported: bool,
    /// It is drawn as pixels now.
    pub pixels: bool,
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
    pub agents: &'a crate::components::AgentMarks,
    /// The extensions' logos, by extension id.
    pub logos: &'a crate::plugins::logos::Logos,
    pub shots: &'a crate::plugins::shots::Shots,
    pub size: (f32, f32),
    /// The app draws its own window controls (no system title bar).
    pub chrome: bool,
    /// macOS: the traffic lights sit over the top bar's left end.
    pub traffic: bool,
    /// The web client of the vornd this app talks to, on this machine.
    pub web_url: &'a str,
    /// In the terminal UI: how it draws.
    pub term_drawing: Option<TermDrawing>,
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
        // Over the system's material only the sidebar is see-through.
        let (root_bg, main_bg) = if self.ui.see_through() {
            (
                crate::ui::theme::shadow(0.0),
                crate::ui::theme::material::content(true),
            )
        } else {
            (c(look::SURFACE_BASE), crate::ui::theme::shadow(0.0))
        };
        let mut root = div()
            .col()
            .w_full()
            .h_full()
            .bg(root_bg)
            // Settings over the material covers the app, so it is not drawn under it.
            .child_if(
                (!(self.st.settings.open && self.ui.see_through()))
                    .then(|| row.child(main.bg(main_bg))),
            );
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
        root = root
            .child_if(points::floating(self))
            .child_if(dialogs::dialog(self))
            .child_if(points::dialog(self));
        if let Some(menu) = &self.st.menu {
            let w = menus::width(&menu.kind);
            let panel = match &menu.kind {
                crate::state::MenuKind::Popover(ref key) => points::popover(self, key),
                crate::state::MenuKind::Headless => chrome::headless_panel(self),
                crate::state::MenuKind::WfIcon => workflows::icon_picker(self),
                crate::state::MenuKind::GroupIcon(g) => sidebar::group_icon_picker(self, g),
                crate::state::MenuKind::Workspace => sidebar::workspace_panel(self),
                crate::state::MenuKind::SetShell => settings::shell_panel(self),
                crate::state::MenuKind::AgentPicker => settings::agent_picker(self),
                crate::state::MenuKind::ThemePicker(look) => settings::theme_picker(self, *look),
                crate::state::MenuKind::ThemeInfo(ref id) => settings::theme_info(self, id),
                crate::state::MenuKind::FontPicker => settings::font_picker(self),
                crate::state::MenuKind::NotifSources => settings::notice_sources(self),
                crate::state::MenuKind::LaunchModel => composer::model_panel(self),
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
            let place = if menu.kind.opens_up() {
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
            MainView::Sessions if st.composer_inline(self.store) => composer::empty_state(self),
            MainView::Sessions if st.layout == LayoutMode::List => cards::list(self),
            MainView::Sessions => cards::grid(self),
            MainView::Tasks => tasks::view(self),
            MainView::Workflows => workflows::view(self),
            MainView::Extension => points::main_view(self),
        };
        let mut col = div().col().grow().min_h(0.0);
        if st.view == MainView::Sessions {
            col = col.child_if(chrome::banner(self));
        }
        col.child(body)
    }

    /// The left padding of a bar at the window's top left: room for the
    /// traffic lights on macOS while the sidebar,
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

    /// The sidebar's width: in a narrow terminal it takes under a third of the window.
    pub fn sidebar_w(&self) -> f32 {
        let w = self.st.sidebar_width.unwrap_or(look::SIDEBAR_W);
        if self.ui.is_cells() {
            w.min((self.size.0 * 0.3).round())
        } else {
            w
        }
    }

    /// The grid's size: the window under the top bar, beside the sidebar.
    pub fn grid_size(&self) -> (f32, f32) {
        let size = self.size;
        let side = if self.st.sidebar {
            self.sidebar_w()
        } else {
            0.0
        };
        let panel = match &self.st.git.panel {
            Some(_) if self.st.view == MainView::Sessions && self.st.git.maximized => {
                (size.0 - 48.0).max(look::MIN_CARD.0)
            }
            Some(_) if self.st.view == MainView::Sessions => self.st.git.width(),
            _ => 0.0,
        };
        (
            (size.0 - side - panel).max(0.0),
            (size.1 - look::TOP_BAR_H).max(0.0),
        )
    }
}

/// A token, or a content colour as `0xRRGGBB`, in the installed theme.
pub(crate) fn c(color: impl Into<Color>) -> Rgba {
    color.into().rgba()
}

/// Tailwind's `white/[a]`: the theme's brightest ink, black on a light theme.
pub(crate) fn white(a: f32) -> Rgba {
    crate::ui::theme::bright(a)
}

/// A lucide icon at `size` with `stroke`, tinted.
pub(crate) fn ico(name: &str, size: f32, stroke: f32, color: impl Into<Rgba>) -> El {
    let color = color.into();
    icon(look::icon(name, stroke).unwrap_or_default(), size, color)
}

/// An agent's mark, or a shell's terminal.
pub(crate) fn agent_icon(s: &Screen<'_>, agent: &str, size: f32) -> El {
    crate::components::agent_icon(s.agents, agent, size)
}

/// The Alt key's label in shortcut hints.
pub(crate) fn alt_label() -> &'static str {
    if crate::ui::mods::mac() {
        "\u{2325}"
    } else {
        "Alt+"
    }
}

/// Whether Mod+Shift+T reaches the app: a browser keeps it to reopen a tab.
pub(crate) const SHELLS_KEY: bool = !cfg!(target_arch = "wasm32");

/// The modifier the shortcuts use: ⌘ on a Mac, Ctrl elsewhere.
pub(crate) fn mod_label() -> &'static str {
    if crate::ui::mods::mac() {
        "\u{2318}"
    } else {
        "Ctrl+"
    }
}

/// `s` cut to `max` characters with an ellipsis: text is not clipped, so
/// what would overflow its box is cut instead.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((at, _)) => format!("{}\u{2026}", s[..at].trim_end()),
        None => s.to_owned(),
    }
}

/// A dashed outline over a `w`×`h` box.
pub(crate) fn dashed(w: f32, h: f32, r: f32, color: impl Into<Rgba>) -> El {
    let color = color.into();
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
