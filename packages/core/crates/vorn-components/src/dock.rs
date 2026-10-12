//! The session dock (`SessionDock`) and what it holds: a minimized
//! session's pill (`MinimizedPill`) and a waiting approval's
//! (`WaitingApprovalPill`), grouped by kind, four inline and the rest in
//! a popover, or all of them behind one chip when the dock is collapsed.

use vornui::accesskit::Role;
use vornui::theme::{color, radius};
use vornui::widgets::{popover, tooltip, tooltip_at};
use vornui::{div, text, Action, El, Id, Place, Rgba, Ui};

use crate::agent::{agent_icon, AgentMarks};
use crate::lucide;
use crate::status::{status_dot, Status};

/// How many pills the expanded dock shows before the `+N` button.
pub const MAX_INLINE: usize = 4;

/// Body text at `size` with the page's `leading-normal` (1.5).
fn label(s: &str, size: f32, fg: Rgba) -> El {
    text(s, size, fg).line_height(size * 1.5).silent()
}

/// The `·` between a pill's name and its detail.
fn middot() -> El {
    label("\u{b7}", 10.0, color::GRAY_600).shrink0()
}

/// A minimized session, as its pill shows it.
pub struct Minimized<'a> {
    /// `getDisplayName`: the session's name, or its project's.
    pub name: &'a str,
    pub agent: &'a str,
    pub status: Status,
    /// `getBranchLabel`: the branch, or the worktree's name.
    pub branch: Option<&'a str>,
    pub worktree: bool,
}

/// `MinimizedPill`: status dot, agent mark, name and branch. Clicking
/// (`id`) restores the card.
pub fn minimized_pill(ui: &Ui, marks: &AgentMarks, id: Id, m: &Minimized<'_>) -> El {
    let hot = ui.hover(id);
    let mut pill = div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(6.0)
        .px(10.0)
        .py(4.0)
        .rounded(radius::MD)
        .border(1.0, Rgba::white(0.06).mix(Rgba::white(0.12), hot))
        .bg(color::SURFACE_RAISED)
        .ring(radius::MD)
        .role(Role::Button, m.name)
        .child(status_dot(ui, m.status))
        .child(agent_icon(marks, m.agent, 14.0).shrink0())
        .child(
            label(m.name, 11.0, color::GRAY_200)
                .weight(500)
                .truncate()
                .max_w(120.0),
        );
    if let Some(branch) = m.branch {
        let (glyph, tint) = if m.worktree {
            ("folder-git-2", color::INK_FAINT)
        } else {
            ("git-branch", color::GRAY_500)
        };
        pill = pill.child(middot()).child(
            div()
                .row()
                .items_center()
                .gap(2.0)
                .max_w(90.0)
                .min_w(0.0)
                .child(lucide(glyph, 9.0, 1.5, tint).shrink0())
                .child(label(branch, 10.0, color::GRAY_500).mono().truncate()),
        );
    }
    tooltip_at(ui, pill, id, "Click to restore", None, Place::Below)
}

/// A waiting approval, as its pill shows it.
pub struct Approval<'a> {
    /// The workflow's name; `Workflow` when it has none.
    pub workflow: &'a str,
    /// The workflow's Lucide icon.
    pub icon: &'a str,
    pub icon_color: Option<Rgba>,
    /// What the gate asks, if it says.
    pub message: Option<&'a str>,
}

/// The ids a [`approval_pill`] reports: opening the run, approving and
/// rejecting.
#[derive(Debug, Clone, Copy)]
pub struct ApprovalIds {
    pub open: Id,
    pub approve: Id,
    pub reject: Id,
}

/// `WaitingApprovalPill`: a bronzo dot, the workflow's icon and name,
/// what the gate asks, and Approve and Reject.
pub fn approval_pill(ui: &Ui, ids: ApprovalIds, a: &Approval<'_>) -> El {
    let hot = ui.hover(ids.open);
    let small = |id: Id, glyph: &str, name: &str, rest: Rgba, fg: Rgba, wash: Rgba| {
        let h = ui.hover(id);
        let b = div()
            .id(id)
            .on(Action::Click)
            .p(2.0)
            .rounded(radius::SM)
            .bg(Rgba::TRANSPARENT.mix(wash, h))
            .role(Role::Button, name)
            .child(lucide(glyph, 12.0, 2.5, rest.mix(fg, h)));
        tooltip(ui, b, id, name, None)
    };
    let name = if a.workflow.is_empty() {
        "Workflow"
    } else {
        a.workflow
    };
    let mut pill = div()
        .id(ids.open)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(6.0)
        .px(10.0)
        .py(4.0)
        .rounded(radius::MD)
        .border(
            1.0,
            color::BRONZO.alpha(0.4).mix(color::BRONZO.alpha(0.7), hot),
        )
        .bg(color::SURFACE_RAISED)
        .role(Role::Button, name)
        .child(
            div()
                .size(6.0, 6.0)
                .rounded(3.0)
                .shrink0()
                .bg(color::BRONZO),
        )
        .child(lucide(a.icon, 12.0, 1.5, a.icon_color.unwrap_or(color::GRAY_100)).shrink0())
        .child(
            label(name, 11.0, color::GRAY_200)
                .weight(500)
                .truncate()
                .max_w(140.0),
        );
    if let Some(m) = a.message.filter(|m| !m.is_empty()) {
        pill = pill
            .child(middot())
            .child(label(m, 10.0, color::GRAY_500).truncate().max_w(230.0));
    }
    pill.child(
        div()
            .row()
            .items_center()
            .gap(2.0)
            .ml(4.0)
            .shrink0()
            .child(small(
                ids.approve,
                "check",
                "Approve",
                color::INK_SECONDARY,
                color::INK,
                Rgba::white(0.08),
            ))
            .child(small(
                ids.reject,
                "x",
                "Reject",
                color::INK_FAINT,
                color::DANGER,
                color::DANGER.alpha(0.1),
            )),
    )
}

/// Which group a dock item sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockKind {
    Waiting,
    Minimized,
}

impl DockKind {
    fn title(self) -> (&'static str, Rgba) {
        match self {
            DockKind::Waiting => ("WAITING APPROVAL", color::INK_SECONDARY),
            DockKind::Minimized => ("MINIMIZED", color::GRAY_500),
        }
    }
}

/// The ids of the dock's own controls.
#[derive(Debug, Clone, Copy)]
pub struct DockIds {
    /// The collapsed chip, which opens the popover.
    pub chip: Id,
    /// The `+N` button, which opens the overflow popover.
    pub overflow: Id,
    /// Collapse in the expanded dock; Expand in the collapsed one's popover.
    pub collapse: Id,
    pub expand: Id,
}

/// The dock's state, kept by the caller.
#[derive(Debug, Clone, Copy, Default)]
pub struct DockState {
    pub collapsed: bool,
    /// The chip's or the overflow's popover is open.
    pub open: bool,
}

/// `SessionDock`: `items` in order (approvals first), or nothing when
/// there are none.
pub fn dock(ui: &Ui, ids: DockIds, state: DockState, items: Vec<(DockKind, El)>) -> Option<El> {
    if items.is_empty() {
        return None;
    }
    let waiting = items
        .iter()
        .filter(|(k, _)| *k == DockKind::Waiting)
        .count();
    let minimized = items.len() - waiting;
    if state.collapsed {
        return Some(collapsed(ui, ids, state.open, waiting, minimized, items));
    }
    let mut row = div().row().items_center().gap(6.0).min_w(0.0);
    let mut items = items;
    let overflow = items.split_off(items.len().min(MAX_INLINE));
    for (i, (_, group)) in grouped(items).into_iter().enumerate() {
        if i > 0 {
            row = row.child(div().w(1.0).h(16.0).shrink0().bg(Rgba::white(0.08)));
        }
        row = row.child(
            div()
                .row()
                .items_center()
                .gap(6.0)
                .min_w(0.0)
                .children(group),
        );
    }
    if !overflow.is_empty() {
        let n = overflow.len();
        let hot = ui.hover(ids.overflow);
        let more = div()
            .id(ids.overflow)
            .on(Action::Click)
            .row()
            .items_center()
            .justify_center()
            .h(26.0)
            .min_w(28.0)
            .px(6.0)
            .rounded(radius::MD)
            .border(1.0, Rgba::white(0.06).mix(Rgba::white(0.12), hot))
            .bg(color::SURFACE_RAISED)
            .ring(radius::MD)
            .role(Role::Button, format!("{n} more dock items"))
            .has_popup()
            .expanded(state.open)
            .child(
                label(
                    &format!("+{n}"),
                    11.0,
                    color::GRAY_400.mix(color::WHITE, hot),
                )
                .weight(500),
            );
        let mut more = tooltip_at(
            ui,
            more,
            ids.overflow,
            &format!("{n} more in dock"),
            None,
            Place::Below,
        );
        if state.open {
            more = more.overlay(panel(overflow, None), Place::BelowEnd, 6.0);
        }
        row = row.child(more);
    }
    let hot = ui.hover(ids.collapse);
    let collapse = div()
        .id(ids.collapse)
        .on(Action::Click)
        .row()
        .items_center()
        .justify_center()
        .size(22.0, 22.0)
        .rounded(radius::SM)
        .bg(Rgba::TRANSPARENT.mix(Rgba::white(0.06), hot))
        .ring(radius::SM)
        .role(Role::Button, "Collapse session dock")
        .child(lucide(
            "chevrons-left",
            12.0,
            1.75,
            color::GRAY_500.mix(color::WHITE, hot),
        ));
    Some(row.child(tooltip_at(
        ui,
        collapse,
        ids.collapse,
        "Collapse dock",
        None,
        Place::Below,
    )))
}

/// "1 waiting approval · 2 minimized".
pub fn breakdown(waiting: usize, minimized: usize) -> String {
    let mut parts = Vec::new();
    if waiting > 0 {
        let s = if waiting == 1 { "" } else { "s" };
        parts.push(format!("{waiting} waiting approval{s}"));
    }
    if minimized > 0 {
        parts.push(format!("{minimized} minimized"));
    }
    parts.join(" \u{b7} ")
}

fn collapsed(
    ui: &Ui,
    ids: DockIds,
    open: bool,
    waiting: usize,
    minimized: usize,
    items: Vec<(DockKind, El)>,
) -> El {
    let tip = breakdown(waiting, minimized);
    let hot = ui.hover(ids.chip);
    let fg = color::GRAY_300.mix(color::WHITE, hot);
    let chip = div()
        .id(ids.chip)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(6.0)
        .h(26.0)
        .px(8.0)
        .rounded(radius::MD)
        .border(1.0, Rgba::white(0.06).mix(Rgba::white(0.12), hot))
        .bg(color::SURFACE_RAISED)
        .ring(radius::MD)
        .role(Role::Button, &tip)
        .has_popup()
        .expanded(open)
        .child(lucide("layers", 11.0, 1.5, fg))
        .child(
            text((waiting + minimized).to_string(), 11.0, fg)
                .weight(500)
                .mono()
                .line_height(11.0)
                .silent(),
        )
        .child_if((waiting > 0).then(|| {
            div()
                .size(6.0, 6.0)
                .rounded(3.0)
                .bg(color::BRONZO)
                .absolute_right(-2.0, -2.0)
        }));
    let chip = tooltip_at(ui, chip, ids.chip, &tip, None, Place::Below);
    if !open {
        return chip;
    }
    let hot = ui.hover(ids.expand);
    let fg = color::GRAY_400.mix(color::WHITE, hot);
    let expand = div()
        .id(ids.expand)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(4.0)
        .role(Role::Button, "Expand dock")
        .child(lucide("chevrons-right", 11.0, 1.75, fg))
        .child(label("Expand", 10.0, fg));
    chip.overlay(panel(items, Some(expand)), Place::BelowStart, 6.0)
}

/// `DockGroupedPopover`: the footer, then each group under its title when
/// there is more than one.
fn panel(items: Vec<(DockKind, El)>, footer: Option<El>) -> El {
    let groups = grouped(items);
    let titled = groups.len() > 1;
    let mut p = popover("Dock items").p(6.0).gap(8.0).min_w(240.0);
    if let Some(f) = footer {
        p = p.child(
            div()
                .row()
                .items_center()
                .justify_end()
                .px(4.0)
                .pb(4.0)
                .mb(2.0)
                .border_bottom(1.0, Rgba::white(0.06))
                .child(f),
        );
    }
    for (kind, group) in groups {
        let (title, tint) = kind.title();
        p = p.child(
            div()
                .col()
                .gap(4.0)
                .child_if(titled.then(|| {
                    div().px(4.0).child(
                        label(title, 9.0, tint)
                            .weight(500)
                            .tracking(0.05)
                            .role(Role::Label, title),
                    )
                }))
                .child(div().row().wrap_children().gap(6.0).children(group)),
        );
    }
    p
}

/// Consecutive items of one kind, together.
fn grouped(items: Vec<(DockKind, El)>) -> Vec<(DockKind, Vec<El>)> {
    let mut out: Vec<(DockKind, Vec<El>)> = Vec::new();
    for (kind, el) in items {
        match out.last_mut() {
            Some((k, group)) if *k == kind => group.push(el),
            _ => out.push((kind, vec![el])),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breakdown_counts_each_kind_it_holds() {
        assert_eq!(breakdown(1, 0), "1 waiting approval");
        assert_eq!(breakdown(2, 3), "2 waiting approvals \u{b7} 3 minimized");
        assert_eq!(breakdown(0, 1), "1 minimized");
    }

    #[test]
    fn consecutive_kinds_group_and_a_return_starts_a_new_group() {
        use DockKind::{Minimized as M, Waiting as W};
        let items = [W, W, M, W].map(|k| (k, div())).into_iter().collect();
        let kinds: Vec<_> = grouped(items)
            .into_iter()
            .map(|(k, g)| (k, g.len()))
            .collect();
        assert_eq!(kinds, vec![(W, 2), (M, 1), (W, 1)]);
    }
}
