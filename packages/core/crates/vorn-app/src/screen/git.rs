//! A session's git on screen: `GitChangesIndicator` and `OpenInButton` in
//! the card's status bar, the `BranchPicker` popover, the Changes panel
//! (`RightPanel`), `WorktreeCleanupDialog`, and the worktree toasts.

use super::tasks::diff::{self, Scope};
use super::{c, ico, sidebar_project_icon, truncate, white, Screen};
use crate::act::Act;
use crate::client::Session;
use crate::git::{files_label, CommitOp, DiffOp, GitAct};
use crate::look::{self, AMBER_500, BLUE_300, BLUE_500, DIFF_ADD, DIFF_REMOVE, RED_300};
use crate::ui::{div, field, icon, text, tooltip, Action, El, Id, Rgba, Role, Sense, TextStyle};

pub fn commit_id() -> Id {
    Id::new("git-commit-message")
}

pub fn comment_id() -> Id {
    Id::new("git-comment")
}

pub fn branch_filter_id() -> Id {
    Id::new("git-branch-filter")
}

/// The row the Changes panel sits in, which its width is measured against.
pub fn changes_row_id() -> Id {
    Id::new("changes-row")
}

/// The Changes panel's resize handle.
pub fn changes_resize_id() -> Id {
    Id::new("changes-resize")
}

fn git(a: GitAct) -> Act {
    Act::Git(a)
}

/// `GitChangesIndicator`: `+N -N` once a session has changes.
pub(super) fn indicator(s: &Screen<'_>, session: &Session, dim: f32) -> Option<El> {
    let st = s.st.git.stats.get(&session.info.id)?;
    if !st.changed() {
        return None;
    }
    let id = s
        .acts
        .on("git-indicator", git(GitAct::Open(session.info.id.clone())));
    let el = div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(6.0)
        .px(6.0)
        .py(2.0)
        .rounded(4.0)
        .bg(white(0.06 * s.ui.hover(id)))
        .role(Role::Button, format!("{} changed", files_label(st.files)))
        .child(
            text(format!("+{}", st.insertions), 11.0, c(DIFF_ADD).alpha(dim))
                .mono()
                .line_height(14.0)
                .silent(),
        )
        .child(
            text(
                format!("-{}", st.deletions),
                11.0,
                c(DIFF_REMOVE).alpha(dim),
            )
            .mono()
            .line_height(14.0)
            .silent(),
        );
    Some(tooltip(
        s.ui,
        el,
        id,
        &format!("{} changed", files_label(st.files)),
        None,
    ))
}

/// An editor's mark, tinted like the text beside it.
fn ide_mark(id: &str, size: f32, color: Rgba) -> El {
    match crate::git::ide_icon(id) {
        Some(svg) => icon(svg, size, color),
        None => div().size(size, size),
    }
}

/// `OpenInButton`, opening upwards from a card's status bar.
pub(super) fn open_in(s: &Screen<'_>, session: &Session, dim: f32) -> Option<El> {
    let ide = s.st.git.ides.as_ref()?.first()?;
    let path = session.info.project_path.clone();
    let open = s.acts.on("open-in", git(GitAct::OpenIn(path, None)));
    let more = s.acts.on(
        "open-in-more",
        git(GitAct::OpenInMenu(session.info.id.clone())),
    );
    let tint = |id| {
        c(look::GRAY_500)
            .mix(c(look::GRAY_300), s.ui.hover(id))
            .alpha(dim)
    };
    let main = div()
        .id(open)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(4.0)
        .px(4.0)
        .py(2.0)
        .bg(white(0.06 * s.ui.hover(open)))
        .role(Role::Button, format!("Open in {}", ide.name))
        .child(ide_mark(&ide.id, 11.0, tint(open)))
        .child(text("Open", 10.0, tint(open)).line_height(12.0).silent());
    let chevron = div()
        .id(more)
        .on(Action::Click)
        .row()
        .items_center()
        .px(2.0)
        .py(3.0)
        .bg(white(0.06 * s.ui.hover(more)))
        .role(Role::Button, "Choose IDE")
        .child(ico("chevron-down", 9.0, 2.0, tint(more)));
    let main = tooltip(s.ui, main, open, &format!("Open in {}", ide.name), None);
    Some(
        div()
            .row()
            .items_center()
            .rounded(4.0)
            .child(main)
            .child(chevron),
    )
}

/// The `Open in` menu: its title and each editor with its mark.
pub(super) fn open_in_panel(s: &Screen<'_>, session: &str) -> El {
    let Some(path) = s
        .store
        .session(session)
        .map(|x| x.info.project_path.clone())
    else {
        return div();
    };
    let mut col = div()
        .col()
        .w(180.0)
        .py(4.0)
        .rounded(8.0)
        .border(1.0, white(0.08))
        .bg(c(look::SURFACE_OVERLAY))
        .role(Role::Menu, "Open in")
        .child(
            div().px(12.0).py(6.0).child(
                text("Open in", 11.0, c(look::GRAY_500))
                    .weight(500)
                    .line_height(16.0)
                    .silent(),
            ),
        );
    for ide in s.st.git.ides.iter().flatten() {
        let id = s.acts.on(
            "open-in-pick",
            git(GitAct::OpenIn(path.clone(), Some(ide.id.clone()))),
        );
        let hot = s.ui.hover(id);
        let fg = c(look::GRAY_300).mix(c(look::WHITE), hot);
        col = col.child(
            div()
                .id(id)
                .on(Action::Click)
                .row()
                .items_center()
                .gap(10.0)
                .px(12.0)
                .py(6.0)
                .bg(white(0.06 * hot))
                .role(Role::MenuItem, ide.name.as_str())
                .child(ide_mark(&ide.id, 14.0, fg))
                .child(text(&ide.name, 13.0, fg).line_height(18.0).silent()),
        );
    }
    col
}

/// `BranchPicker`: the filter, the fetch, and the branches.
pub(super) fn branch_panel(s: &Screen<'_>) -> El {
    let Some(b) = &s.st.git.branches else {
        return div();
    };
    let fetch = s.acts.on("branch-fetch", git(GitAct::FetchRemotes));
    let head = div()
        .row()
        .items_center()
        .gap(4.0)
        .p(8.0)
        .border_bottom(1.0, white(0.06))
        .child(
            div()
                .grow()
                .px(8.0)
                .py(4.0)
                .frames(branch_filter_id())
                .child(
                    field(
                        branch_filter_id(),
                        "Filter branches",
                        "Filter branches...",
                        TextStyle::ui(12.0),
                        c(look::GRAY_200),
                    )
                    .w_full(),
                ),
        )
        .child(tooltip(
            s.ui,
            div()
                .id(fetch)
                .on(Action::Click)
                .p(4.0)
                .role(Role::Button, "Fetch remotes")
                .child(ico(
                    if b.fetching {
                        "loader-circle"
                    } else {
                        "refresh-cw"
                    },
                    10.0,
                    2.0,
                    c(look::GRAY_500).mix(c(look::GRAY_300), s.ui.hover(fetch)),
                )),
            fetch,
            "Fetch remotes",
            None,
        ));
    let filter = s.ui.edit_text(branch_filter_id());
    let shown = b.shown(filter);
    let mut list = div()
        .id(Id::new("branch-list"))
        .col()
        .py(4.0)
        .max_h(232.0)
        .scroll_y();
    if b.loading {
        list = list.child(div().row().justify_center().py(12.0).child(ico(
            "loader-circle",
            14.0,
            2.0,
            c(look::GRAY_500),
        )));
    } else if shown.is_empty() {
        list = list.child(
            div().px(12.0).py(8.0).child(
                text("No branches found", 12.0, c(look::GRAY_500))
                    .line_height(16.0)
                    .silent(),
            ),
        );
    }
    let current = s
        .store
        .session(&b.session)
        .and_then(|x| x.info.branch.clone());
    for (name, remote) in shown.into_iter().filter(|_| !b.loading) {
        let id = s
            .acts
            .on("branch-pick", git(GitAct::Switch(name.to_owned())));
        let active = current.as_deref() == Some(name);
        let mut row = div()
            .id(id)
            .on(Action::Click)
            .row()
            .items_center()
            .gap(8.0)
            .px(12.0)
            .py(6.0)
            .bg(if active {
                white(0.04)
            } else {
                white(0.06 * s.ui.hover(id))
            })
            .role(Role::MenuItem, name)
            .child(ico(
                "git-branch",
                10.0,
                2.0,
                if remote {
                    c(look::BLUE_400)
                } else {
                    c(look::GRAY_500)
                },
            ))
            .child(
                div().grow().min_w0().child(
                    text(
                        name,
                        12.0,
                        if active {
                            c(look::WHITE)
                        } else {
                            c(look::GRAY_400)
                        },
                    )
                    .line_height(16.0)
                    .truncate()
                    .silent(),
                ),
            );
        if remote {
            row = row.child(
                text("remote", 9.0, c(look::BLUE_400).alpha(0.6))
                    .line_height(12.0)
                    .silent(),
            );
        }
        if active {
            row = row.child(
                text("current", 9.0, c(look::GREEN_400).alpha(0.6))
                    .line_height(12.0)
                    .silent(),
            );
        }
        list = list.child(row);
    }
    div()
        .col()
        .min_w(220.0)
        .max_h(280.0)
        .rounded(8.0)
        .border(1.0, white(0.08))
        .bg(c(look::SURFACE_OVERLAY))
        .role(Role::Menu, "Branches")
        .child(head)
        .child(list)
}

/// The panel's `p-1` icon buttons.
fn panel_button(s: &Screen<'_>, act: Act, name: &str, label: &str, base: u32) -> El {
    let id = s.acts.on("changes", act);
    let b = div()
        .id(id)
        .on(Action::Click)
        .p(4.0)
        .rounded(4.0)
        .role(Role::Button, label)
        .child(ico(
            name,
            13.0,
            1.5,
            c(base).mix(c(look::WHITE), s.ui.hover(id)),
        ));
    tooltip(s.ui, b, id, label, None)
}

/// `RightPanel`: the open session's changes, beside the sessions.
pub(super) fn right_panel(s: &Screen<'_>) -> Option<El> {
    let sid = s.st.git.panel.as_deref()?;
    let session = s.store.session(sid)?;
    let info = &session.info;
    let g = &s.st.git;
    let stat = g.stats.get(sid).copied();
    let gray = c(look::GRAY_500);
    // The project, worktree and branch the diff is of.
    let mut ctx = div()
        .row()
        .items_center()
        .gap(8.0)
        .px(12.0)
        .py(6.0)
        .border_bottom(1.0, white(0.06))
        .shrink0();
    // The sidebar's active project, as today's header reads it.
    let project =
        s.st.project
            .as_deref()
            .and_then(|n| s.store.projects.iter().find(|p| p.name == n));
    let mut parts = 0;
    if let Some(p) = project {
        ctx = ctx.child(
            div()
                .row()
                .items_center()
                .gap(4.0)
                .child(sidebar_project_icon(p, 11.0, gray))
                .child(
                    text(truncate(&p.name, 16), 11.0, gray)
                        .line_height(16.0)
                        .silent(),
                ),
        );
        parts += 1;
    }
    if let Some(wt) = info.worktree_name.as_deref().filter(|w| !w.is_empty()) {
        if parts > 0 {
            ctx = ctx.child(
                text("/", 11.0, c(look::GRAY_700))
                    .line_height(16.0)
                    .silent(),
            );
        }
        ctx = ctx.child(
            div()
                .row()
                .items_center()
                .gap(4.0)
                .child(ico("folder-git-2", 11.0, 1.5, c(AMBER_500)))
                .child(
                    text(truncate(wt, 16), 11.0, gray)
                        .line_height(16.0)
                        .silent(),
                ),
        );
        parts += 1;
    }
    if let Some(br) = info.branch.as_deref().filter(|b| !b.is_empty()) {
        if parts > 0 {
            ctx = ctx.child(
                text("/", 11.0, c(look::GRAY_700))
                    .line_height(16.0)
                    .silent(),
            );
        }
        ctx = ctx.child(
            div()
                .row()
                .items_center()
                .gap(4.0)
                .min_w0()
                .child(ico("git-branch", 11.0, 1.5, c(look::GRAY_600)))
                .child(text(br, 11.0, gray).line_height(16.0).truncate().silent()),
        );
    }
    let tab = div()
        .px(10.0)
        .py(4.0)
        .rounded(6.0)
        .bg(white(0.1))
        .role(Role::Tab, "Changes")
        .selected(true)
        .child(
            text("Changes", 11.0, c(look::GRAY_200))
                .weight(500)
                .line_height(16.0)
                .silent(),
        );
    let (max_icon, max_label) = if g.maximized {
        ("minimize-2", "Restore size")
    } else {
        ("maximize-2", "Maximize")
    };
    let tabs = div()
        .row()
        .items_center()
        .gap(2.0)
        .h(36.0)
        .px(8.0)
        .border_bottom(1.0, white(0.06))
        .shrink0()
        .child(tab)
        .child(div().grow())
        .child(panel_button(
            s,
            git(GitAct::Maximize),
            max_icon,
            max_label,
            look::GRAY_500,
        ))
        .child(panel_button(
            s,
            git(GitAct::Close),
            "x",
            "Close panel",
            look::GRAY_500,
        ));
    let mut head = div()
        .row()
        .items_center()
        .gap(12.0)
        .px(12.0)
        .py(8.0)
        .border_bottom(1.0, white(0.06))
        .shrink0();
    if let Some(st) = stat {
        head = head.child(
            div()
                .row()
                .items_center()
                .gap(6.0)
                .child(
                    text(format!("+{}", st.insertions), 11.0, c(look::GREEN_400))
                        .mono()
                        .line_height(16.0)
                        .silent(),
                )
                .child(
                    text(format!("-{}", st.deletions), 11.0, c(look::RED_400))
                        .mono()
                        .line_height(16.0)
                        .silent(),
                )
                .child(
                    text(files_label(st.files), 11.0, gray)
                        .mono()
                        .line_height(16.0)
                        .silent(),
                ),
        );
    }
    head = head.child(div().grow());
    if stat.is_some_and(|st| st.changed()) {
        let id = s
            .acts
            .on("changes-commit", git(GitAct::Commit(CommitOp::Open)));
        let hot = s.ui.hover(id);
        let b = div()
            .id(id)
            .on(Action::Click)
            .row()
            .items_center()
            .gap(6.0)
            .px(8.0)
            .py(4.0)
            .rounded(6.0)
            .bg(white(0.04).mix(white(0.08), hot))
            .border(1.0, white(0.06))
            .role(Role::Button, "Commit changes")
            .child(ico(
                "git-commit-horizontal",
                13.0,
                1.5,
                c(look::GRAY_300).mix(c(look::GRAY_100), hot),
            ))
            .child(
                text(
                    "Commit",
                    11.0,
                    c(look::GRAY_300).mix(c(look::GRAY_100), hot),
                )
                .weight(500)
                .line_height(16.0)
                .silent(),
            );
        head = head.child(tooltip(s.ui, b, id, "Commit changes", None));
    }
    let refresh_icon = if g.diff.loading {
        "loader-circle"
    } else {
        "refresh-cw"
    };
    head = head.child(panel_button(
        s,
        git(GitAct::Refresh),
        refresh_icon,
        "Refresh",
        look::GRAY_400,
    ));
    let mut col = div()
        .col()
        .grow()
        .min_h(0.0)
        .child(ctx)
        .child(tabs)
        .child(head);
    if !g.diff.comments.is_empty() {
        col = col.child(feedback(s));
    }
    let d = &g.diff;
    if d.loading && d.files.is_empty() {
        col = col.child(
            div()
                .col()
                .center()
                .grow()
                .child(ico("loader-circle", 20.0, 2.0, gray)),
        );
    } else if !d.files.is_empty() {
        col = col
            .child(diff::file_list(s, Scope::Session, &d.files))
            .child(
                div()
                    .id(Id::new("changes-scroll"))
                    .col()
                    .grow()
                    .min_h(0.0)
                    .scroll_y()
                    .child(diff::content(s, Scope::Session, &d.files)),
            );
    } else {
        col = col.child(
            div()
                .col()
                .center()
                .grow()
                .child(text("No changes", 14.0, gray).line_height(20.0).silent()),
        );
    }
    let handle = div()
        .id(changes_resize_id())
        .sense(Sense::DRAG)
        .absolute(0.0, 0.0)
        .w(4.0)
        .h_full()
        .bg(white(0.14 * s.ui.hover(changes_resize_id())))
        .role(Role::Splitter, "Resize changes panel")
        .vertical(true);
    let w = if g.maximized {
        (s.size.0 - 48.0).max(look::MIN_CARD.0)
    } else {
        g.width()
    };
    Some(
        div()
            .col()
            .w(w)
            .shrink0()
            .min_h(0.0)
            .border_left(1.0, white(0.06))
            .bg(c(look::SURFACE_PANEL))
            .role(Role::Group, "Changes")
            .child(col)
            .child(handle),
    )
}

/// The panel's review feedback bar, in blue.
fn feedback(s: &Screen<'_>) -> El {
    let n = s.st.git.diff.comments.len();
    let clear = s
        .acts
        .on("changes-clear", git(GitAct::Diff(DiffOp::ClearComments)));
    let send = s
        .acts
        .on("changes-send", git(GitAct::Diff(DiffOp::SendFeedback)));
    let shot = s.ui.hover(send);
    div()
        .row()
        .items_center()
        .gap(8.0)
        .px(12.0)
        .py(8.0)
        .border_bottom(1.0, c(BLUE_500).alpha(0.15))
        .bg(c(BLUE_500).alpha(0.05))
        .shrink0()
        .child(ico("message-square", 13.0, 2.0, c(look::BLUE_400)))
        .child(
            div().grow().child(
                text(
                    format!("{n} review comment{}", if n == 1 { "" } else { "s" }),
                    12.0,
                    c(BLUE_300),
                )
                .line_height(16.0)
                .silent(),
            ),
        )
        .child(
            div()
                .id(clear)
                .on(Action::Click)
                .role(Role::Button, "Clear")
                .child(
                    text(
                        "Clear",
                        11.0,
                        c(look::GRAY_500).mix(c(look::GRAY_300), s.ui.hover(clear)),
                    )
                    .line_height(16.0)
                    .silent(),
                ),
        )
        .child(
            div()
                .id(send)
                .on(Action::Click)
                .row()
                .items_center()
                .gap(6.0)
                .px(10.0)
                .py(4.0)
                .rounded(6.0)
                .bg(c(BLUE_500).alpha(0.2 + 0.1 * shot))
                .border(1.0, c(BLUE_500).alpha(0.2))
                .role(Role::Button, "Send to Agent")
                .child(ico("send", 11.0, 2.0, c(BLUE_300)))
                .child(
                    text("Send to Agent", 11.0, c(BLUE_300))
                        .weight(500)
                        .line_height(16.0)
                        .silent(),
                ),
        )
}

/// One toast in today's style, with its action buttons.
pub(super) fn toast(
    s: &Screen<'_>,
    (glyph, tint): (&str, u32),
    message: &str,
    actions: Vec<(&str, Act, bool)>,
    dismiss: Option<Act>,
) -> El {
    // A message with buttons beside it takes the toast's full width to wrap in.
    let wide = !actions.is_empty() || message.chars().count() > 40;
    let mut row = div()
        .row()
        .items_center()
        .gap(10.0)
        .px(16.0)
        .py(10.0)
        .min_w(200.0)
        .max_w(360.0)
        .rounded(8.0)
        .border(1.0, c(tint).alpha(0.2))
        .bg(Rgba::hexa(0x1a1a1e, 0.92))
        .role(Role::Status, message)
        .child(ico(glyph, 15.0, 2.5, c(tint)))
        .child(
            div().grow().min_w0().child(
                text(message, 14.0, c(look::GRAY_200))
                    .line_height(20.0)
                    .wrapping(),
            ),
        );
    if wide {
        row = row.w(360.0);
    }
    for (label, act, danger) in actions {
        let id = s.acts.on("toast-action", act);
        let hot = s.ui.hover(id);
        let color = if danger {
            c(look::RED_400).mix(c(RED_300), hot)
        } else {
            c(look::GRAY_300).mix(c(look::WHITE), hot)
        };
        row = row.child(
            div()
                .id(id)
                .on(Action::Click)
                .px(8.0)
                .py(2.0)
                .rounded(4.0)
                .bg(white(0.06 * hot))
                .role(Role::Button, label)
                .child(text(label, 12.0, color).line_height(16.0).silent()),
        );
    }
    if let Some(d) = dismiss {
        let id = s.acts.on("toast-dismiss", d);
        row = row.child(
            div()
                .id(id)
                .on(Action::Click)
                .p(2.0)
                .role(Role::Button, "Dismiss")
                .child(ico(
                    "x",
                    12.0,
                    2.0,
                    c(look::GRAY_500).mix(c(look::GRAY_300), s.ui.hover(id)),
                )),
        );
    }
    row
}
