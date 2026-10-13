//! The command palette (`CommandPalette`): a search field over the
//! commands, grouped by category until something is typed.

use super::{agent_icon, c, ico, white, Screen};
use crate::act::Act;
use crate::look;
use crate::palette::{self, Glyph, PalAct};
use crate::ui::widgets::kbd;
use crate::ui::{div, field, text, El, Id, Role, TextStyle};

/// The search field.
pub const PALETTE_INPUT: Id = Id(0x7061_6c65_7474_6669);
/// The results' scroll view.
pub const PALETTE_LIST: Id = Id(0x7061_6c65_7474_6c69);

const W: f32 = 560.0;

pub(super) fn palette(s: &Screen<'_>) -> El {
    let query = s.st.palette.query.trim();
    let rows = palette::shown(
        palette::commands(s.store, s.st, super::MOD_LABEL, super::ALT_LABEL),
        &s.st.palette.query,
    );
    let search = div()
        .row()
        .items_center()
        .gap(12.0)
        .px(16.0)
        .py(12.0)
        .border_bottom(1.0, white(0.06))
        .child(ico("search", 16.0, 1.5, c(look::GRAY_500)))
        .child(
            field(
                PALETTE_INPUT,
                "Search commands",
                "Type a command...",
                TextStyle {
                    line_height: 20.0,
                    ..TextStyle::ui(14.0)
                },
                c(look::GRAY_200),
            )
            .grow()
            .min_w0(),
        )
        .child(kbd("ESC", (10.0, 15.0), c(look::GRAY_600)));
    // vornui scroll views take no height from their content: cap the list at
    // what the 420 px panel leaves after the search row (45) and footer (33).
    let mut list = div().id(PALETTE_LIST).col().max_h(342.0).py(4.0).scroll_y();
    if rows.is_empty() {
        list = list.child(
            div().px(16.0).py(32.0).center().child(
                text("No matching commands", 14.0, c(look::GRAY_600))
                    .line_height(20.0)
                    .role(Role::Label, "No matching commands"),
            ),
        );
    }
    let mut last = None;
    for (i, cmd) in rows.iter().enumerate() {
        if query.is_empty() && last != Some(cmd.cat) {
            last = Some(cmd.cat);
            list = list.child(
                div().px(12.0).py(6.0).child(
                    text(cmd.cat.label().to_uppercase(), 11.0, c(look::GRAY_500))
                        .weight(500)
                        .tracking(0.05)
                        .line_height(16.0)
                        .role(Role::Heading, cmd.cat.label()),
                ),
            );
        }
        list = list.child(row(s, i, cmd, query));
    }
    let hint = |keys: &str, label: &str| {
        div()
            .row()
            .items_center()
            .gap(4.0)
            .child(kbd(keys, (10.0, 15.0), c(look::GRAY_600)).px(4.0))
            .child(
                text(label, 11.0, c(look::GRAY_600))
                    .line_height(16.0)
                    .silent(),
            )
    };
    let footer = div()
        .row()
        .items_center()
        .gap(16.0)
        .px(16.0)
        .py(8.0)
        .border_top(1.0, white(0.06))
        .child(hint("\u{2191}\u{2193}", "navigate"))
        .child(hint("\u{21b5}", "select"))
        .child(hint("esc", "close"));
    let panel = div()
        .col()
        .w(W)
        .max_h(420.0)
        .rounded(12.0)
        .border(1.0, white(0.08))
        .bg(c(look::SURFACE_OVERLAY))
        .clip()
        .role(Role::Dialog, "Command palette")
        .child(search)
        .child(list)
        .child(footer);
    let panel = s.acts.click(panel, "dialog", Act::Nothing);
    let x = ((s.size.0 - W) / 2.0).max(0.0);
    super::dialogs::backdrop(s, 0.4).child(panel.absolute(x, s.size.1 * 0.2))
}

fn row(s: &Screen<'_>, i: usize, cmd: &palette::Entry, query: &str) -> El {
    let id = s.acts.on("palette", Act::Palette(PalAct::Execute(i)));
    let active = i == s.st.palette.active;
    let bg = if active {
        white(0.06)
    } else {
        white(0.0).mix(white(0.04), s.ui.hover(id))
    };
    let fg = if active {
        c(look::WHITE)
    } else {
        c(look::GRAY_300)
    };
    let glyph = match &cmd.glyph {
        Glyph::Icon(name) => ico(name, 14.0, 1.5, c(look::GRAY_500)),
        Glyph::Agent(agent) => agent_icon(s, agent, 14.0),
        Glyph::None => div(),
    };
    let mut label = div()
        .col()
        .grow()
        .min_w0()
        .child(highlighted(&cmd.label, query, fg));
    if let Some(sub) = &cmd.sublabel {
        label = label.child(
            text(sub, 10.0, c(look::GRAY_600))
                .line_height(15.0)
                .truncate()
                .silent(),
        );
    }
    let mut el = div()
        .id(id)
        .on(crate::ui::Action::Click)
        .row()
        .items_center()
        .gap(12.0)
        .px(12.0)
        .py(8.0)
        .bg(bg)
        .role(Role::Button, &cmd.label)
        .child(div().w(20.0).shrink0().row().justify_center().child(glyph))
        .child(label);
    if let Some(keys) = &cmd.shortcut {
        el = el.child(kbd(keys, (10.0, 15.0), c(look::GRAY_500)));
    }
    el
}

/// `highlightMatch`: the first match of what is typed, white and bold.
fn highlighted(label: &str, query: &str, fg: crate::ui::Rgba) -> El {
    let style = |t: &str, color, weight| {
        text(t, 13.0, color)
            .weight(weight)
            .line_height(18.0)
            .silent()
    };
    let at = (!query.is_empty())
        .then(|| label.to_lowercase().find(&query.to_lowercase()))
        .flatten()
        // Lowercasing can move byte offsets; only trust ones on the label's boundaries.
        .filter(|&at| label.is_char_boundary(at) && label.is_char_boundary(at + query.len()));
    let Some(at) = at else {
        return style(label, fg, 400).truncate();
    };
    let end = at + query.len();
    div()
        .row()
        .min_w0()
        .child(style(&label[..at], fg, 400))
        .child(style(&label[at..end], c(look::WHITE), 600))
        .child(style(&label[end..], fg, 400).truncate())
}
