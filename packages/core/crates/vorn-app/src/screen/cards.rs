//! The terminal cards in the grid (`GridView`, `AgentCard`): a header with
//! the status icon, title and position badge, the terminal, and the status
//! bar with the branch and the task in progress.

use super::{c, ico, truncate, white, Screen, MOD_LABEL};
use crate::client::Session;
use crate::layout::{self, Mode};
use crate::look::{self, Rgb};
use crate::ui::{custom, div, text, El, Rgba, Role};

/// Custom ids at and above this are cards: id `CARD_ID_BASE + i` is the
/// terminal of `store.sessions[i]`.
pub const CARD_ID_BASE: u64 = 1_000;

/// The session index a custom box paints, if it is a card's terminal.
pub fn card_index(id: u64) -> Option<usize> {
    id.checked_sub(CARD_ID_BASE)
        .and_then(|i| usize::try_from(i).ok())
}

/// Which cards a frame shows: all of them when they fit; otherwise the
/// page of rows holding the selected card, since the grid does not scroll.
pub(super) fn visible(
    n: usize,
    selected: Option<usize>,
    size: (f32, f32),
) -> (layout::Layout, usize, usize) {
    let l = layout::pick(n, size.0, size.1);
    if l.mode == Mode::Fit {
        return (l, 0, l.rows);
    }
    let page = layout::fit_max_rows(size.1).min(l.rows);
    let row = selected.map_or(0, |i| i / l.cols);
    let first = row.saturating_sub(page - 1);
    (l, first, page)
}

/// `color` at `a` of its alpha: vornui has no subtree opacity, so a dimmed
/// card's chrome multiplies it in.
fn fade(color: Rgb, alpha: f32, dim: f32) -> Rgba {
    Rgba::hexa(color, alpha * dim)
}

/// AgentStatusIcon at 18: the agent's icon, or the running glyph while an
/// agent works.
fn status_icon(s: &Session, dim: f32) -> El {
    if s.is_shell() {
        return ico("terminal", 18.0, 2.0, fade(look::GRAY_400, 1.0, dim));
    }
    let (name, color, alpha) = match s.status() {
        "running" => return running(dim),
        "waiting" => ("bot", look::BRONZO, 1.0),
        "error" => ("bot", look::DANGER, 1.0),
        _ => ("bot", look::GRAY_400, 1.0),
    };
    ico(name, 18.0, 2.0, fade(color, alpha, dim))
}

/// RunningGlyph, still: a dot in `text-white/85` where the glyph spins.
fn running(dim: f32) -> El {
    div()
        .size(18.0, 18.0)
        .center()
        .role(Role::Image, "Running")
        .child(
            div()
                .size(8.0, 8.0)
                .rounded(4.0)
                .bg(fade(look::WHITE, 0.85, dim)),
        )
}

fn header(s: &Session, i: usize, dim: f32) -> El {
    let title = s.title();
    let mut row = div()
        .row()
        .items_center()
        .gap(8.0)
        .px(12.0)
        .h(look::CARD_HEADER_H - 1.0)
        .shrink0()
        .child(status_icon(s, dim))
        .child(
            text(truncate(&title, 48), 13.0, fade(look::GRAY_300, 1.0, dim))
                .weight(500)
                .line_height(18.0)
                .role(Role::Label, title.as_str()),
        )
        .child(div().grow());
    if i < 9 {
        // px-1.5 py-0.5 text-[10px] font-mono text-gray-600 bg-white/[0.04] border rounded
        row = row.child(
            div()
                .px(6.0)
                .py(2.0)
                .rounded(4.0)
                .bg(fade(look::WHITE, 0.04, dim))
                .border(1.0, fade(look::WHITE, 0.06, dim))
                .child(
                    text(
                        format!("{MOD_LABEL}{}", i + 1),
                        10.0,
                        fade(look::GRAY_600, 1.0, dim),
                    )
                    .line_height(10.0)
                    .silent(),
                ),
        );
    }
    div()
        .col()
        .shrink0()
        .child(row)
        .child(div().h(1.0).w_full().bg(white(0.04)))
}

/// BranchChip: the worktree in ink-secondary, then the branch.
fn branch_chip(s: &Session, dim: f32) -> Option<El> {
    let branch = s.info.branch.as_deref()?;
    let mut chip = div()
        .row()
        .items_center()
        .gap(4.0)
        .px(4.0)
        .py(2.0)
        .rounded(4.0)
        .role(Role::Button, format!("Switch branch (current: {branch})"));
    if let (Some(true), Some(name)) = (s.info.is_worktree, s.info.worktree_name.as_deref()) {
        let ink = fade(look::INK, look::INK_SECONDARY, dim);
        chip = chip
            .child(ico("folder-git-2", 10.0, 1.5, ink))
            .child(
                text(truncate(name, 18), 10.0, ink)
                    .line_height(12.0)
                    .silent(),
            )
            .child(
                text("\u{b7}", 10.0, fade(look::GRAY_600, 1.0, dim))
                    .line_height(12.0)
                    .silent(),
            );
    }
    let chip = chip
        .child(ico("git-branch", 10.0, 1.5, fade(look::GRAY_500, 1.0, dim)))
        .child(
            text(truncate(branch, 18), 10.0, fade(look::GRAY_400, 1.0, dim))
                .line_height(12.0)
                .silent(),
        )
        .child(ico(
            "chevron-down",
            9.0,
            2.0,
            fade(look::GRAY_500, 1.0, dim),
        ));
    Some(chip)
}

/// The task in progress on this session, as a pill.
fn task_chip(s: &Screen<'_>, id: &str, dim: f32) -> Option<El> {
    let task = s.store.tasks.iter().find(|t| {
        t.assigned_session_id.as_deref() == Some(id) && t.status.as_str() == "in_progress"
    })?;
    let chip = div()
        .row()
        .items_center()
        .gap(4.0)
        .px(6.0)
        .py(2.0)
        .rounded(999.0)
        .border(1.0, fade(look::WHITE, 0.08, dim))
        .role(Role::Button, task.title.as_str())
        .child(ico(
            "list-todo",
            10.0,
            2.0,
            fade(look::INK, look::INK_FAINT, dim),
        ))
        .child(
            text(
                truncate(&task.title, 24),
                10.0,
                fade(look::INK, look::INK_SECONDARY, dim),
            )
            .line_height(12.0)
            .silent(),
        );
    Some(chip)
}

fn status_bar(s: &Screen<'_>, session: &Session, dim: f32) -> El {
    let mut bar = div()
        .row()
        .items_center()
        .gap(8.0)
        .px(8.0)
        .h(look::CARD_STATUS_H - 1.0)
        .shrink0();
    if let Some(chip) = branch_chip(session, dim) {
        bar = bar.child(chip);
    }
    if let Some(chip) = task_chip(s, &session.info.id, dim) {
        bar = bar.child(chip);
    }
    div()
        .col()
        .shrink0()
        .child(div().h(1.0).w_full().bg(white(0.04)))
        .child(bar)
}

fn card(s: &Screen<'_>, i: usize, session: &Session) -> El {
    let selected = s.selected == Some(session.info.id.as_str());
    let dim = if s.selected.is_some() && !selected {
        0.6
    } else {
        1.0
    };
    // CommandSpine: an 8 px gutter with its hairline, then mr-2 to the terminal.
    let spine = div()
        .col()
        .items_center()
        .w(8.0)
        .shrink0()
        .child(div().w(1.0).grow().bg(white(0.035)));
    let terminal = div()
        .row()
        .grow()
        .child(spine)
        .child(div().w(8.0).shrink0())
        .child(custom(CARD_ID_BASE + i as u64).grow());
    let body = div()
        .col()
        .grow()
        .bg(c(look::SURFACE_SUNKEN))
        .child(div().h(look::CARD_BODY_PT).shrink0())
        .child(terminal);
    div()
        .col()
        .grow()
        .bg(c(look::SURFACE_RAISED))
        .border(1.0, white(0.06))
        .role(Role::Group, session.title())
        .selected(selected)
        .child(header(session, i, dim))
        .child(body)
        .child(status_bar(s, session, dim))
}

pub(super) fn grid(s: &Screen<'_>) -> El {
    let sessions = &s.store.sessions;
    let selected = s
        .selected
        .and_then(|id| sessions.iter().position(|x| x.info.id == id));
    let (l, first, rows) = visible(sessions.len(), selected, s.grid_size());
    let mut body = div().col().grow().role(Role::Grid, "Sessions");
    for r in first..first + rows {
        let mut row = div().row().grow();
        for col in 0..l.cols {
            let i = r * l.cols + col;
            row = row.child(match sessions.get(i) {
                Some(session) => card(s, i, session),
                None => div().grow(),
            });
        }
        body = body.child(row);
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_ids_map_back_to_sessions() {
        assert_eq!(card_index(CARD_ID_BASE + 3), Some(3));
        assert_eq!(card_index(7), None);
    }

    #[test]
    fn a_scrolling_grid_shows_the_page_with_the_selection() {
        // 1440x860 fits 4 columns and 3 rows, so 20 cards scroll.
        let size = (1440.0, 860.0);
        let (l, first, rows) = visible(20, None, size);
        assert_eq!((l.cols, l.mode, first, rows), (4, Mode::Scroll, 0, 3));
        assert_eq!(visible(20, Some(19), size).1, 2);
        assert_eq!(visible(20, Some(5), size).1, 0);
        let (_, first, rows) = visible(3, Some(2), size);
        assert_eq!((first, rows), (0, 1));
    }
}
