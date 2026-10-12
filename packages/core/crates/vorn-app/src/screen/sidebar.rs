//! ProjectSidebar: the header with the view modes, the projects with their
//! sessions (or the workflows), and the footer.

use super::chrome::{draggable, tool};
use super::{c, ico, white, MainView, Screen, MOD_LABEL};
use crate::act::{Act, Acts};
use crate::client::Session;
use crate::look;
use crate::state::MenuKind;
use crate::ui::{div, text, tooltip, Action, El, Rgba, Role};
use vorn_protocol::ProjectConfig;

/// A project's configured colour, `#rrggbb`, else gray-400.
pub(super) fn project_color(p: &ProjectConfig) -> Rgba {
    p.icon_color
        .as_deref()
        .and_then(|h| u32::from_str_radix(h.trim_start_matches('#'), 16).ok())
        .map_or(c(look::GRAY_400), Rgba::hex)
}

/// A project's configured icon when the app carries it, else a folder.
pub(crate) fn project_icon(p: &ProjectConfig, size: f32) -> El {
    let name = p
        .icon
        .as_deref()
        .map(lucide_name)
        .filter(|n| look::icon(n, 1.5).is_some())
        .unwrap_or_else(|| "folder".to_owned());
    ico(&name, size, 1.5, project_color(p))
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
        .pl(if s.traffic {
            look::TRAFFIC_LIGHT_PAD
        } else {
            12.0
        })
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

/// SidebarSectionHeader: the label and its controls.
pub(super) fn section(s: &Screen<'_>, label: &str, controls: Vec<El>) -> El {
    let _ = s;
    div()
        .row()
        .items_center()
        .justify_between()
        .px(8.0)
        .pt(8.0)
        .pb(4.0)
        .child(
            text(label, 13.0, c(look::GRAY_500))
                .line_height(18.0)
                .role(Role::Heading, label),
        )
        .child(div().row().items_center().gap(2.0).children(controls))
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
    tooltip(s.ui, b, id, label, None)
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

/// SessionItem: the marker, status icon, name and branch, and close on
/// hover.
fn session_row(s: &Screen<'_>, session: &Session) -> El {
    let sid = session.info.id.as_str();
    let act = if s.st.layout == crate::state::LayoutMode::Tabs {
        Act::Select(sid.to_owned())
    } else {
        Act::Expand(sid.to_owned())
    };
    let id = s.acts.on("sidebar-session", act);
    let close_id = Acts::id("sidebar-session", &Act::CloseSession(sid.to_owned()));
    let hot = s.ui.hovered(id) || s.ui.hovered(close_id);
    let active = s.st.focused.as_deref() == Some(sid)
        || (s.st.layout == crate::state::LayoutMode::Tabs && s.st.selected.as_deref() == Some(sid));
    let fg = if active || hot {
        c(look::WHITE)
    } else {
        c(look::GRAY_400)
    };
    let mut name = div().row().items_center().gap(6.0).grow().min_w0().child(
        text(session.title(), 12.0, fg)
            .line_height(16.0)
            .truncate()
            .silent(),
    );
    if let Some(b) = session.info.branch.as_deref() {
        name = name.child(
            text(b, 10.0, c(look::GRAY_600))
                .line_height(14.0)
                .truncate()
                .silent(),
        );
    }
    let mut row = div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(8.0)
        .px(8.0)
        .py(4.0)
        .rounded(6.0)
        .bg(white(0.0).mix(white(0.04), if hot { 1.0 } else { 0.0 }))
        .role(Role::Button, session.title())
        .selected(active)
        .child(super::cards::status_icon(session, 14.0, 1.0))
        .child(name);
    if active {
        row = row.child(div().absolute(0.0, 4.0).size(1.0, 16.0).bg(c(look::WHITE)));
    }
    if hot {
        s.acts.bind(close_id, Act::CloseSession(sid.to_owned()));
        let b = div()
            .id(close_id)
            .on(Action::Click)
            .role(Role::Button, "Close session")
            .child(ico(
                "x",
                12.0,
                2.0,
                c(look::GRAY_500).mix(c(look::DANGER), s.ui.hover(close_id)),
            ));
        row = row.child(tooltip(s.ui, b, close_id, "Close session", None));
    }
    row
}

/// WorktreeItem: a worktree's name and branch, and on hover a terminal or
/// session in it and its removal.
fn worktree_row(s: &Screen<'_>, p: &ProjectConfig, wt: &crate::state::Worktree) -> El {
    let quick = [
        (
            Act::NewShellIn(wt.path.clone()),
            "terminal",
            "New terminal",
            1.5,
        ),
        (
            Act::NewSessionIn {
                project: p.name.clone(),
                worktree: Some(wt.path.clone()),
                agent: None,
            },
            "plus",
            "New session",
            2.0,
        ),
        (
            Act::Git(crate::git::GitAct::Remove {
                project: p.path.clone(),
                worktree: wt.path.clone(),
            }),
            "trash-2",
            "Remove worktree",
            2.0,
        ),
    ];
    let id = Acts::id("sidebar-worktree", &Act::Nothing).child(&wt.path);
    let hot = s.ui.hovered(id)
        || quick
            .iter()
            .any(|(a, ..)| s.ui.hovered(Acts::id("sidebar-worktree", a)));
    let label = div()
        .col()
        .grow()
        .min_w0()
        .child(
            text(
                &wt.name,
                13.0,
                c(look::GRAY_400).mix(c(look::WHITE), if hot { 1.0 } else { 0.0 }),
            )
            .line_height(17.0)
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
                        .line_height(14.0)
                        .truncate()
                        .silent(),
                ),
        );
    let mut row = div()
        .id(id)
        .sense(crate::ui::Sense::HOVER)
        .row()
        .items_center()
        .gap(8.0)
        .pl(8.0)
        .pr(8.0)
        .py(6.0)
        .rounded(6.0)
        .bg(white(0.04 * if hot { 1.0 } else { 0.0 }))
        .role(Role::Group, wt.name.as_str())
        .child(ico("folder-git-2", 14.0, 1.5, c(look::GRAY_500)))
        .child(label);
    if hot {
        let mut acts = div().row().items_center().gap(2.0);
        for (act, name, tip, stroke) in quick {
            let danger = matches!(act, Act::Git(_));
            let bid = s.acts.on("sidebar-worktree", act);
            let h = s.ui.hover(bid);
            let to = if danger {
                c(look::RED_400)
            } else {
                c(look::WHITE)
            };
            let b = div()
                .id(bid)
                .on(Action::Click)
                .p(2.0)
                .rounded(4.0)
                .bg(white(0.08 * h))
                .role(Role::Button, tip)
                .child(ico(name, 14.0, stroke, c(look::GRAY_500).mix(to, h)));
            acts = acts.child(tooltip(s.ui, b, bid, tip, None));
        }
        row = row.child(acts);
    }
    row
}

/// ProjectItem: icon, name and count, its quick actions on hover, and its
/// sessions under it while expanded, those in a worktree under its row.
fn project_row(s: &Screen<'_>, p: &ProjectConfig) -> El {
    let sessions: Vec<&Session> = s
        .store
        .sessions
        .iter()
        .filter(|x| x.info.project_name == p.name && s.st.panes.holder(&x.info.id).is_none())
        .collect();
    let worktrees: Vec<&crate::state::Worktree> =
        s.st.worktrees
            .get(&p.path)
            .into_iter()
            .flatten()
            .filter(|w| !w.main)
            .collect();
    let active = s.st.project.as_deref() == Some(p.name.as_str());
    let id = s
        .acts
        .on("sidebar-project", Act::Project(Some(p.name.clone())));
    let quick = [
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
        (
            Act::NewSessionIn {
                project: p.name.clone(),
                worktree: Some(String::new()),
                agent: None,
            },
            "folder-git-2",
            "New worktree",
            1.5,
        ),
        (
            Act::Open(MenuKind::Project(p.name.clone())),
            "ellipsis",
            "More",
            2.0,
        ),
    ];
    let hot = s.ui.hovered(id)
        || quick
            .iter()
            .any(|(a, ..)| s.ui.hovered(Acts::id("sidebar-project", a)));
    let open_menu =
        s.st.menu
            .as_ref()
            .is_some_and(|m| m.kind == MenuKind::Project(p.name.clone()));
    let fg = if active {
        c(look::WHITE)
    } else {
        c(look::GRAY_300).mix(c(look::WHITE), if hot { 1.0 } else { 0.0 })
    };
    let expanded =
        (!sessions.is_empty() || !worktrees.is_empty()) && !s.st.collapsed.contains(&p.name);
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
        .expanded(expanded);
    row = if hot && (!sessions.is_empty() || !worktrees.is_empty()) {
        let toggle = s
            .acts
            .on("sidebar-project", Act::ToggleProject(p.name.clone()));
        row.child(
            div()
                .id(toggle)
                .on(Action::Click)
                .size(14.0, 14.0)
                .center()
                .role(Role::Button, "Toggle sessions")
                .child(ico(
                    if expanded {
                        "chevron-down"
                    } else {
                        "chevron-right"
                    },
                    12.0,
                    2.0,
                    c(look::GRAY_500),
                )),
        )
    } else {
        row.child(project_icon(p, 14.0))
    };
    row = row.child(
        div().row().grow().min_w0().child(
            text(p.name.as_str(), 13.0, fg)
                .line_height(18.0)
                .truncate()
                .silent(),
        ),
    );
    if hot || open_menu {
        let mut acts = div().row().items_center().gap(2.0);
        for (act, name, label, stroke) in quick {
            let bid = s.acts.on("sidebar-project", act);
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
            acts = acts.child(tooltip(s.ui, b, bid, label, None));
        }
        row = row.child(acts);
    } else if !sessions.is_empty() {
        row = row.child(
            text(sessions.len().to_string(), 12.0, c(look::GRAY_600))
                .line_height(16.0)
                .silent(),
        );
    }
    let row = s
        .acts
        .context(row, id, Act::Open(MenuKind::Project(p.name.clone())));
    let mut col = div().col().child(row);
    if expanded {
        let in_wt = |x: &Session, w: &str| x.info.worktree_path.as_deref() == Some(w);
        let mut kids = div().col().ml(16.0);
        for x in sessions
            .iter()
            .filter(|x| !worktrees.iter().any(|w| in_wt(x, &w.path)))
        {
            kids = kids.child(session_row(s, x));
        }
        for w in &worktrees {
            kids = kids.child(worktree_row(s, p, w));
            for x in sessions.iter().filter(|x| in_wt(x, &w.path)) {
                kids = kids.child(div().ml(16.0).child(session_row(s, x)));
            }
        }
        col = col.child(kids);
    }
    col
}

fn projects(s: &Screen<'_>) -> El {
    let add = control(s, Act::AddProject, "folder-plus", "Add project", 13.0, 1.5);
    let mut col = div()
        .col()
        .gap(2.0)
        .child(section(s, "Projects", vec![add]))
        .child(nav_item(
            s,
            Act::Project(None),
            ico("layers", 14.0, 1.5, c(look::GRAY_400)),
            "All Projects",
            (s.st.view == MainView::Sessions).then_some(s.store.sessions.len()),
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
        col = col.child(if s.st.view == MainView::Sessions {
            project_row(s, p)
        } else {
            nav_item(
                s,
                Act::Project(Some(p.name.clone())),
                project_icon(p, 14.0),
                &p.name,
                None,
                s.st.project.as_deref() == Some(p.name.as_str()),
            )
        });
    }
    col
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

/// SidebarFooter: the ready-update notice above the guide and settings
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
        true,
    );
    let close = s.acts.on("update-notice", Act::Set(SetAct::DismissUpdate));
    let go = s.acts.on("update-notice", Act::Set(SetAct::CheckUpdates));
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
        _ => projects(s),
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
