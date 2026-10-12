//! The sidebar's session row (`SessionItem`): the agent's status icon, the
//! name over its branch, and More and Close on hover.

use vornui::accesskit::Role;
use vornui::theme::{color, radius};
use vornui::widgets::tooltip;
use vornui::{div, text, Action, El, Id, Rgba, Ui};

use crate::agent::{status_icon, AgentMarks};
use crate::status::Status;

/// A session as its row shows it.
pub struct SessionRow<'a> {
    pub name: &'a str,
    /// Shown under the name when the project lists branches.
    pub branch: Option<&'a str>,
    pub agent: &'a str,
    pub status: Status,
    /// The focused card, or the active tab.
    pub active: bool,
}

/// The ids a [`session_row`] reports clicks on.
#[derive(Debug, Clone, Copy)]
pub struct SessionRowIds {
    pub row: Id,
    /// The session menu (`SessionContextMenu`), when the host has one.
    pub more: Option<Id>,
    pub close: Id,
}

/// `SessionItem`: `px-2 py-1 rounded-md text-[12px] gap-2`, white with a
/// 1 px marker while active, the hover wash and its buttons otherwise.
pub fn session_row(ui: &Ui, marks: &AgentMarks, ids: SessionRowIds, r: &SessionRow<'_>) -> El {
    let hot =
        ui.hovered(ids.row) || ids.more.is_some_and(|m| ui.hovered(m)) || ui.hovered(ids.close);
    let fg = if r.active || hot {
        color::WHITE
    } else {
        color::GRAY_400
    };
    let mut names = div()
        .col()
        .grow()
        .min_w0()
        .child(text(r.name, 12.0, fg).line_height(18.0).truncate().silent());
    if let Some(b) = r.branch {
        names = names.child(
            text(b, 10.0, color::GRAY_600)
                .line_height(15.0)
                .truncate()
                .silent(),
        );
    }
    let tip = format!("{} \u{b7} {}", r.agent, r.status.label());
    let icon_id = ids.row.child("status");
    let icon = div()
        .id(icon_id)
        .tooltip_owner()
        .shrink0()
        .child(status_icon(ui, marks, r.agent, r.status, 14.0, 1.0));
    let mut row = div()
        .id(ids.row)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(8.0)
        .px(8.0)
        .py(4.0)
        .rounded(radius::MD)
        .bg(Rgba::TRANSPARENT.mix(Rgba::white(0.04), f32::from(hot && !r.active)))
        .ring(radius::MD)
        .role(Role::Button, r.name)
        .selected(r.active)
        .child(tooltip(ui, icon, icon_id, &tip, None))
        .child(names);
    if r.active {
        // `top-1 bottom-1` of a row one or two lines tall.
        let h = if r.branch.is_some() { 33.0 } else { 18.0 };
        row = row.child(
            div()
                .absolute(0.0, 4.0)
                .size(1.0, h)
                .rounded(radius::FULL)
                .bg(color::WHITE),
        );
    }
    // `opacity-0 group-hover:opacity-100`: the buttons keep their room, so
    // the name truncates at the same width hovered or not.
    let shown = |id: Id| if hot || ui.focused(id) { 1.0 } else { 0.0 };
    if let Some(more) = ids.more {
        row = row
            .child(row_button(ui, more, "ellipsis", "More", color::GRAY_200).opacity(shown(more)));
    }
    row.child(
        row_button(ui, ids.close, "x", "Close session", color::RED_400).opacity(shown(ids.close)),
    )
}

/// `p-0.5 rounded hover:bg-white/[0.08]`, a 12 px icon in gray-500 that
/// takes `hot` when hovered.
fn row_button(ui: &Ui, id: Id, glyph: &str, label: &str, hot: Rgba) -> El {
    let h = ui.hover(id);
    let b = div()
        .id(id)
        .on(Action::Click)
        .p(2.0)
        .rounded(radius::SM)
        .shrink0()
        .bg(Rgba::TRANSPARENT.mix(Rgba::white(0.08), h))
        .ring(radius::SM)
        .role(Role::Button, label)
        .child(crate::lucide(glyph, 12.0, 2.0, color::GRAY_500.mix(hot, h)));
    tooltip(ui, b, id, label, None)
}
