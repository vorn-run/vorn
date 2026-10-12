//! ProjectSidebar: the header with the view modes, the sessions by
//! project, by group or flat (or the tasks' projects, or the workflows),
//! and the footer.

use super::chrome::{draggable, tool};
use super::{c, ico, white, MainView, Screen, MOD_LABEL};
use crate::act::{Act, Acts};
use crate::app::GROUP_NAME;
use crate::client::Session;
use crate::look;
use crate::state::{MenuKind, SidebarMode};
use crate::ui::{div, field, text, tooltip, tooltip_at, Action, El, Place, Rgba, Role, TextStyle};
use vorn_protocol::{ProjectConfig, SessionGroupConfig};

/// A configured colour, `#rrggbb`, else gray-400.
fn stored_color(hex: Option<&str>) -> Rgba {
    hex.and_then(|h| u32::from_str_radix(h.trim_start_matches('#'), 16).ok())
        .map_or(c(look::GRAY_400), Rgba::hex)
}

/// A project's configured icon when the app carries it, else a folder.
pub(crate) fn project_icon(p: &ProjectConfig, size: f32) -> El {
    stored_icon(p.icon.as_deref(), p.icon_color.as_deref(), size)
}

/// `ProjectIcon`: a stored icon name and colour, else a gray folder.
pub(crate) fn stored_icon(icon: Option<&str>, color: Option<&str>, size: f32) -> El {
    let name = icon
        .map(lucide_name)
        .filter(|n| look::icon(n, 1.5).is_some())
        .unwrap_or_else(|| "folder".to_owned());
    ico(&name, size, 1.5, stored_color(color))
}

/// `FolderGit2` to `folder-git-2`.
fn lucide_name(pascal: &str) -> String {
    let mut out = String::new();
    for (i, ch) in pascal.chars().enumerate() {
        if (ch.is_ascii_uppercase()
            || (ch.is_ascii_digit() && !out.ends_with(|c: char| c.is_ascii_digit())))
            && i > 0
        {
            out.push('-');
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}

fn header(s: &Screen<'_>) -> El {
    let workspace = s
        .store
        .config
        .pointer("/workspaces/0/name")
        .and_then(|v| v.as_str())
        .unwrap_or("Personal");
    let title = div()
        .row()
        .items_center()
        .gap(8.0)
        .px(10.0)
        .py(6.0)
        .grow()
        .min_w0()
        .role(Role::Label, workspace)
        .child(ico("user", 14.0, 1.5, c(look::GRAY_500)))
        .child(
            text(workspace, 13.0, c(look::GRAY_200))
                .weight(500)
                .line_height(18.0)
                .truncate()
                .silent(),
        );
    let bar = div()
        .row()
        .items_center()
        .h(look::TOP_BAR_H)
        .pl(12.0)
        .pr(12.0)
        .child(title)
        .child(tool(
            s,
            "sidebar",
            Act::ToggleSidebar,
            "panel-left",
            2.0,
            "Toggle sidebar",
            Some(&format!("{MOD_LABEL}B")),
            false,
        ));
    let bar = draggable(s, div().col().shrink0().child(bar));
    let modes = [
        (
            MainView::Sessions,
            "monitor",
            "Sessions",
            format!("{MOD_LABEL}S"),
        ),
        (
            MainView::Tasks,
            "list-todo",
            "Tasks",
            format!("{MOD_LABEL}T"),
        ),
        (
            MainView::Workflows,
            "workflow",
            "Workflows",
            format!("{MOD_LABEL}\u{21e7}W"),
        ),
    ];
    let mut row = div()
        .row()
        .items_center()
        .gap(4.0)
        .px(12.0)
        .py(8.0)
        .role(Role::TabList, "View mode");
    for (view, name, label, keys) in modes {
        let on = s.st.view == view;
        let id = s.acts.on("sidebar-mode", Act::View(view));
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
            .py(6.0)
            .rounded(8.0)
            .bg(if on {
                white(0.10)
            } else {
                white(0.0).mix(white(0.04), hot)
            })
            .role(Role::Tab, label)
            .selected(on)
            .child(ico(name, 14.0, 2.0, fg));
        if on {
            b = b.child(text(label, 12.0, fg).weight(500).line_height(18.0).silent());
        }
        row = row.child(tooltip(s.ui, b, id, label, Some(&keys)));
    }
    // `border-b` under the whole header, the view pills included.
    div()
        .col()
        .shrink0()
        .child(bar)
        .child(row)
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

/// SidebarNavItem: an icon, a label and a count.
pub(super) fn nav_item(
    s: &Screen<'_>,
    act: Act,
    icon: El,
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
        .child(icon)
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
    use crate::components::sidebar::{session_row, SessionRow, SessionRowIds};
    use crate::components::Status;
    let sid = session.info.id.as_str();
    let tabs = s.st.layout == crate::state::LayoutMode::Tabs;
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
        },
    )
}

/// A sidebar row's quick actions, drawn on hover with their tooltips to
/// the right as today's `Tooltip position="right"`.
type Quick = (Act, &'static str, &'static str, f32);

fn quick_hot(s: &Screen<'_>, place: &str, quick: &[Quick]) -> bool {
    quick.iter().any(|(a, ..)| s.ui.hovered(Acts::id(place, a)))
}

fn quick_actions(s: &Screen<'_>, place: &str, quick: Vec<Quick>) -> El {
    let mut acts = div().row().items_center().gap(2.0);
    for (act, name, label, stroke) in quick {
        let bid = s.acts.on(place, act);
        let h = s.ui.hover(bid);
        let b = div()
            .id(bid)
            .on(Action::Click)
            .p(2.0)
            .rounded(4.0)
            .bg(white(0.0).mix(white(0.08), h))
            .role(Role::Button, label)
            .child(ico(
                name,
                14.0,
                stroke,
                c(look::GRAY_500).mix(c(look::WHITE), h),
            ));
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

/// ProjectItem: icon and name, its quick actions on hover, and under it
/// while expanded its worktrees or sessions as the view mode arranges them.
fn project_row(s: &Screen<'_>, p: &ProjectConfig) -> El {
    let mode = s.st.sidebar_mode;
    let sessions: Vec<&Session> = s
        .store
        .sessions
        .iter()
        .filter(|x| x.info.project_name == p.name)
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
            project_icon(p, 14.0),
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
        let mut rest: Vec<&crate::state::Worktree> = worktrees.iter().filter(|w| !w.main).collect();
        rest.sort_by(|a, b| a.name.cmp(&b.name));
        for wt in rest {
            let mine: Vec<&Session> = sessions
                .iter()
                .copied()
                .filter(|x| x.info.worktree_path.as_deref() == Some(wt.path.as_str()))
                .collect();
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
    let quick: Vec<Quick> = vec![
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
/// view is not the default, which this sidebar never draws.
fn filter_sort(s: &Screen<'_>) -> El {
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
        .bg(white(0.0).mix(white(0.08), if open { 1.0 } else { hot }))
        .role(Role::Button, "Filter & sort")
        .expanded(open)
        .child(ico("list-filter", 13.0, 1.5, c(look::WHITE)))
        .child_if((!open).then(|| {
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
    let mut list: Vec<&Session> = s.store.sessions.iter().collect();
    list.sort_by_key(|x| std::cmp::Reverse(s.st.last_output.get(&x.info.id).copied()));
    list
}

/// The sessions view's sidebar in its arrangement.
fn sessions(s: &Screen<'_>) -> El {
    match s.st.sidebar_mode {
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
        ico("layers", 14.0, 1.5, c(look::GRAY_400)),
        "All Projects",
        badge,
        s.st.project.is_none() && s.st.group.is_none(),
    )
}

/// ProjectsSection: the projects, each with its sessions.
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
    col = col.child(all_projects(s, Some(s.store.sessions.len())));
    if s.store.projects.is_empty() {
        col = col.child(
            div().px(10.0).py(4.0).child(
                text("No projects", 13.0, c(look::GRAY_600))
                    .line_height(18.0)
                    .role(Role::Label, "No projects"),
            ),
        );
    }
    for p in &s.store.projects {
        col = col.child(project_row(s, p));
    }
    col
}

/// FlatSessionsSection: every session, newest activity first.
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
    col = col.child(all_projects(s, Some(s.store.sessions.len())));
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

/// GroupSessionsSection: All Sessions, each group with its sessions, then
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
        ico("layers", 14.0, 1.5, c(look::GRAY_400)),
        "All Sessions",
        Some(s.store.sessions.len()),
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

/// SessionGroupItem: its icon (a disclosure on hover), name and count, a
/// More button on hover; the name field while renaming.
fn group_row(s: &Screen<'_>, g: &SessionGroupConfig, members: &[&Session], expanded: bool) -> El {
    let icon = stored_icon(g.icon.as_deref(), g.icon_color.as_deref(), 14.0);
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
            .child(icon)
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
            icon
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

/// TasksProjectsSection: All Projects and each project, under a fold.
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
        ico("layers", 14.0, 1.5, c(look::GRAY_400)),
        "All Projects",
        None,
        s.st.project.is_none(),
    ));
    if s.store.projects.is_empty() {
        col = col.child(
            div().px(10.0).py(4.0).child(
                text("No projects", 13.0, c(look::GRAY_600))
                    .line_height(18.0)
                    .role(Role::Label, "No projects"),
            ),
        );
    }
    for p in &s.store.projects {
        col = col.child(nav_item(
            s,
            Act::Project(Some(p.name.clone())),
            project_icon(p, 14.0),
            &p.name,
            None,
            s.st.project.as_deref() == Some(p.name.as_str()),
        ));
    }
    col
}

fn footer(s: &Screen<'_>) -> El {
    let button = |act: Act, name: &str, label: &str| {
        let id = s.acts.on("sidebar-footer", act);
        let hot = s.ui.hover(id);
        let b = div()
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
        tooltip(s.ui, b, id, label, None)
    };
    div()
        .row()
        .items_center()
        .gap(2.0)
        .px(8.0)
        .py(6.0)
        .shrink0()
        .child(button(Act::Welcome, "circle-help", "Welcome Guide"))
        .child(button(Act::Settings, "settings", "Settings"))
}

pub(super) fn sidebar(s: &Screen<'_>) -> El {
    let body = match s.st.view {
        MainView::Workflows => super::workflows::sidebar(s),
        MainView::Tasks => tasks_projects(s),
        MainView::Sessions => sessions(s),
    };
    let scroll = div()
        .id(crate::ui::Id::new("sidebar-scroll"))
        .col()
        .grow()
        .min_h(0.0)
        .px(12.0)
        .scroll_y()
        .child(body);
    div()
        .col()
        .w(look::SIDEBAR_W)
        .h_full()
        .shrink0()
        .bg(c(look::SURFACE_PANEL))
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
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_icon_names_map_to_their_files() {
        assert_eq!(lucide_name("FolderGit2"), "folder-git-2");
        assert_eq!(lucide_name("Layers"), "layers");
        assert_eq!(lucide_name("SquareTerminal"), "square-terminal");
    }
}
