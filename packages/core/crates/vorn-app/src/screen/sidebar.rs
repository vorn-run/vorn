//! The header with the view modes, the sessions by
//! project, by group or flat (or the tasks' projects, or the workflows),
//! and the footer.

use super::chrome::{draggable, tool};
use super::{c, ico, mod_label, white, MainView, Screen};
use crate::act::{Act, Acts};
use crate::app::GROUP_NAME;
use crate::client::Session;
use crate::look;
use crate::state::{MenuKind, SidebarMode};
use crate::ui::{
    div, field, text, tooltip, tooltip_at, Action, El, Id, Place, Rgba, Role, Sense, TextStyle,
};
use vorn_protocol::{ProjectConfig, SessionGroupConfig, WorkspaceConfig};

/// A configured colour, `#rrggbb`.
pub(crate) fn stored_color(hex: Option<&str>) -> Option<Rgba> {
    hex.and_then(|h| u32::from_str_radix(h.trim_start_matches('#'), 16).ok())
        .map(Rgba::hex)
}

/// A project's configured colour, `#rrggbb`.
fn project_color(p: &ProjectConfig) -> Option<Rgba> {
    stored_color(p.icon_color.as_deref())
}

/// `ProjectIcon` for a project.
pub(crate) fn project_icon(p: &ProjectConfig, size: f32, current: Rgba) -> El {
    stored_icon(p.icon.as_deref(), p.icon_color.as_deref(), size, current)
}

/// The `ICON_MAP` glyph in its colour (else `#6b7280`), or
/// the drawn folder in its colour (else `current`, the row's text colour).
pub(crate) fn stored_icon(icon: Option<&str>, color: Option<&str>, size: f32, current: Rgba) -> El {
    let color = stored_color(color);
    match icon.and_then(look::icon_map) {
        Some(name) => ico(name, size, 1.5, color.unwrap_or(c(look::ICON_DEFAULT))),
        None => folder_icon(size, color.unwrap_or(current)),
    }
}

/// `ProjectPicker`'s `ICON_MAP`: a smaller set than the sidebar's.
const PICKER_ICONS: [&str; 20] = [
    "Folder",
    "FolderGit2",
    "Code",
    "Globe",
    "Database",
    "Server",
    "Smartphone",
    "Package",
    "FileCode",
    "Terminal",
    "Cpu",
    "Cloud",
    "Shield",
    "Zap",
    "Gamepad2",
    "Music",
    "Image",
    "BookOpen",
    "FlaskConical",
    "Rocket",
];

/// `ProjectPicker`'s own `ProjectIcon`: lucide's folder for no or an unknown
/// icon, and `#6b7280` rather than the row colour when none is set.
pub(crate) fn picker_project_icon(p: Option<&ProjectConfig>, size: f32) -> El {
    let name = p
        .and_then(|p| p.icon.as_deref())
        .filter(|i| PICKER_ICONS.contains(i))
        .and_then(look::icon_map)
        .unwrap_or("folder");
    let color = p.and_then(project_color);
    ico(name, size, 1.5, color.unwrap_or(c(look::ICON_DEFAULT)))
}

/// `ProjectIcon` with no icon: its own folder path, not lucide's.
fn folder_icon(size: f32, color: Rgba) -> El {
    crate::ui::icon(look::PROJECT_FOLDER, size, color).glyph("▪")
}

fn header(s: &Screen<'_>) -> El {
    let ws = s.store.current_workspace();
    let open =
        s.st.menu
            .as_ref()
            .is_some_and(|m| m.kind == MenuKind::Workspace);
    let id = s.acts.on("workspace", Act::Open(MenuKind::Workspace));
    let hot = s.ui.hover(id);
    let title = div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(8.0)
        .px(10.0)
        .py(6.0)
        .rounded(6.0)
        .bg(white(0.0).mix(white(0.06), hot))
        .grow()
        .min_w0()
        .role(Role::Button, ws.name.as_str())
        .child(workspace_icon(&ws, 14.0))
        .child(
            text(ws.name.as_str(), 13.0, c(look::GRAY_200))
                .weight(500)
                .line_height(18.0)
                .truncate()
                .silent(),
        )
        .child(div().grow())
        .child(ico(
            if open { "chevron-up" } else { "chevron-down" },
            12.0,
            2.0,
            c(look::GRAY_500),
        ));
    let bar = div()
        .row()
        .items_center()
        .h(look::TOP_BAR_H)
        .pl(if s.traffic {
            look::TRAFFIC_LIGHT_PAD
        } else {
            12.0
        })
        .pr(12.0)
        .child(title)
        .child(super::chrome::group(
            s,
            [tool(
                s,
                "sidebar",
                Act::ToggleSidebar,
                "panel-left",
                2.0,
                "Toggle sidebar",
                Some(&format!("{m}B", m = mod_label())),
                false,
            )],
        ));
    let bar = draggable(s, div().col().shrink0().child(bar));
    let mut row = div()
        .row()
        .items_center()
        .gap(4.0)
        .px(12.0)
        .py(8.0)
        .role(Role::TabList, "View mode");
    let glass = super::chrome::glass(s);
    let tabs = view_tabs(s, "sidebar-mode", true);
    row = if glass {
        row.child(super::chrome::group(s, tabs))
    } else {
        tabs.into_iter().fold(row, El::child)
    };
    // `border-b` under the whole header, the view pills included.
    div()
        .col()
        .shrink0()
        .child(bar)
        .child_if(super::chrome::views(s).then_some(row))
        .child(div().h(1.0).w_full().bg(white(0.06)))
}

/// A section control: `p-0.5 rounded`, gray-600 turning white.
pub(super) fn control(
    s: &Screen<'_>,
    act: Act,
    name: &str,
    label: &str,
    size: f32,
    stroke: f32,
) -> El {
    let id = s.acts.on("sidebar", act);
    let hot = s.ui.hover(id);
    let b = div()
        .id(id)
        .on(Action::Click)
        .p(2.0)
        .rounded(4.0)
        .bg(white(0.0).mix(white(0.08), hot))
        .role(Role::Button, label)
        .child(ico(
            name,
            size,
            stroke,
            c(look::GRAY_600).mix(c(look::WHITE), hot),
        ));
    tooltip_at(s.ui, b, id, label, None, Place::Below)
}

/// An icon (given the row's text colour), a label and a
/// count.
pub(super) fn nav_item(
    s: &Screen<'_>,
    act: Act,
    icon: impl FnOnce(Rgba) -> El,
    label: &str,
    badge: Option<usize>,
    active: bool,
) -> El {
    let id = s.acts.on("sidebar", act);
    let hot = s.ui.hover(id);
    let fg = if active {
        c(look::WHITE)
    } else {
        c(look::GRAY_300).mix(c(look::WHITE), hot)
    };
    div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(8.0)
        .px(8.0)
        .py(6.0)
        .rounded(6.0)
        .bg(if active {
            white(0.08)
        } else {
            white(0.0).mix(white(0.04), hot)
        })
        .role(Role::Button, label)
        .selected(active)
        .child(icon(fg))
        .child(
            div()
                .row()
                .grow()
                .min_w0()
                .child(text(label, 13.0, fg).line_height(19.5).truncate().silent()),
        )
        .child_if(badge.map(|n| {
            text(n.to_string(), 12.0, c(look::GRAY_500))
                .line_height(16.0)
                .silent()
        }))
}

/// SessionItem, its branch under the name where the list asks for it.
fn session_row(s: &Screen<'_>, session: &Session, show_branch: bool) -> El {
    let row = session_line(s, session, show_branch);
    match super::points::decorations(s, &session.info.id) {
        Some(d) => div().col().child(row).child(d.pl(30.0).pb(2.0)),
        None => row,
    }
}

fn session_line(s: &Screen<'_>, session: &Session, show_branch: bool) -> El {
    use crate::components::sidebar::{session_row, Detail, SessionRow, SessionRowIds, StepRow};
    use crate::components::Status;
    let sid = session.info.id.as_str();
    let tabs = s.st.layout != crate::state::LayoutMode::Grid;
    let act = if tabs {
        Act::Select(sid.to_owned())
    } else {
        Act::Expand(sid.to_owned())
    };
    let active = if tabs {
        s.st.selected.as_deref() == Some(sid)
    } else {
        s.st.focused.as_deref() == Some(sid)
    };
    let ids = SessionRowIds {
        row: s.acts.on("sidebar-session", act),
        more: Some(s.acts.on(
            "sidebar-session",
            Act::Open(MenuKind::SessionGroups(sid.to_owned())),
        )),
        close: s
            .acts
            .on("sidebar-session", Act::CloseSession(sid.to_owned())),
    };
    let title = session.title();
    let report = session.report();
    let steps: Vec<StepRow<'_>> = report
        .iter()
        .flat_map(|r| &r.steps)
        .map(|st| StepRow {
            depth: st.depth,
            line: &st.line,
            status: Status::parse(st.status),
            progress: st.progress,
        })
        .collect();
    session_row(
        s.ui,
        s.agents,
        ids,
        &SessionRow {
            name: &title,
            branch: session
                .info
                .branch
                .as_deref()
                .filter(|b| show_branch && !b.is_empty()),
            agent: &session.info.agent_type,
            status: Status::parse(session.status()),
            active,
            detail: report
                .as_ref()
                .filter(|r| !r.line.is_empty())
                .map(|r| Detail {
                    line: &r.line,
                    progress: r.progress,
                    unseen: r.unseen,
                }),
            steps: &steps,
        },
    )
}

/// A sidebar row's quick actions, drawn on hover with their tooltips to
/// the right.
type Quick<'a> = (Act, &'a str, &'a str, f32);

fn quick_hot(s: &Screen<'_>, place: &str, quick: &[Quick<'_>]) -> bool {
    quick.iter().any(|(a, ..)| s.ui.hovered(Acts::id(place, a)))
}

fn quick_actions(s: &Screen<'_>, place: &str, quick: Vec<Quick<'_>>) -> El {
    let mut acts = div().row().items_center().gap(2.0);
    for (act, name, label, stroke) in quick {
        let to = if matches!(act, Act::Git(_)) {
            c(look::RED_400)
        } else {
            c(look::WHITE)
        };
        let bid = s.acts.on(place, act);
        let h = s.ui.hover(bid);
        let b = div()
            .id(bid)
            .on(Action::Click)
            .p(2.0)
            .rounded(4.0)
            .bg(white(0.0).mix(white(0.08), h))
            .role(Role::Button, label)
            .child(ico(name, 14.0, stroke, c(look::GRAY_500).mix(to, h)));
        acts = acts.child(tooltip_at(s.ui, b, bid, label, None, Place::Right));
    }
    acts
}

/// The 14 px leading slot that turns into a fold chevron on hover.
fn fold_slot(s: &Screen<'_>, place: &str, toggle: Act, hot: bool, open: bool, icon: El) -> El {
    if !hot {
        return div().size(14.0, 14.0).center().child(icon);
    }
    div()
        .id(s.acts.on(place, toggle))
        .on(Action::Click)
        .size(14.0, 14.0)
        .center()
        .role(Role::Button, "Toggle sessions")
        .expanded(open)
        .child(ico(
            if open {
                "chevron-down"
            } else {
                "chevron-right"
            },
            12.0,
            2.5,
            c(look::GRAY_500),
        ))
}

/// The `text-xs text-gray-600` session count a row shows while not hovered.
fn row_count(n: usize, size: f32) -> El {
    let label = n.to_string();
    text(&label, size, c(look::GRAY_600))
        .line_height(16.0)
        .silent()
}

/// Icon and name, its quick actions on hover, and under it
/// while expanded its worktrees or sessions as the view mode arranges them.
fn project_row(s: &Screen<'_>, p: &ProjectConfig) -> El {
    let view = s.st.sidebar_view;
    let mode = view.view_mode;
    let sessions: Vec<&Session> = s
        .store
        .sessions
        .iter()
        .filter(|x| x.info.project_name == p.name && s.st.side.holder(&x.info.id).is_none())
        .collect();
    let worktrees =
        s.st.worktrees
            .get(&p.path)
            .map(Vec::as_slice)
            .unwrap_or_default();
    let git = !worktrees.is_empty();
    let active = s.st.project.as_deref() == Some(p.name.as_str()) && s.st.worktree.is_none();
    let id = s
        .acts
        .on("sidebar-project", Act::Project(Some(p.name.clone())));
    let mut quick: Vec<Quick> = vec![
        (
            Act::NewShellIn(p.path.clone()),
            "terminal",
            "New terminal",
            1.5,
        ),
        (
            Act::NewSessionIn {
                project: p.name.clone(),
                worktree: None,
                agent: None,
            },
            "plus",
            "New session",
            2.0,
        ),
    ];
    if git {
        quick.push((
            Act::NewSessionIn {
                project: p.name.clone(),
                worktree: Some(String::new()),
                agent: None,
            },
            "folder-git-2",
            "New worktree",
            1.5,
        ));
    }
    quick.push((
        Act::Open(MenuKind::Project(p.name.clone())),
        "ellipsis",
        "More",
        2.0,
    ));
    let toggle = Act::ToggleProject(p.name.clone());
    let hot = s.ui.hovered(id)
        || s.ui.hovered(Acts::id("sidebar-project", &toggle))
        || quick_hot(s, "sidebar-project", &quick);
    let open_menu =
        s.st.menu
            .as_ref()
            .is_some_and(|m| m.kind == MenuKind::Project(p.name.clone()));
    let fg = if active {
        c(look::WHITE)
    } else {
        c(look::GRAY_300).mix(c(look::WHITE), if hot { 1.0 } else { 0.0 })
    };
    let expanded = !s.st.collapsed.contains(&p.name);
    let mut row = div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(8.0)
        .px(8.0)
        .py(6.0)
        .rounded(6.0)
        .bg(if active {
            white(0.08)
        } else {
            white(0.0).mix(white(0.04), if hot { 1.0 } else { 0.0 })
        })
        .role(Role::Button, p.name.as_str())
        .selected(active)
        .expanded(expanded)
        .child(fold_slot(
            s,
            "sidebar-project",
            toggle,
            hot,
            expanded,
            project_icon(p, 14.0, fg),
        ))
        .child(
            div().row().grow().min_w0().child(
                text(p.name.as_str(), 13.0, fg)
                    .line_height(19.5)
                    .truncate()
                    .silent(),
            ),
        );
    if hot || open_menu {
        row = row.child(quick_actions(s, "sidebar-project", quick));
    } else if mode == SidebarMode::Worktrees && !sessions.is_empty() {
        row = row.child(row_count(sessions.len(), 12.0));
    }
    let row = s
        .acts
        .context(row, id, Act::Open(MenuKind::Project(p.name.clone())));
    // The drag-reorder wrapper's transparent `border-t-2`.
    let col = div().col().pt(2.0).child(row);
    if !expanded {
        return col;
    }
    let mut kids = div().col().ml(8.0);
    let worktree_modes = matches!(
        mode,
        SidebarMode::Worktrees | SidebarMode::WorktreesSessions
    );
    if worktree_modes && git {
        let nested = mode == SidebarMode::WorktreesSessions;
        let main = worktrees.iter().find(|w| w.main);
        if let Some(main) = main {
            let mine: Vec<&Session> = sessions
                .iter()
                .copied()
                .filter(|x| x.info.worktree_path.as_deref().is_none_or(str::is_empty))
                .collect();
            kids = kids.child(worktree_row(s, p, main, &mine, nested));
        }
        for (wt, mine) in sorted_worktrees(s, worktrees, &sessions) {
            kids = kids.child(worktree_row(s, p, wt, &mine, nested));
        }
    } else if mode != SidebarMode::Worktrees || !git {
        if sessions.is_empty() {
            kids = kids.child(quiet("No active sessions"));
        }
        for x in sessions {
            kids = kids.child(session_row(s, x, true));
        }
    }
    col.child(kids)
}

/// The linked worktrees with their sessions, through
/// the worktree filter, by name or by their first session's last output.
fn sorted_worktrees<'a, 's>(
    s: &Screen<'s>,
    worktrees: &'a [crate::state::Worktree],
    sessions: &[&'s Session],
) -> Vec<(&'a crate::state::Worktree, Vec<&'s Session>)> {
    use crate::sidebar_view::{WorktreeFilter, WorktreeSort};
    let view = s.st.sidebar_view;
    let mut rest: Vec<_> = worktrees
        .iter()
        .filter(|w| !w.main)
        .map(|wt| {
            let mine: Vec<&Session> = sessions
                .iter()
                .copied()
                .filter(|x| x.info.worktree_path.as_deref() == Some(wt.path.as_str()))
                .collect();
            (wt, mine)
        })
        .filter(|(_, mine)| view.worktree_filter == WorktreeFilter::All || !mine.is_empty())
        .collect();
    rest.sort_by_cached_key(|(wt, _)| wt.name.to_lowercase());
    if view.worktree_sort == WorktreeSort::Recent {
        // The session `linkedSessionId` names: the first on the worktree's path.
        let linked = |wt: &crate::state::Worktree| {
            s.store
                .sessions
                .iter()
                .find(|x| x.info.worktree_path.as_deref() == Some(wt.path.as_str()))
                .and_then(|x| s.st.last_output.get(&x.info.id).copied())
        };
        rest.sort_by_key(|(wt, _)| std::cmp::Reverse(linked(wt)));
    }
    rest
}

/// The main checkout's branch row (`group/main`) or a WorktreeItem, with
/// their sessions under them in the nested mode.
fn worktree_row(
    s: &Screen<'_>,
    p: &ProjectConfig,
    wt: &crate::state::Worktree,
    sessions: &[&Session],
    nested: bool,
) -> El {
    let path = if wt.main {
        String::new()
    } else {
        wt.path.clone()
    };
    let key = (p.name.clone(), path.clone());
    let act = Act::Worktree {
        project: p.name.clone(),
        path: path.clone(),
    };
    let id = s.acts.on("sidebar-worktree", act);
    let toggle = Act::ToggleWorktree {
        project: p.name.clone(),
        path,
    };
    let mut quick: Vec<Quick> = vec![
        (
            Act::NewShellIn(wt.path.clone()),
            "terminal",
            "New terminal",
            1.5,
        ),
        (
            Act::NewSessionIn {
                project: p.name.clone(),
                worktree: (!wt.main).then(|| wt.path.clone()),
                agent: None,
            },
            "plus",
            "New session",
            2.0,
        ),
    ];
    if !wt.main {
        let args = serde_json::json!({ "worktree": wt.path, "project": p.path }).to_string();
        quick.extend(super::points::menu_items(s, "worktree", &args));
    }
    let hot = s.ui.hovered(id)
        || s.ui.hovered(Acts::id("sidebar-worktree", &toggle))
        || quick_hot(s, "sidebar-worktree", &quick);
    let active = s.st.project.as_deref() == Some(p.name.as_str())
        && s.st.worktree.as_deref() == Some(key.1.as_str());
    let open = !s.st.worktrees_folded.contains(&key);
    let fg = if active || hot {
        c(look::WHITE)
    } else {
        c(look::GRAY_400)
    };
    let icon = if wt.main {
        ico("git-branch", 14.0, 1.5, c(look::GRAY_500))
    } else {
        ico("folder-git-2", 14.0, 1.5, c(look::GRAY_500))
    };
    let lead = if nested {
        fold_slot(s, "sidebar-worktree", toggle, hot, open, icon)
    } else {
        div().size(14.0, 14.0).center().child(icon)
    };
    let label = if wt.main {
        div().row().grow().min_w0().child(
            text(&wt.branch, 13.0, fg)
                .line_height(19.5)
                .truncate()
                .silent(),
        )
    } else {
        div()
            .col()
            .grow()
            .min_w0()
            .child(
                text(&wt.name, 13.0, fg)
                    .line_height(18.0)
                    .truncate()
                    .silent(),
            )
            .child(
                div()
                    .row()
                    .items_center()
                    .gap(4.0)
                    .child(ico("git-branch", 8.0, 2.0, c(look::GRAY_600)))
                    .child(
                        text(&wt.branch, 10.0, c(look::GRAY_600))
                            .line_height(15.0)
                            .truncate()
                            .silent(),
                    ),
            )
    };
    let mut row = div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(8.0)
        .px(8.0)
        .py(6.0)
        .rounded(6.0)
        .bg(if active && wt.main {
            white(0.08)
        } else {
            white(0.0).mix(white(0.04), if hot { 1.0 } else { 0.0 })
        })
        .role(
            Role::Button,
            if wt.main {
                wt.branch.as_str()
            } else {
                wt.name.as_str()
            },
        )
        .selected(active)
        .child(lead)
        .child(label);
    if hot {
        row = row.child(quick_actions(s, "sidebar-worktree", quick));
    } else if !nested && !sessions.is_empty() {
        row = row.child(row_count(sessions.len(), if wt.main { 12.0 } else { 11.0 }));
    }
    if active && !wt.main {
        row = row.child(
            div()
                .absolute(0.0, 4.0)
                .size(1.0, 26.0)
                .rounded(0.5)
                .bg(c(look::WHITE)),
        );
    }
    let mut col = div().col().child(row);
    if nested && open {
        let mut kids = div().col().ml(16.0);
        for x in sessions {
            kids = kids.child(session_row(s, x, false));
        }
        col = col.child(kids);
    }
    col
}

/// `ProjectsSectionToolbar`'s Filter & sort: white with a dot while the
/// options differ from `hasNonDefault`'s baseline, else gray-600.
fn filter_sort(s: &Screen<'_>) -> El {
    let lit = s.st.sidebar_view.non_default();
    let act = Act::Open(MenuKind::SidebarOptions);
    let id = s.acts.on("sidebar", act);
    let open =
        s.st.menu
            .as_ref()
            .is_some_and(|m| m.kind == MenuKind::SidebarOptions);
    let hot = s.ui.hover(id);
    let b = div()
        .id(id)
        .on(Action::Click)
        .p(2.0)
        .rounded(4.0)
        .bg(white(0.0).mix(
            white(0.08),
            if open {
                1.0
            } else if lit {
                0.0
            } else {
                hot
            },
        ))
        .role(Role::Button, "Filter & sort")
        .expanded(open)
        .child(ico(
            "list-filter",
            13.0,
            1.5,
            if open || lit {
                c(look::WHITE)
            } else {
                c(look::GRAY_600).mix(c(look::WHITE), hot)
            },
        ))
        .child_if((lit && !open).then(|| {
            div()
                .absolute(12.0, -2.0)
                .size(6.0, 6.0)
                .rounded(3.0)
                .bg(white(look::INK_SECONDARY))
        }));
    tooltip_at(s.ui, b, id, "Filter & sort", None, Place::Below)
}

/// A sidebar list's quiet line (`text-[11px] text-gray-600 px-2 py-1`).
fn quiet(label: &str) -> El {
    div().px(8.0).py(4.0).child(
        text(label, 11.0, c(look::GRAY_600))
            .line_height(16.5)
            .role(Role::Label, label),
    )
}

/// The sessions newest activity first, as the group and flat lists sort.
fn by_activity<'a>(s: &Screen<'a>) -> Vec<&'a Session> {
    let mut list: Vec<&Session> = s
        .store
        .scoped_sessions()
        .filter(|x| s.st.side.holder(&x.info.id).is_none())
        .collect();
    list.sort_by_key(|x| std::cmp::Reverse(s.st.last_output.get(&x.info.id).copied()));
    list
}

/// The sessions view's sidebar in its arrangement.
fn sessions(s: &Screen<'_>) -> El {
    match s.st.sidebar_view.view_mode {
        SidebarMode::Worktrees | SidebarMode::WorktreesSessions | SidebarMode::Projects => {
            projects(s)
        }
        SidebarMode::Groups => groups(s),
        SidebarMode::Flat => flat(s),
    }
}

fn all_projects(s: &Screen<'_>, badge: Option<usize>) -> El {
    nav_item(
        s,
        Act::Project(None),
        |_| ico("layers", 14.0, 1.5, c(look::GRAY_400)),
        "All Projects",
        badge,
        s.st.project.is_none() && s.st.group.is_none(),
    )
}

/// The projects, each with its sessions.
fn projects(s: &Screen<'_>) -> El {
    let open = !s.st.section_folded;
    let add = control(s, Act::AddProject, "folder-plus", "Add project", 13.0, 1.5);
    let mut col = div().col().gap(2.0).child(toggle_section(
        s,
        "Projects",
        open,
        Act::ToggleSection,
        vec![filter_sort(s), add],
    ));
    if !open {
        return col;
    }
    col = col.child(all_projects(s, Some(s.store.scoped_sessions().count())));
    if s.store.workspace_projects().next().is_none() {
        col = col.child(
            div().px(10.0).py(4.0).child(
                text("No projects", 13.0, c(look::GRAY_600))
                    .line_height(18.0)
                    .role(Role::Label, "No projects"),
            ),
        );
    }
    for p in sorted_projects(s) {
        col = col.child(project_row(s, p));
    }
    col
}

/// The workspace's projects through the filter, in the
/// chosen order; recent activity is a project's latest output.
fn sorted_projects<'a>(s: &Screen<'a>) -> Vec<&'a ProjectConfig> {
    use crate::sidebar_view::{ProjectSort, WorktreeFilter};
    let view = s.st.sidebar_view;
    fn mine<'b>(
        s: &'b Screen<'_>,
        p: &'b ProjectConfig,
    ) -> impl Iterator<Item = &'b crate::client::Session> {
        s.store.sessions.iter().filter(move |x| {
            x.info.project_name == p.name && s.st.side.holder(&x.info.id).is_none()
        })
    }
    let mut out: Vec<&ProjectConfig> = s
        .store
        .workspace_projects()
        .filter(|p| view.worktree_filter == WorktreeFilter::All || mine(s, p).next().is_some())
        .collect();
    match view.project_sort {
        ProjectSort::Manual => {}
        ProjectSort::Name => out.sort_by_cached_key(|p| p.name.to_lowercase()),
        ProjectSort::Recent => out.sort_by_key(|p| {
            std::cmp::Reverse(
                mine(s, p)
                    .filter_map(|x| s.st.last_output.get(&x.info.id).copied())
                    .max(),
            )
        }),
    }
    out
}

/// Every session, newest activity first.
fn flat(s: &Screen<'_>) -> El {
    let open = !s.st.section_folded;
    let mut col = div().col().gap(2.0).child(toggle_section(
        s,
        "Sessions",
        open,
        Act::ToggleSection,
        vec![filter_sort(s)],
    ));
    if !open {
        return col;
    }
    col = col.child(all_projects(s, Some(s.store.scoped_sessions().count())));
    let mut list = div().col().gap(2.0).mt(4.0);
    let sessions: Vec<&Session> = by_activity(s)
        .into_iter()
        .filter(|x| {
            s.st.project
                .as_deref()
                .is_none_or(|p| x.info.project_name == p)
        })
        .collect();
    if sessions.is_empty() {
        list = list.child(quiet("No active sessions"));
    }
    for x in sessions {
        list = list.child(session_row(s, x, true));
    }
    col.child(list)
}

/// All Sessions, each group with its sessions, then
/// the sessions in none.
fn groups(s: &Screen<'_>) -> El {
    let open = !s.st.section_folded;
    let new = control(s, Act::NewGroup(None), "group", "New group", 13.0, 1.5);
    let mut col = div().col().gap(2.0).child(toggle_section(
        s,
        "Groups",
        open,
        Act::ToggleSection,
        vec![filter_sort(s), new],
    ));
    if !open {
        return col;
    }
    col = col.child(nav_item(
        s,
        Act::Group(None),
        |_| ico("layers", 14.0, 1.5, c(look::GRAY_400)),
        "All Sessions",
        Some(s.store.scoped_sessions().count()),
        s.st.project.is_none() && s.st.group.is_none(),
    ));
    let sessions = by_activity(s);
    let in_group =
        |x: &Session, g: &SessionGroupConfig| x.info.group_id.as_deref() == Some(g.id.as_str());
    let mut any = false;
    for g in &s.store.groups {
        let members: Vec<&Session> = sessions
            .iter()
            .copied()
            .filter(|x| in_group(x, g))
            .collect();
        any |= !members.is_empty();
        let expanded = !s.st.groups_folded.contains(&g.id);
        let mut item = div().col().child(group_row(s, g, &members, expanded));
        if expanded {
            let mut kids = div().col().ml(8.0);
            if members.is_empty() {
                kids = kids.child(quiet("Drop a session here"));
            }
            for x in members {
                kids = kids.child(session_row(s, x, true));
            }
            item = item.child(kids);
        }
        col = col.child(item);
    }
    // A deleted group leaves its sessions loose, not lost.
    let loose: Vec<&Session> = sessions
        .into_iter()
        .filter(|x| !s.store.groups.iter().any(|g| in_group(x, g)))
        .collect();
    let mut list = div().col().gap(2.0).mt(4.0);
    if loose.is_empty() && !any {
        list = list.child(quiet("No active sessions"));
    }
    for x in loose {
        list = list.child(session_row(s, x, true));
    }
    col.child(list)
}

/// Its icon (a disclosure on hover), name and count, a
/// More button on hover; the name field while renaming.
fn group_row(s: &Screen<'_>, g: &SessionGroupConfig, members: &[&Session], expanded: bool) -> El {
    let icon = |fg| stored_icon(g.icon.as_deref(), g.icon_color.as_deref(), 14.0, fg);
    if s.st.renaming_group.as_deref() == Some(g.id.as_str()) {
        let focused = s.ui.focused(GROUP_NAME);
        let input = div()
            .grow()
            .min_w0()
            .px(6.0)
            .py(2.0)
            .rounded(4.0)
            .bg(white(0.06))
            .border(1.0, if focused { white(0.25) } else { white(0.15) })
            .frames(GROUP_NAME)
            .child(field(
                GROUP_NAME,
                "Rename group",
                "",
                TextStyle {
                    line_height: 19.5,
                    ..TextStyle::ui(13.0)
                },
                c(look::GRAY_200),
            ));
        return div()
            .row()
            .items_center()
            .gap(8.0)
            .px(8.0)
            .py(6.0)
            .min_w0()
            .child(icon(c(look::GRAY_300)))
            .child(input);
    }
    let id = s.acts.on("sidebar-group", Act::Group(Some(g.id.clone())));
    let toggle = s.acts.on("sidebar-group", Act::ToggleGroup(g.id.clone()));
    let more = s
        .acts
        .on("sidebar-group", Act::Open(MenuKind::Group(g.id.clone())));
    // The menu sits inside the row today, so hovering it keeps the row hot.
    let menu_in =
        s.st.menu
            .as_ref()
            .is_some_and(|m| m.kind == MenuKind::Group(g.id.clone()));
    let hot = menu_in || s.ui.hovered(id) || s.ui.hovered(toggle) || s.ui.hovered(more);
    let active = s.st.group.as_deref() == Some(g.id.as_str());
    let fg = if active || hot {
        c(look::WHITE)
    } else {
        c(look::GRAY_300)
    };
    let slot = div()
        .id(toggle)
        .on(Action::Click)
        .size(14.0, 14.0)
        .center()
        .shrink0()
        .role(Role::Button, format!("Toggle {}", g.name))
        .expanded(expanded)
        .child(if hot {
            let glyph = if expanded {
                "chevron-down"
            } else {
                "chevron-right"
            };
            ico(glyph, 12.0, 2.5, c(look::GRAY_500))
        } else {
            icon(fg)
        });
    let waiting = !expanded && members.iter().any(|x| x.status() == "waiting");
    let mut row = div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(8.0)
        .px(8.0)
        .py(6.0)
        .rounded(6.0)
        .bg(if active {
            white(0.08)
        } else {
            white(0.0).mix(white(0.04), if hot { 1.0 } else { 0.0 })
        })
        .role(Role::Button, g.name.as_str())
        .selected(active)
        .child(slot)
        .child(
            div().row().grow().min_w0().child(
                text(g.name.as_str(), 13.0, fg)
                    .line_height(19.5)
                    .truncate()
                    .silent(),
            ),
        )
        .child_if(waiting.then(|| {
            div()
                .size(5.0, 5.0)
                .rounded(2.5)
                .bg(c(look::BRONZO))
                .shrink0()
                .role(Role::Label, "A session in this group is waiting")
        }));
    if hot {
        let h = s.ui.hover(more);
        let b = div()
            .id(more)
            .on(Action::Click)
            .p(2.0)
            .rounded(4.0)
            .bg(white(0.0).mix(white(0.08), h))
            .role(Role::Button, format!("More actions for {}", g.name))
            .child(ico(
                "ellipsis",
                14.0,
                2.0,
                c(look::GRAY_500).mix(c(look::WHITE), h),
            ));
        row = row.child(tooltip_at(s.ui, b, more, "More", None, Place::Right));
    } else if !members.is_empty() {
        row = row.child(
            text(members.len().to_string(), 12.0, c(look::GRAY_500))
                .line_height(16.0)
                .ml(if waiting { 6.0 } else { 0.0 })
                .silent(),
        );
    }
    row
}

/// Its stored glyph in its colour, else a gray user.
fn workspace_icon(ws: &WorkspaceConfig, size: f32) -> El {
    workspace_glyph(ws.icon.as_deref(), ws.icon_color.as_deref(), size)
}

fn workspace_glyph(icon: Option<&str>, color: Option<&str>, size: f32) -> El {
    let glyph = icon
        .and_then(|n| look::WORKSPACE_ICONS.iter().find(|(name, ..)| *name == n))
        .map_or("user", |(_, glyph, _)| *glyph);
    let tint = color
        .and_then(|h| u32::from_str_radix(h.trim_start_matches('#'), 16).ok())
        .map_or(c(look::GRAY_500), Rgba::hex);
    ico(glyph, size, 1.5, tint)
}

/// `WorkspaceSwitcher`'s dropdown: the workspaces with their project
/// counts, each editable and (but `personal`) deletable, then Create.
pub(super) fn workspace_panel(s: &Screen<'_>) -> El {
    let active = s.store.workspace();
    let form = s.st.ws_form.as_ref();
    let mut list = div()
        .id(s.acts.on("workspace-list", Act::Nothing))
        .col()
        .scroll_y()
        .max_h(300.0);
    for ws in s.store.workspaces() {
        if form.is_some_and(|f| f.id.as_deref() == Some(ws.id.as_str())) {
            list = list.child(workspace_form(s));
            continue;
        }
        let count = s
            .store
            .projects
            .iter()
            .filter(|p| {
                p.workspace_id
                    .as_deref()
                    .unwrap_or(crate::client::store::WORKSPACE)
                    == ws.id
            })
            .count();
        list = list.child(workspace_row(s, &ws, ws.id == active, count));
    }
    let mut panel = div()
        .col()
        .w(260.0)
        .py(4.0)
        .rounded(8.0)
        .border(1.0, white(0.08))
        .bg(c(look::SURFACE_OVERLAY))
        .role(Role::Menu, "Workspaces")
        .child(list)
        .child(div().h(1.0).mt(4.0).mb(4.0).bg(white(0.06)));
    if form.is_some_and(|f| f.id.is_none()) {
        panel = panel.child(workspace_form(s));
    } else {
        let id = s.acts.on("workspace", Act::EditWorkspace(None));
        let hot = s.ui.hover(id);
        let fg = c(look::GRAY_400).mix(c(look::GRAY_200), hot);
        panel = panel.child(
            div()
                .id(id)
                .on(Action::Click)
                .row()
                .items_center()
                .gap(8.0)
                .px(12.0)
                .py(8.0)
                .bg(white(0.0).mix(white(0.04), hot))
                .role(Role::MenuItem, "Create Workspace")
                .child(ico("plus", 14.0, 1.5, fg))
                .child(
                    text("Create Workspace", 13.0, fg)
                        .line_height(19.5)
                        .silent(),
                ),
        );
    }
    panel
}

/// A workspace in the switcher: its icon, name and project count, and on
/// hover Edit and Delete (armed by the first click).
fn workspace_row(s: &Screen<'_>, ws: &WorkspaceConfig, on: bool, count: usize) -> El {
    let pick = s.acts.on("workspace", Act::SwitchWorkspace(ws.id.clone()));
    let row_id = pick.child("row");
    let hot = s.ui.hovered(row_id);
    let armed = s.st.confirm_delete.as_deref() == Some(ws.id.as_str());
    let bg = if on {
        white(0.08)
    } else {
        white(0.0).mix(white(0.04), s.ui.hover(row_id))
    };
    let fg = if on {
        c(look::WHITE)
    } else {
        c(look::GRAY_300)
    };
    let button = div()
        .id(pick)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(8.0)
        .grow()
        .min_w0()
        .role(Role::MenuItem, ws.name.as_str())
        .selected(on)
        .child(workspace_icon(ws, 14.0))
        .child(
            text(ws.name.as_str(), 13.0, fg)
                .line_height(19.5)
                .truncate()
                .silent(),
        )
        .child(div().grow())
        .child(
            text(count.to_string(), 11.0, c(look::GRAY_600))
                .line_height(16.5)
                .silent(),
        );
    let mut actions = div().row().items_center().gap(2.0).shrink0();
    if hot || armed {
        let edit = s
            .acts
            .on("workspace", Act::EditWorkspace(Some(ws.id.clone())));
        let edit_hot = s.ui.hover(edit);
        actions = actions.child(tooltip(
            s.ui,
            div()
                .id(edit)
                .on(Action::Click)
                .p(2.0)
                .rounded(4.0)
                .role(Role::Button, "Edit")
                .child(ico(
                    "pencil",
                    11.0,
                    1.5,
                    c(look::GRAY_500).mix(c(look::GRAY_300), edit_hot),
                )),
            edit,
            "Edit",
            None,
        ));
        if ws.id != crate::client::store::WORKSPACE {
            let (act, label, tint, stroke) = if armed {
                (
                    Act::DeleteWorkspace(ws.id.clone()),
                    "Confirm delete",
                    c(look::RED_400),
                    2.0,
                )
            } else {
                (
                    Act::AskDeleteWorkspace(ws.id.clone()),
                    "Delete",
                    c(look::GRAY_500),
                    1.5,
                )
            };
            let del = s.acts.on("workspace", act);
            let del_hot = s.ui.hover(del);
            let tint = if armed {
                tint
            } else {
                tint.mix(c(look::RED_400), del_hot)
            };
            actions = actions.child(tooltip(
                s.ui,
                div()
                    .id(del)
                    .on(Action::Click)
                    .p(2.0)
                    .rounded(4.0)
                    .role(Role::Button, label)
                    .child(ico("trash-2", 11.0, stroke, tint)),
                del,
                label,
                None,
            ));
        }
    }
    div()
        .id(row_id)
        .sense(crate::ui::Sense::HOVER)
        .row()
        .items_center()
        .gap(8.0)
        .px(10.0)
        .py(8.0)
        .bg(bg)
        .child(button)
        .child(actions)
}

/// The switcher's create or edit row: icon button, name field, save and
/// cancel, and the icon picker under them when open.
fn workspace_form(s: &Screen<'_>) -> El {
    let Some(f) = s.st.ws_form.as_ref() else {
        return div();
    };
    let picker = s.acts.on("workspace", Act::WorkspacePicker);
    let save = s.acts.on("workspace", Act::SaveWorkspace);
    let cancel = s.acts.on("workspace", Act::CancelWorkspace);
    let empty = s.ui.edit_text(crate::app::WS_NAME).trim().is_empty();
    let focused = s.ui.focused(crate::app::WS_NAME);
    let input = div()
        .grow()
        .min_w0()
        .px(8.0)
        .py(4.0)
        .rounded(4.0)
        .bg(white(0.06))
        .border(1.0, if focused { white(0.2) } else { white(0.1) })
        .frames(crate::app::WS_NAME)
        .child(field(
            crate::app::WS_NAME,
            "Workspace name",
            if f.id.is_none() {
                "Workspace name..."
            } else {
                ""
            },
            TextStyle {
                line_height: 19.5,
                ..TextStyle::ui(13.0)
            },
            c(look::GRAY_200),
        ));
    let save_fg = c(look::GREEN_400).alpha(if f.id.is_none() && empty { 0.3 } else { 1.0 });
    let row = div()
        .row()
        .items_center()
        .gap(8.0)
        .child(
            div()
                .id(picker)
                .on(Action::Click)
                .p(4.0)
                .rounded(4.0)
                .bg(white(0.0).mix(white(0.08), s.ui.hover(picker)))
                .role(Role::Button, "Change icon")
                .child(workspace_glyph(Some(&f.icon), Some(&f.color), 14.0)),
        )
        .child(input)
        .child(
            div()
                .id(save)
                .on(Action::Click)
                .p(4.0)
                .role(Role::Button, "Save")
                .child(ico("check", 14.0, 2.0, save_fg)),
        )
        .child(
            div()
                .id(cancel)
                .on(Action::Click)
                .p(4.0)
                .role(Role::Button, "Cancel")
                .child(ico(
                    "x",
                    14.0,
                    2.0,
                    c(look::GRAY_500).mix(c(look::GRAY_300), s.ui.hover(cancel)),
                )),
        );
    let mut col = div().col().px(8.0).py(6.0).child(row);
    if f.picker {
        col = col.child(div().mt(8.0).child(super::workflows::icon_colors_from(
            s,
            "workspace",
            &look::WORKSPACE_ICONS,
            26.0,
            &f.icon,
            &f.color,
            Act::WorkspaceIcon,
            Act::WorkspaceColor,
        )));
    }
    col
}

/// SessionGroupItem's icon popover: `IconColorPicker` over a Done button.
pub(super) fn group_icon_picker(s: &Screen<'_>, id: &str) -> El {
    let Some(g) = s.store.group(id) else {
        return div();
    };
    // Opening what is open closes it.
    let done = s
        .acts
        .on("group-icon", Act::Open(MenuKind::GroupIcon(id.to_owned())));
    let hot = s.ui.hover(done);
    div()
        .col()
        .w(248.0)
        .p(12.0)
        .rounded(8.0)
        .border(1.0, white(0.08))
        .bg(c(look::SURFACE_OVERLAY))
        .role(Role::Dialog, "Change icon")
        .child(super::workflows::icon_colors(
            s,
            "group-icon",
            g.icon.as_deref().unwrap_or("Folder"),
            g.icon_color.as_deref().unwrap_or("#6b7280"),
            |n| Act::GroupIconName(id.to_owned(), n),
            |h| Act::GroupIconColor(id.to_owned(), h),
        ))
        .child(
            div()
                .id(done)
                .on(Action::Click)
                .mt(12.0)
                .w_full()
                .px(8.0)
                .py(4.0)
                .center()
                .rounded(4.0)
                .bg(white(0.04).mix(white(0.08), hot))
                .border(1.0, white(0.08))
                .role(Role::Button, "Done")
                .child(
                    text("Done", 11.0, c(look::GRAY_300))
                        .line_height(16.5)
                        .silent(),
                ),
        )
}

/// SidebarSectionHeader with a fold: a chevron and an uppercase title.
pub(super) fn toggle_section(
    s: &Screen<'_>,
    title: &str,
    open: bool,
    act: Act,
    actions: Vec<El>,
) -> El {
    let id = s.acts.on("sidebar-section", act);
    let hot = s.ui.hover(id);
    let chevron = if open {
        "chevron-down"
    } else {
        "chevron-right"
    };
    div()
        .row()
        .items_center()
        .justify_between()
        .pt(12.0)
        .pb(6.0)
        .child(
            div()
                .id(id)
                .on(Action::Click)
                .row()
                .items_center()
                .gap(6.0)
                .role(Role::Button, title)
                .expanded(open)
                .child(ico(chevron, 10.0, 2.0, c(look::GRAY_600)))
                .child(
                    text(
                        title.to_uppercase(),
                        11.0,
                        c(look::GRAY_500).mix(c(look::GRAY_300), hot),
                    )
                    .weight(500)
                    .tracking(0.05)
                    .line_height(16.5)
                    .silent(),
                ),
        )
        .child(div().row().items_center().gap(2.0).children(actions))
}

/// All Projects and each project, under a fold.
fn tasks_projects(s: &Screen<'_>) -> El {
    let open = !s.st.tasks.projects_folded;
    let mut col = div().col().gap(2.0).child(toggle_section(
        s,
        "Projects",
        open,
        Act::Task(crate::tasks::TaskAct::ToggleProjects),
        Vec::new(),
    ));
    if !open {
        return col;
    }
    col = col.child(nav_item(
        s,
        Act::Project(None),
        |_| ico("layers", 14.0, 1.5, c(look::GRAY_400)),
        "All Projects",
        None,
        s.st.project.is_none(),
    ));
    if s.store.workspace_projects().next().is_none() {
        col = col.child(
            div().px(10.0).py(4.0).child(
                text("No projects", 13.0, c(look::GRAY_600))
                    .line_height(18.0)
                    .role(Role::Label, "No projects"),
            ),
        );
    }
    for p in s.store.workspace_projects() {
        col = col.child(nav_item(
            s,
            Act::Project(Some(p.name.clone())),
            |fg| project_icon(p, 14.0, fg),
            &p.name,
            None,
            s.st.project.as_deref() == Some(p.name.as_str()),
        ));
    }
    col
}

/// The ready-update notice above the guide and settings
/// buttons, or a dot on the gear once the notice is dismissed.
fn footer(s: &Screen<'_>) -> El {
    use crate::settings::{describe_update, update_cost, SetAct, UpdateStatus};
    let status = s.st.settings.update.clone();
    let pending = matches!(status, Some(UpdateStatus::Ready { .. }));
    let dismissed = s.st.settings.update_dismissed;
    let button = |act: Act, name: &str, label: &str, dot: bool| {
        let id = s.acts.on("sidebar-footer", act);
        let hot = s.ui.hover(id);
        let mut b = div()
            .id(id)
            .on(Action::Click)
            .p(6.0)
            .rounded(6.0)
            .bg(white(0.0).mix(white(0.06), hot))
            .role(Role::Button, label)
            .child(ico(
                name,
                16.0,
                1.5,
                c(look::GRAY_500).mix(c(look::WHITE), hot),
            ));
        if dot {
            b = b.child(
                div()
                    .absolute(19.0, 2.0)
                    .size(9.0, 9.0)
                    .rounded(5.0)
                    .bg(c(look::SURFACE_PANEL))
                    .center()
                    .child(div().size(5.0, 5.0).rounded(3.0).bg(c(look::BRONZO))),
            );
        }
        tooltip(s.ui, b, id, label, None)
    };
    let row = div()
        .row()
        .items_center()
        .gap(2.0)
        .px(8.0)
        .py(6.0)
        .shrink0()
        .child(button(Act::Welcome, "circle-help", "Welcome Guide", false));
    let row = if pending && dismissed {
        row.child(button(
            Act::Set(SetAct::Open(crate::settings::Category::Updates)),
            "settings",
            "Settings \u{2014} update ready",
            true,
        ))
    } else {
        row.child(button(Act::Settings, "settings", "Settings", false))
    };
    let Some(status) = status.filter(|_| pending && !dismissed) else {
        return row;
    };
    let view = describe_update(&status, "stable", crate::state::now_ms());
    let short = match &status {
        UpdateStatus::Ready { version } => format!("v{version} ready"),
        _ => view.label.clone(),
    };
    let live: Vec<_> = s.store.sessions.iter().filter(|x| !x.ended()).collect();
    let cost = update_cost(
        live.len(),
        live.iter().any(|x| x.status() == "running"),
        !crate::update::ENDS_SESSIONS,
    );
    let close = s.acts.on("update-notice", Act::Set(SetAct::DismissUpdate));
    let go = s.acts.on("update-notice", Act::Set(SetAct::UpdateNow));
    let mut notice = div()
        .col()
        .mx(8.0)
        .mb(6.0)
        .px(10.0)
        .py(8.0)
        .rounded(6.0)
        .border(1.0, white(0.08))
        .bg(white(0.03))
        .role(Role::Status, short.as_str())
        .child(
            div()
                .row()
                .items_center()
                .gap(6.0)
                .child(div().size(5.0, 5.0).rounded(3.0).bg(c(look::BRONZO)))
                .child(
                    div().grow().min_w0().child(
                        text(&short, 11.5, c(look::GRAY_200))
                            .line_height(16.0)
                            .truncate()
                            .silent(),
                    ),
                )
                .child(
                    div()
                        .id(close)
                        .on(Action::Click)
                        .role(Role::Button, "Dismiss update notice")
                        .child(
                            text(
                                "\u{2715}",
                                11.0,
                                c(look::GRAY_600).mix(c(look::GRAY_300), s.ui.hover(close)),
                            )
                            .line_height(11.0)
                            .silent(),
                        ),
                ),
        );
    if let Some(cost) = cost {
        notice = notice.child(
            text(cost, 10.5, c(look::BRONZO))
                .line_height(14.0)
                .mt(6.0)
                .wrapping(),
        );
    }
    notice = notice.child(
        div()
            .id(go)
            .on(Action::Click)
            .row()
            .justify_center()
            .mt(8.0)
            .px(8.0)
            .py(4.0)
            .rounded(4.0)
            .bg(white(0.04).mix(white(0.08), s.ui.hover(go)))
            .border(1.0, white(0.08))
            .role(Role::Button, "Restart to update")
            .child(
                text("Restart to update", 11.0, c(look::GRAY_300))
                    .line_height(16.0)
                    .silent(),
            ),
    );
    div().col().shrink0().child(notice).child(row)
}

pub(super) fn sidebar(s: &Screen<'_>) -> El {
    let body = match s.st.view {
        MainView::Workflows => super::workflows::sidebar(s),
        MainView::Tasks => tasks_projects(s),
        MainView::Sessions | MainView::Extension => div()
            .col()
            .child(sessions(s))
            .child_if(super::points::sidebar(s)),
    };
    let scroll = div()
        .id(crate::ui::Id::new("sidebar-scroll"))
        .col()
        .grow()
        .min_h(0.0)
        .px(12.0)
        .scroll_y()
        .child(body);
    let mut aside = div()
        .col()
        .w(s.sidebar_w())
        .h_full()
        .shrink0()
        .bg(crate::ui::theme::material::sidebar(s.ui.see_through()))
        .role(Role::Navigation, "Project sidebar")
        .child(
            div()
                .row()
                .grow()
                .min_h(0.0)
                .child(
                    div()
                        .col()
                        .grow()
                        .min_w0()
                        .child(header(s))
                        .child(scroll)
                        .child(footer(s)),
                )
                .child(div().w(1.0).h_full().bg(white(0.06))),
        );
    // A terminal has no pointer to drag it with.
    if !s.ui.is_cells() {
        let hot = if s.ui.pressed(SIDEBAR_HANDLE) {
            white(0.12)
        } else {
            white(0.0).mix(white(0.08), s.ui.hover(SIDEBAR_HANDLE))
        };
        aside = aside.child(
            div()
                .id(SIDEBAR_HANDLE)
                .absolute_right(0.0, 0.0)
                .w(6.0)
                .h_full()
                .sense(Sense::DRAG)
                .vertical(true)
                .bg(hot)
                .role(Role::Splitter, "Resize sidebar"),
        );
    }
    aside
}

/// The handle on the sidebar's right edge that resizes it.
pub const SIDEBAR_HANDLE: Id = Id(0x7369_6465_6261_7268);

#[cfg(test)]
mod tests {
    use super::*;

    fn project(color: &str) -> ProjectConfig {
        serde_json::from_value(serde_json::json!({
            "name": "p",
            "path": "/p",
            "preferredAgents": [],
            "iconColor": color,
        }))
        .expect("project")
    }

    #[test]
    fn every_picker_icon_has_a_glyph() {
        for name in PICKER_ICONS {
            assert!(look::icon_map(name).is_some(), "{name}");
        }
    }

    #[test]
    fn project_colour_reads_the_configured_hex() {
        assert_eq!(
            project_color(&project("#ec4899")),
            Some(Rgba::hex(0xec4899))
        );
        assert_eq!(project_color(&project("nope")), None);
    }
}

/// The view tabs: icon buttons, the chosen one labelled; on glass over the
/// system's material the system draws their icons and label.
pub(super) fn view_tabs(s: &Screen<'_>, place: &str, labels: bool) -> Vec<El> {
    let glass = super::chrome::glass(s);
    let mut tabs = Vec::new();
    for b in super::chrome::top_buttons(s) {
        let (on, name, label, keys) = (b.on, b.icon.as_str(), b.label.as_str(), b.keys);
        let id = s.acts.on(place, b.act);
        let hot = s.ui.hover(id);
        let fg = if on {
            c(look::WHITE)
        } else {
            c(look::GRAY_500).mix(c(look::GRAY_300), hot)
        };
        let mut b = div()
            .id(id)
            .on(Action::Click)
            .row()
            .items_center()
            .gap(6.0)
            .px(10.0)
            .py(if glass { 4.0 } else { 6.0 })
            .rounded(if glass { 13.0 } else { 8.0 })
            .bg(if glass {
                white(0.0)
            } else if on {
                white(0.10)
            } else {
                white(0.0).mix(white(0.04), hot)
            })
            .role(Role::Tab, label)
            .selected(on);
        let svg = glass.then(|| look::icon(name, 2.0)).flatten();
        b = match &svg {
            Some(svg) => b.child(div().size(14.0, 14.0).native_icon(svg, 14.0, fg)),
            None => b.child(ico(name, 14.0, 2.0, fg)),
        };
        if on && labels {
            // Over glass the system draws the label; the app keeps its room.
            let ink = if svg.is_some() { white(0.0) } else { fg };
            let mut t = text(label, 12.0, ink)
                .weight(500)
                .line_height(18.0)
                .silent();
            if svg.is_some() {
                t = t.native_label(label, 12.0, fg);
            }
            b = b.child(t);
        }
        tabs.push(tooltip(s.ui, b, id, label, keys.as_deref()));
    }
    tabs
}
