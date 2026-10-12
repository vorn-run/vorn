//! The terminal cards (`GridView`, `AgentCard`): a header with the status
//! icon, title and actions, the terminal beside its command spine, the
//! intent bar or ended strip, and the status bar. Also the focused stage
//! (`FocusedStage`) and the tabs layout (`TabView`), which show one card.

use super::chrome::{self, divider, draggable};
use super::panes;
use super::{c, ico, truncate, white, Screen, MOD_LABEL};
use crate::act::{Act, Acts};
use crate::client::Session;
use crate::layout::{self, Mode};
use crate::look::{self, Rgb};
use crate::state::MenuKind;
use crate::ui::{
    custom, div, field, icon, text, text_input, tooltip, Action, El, Id, Rgba, Role, Sense,
    TextStyle,
};

/// Custom ids at and above this are cards: id `CARD_ID_BASE + i` is the
/// terminal of `store.sessions[i]`.
pub const CARD_ID_BASE: u64 = 1_000;

/// The session index a custom box paints, if it is a card's terminal.
pub fn card_index(id: u64) -> Option<usize> {
    id.checked_sub(CARD_ID_BASE)
        .and_then(|i| usize::try_from(i).ok())
}

/// The inline rename field of a session.
pub fn rename_id(session: &str) -> Id {
    Id::new("rename").child(session)
}

/// A shell's intent bar.
pub fn intent_id(session: &str) -> Id {
    Id::new("intent").child(session)
}

/// The terminal's box: right-click opens its menu, the wheel scrolls it.
pub fn term_id(session: &str) -> Id {
    Id::new("term").child(session)
}

/// Which cards a frame shows: all of them when they fit; otherwise the
/// page of rows holding the selected card, since the grid does not scroll.
pub(super) fn visible(
    n: usize,
    selected: Option<usize>,
    size: (f32, f32),
    columns: i32,
) -> (layout::Layout, usize, usize) {
    let mut l = layout::pick(n, size.0, size.1);
    if columns > 0 && n > 0 {
        let cols = (columns as usize).min(n);
        let rows = n.div_ceil(cols);
        let fit = layout::fit_max_rows(size.1);
        l = layout::Layout {
            cols,
            rows,
            mode: if rows <= fit { Mode::Fit } else { Mode::Scroll },
        };
    }
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

/// AgentStatusIcon: the agent's own icon, or the running glyph while an
/// agent works. A shell is always its terminal icon.
pub(super) fn status_icon(s: &Screen<'_>, session: &Session, size: f32, dim: f32) -> El {
    use crate::components::{status_icon, Status};
    let status = Status::parse(session.status());
    status_icon(s.ui, s.agents, &session.info.agent_type, status, size, dim)
}

/// ConfirmPopover under `trigger`, its buttons running `ok` and `cancel`.
fn confirm(s: &Screen<'_>, trigger: El, message: &str, ok: Act, cancel: Act, label: &str) -> El {
    let ok = (s.acts.on("confirm", ok), label);
    let cancel = s.acts.on("confirm", cancel);
    crate::ui::widgets::confirm(
        s.ui,
        trigger,
        message,
        ok,
        cancel,
        crate::ui::widgets::Tone::Danger,
    )
}

/// CardActionCluster: more actions, minimize on a grid card, expand or
/// collapse, and close behind its confirmation.
fn actions(s: &Screen<'_>, session: &Session, focused: bool) -> El {
    use crate::components::card::{card_actions_with, CardActions, CardButton, CardVariant};
    let sid = session.info.id.clone();
    let (variant, toggle, key) = if focused {
        (CardVariant::Focused, Act::Collapse, 'W')
    } else {
        (CardVariant::Mini, Act::Expand(sid.clone()), 'O')
    };
    let confirming = (s.st.confirm_close.as_deref() == Some(sid.as_str())).then(|| {
        (
            s.acts.on("confirm", Act::CloseSession(sid.clone())),
            s.acts.on("confirm", Act::AskClose(String::new())),
        )
    });
    let specs = super::side::header_buttons(s, session);
    let side: Vec<CardButton<'_>> = specs
        .iter()
        .map(|(act, icon, label)| CardButton {
            id: s.acts.on("card", act.clone()),
            icon,
            label,
            tip: label,
            shortcut: None,
            danger: false,
        })
        .collect();
    card_actions_with(
        s.ui,
        &CardActions {
            variant,
            more: s.acts.on("card", Act::Open(MenuKind::Card(sid.clone()))),
            panes: &side,
            minimize: s.acts.on("card", Act::ToggleMinimized(sid.clone())),
            toggle: s.acts.on("card", toggle),
            close: s.acts.on("card", Act::AskClose(sid)),
            toggle_key: &format!("{MOD_LABEL}{key}"),
            confirming,
        },
        panes::card_buttons(s, session, variant),
    )
}

/// The title, or the inline rename field while renaming.
fn title(s: &Screen<'_>, session: &Session, size: f32, dim: f32) -> El {
    let sid = session.info.id.as_str();
    let name = session.title();
    if s.st.renaming.as_deref() == Some(sid) {
        return text_input(s.ui, rename_id(sid), "Rename session", "")
            .bg(white(0.06))
            .border(1.0, white(0.25))
            .rounded(4.0)
            .min_w(120.0)
            .grow();
    }
    let title_id = Id::new("title").child(sid);
    s.acts.double(title_id, Act::Expand(sid.to_owned()));
    let hot =
        s.ui.hovered(title_id) || s.ui.hovered(Acts::id("card", &Act::Rename(sid.to_owned())));
    let mut row = div().row().items_center().gap(4.0).min_w0().child(
        div()
            .id(title_id)
            .sense(Sense::DOUBLE | Sense::HOVER)
            .min_w0()
            .child(
                text(name.as_str(), size, fade(look::GRAY_300, 1.0, dim))
                    .weight(500)
                    .line_height(18.0)
                    .truncate()
                    .role(Role::Label, name.as_str()),
            ),
    );
    if hot {
        let id = s.acts.on("card", Act::Rename(sid.to_owned()));
        row = row.child(
            div()
                .id(id)
                .on(Action::Click)
                .role(Role::Button, "Rename session")
                .child(ico(
                    "pencil",
                    10.0,
                    2.0,
                    c(look::GRAY_500).mix(c(look::GRAY_300), s.ui.hover(id)),
                )),
        );
    }
    row
}

/// The `⌘N` badge an idle mini card shows where its actions appear.
fn position_badge(i: usize, dim: f32) -> El {
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
            .mono()
            .line_height(10.0)
            .silent(),
        )
}

/// CardHeader, mini variant: actions on hover, the position badge idle.
fn header(s: &Screen<'_>, session: &Session, i: usize, dim: f32, hot: bool) -> El {
    let right = if hot || s.st.confirm_close.as_deref() == Some(session.info.id.as_str()) {
        actions(s, session, false)
    } else if i < 9 {
        position_badge(i, dim)
    } else {
        div()
    };
    let row = div()
        .row()
        .items_center()
        .gap(8.0)
        .px(12.0)
        .h(look::CARD_HEADER_H - 1.0)
        .shrink0()
        .child(status_icon(s, session, 18.0, dim))
        .child(
            div()
                .row()
                .grow()
                .min_w0()
                .child(title(s, session, 13.0, dim)),
        )
        .child(right);
    div()
        .col()
        .shrink0()
        .child(row)
        .child(div().h(1.0).w_full().bg(white(0.04)))
}

/// FocusedNavHint: where this session sits among the others, and the
/// arrows to its neighbours.
fn nav_hint(s: &Screen<'_>, sid: &str) -> Option<El> {
    let ids = s.st.visible(s.store);
    let total = ids.len();
    let index = ids
        .iter()
        .position(|&i| s.store.sessions[i].info.id == sid)?;
    if total < 2 {
        return None;
    }
    let neighbour = |d: usize| s.store.sessions[ids[(index + d) % total]].info.id.clone();
    let arrow = |act: Act, name: &str, label: &str, tip: &str, keys: String| {
        let id = s.acts.on("nav", act);
        let b = div()
            .id(id)
            .on(Action::Click)
            .p(4.0)
            .rounded(4.0)
            .bg(white(0.0).mix(white(0.10), s.ui.hover(id)))
            .role(Role::Button, label)
            .child(ico(name, 14.0, 2.0, c(look::INK)));
        tooltip(s.ui, b, id, tip, Some(&keys))
    };
    Some(
        div()
            .row()
            .items_center()
            .gap(4.0)
            .pr(4.0)
            .shrink0()
            .child(
                text(format!("{}", index + 1), 11.0, c(look::GRAY_500))
                    .mono()
                    .line_height(14.0)
                    .silent(),
            )
            .child(
                text(" / ", 11.0, c(look::GRAY_700))
                    .mono()
                    .line_height(14.0)
                    .silent(),
            )
            .child(
                text(total.to_string(), 11.0, c(look::GRAY_500))
                    .mono()
                    .line_height(14.0)
                    .silent(),
            )
            .child(arrow(
                Act::Expand(neighbour(total - 1)),
                "chevron-left",
                "Previous cell",
                "Previous",
                format!("{MOD_LABEL}["),
            ))
            .child(arrow(
                Act::Expand(neighbour(1)),
                "chevron-right",
                "Next cell",
                "Next",
                format!("{MOD_LABEL}]"),
            )),
    )
}

/// CardHeader, focused variant: it stands in for the toolbar, so it carries
/// the nav cluster and the window controls; double-clicking it collapses.
fn focused_header(s: &Screen<'_>, session: &Session) -> El {
    let mut row = div()
        .row()
        .items_center()
        .gap(8.0)
        .pl(s.lead())
        .pr(if s.chrome { 0.0 } else { 12.0 })
        .h(look::TOP_BAR_H - 1.0)
        .bg(c(look::SURFACE_RAISED));
    if !s.st.sidebar {
        row = row.child(chrome::nav_cluster(s)).child(divider());
    }
    row = row
        .child(status_icon(s, session, 18.0, 1.0))
        .child(
            div()
                .row()
                .grow()
                .min_w0()
                .child(title(s, session, 13.0, 1.0)),
        )
        .child_if(nav_hint(s, &session.info.id))
        .child(actions(s, session, true))
        .child_if(chrome::window_controls(s));
    let bar = div()
        .col()
        .shrink0()
        .child(row)
        .child(div().h(1.0).w_full().bg(white(0.04)));
    if s.chrome || s.traffic {
        s.acts.double(chrome::TITLE_BAR, Act::Collapse);
        bar.id(chrome::TITLE_BAR)
            .sense(Sense::WINDOW_DRAG | Sense::DOUBLE)
    } else {
        let id = Id::new("focused-header");
        s.acts.double(id, Act::Collapse);
        bar.id(id).sense(Sense::DOUBLE)
    }
}

/// BranchChip: the worktree in ink-secondary, then the branch; a click
/// opens the branch switcher.
pub(super) fn branch_chip(sc: &Screen<'_>, s: &Session, dim: f32) -> Option<El> {
    let branch = s.info.branch.as_deref()?;
    let id = sc.acts.on("branch-chip", super::side::branch_act(s));
    let open = sc
        .st
        .menu
        .as_ref()
        .is_some_and(|m| m.kind == MenuKind::Branch(s.info.id.clone()));
    let mut chip = div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(4.0)
        .px(4.0)
        .py(2.0)
        .rounded(4.0)
        .bg(if open {
            white(0.08)
        } else {
            white(0.06 * sc.ui.hover(id))
        })
        .role(Role::Button, format!("Switch branch (current: {branch})"));
    if let (Some(true), Some(name)) = (s.info.is_worktree, s.info.worktree_name.as_deref()) {
        let ink = fade(look::INK, look::INK_SECONDARY, dim);
        chip = chip
            .child(ico("folder-git-2", 10.0, 1.5, ink))
            .child(
                text(truncate(name, 18), 10.0, ink)
                    .mono()
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
                .mono()
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
    if let Some(chip) = branch_chip(s, session, dim) {
        bar = bar.child(chip);
    }
    if let Some(chip) = task_chip(s, &session.info.id, dim) {
        bar = bar.child(chip);
    }
    bar = bar.child(div().grow());
    for item in super::side::status_items(s, session, dim) {
        bar = bar.child(item);
    }
    div()
        .col()
        .shrink0()
        .child(div().h(1.0).w_full().bg(white(0.04)))
        .child(bar)
}

/// CommandSpine: an 8 px gutter with its hairline and a mark per command
/// block, from each prompt to the next.
fn spine(s: &Screen<'_>, session: &Session) -> El {
    let mut gutter = div()
        .col()
        .items_center()
        .w(8.0)
        .shrink0()
        .child(div().w(1.0).grow().bg(white(0.035)));
    let Some(view) = s.panes.get(&session.info.id) else {
        return gutter;
    };
    if !session.is_shell() || view.prompts.is_empty() {
        return gutter;
    }
    let cell = s.ui.text.cell().h;
    let ends = view
        .prompts
        .iter()
        .skip(1)
        .copied()
        .chain([view.cursor_y.saturating_add(1)]);
    let last = view.prompts.len() - 1;
    for (n, (&start, end)) in view.prompts.iter().zip(ends).enumerate() {
        let rows = f32::from(end.saturating_sub(start).max(1));
        let (color, alpha) = if n == last && session.ended_cause().is_none() {
            (look::BLUE_400, 0.85)
        } else {
            (0x6b6b76, 0.55)
        };
        gutter = gutter.child(
            div()
                .absolute(3.0, f32::from(start) * cell + look::CARD_BODY_PT)
                .size(2.0, (rows * cell - 2.0).max(2.0))
                .rounded(1.0)
                .bg(Rgba::hexa(color, alpha))
                .role(Role::Image, "Command"),
        );
    }
    gutter
}

/// The terminal beside its spine, in a box that takes the right-click and
/// the wheel.
fn terminal(s: &Screen<'_>, i: usize, session: &Session) -> El {
    let sid = session.info.id.as_str();
    let id = term_id(sid);
    let inner = div()
        .row()
        .grow()
        .min_h(0.0)
        .child(spine(s, session))
        .child(div().w(8.0).shrink0())
        .child(custom(CARD_ID_BASE + i as u64).grow());
    s.acts.bind(id, Act::Select(sid.to_owned()));
    s.acts
        .context(inner, id, Act::Open(MenuKind::Terminal(sid.to_owned())))
        .on(Action::Click)
        .sense(Sense::WHEEL)
}

/// EndedStrip: why the session ended, and how to pick it back up.
fn ended_strip(s: &Screen<'_>, session: &Session, compact: bool) -> El {
    let sid = session.info.id.as_str();
    let cause = session.ended_cause().unwrap_or_else(|| "exited".to_owned());
    let mut copy = format!("Ended \u{b7} {cause}");
    if session.is_shell() && !compact {
        if let Some(cwd) = session.info.shell_cwd.as_deref().filter(|c| !c.is_empty()) {
            copy.push_str(&format!(" \u{b7} {cwd}"));
        }
    }
    let button = |act: Act, name: &str, label: &str| {
        let id = s.acts.on("ended", act);
        div()
            .id(id)
            .on(Action::Click)
            .row()
            .items_center()
            .gap(4.0)
            .px(6.0)
            .py(2.0)
            .rounded(4.0)
            .bg(white(0.0).mix(white(0.06), s.ui.hover(id)))
            .role(Role::Button, label)
            .child(ico(name, 11.0, 2.0, c(look::GRAY_300)))
            .child(
                text(label, 11.0, c(look::GRAY_300))
                    .line_height(14.0)
                    .silent(),
            )
    };
    let action = if session.is_shell() {
        button(Act::Resume(sid.to_owned()), "terminal", "New shell here")
    } else {
        button(Act::Resume(sid.to_owned()), "play", "Resume")
    };
    let close_id = s.acts.on("ended", Act::CloseSession(sid.to_owned()));
    let close = div()
        .id(close_id)
        .on(Action::Click)
        .p(2.0)
        .rounded(4.0)
        .role(Role::Button, "Close this pane")
        .child(ico(
            "x",
            11.0,
            2.0,
            c(look::GRAY_500).mix(c(look::GRAY_300), s.ui.hover(close_id)),
        ));
    let close = tooltip(s.ui, close, close_id, "Close this pane", None);
    div()
        .col()
        .shrink0()
        .role(Role::Status, copy.as_str())
        .child(div().h(1.0).w_full().bg(white(0.06)))
        .child(
            div()
                .row()
                .items_center()
                .gap(10.0)
                .py(6.0)
                .pl(12.0)
                .pr(8.0)
                .child(
                    div()
                        .size(7.0, 7.0)
                        .bg(Rgba::hexa(look::INK, look::INK_FAINT)),
                )
                .child(
                    div().row().grow().min_w0().child(
                        text(
                            copy.as_str(),
                            11.0,
                            Rgba::hexa(look::INK, look::INK_SECONDARY),
                        )
                        .line_height(14.0)
                        .truncate()
                        .silent(),
                    ),
                )
                .child(action)
                .child(close),
        )
}

/// IntentBar, command mode: a line typed here runs in the shell on Enter.
fn intent_bar(s: &Screen<'_>, session: &Session, compact: bool) -> El {
    let sid = session.info.id.as_str();
    let id = intent_id(sid);
    let focused = s.ui.focused(id);
    let glyph = text(
        "\u{276f}",
        13.0,
        c(if focused {
            look::GRAY_300
        } else {
            look::GRAY_600
        }),
    )
    .mono()
    .line_height(19.0)
    .role(Role::Label, "Command mode");
    let input = field(
        id,
        "Type a command",
        "Type a command or send a prompt for the agent",
        TextStyle {
            line_height: 19.0,
            ..TextStyle::mono(13.0)
        },
        c(look::GRAY_200),
    )
    .grow()
    .min_w0();
    let row = div()
        .row()
        .gap(8.0)
        .pl(16.0)
        .pr(8.0)
        .pt(if compact { 4.0 } else { 6.0 })
        .pb(4.0)
        .child(glyph)
        .child(input);
    let mut col = div()
        .col()
        .shrink0()
        .bg(if focused {
            white(0.02)
        } else {
            c(look::SURFACE_SUNKEN)
        })
        .child(div().h(1.0).w_full().bg(white(0.05)))
        .child(row);
    if !compact {
        let cwd = session.info.shell_cwd.as_deref().unwrap_or("");
        let shown = focused || !s.ui.edit_text(id).is_empty();
        let tone = Rgba::hexa(look::GRAY_700, if shown { 1.0 } else { 0.0 });
        col = col.child(
            div()
                .row()
                .items_center()
                .gap(8.0)
                .pl(16.0)
                .pr(8.0)
                .pb(4.0)
                .child(ico("folder", 9.0, 2.0, tone))
                .child(
                    text(truncate(cwd, 40), 10.0, tone)
                        .mono()
                        .line_height(12.0)
                        .silent(),
                )
                .child(div().grow())
                .child(text("\u{21b5} run", 10.0, tone).line_height(12.0).silent()),
        );
    }
    col
}

/// SessionComposer: the ended strip, or a shell's intent bar.
pub(super) fn composer(s: &Screen<'_>, session: &Session, compact: bool) -> Option<El> {
    if session.ended() {
        return Some(ended_strip(s, session, compact));
    }
    session.is_shell().then(|| intent_bar(s, session, compact))
}

/// The terminal and its composer: what every layout shows of a session
/// under its header or tab.
fn term_body(s: &Screen<'_>, i: usize, session: &Session, compact: bool) -> El {
    div()
        .col()
        .grow()
        .min_h(0.0)
        .bg(c(look::SURFACE_SUNKEN))
        .child(div().h(look::CARD_BODY_PT).shrink0())
        .child(terminal(s, i, session))
        .child_if(composer(s, session, compact))
}

/// The terminal with its status bar, beside the session's panes.
fn body(s: &Screen<'_>, i: usize, session: &Session, compact: bool, dim: f32) -> El {
    super::side::frame(
        s,
        session,
        None,
        term_body(s, i, session, compact),
        status_bar(s, session, dim),
    )
}

fn card(s: &Screen<'_>, i: usize, pos: usize, session: &Session) -> El {
    let sid = session.info.id.as_str();
    let selected = s.st.selected.as_deref() == Some(sid);
    let card_id = Acts::id("card-box", &Act::Select(sid.to_owned()));
    let hot = s.ui.hover(card_id) > 0.0 || s.ui.hovered(term_id(sid));
    let dim = if s.st.selected.is_some() && !selected && !hot {
        0.6
    } else {
        1.0
    };
    let el = div()
        .col()
        .grow()
        .min_w0()
        .bg(c(look::SURFACE_RAISED))
        .border(1.0, white(0.06).mix(white(0.12), s.ui.hover(card_id)))
        .role(Role::Group, session.title())
        .selected(selected)
        .child(super::side::frame(
            s,
            session,
            Some(header(s, session, pos, dim, hot)),
            term_body(s, i, session, true),
            status_bar(s, session, dim),
        ))
        .child(div().h(look::CARD_FOOT_H).shrink0());
    s.acts
        .click(el, "card-box", Act::Select(sid.to_owned()))
        .sense(Sense::HOVER)
}

/// The grid's empty state when the filter hides every card.
/// GridView's filtered-out state: a terminal on a monitor, and the
/// heading over its hint.
const NO_MATCH_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#ffffff" stroke-width="1"><rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8M12 17v4"/><path d="M7 8l3 3-3 3M12 14h4"/></svg>"##;

fn no_match() -> El {
    div()
        .col()
        .center()
        .grow()
        .child(icon(NO_MATCH_SVG, 64.0, white(0.2)).mb(24.0))
        .child(
            text("No matching agents", 24.0, c(look::WHITE))
                .weight(600)
                .line_height(32.0)
                .role(Role::Heading, "No matching agents"),
        )
        .child(
            text("Try changing the status filter", 14.0, c(look::GRAY_500))
                .line_height(20.0)
                .mt(8.0)
                .mb(24.0)
                .silent(),
        )
}

/// A grid cell: a session's card, or a file popped out of one.
enum Cell<'a> {
    Session(usize),
    Card(&'a str),
}

/// The grid's cells: each session, then the cards it popped out.
fn cells<'a>(s: &'a Screen<'_>) -> Vec<Cell<'a>> {
    let mut out = Vec::new();
    for i in s.st.gridded(s.store) {
        out.push(Cell::Session(i));
        for card in s.st.side.cards_of(&s.store.sessions[i].info.id) {
            out.push(Cell::Card(card));
        }
    }
    out
}

pub(super) fn grid(s: &Screen<'_>) -> El {
    let cells = cells(s);
    if cells.is_empty() {
        return no_match();
    }
    let selected = s.st.selected.as_deref().and_then(|id| {
        cells.iter().position(|cell| match cell {
            Cell::Session(i) => s.store.sessions[*i].info.id == id,
            Cell::Card(c) => *c == id,
        })
    });
    let (l, first, rows) = visible(cells.len(), selected, s.grid_size(), s.st.columns);
    let mut body = div().col().grow().min_h(0.0).role(Role::Grid, "Sessions");
    let grid_id = Id::new("grid");
    for r in first..first + rows {
        let mut row = div().row().grow().min_h(0.0);
        for col in 0..l.cols {
            let n = r * l.cols + col;
            row = row.child(match cells.get(n) {
                Some(Cell::Session(i)) => card(s, *i, n, &s.store.sessions[*i]).flex(1.0),
                Some(Cell::Card(c)) => super::side::editor_card(s, c).flex(1.0),
                None => div().grow(),
            });
        }
        body = body.child(row);
    }
    s.acts.context(body, grid_id, Act::Open(MenuKind::Grid))
}

/// FocusedStage: one session filling the window under its header.
pub(super) fn focused(s: &Screen<'_>) -> El {
    let Some((i, session)) = s.st.focused.as_deref().and_then(|id| {
        s.store
            .sessions
            .iter()
            .enumerate()
            .find(|(_, x)| x.info.id == id)
    }) else {
        return grid(s);
    };
    div()
        .col()
        .grow()
        .min_h(0.0)
        .bg(c(look::SURFACE_RAISED))
        .role(Role::Group, session.title())
        .child(focused_header(s, session))
        .child(body(s, i, session, false, 1.0))
}

/// One tab: status icon, title, the position badge or its actions.
fn tab(s: &Screen<'_>, n: usize, session: &Session, active: bool) -> El {
    let sid = session.info.id.as_str();
    let id = Acts::id("tab", &Act::Select(sid.to_owned()));
    let close_id = Acts::id("tab", &Act::AskClose(sid.to_owned()));
    let rename_btn = Acts::id("tab", &Act::Rename(sid.to_owned()));
    let hot = s.ui.hovered(id) || s.ui.hovered(close_id) || s.ui.hovered(rename_btn);
    let fg = if active {
        c(look::WHITE)
    } else {
        c(look::GRAY_500).mix(c(look::GRAY_300), if hot { 1.0 } else { 0.0 })
    };
    let renaming = s.st.renaming.as_deref() == Some(sid);
    let label = if renaming {
        text_input(s.ui, rename_id(sid), "Rename session", "")
            .w(100.0)
            .bg(white(0.06))
            .border(1.0, white(0.25))
    } else {
        div().row().grow().min_w0().child(
            text(session.title(), 13.0, fg)
                .line_height(18.0)
                .truncate()
                .silent(),
        )
    };
    let right = if hot && !renaming {
        let small = |act: Act, name: &str, label: &str| {
            let bid = s.acts.on("tab", act);
            let b = div()
                .id(bid)
                .on(Action::Click)
                .p(2.0)
                .rounded(4.0)
                .bg(white(0.0).mix(white(0.10), s.ui.hover(bid)))
                .role(Role::Button, label)
                .child(ico(name, 12.0, 2.0, c(look::GRAY_400)));
            tooltip(s.ui, b, bid, label, None)
        };
        let mut close = small(Act::AskClose(sid.to_owned()), "x", "Close session");
        if s.st.confirm_close.as_deref() == Some(sid) {
            close = confirm(
                s,
                close,
                "Close this session?",
                Act::CloseSession(sid.to_owned()),
                Act::AskClose(String::new()),
                "Close",
            );
        }
        Some(
            div()
                .row()
                .items_center()
                .gap(4.0)
                .bg(c(look::SURFACE_OVERLAY))
                .child(small(
                    Act::Rename(sid.to_owned()),
                    "pencil",
                    "Rename session",
                ))
                .child(close),
        )
    } else if n < 9 && !renaming {
        Some(
            div()
                .px(4.0)
                .py(2.0)
                .rounded(4.0)
                .bg(white(0.06))
                .border(1.0, white(0.1))
                .child(
                    text(format!("{MOD_LABEL}{}", n + 1), 9.0, c(look::GRAY_500))
                        .mono()
                        .line_height(9.0)
                        .silent(),
                ),
        )
    } else {
        None
    };
    s.acts.double(id, Act::Rename(sid.to_owned()));
    let el = div()
        .col()
        .flex(1.0)
        .min_w(120.0)
        .max_w(260.0)
        .h(36.0)
        .role(Role::Tab, session.title())
        .selected(active)
        .child(
            div()
                .row()
                .items_center()
                .gap(8.0)
                .pl(12.0)
                .pr(8.0)
                .grow()
                .child(status_icon(s, session, 16.0, 1.0))
                .child(label)
                .child_if(right),
        )
        .child(
            div()
                .h(1.0)
                .w_full()
                .bg(if active { c(look::WHITE) } else { white(0.0) }),
        );
    let el = s.acts.click(el, "tab", Act::Select(sid.to_owned()));
    s.acts
        .context(el, id, Act::Open(MenuKind::Card(sid.to_owned())))
        .sense(Sense::DOUBLE | Sense::HOVER)
}

/// TabView: the tab strip over the active session.
pub(super) fn tabs(s: &Screen<'_>) -> El {
    let shown = s.st.visible(s.store);
    let active =
        s.st.selected
            .as_deref()
            .and_then(|id| {
                shown
                    .iter()
                    .copied()
                    .find(|&i| s.store.sessions[i].info.id == id)
            })
            .or_else(|| shown.first().copied());
    let mut strip = div()
        .row()
        .items_center()
        .min_h(40.0)
        .bg(c(look::SURFACE_OVERLAY));
    if s.traffic && !s.st.sidebar {
        strip = strip.pl(look::TRAFFIC_LIGHT_PAD);
    }
    if !s.st.sidebar {
        strip = strip.child(div().pl(8.0).child(chrome::nav_cluster(s)));
    }
    let mut list = div()
        .row()
        .items_center()
        .gap(4.0)
        .px(4.0)
        .grow()
        .min_w0()
        .role(Role::TabList, "Sessions");
    for (n, &i) in shown.iter().enumerate() {
        list = list.child(tab(s, n, &s.store.sessions[i], Some(i) == active));
    }
    let plus_id = s.acts.on("tabs", Act::QuickSession);
    let more_id = s.acts.on("tabs", Act::Open(MenuKind::Grid));
    let square = |id: Id, w: f32, name: &str, label: &str, size: f32| {
        div()
            .id(id)
            .on(Action::Click)
            .size(w, 36.0)
            .center()
            .rounded(6.0)
            .bg(white(0.0).mix(white(0.10), s.ui.hover(id)))
            .role(Role::Button, label)
            .child(ico(
                name,
                size,
                1.5,
                c(look::GRAY_200).mix(c(look::WHITE), s.ui.hover(id)),
            ))
    };
    list = list
        .child(square(plus_id, 28.0, "plus", "New session", 14.0))
        .child(square(
            more_id,
            22.0,
            "chevron-down",
            "More session options",
            12.0,
        ));
    strip = strip.child(list).child(
        div()
            .row()
            .items_center()
            .gap(4.0)
            .pl(8.0)
            .child(chrome::left_extras(s))
            .child(chrome::right_cluster(s, false)),
    );
    let strip = draggable(
        s,
        div()
            .col()
            .shrink0()
            .child(strip)
            .child(div().h(1.0).w_full().bg(white(0.06))),
    );
    let content = match active {
        Some(i) => body(s, i, &s.store.sessions[i], false, 1.0),
        None if s.store.sessions.is_empty() => super::composer::empty_state(s),
        None => no_match(),
    };
    div()
        .col()
        .grow()
        .min_h(0.0)
        .child(strip)
        .child_if(chrome::banner(s))
        .child(content)
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
        let (l, first, rows) = visible(20, None, size, 0);
        assert_eq!((l.cols, l.mode, first, rows), (4, Mode::Scroll, 0, 3));
        assert_eq!(visible(20, Some(19), size, 0).1, 2);
        assert_eq!(visible(20, Some(5), size, 0).1, 0);
        let (_, first, rows) = visible(3, Some(2), size, 0);
        assert_eq!((first, rows), (0, 1));
    }

    #[test]
    fn a_chosen_column_count_overrides_the_automatic_one() {
        let size = (1440.0, 860.0);
        let (l, _, rows) = visible(4, None, size, 1);
        assert_eq!((l.cols, l.rows), (1, 4));
        assert!(rows <= 4);
        assert_eq!(visible(4, None, size, 2).0.cols, 2);
        assert_eq!(visible(1, None, size, 3).0.cols, 1);
    }
}
