//! The window's chrome: the 40 px toolbar (`App.tsx`), the window controls
//! drawn when there is no system title bar (`WindowControls`), the headless
//! badge and recent-sessions popovers, the restored banner and toasts.

use super::{c, ico, truncate, white, MainView, Screen, MOD_LABEL};
use crate::act::{Act, Acts};
use crate::look;
use crate::state::{now_ms, short_duration, time_ago, MenuKind};
use crate::ui::{div, icon, text, tooltip, Action, El, Id, Role, Sense};

/// The toolbar's drag region; a double press maximizes.
pub const TITLE_BAR: Id = Id(0x7469_746c_6562_6172);

/// A `p-1 rounded-md` toolbar button: gray-400, white on `white/[0.06]`
/// when hovered, white on `white/10` while its popover is open.
#[allow(clippy::too_many_arguments)]
pub(super) fn tool(
    s: &Screen<'_>,
    place: &str,
    act: Act,
    name: &str,
    stroke: f32,
    label: &str,
    shortcut: Option<&str>,
    active: bool,
) -> El {
    let id = s.acts.on(place, act);
    let hot = s.ui.hover(id);
    let (fg, bg) = if active {
        (c(look::WHITE), white(0.10))
    } else {
        (
            c(look::GRAY_400).mix(c(look::WHITE), hot),
            white(0.0).mix(white(0.06), hot),
        )
    };
    let b = div()
        .id(id)
        .on(Action::Click)
        .p(4.0)
        .rounded(6.0)
        .bg(bg)
        .role(Role::Button, label)
        .child(ico(name, 16.0, stroke, fg));
    tooltip(s.ui, b, id, label, shortcut)
}

/// `w-px h-4 bg-white/[0.06] mx-0.5`.
pub(super) fn divider() -> El {
    div().w(1.0).h(16.0).bg(white(0.06)).mx(2.0).shrink0()
}

fn menu_open(s: &Screen<'_>, kind: &MenuKind) -> bool {
    s.st.menu.as_ref().is_some_and(|m| &m.kind == kind)
}

fn pills(s: &Screen<'_>) -> El {
    let views = [
        (MainView::Sessions, format!("{MOD_LABEL}S")),
        (MainView::Tasks, format!("{MOD_LABEL}T")),
        (MainView::Workflows, format!("{MOD_LABEL}\u{21e7}W")),
    ];
    let mut row = div()
        .row()
        .bg(white(0.04))
        .rounded(8.0)
        .p(2.0)
        .gap(2.0)
        .role(Role::TabList, "Main view");
    for ((name, label), (view, keys)) in look::PILLS.iter().zip(views) {
        let on = view == s.st.view;
        let id = s.acts.on("pill", Act::View(view));
        let hot = s.ui.hover(id);
        let fg = if on {
            c(look::WHITE)
        } else {
            c(look::GRAY_500).mix(c(look::GRAY_300), hot)
        };
        let pill = div()
            .id(id)
            .on(Action::Click)
            .px(10.0)
            .py(4.0)
            .rounded(6.0)
            .bg(if on { white(0.1) } else { white(0.0) })
            .role(Role::Tab, *label)
            .selected(on)
            .child(ico(name, 14.0, 2.0, fg));
        row = row.child(tooltip(s.ui, pill, id, label, Some(&keys)));
    }
    row
}

/// SidebarToggle, a divider and the view pills: what the toolbar starts
/// with while the sidebar is closed.
pub(super) fn nav_cluster(s: &Screen<'_>) -> El {
    div()
        .row()
        .items_center()
        .gap(4.0)
        .child(tool(
            s,
            "toolbar",
            Act::ToggleSidebar,
            "panel-left",
            2.0,
            "Toggle sidebar",
            Some(&format!("{MOD_LABEL}B")),
            false,
        ))
        .child(divider())
        .child(pills(s))
}

/// SessionDock, collapsed to its chip; not there when nothing waits.
fn dock(approvals: usize) -> Option<El> {
    if approvals == 0 {
        return None;
    }
    let label = format!(
        "{approvals} waiting approval{}",
        if approvals == 1 { "" } else { "s" }
    );
    let chip = div()
        .row()
        .items_center()
        .gap(6.0)
        .h(26.0)
        .px(8.0)
        .rounded(6.0)
        .border(1.0, white(0.06))
        .bg(c(look::SURFACE_RAISED))
        .role(Role::Button, label)
        .child(ico("layers", 11.0, 1.5, c(look::GRAY_300)))
        .child(
            text(approvals.to_string(), 11.0, c(look::GRAY_300))
                .weight(500)
                .line_height(11.0)
                .silent(),
        )
        .child(
            div()
                .size(6.0, 6.0)
                .rounded(3.0)
                .bg(c(look::BRONZO))
                .absolute_right(-2.0, -2.0),
        );
    Some(chip)
}

/// HeadlessBadge: hidden without headless sessions; a zap and the count,
/// with a green dot while one runs and a red one if one failed.
fn headless_badge(s: &Screen<'_>) -> Option<El> {
    let list = &s.st.headless;
    if list.is_empty() {
        return None;
    }
    let running = list.iter().filter(|h| h.status == "running").count();
    let errored = list.iter().filter(|h| h.status == "error").count();
    let id = s.acts.on("toolbar", Act::Open(MenuKind::Headless));
    let hot = s.ui.hover(id);
    let mut tip = format!("{} headless", list.len());
    if running > 0 {
        tip.push_str(&format!(" \u{b7} {running} running"));
    }
    if errored > 0 {
        tip.push_str(&format!(" \u{b7} {errored} errored"));
    }
    let mut b = div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(6.0)
        .h(26.0)
        .px(8.0)
        .rounded(6.0)
        .border(1.0, white(0.06))
        .bg(c(look::SURFACE_OVERLAY).mix(c(look::SURFACE_RAISED), hot))
        .role(Role::Button, tip.as_str())
        .child(ico("zap", 12.0, 2.0, c(look::GRAY_300)))
        .child(
            text(list.len().to_string(), 11.0, c(look::GRAY_300))
                .mono()
                .line_height(11.0)
                .silent(),
        );
    if running > 0 || errored > 0 {
        let dot = if errored > 0 {
            look::RED_400
        } else {
            look::GREEN_400
        };
        b = b.child(div().size(6.0, 6.0).rounded(3.0).bg(c(dot)));
    }
    Some(tooltip(s.ui, b, id, &tip, None))
}

/// The headless popover: a pill per session with kill and hide on hover.
pub(super) fn headless_panel(s: &Screen<'_>) -> El {
    let now = now_ms();
    let mut col = div()
        .col()
        .min_w(280.0)
        .p(6.0)
        .gap(6.0)
        .bg(c(look::SURFACE_OVERLAY))
        .border(1.0, white(0.08))
        .rounded(8.0)
        .role(Role::Menu, "Headless sessions");
    for h in &s.st.headless {
        let row_id = Acts::id("headless", &Act::ToggleHeadless(h.id.clone()));
        let hot = s.ui.hovered(row_id) || s.ui.hover(row_id) > 0.0;
        let dot = match h.status.as_str() {
            "running" => look::GREEN_400,
            "error" => look::RED_400,
            _ => look::GRAY_500,
        };
        let mut pill = div()
            .id(row_id)
            .sense(Sense::HOVER)
            .row()
            .items_center()
            .gap(6.0)
            .px(8.0)
            .h(26.0)
            .rounded(6.0)
            .border(1.0, white(0.06))
            .bg(white(0.02))
            .role(Role::Group, h.name.as_str())
            .child(div().size(6.0, 6.0).rounded(3.0).bg(c(dot)))
            .child(
                text(truncate(&h.name, 28), 11.0, c(look::GRAY_200))
                    .line_height(14.0)
                    .silent(),
            );
        if let Some(b) = &h.branch {
            pill = pill.child(
                text(truncate(b, 18), 10.0, c(look::GRAY_500))
                    .mono()
                    .line_height(14.0)
                    .silent(),
            );
        }
        pill = pill.child(div().grow());
        if h.started > 0.0 {
            pill = pill.child(
                text(short_duration(now - h.started), 10.0, c(look::GRAY_500))
                    .mono()
                    .line_height(14.0)
                    .silent(),
            );
        }
        if let Some(code) = h.exit {
            pill = pill.child(
                text(
                    format!("exit {code}"),
                    10.0,
                    c(if code == 0 {
                        look::GRAY_500
                    } else {
                        look::RED_400
                    }),
                )
                .mono()
                .line_height(14.0)
                .silent(),
            );
        }
        let small = |place: &str, act: Act, name: &str, label: &str| {
            let id = s.acts.on(place, act);
            div()
                .id(id)
                .on(Action::Click)
                .p(2.0)
                .rounded(4.0)
                .role(Role::Button, label)
                .child(ico(
                    name,
                    11.0,
                    2.0,
                    c(look::GRAY_400).mix(c(look::WHITE), s.ui.hover(id)),
                ))
        };
        if hot {
            if h.status == "running" {
                pill = pill.child(small(
                    "headless",
                    Act::KillHeadless(h.id.clone()),
                    "square",
                    "Kill",
                ));
            }
            pill = pill.child(small(
                "headless",
                Act::HideHeadless(h.id.clone()),
                "x",
                "Hide",
            ));
        }
        col = col.child(pill);
    }
    col
}

/// RecentSessionsPopover: the sessions this machine ran here, newest
/// first, resumable ones on a click.
pub(super) fn recent_panel(s: &Screen<'_>) -> El {
    let scope = s.st.project.as_deref().unwrap_or("All Projects");
    let header = div()
        .row()
        .items_center()
        .gap(6.0)
        .px(16.0)
        .py(12.0)
        .child(
            text("Recent Sessions", 13.0, c(look::WHITE))
                .weight(500)
                .line_height(18.0)
                .role(Role::Heading, "Recent Sessions"),
        )
        .child(
            text(format!("\u{b7} {scope}"), 12.0, c(look::GRAY_500))
                .line_height(16.0)
                .silent(),
        )
        .child(div().grow())
        .child(
            text(s.st.recent.len().to_string(), 11.0, c(look::GRAY_600))
                .line_height(14.0)
                .silent(),
        );
    let now = now_ms();
    let mut list = div()
        .id(Id::new("recent-list"))
        .col()
        .scroll_y()
        .max_h(340.0)
        .px(4.0)
        .pb(4.0);
    if s.st.recent.is_empty() {
        list = list.child(
            div().py(24.0).center().child(
                text("No recent sessions", 12.0, c(look::GRAY_500))
                    .line_height(16.0)
                    .role(Role::Label, "No recent sessions"),
            ),
        );
    }
    for (i, r) in s.st.recent.iter().enumerate() {
        let id = s.acts.on("recent", Act::ResumeRecent(i));
        let hot = s.ui.hover(id);
        let title = if r.display.is_empty() {
            "Untitled session"
        } else {
            r.display.as_str()
        };
        let project = crate::state::basename(&r.project_path);
        let label = if r.activity_label.is_empty() {
            "message"
        } else {
            r.activity_label.as_str()
        };
        let meta = format!(
            "{project} \u{b7} {} \u{b7} {} {label}{}",
            time_ago(r.timestamp, now),
            r.activity,
            if r.activity == 1 { "" } else { "s" }
        );
        let side = if r.can_resume {
            (hot > 0.5).then(|| {
                text("resume", 10.0, c(look::GRAY_400))
                    .line_height(14.0)
                    .silent()
            })
        } else {
            Some(
                text("history only", 10.0, c(look::GRAY_600))
                    .line_height(14.0)
                    .silent(),
            )
        };
        let row = div()
            .id(id)
            .on(Action::Click)
            .row()
            .items_center()
            .gap(10.0)
            .px(12.0)
            .py(8.0)
            .rounded(8.0)
            .bg(white(0.0).mix(white(0.04), hot))
            .role(Role::Button, title)
            .child(ico("bot", 14.0, 2.0, c(look::GRAY_400)))
            .child(
                div()
                    .col()
                    .grow()
                    .min_w0()
                    .child(
                        text(title, 12.0, c(look::GRAY_300))
                            .line_height(16.0)
                            .truncate()
                            .silent(),
                    )
                    .child(
                        text(meta, 10.0, c(look::GRAY_600))
                            .line_height(14.0)
                            .truncate()
                            .silent(),
                    ),
            )
            .child_if(side);
        list = list.child(row);
    }
    div()
        .col()
        .w(380.0)
        .bg(c(look::SURFACE_OVERLAY))
        .border(1.0, white(0.08))
        .rounded(12.0)
        .clip()
        .role(Role::Dialog, "Recent sessions")
        .child(header)
        .child(list)
}

/// The glyphs of `WindowControls`, 10 px at a 1.2 stroke.
fn glyph(kind: &str) -> String {
    let body = match kind {
        "min" => r#"<path d="M0 5h10"/>"#,
        "max" => r#"<rect x="0.6" y="0.6" width="8.8" height="8.8"/>"#,
        "restore" => {
            r#"<rect x="0.6" y="2.6" width="6.8" height="6.8"/><path d="M2.6 2.6V0.6h6.8v6.8H7.4"/>"#
        }
        _ => r#"<path d="M0.5 0.5l9 9M9.5 0.5l-9 9"/>"#,
    };
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="#ffffff" stroke-width="1.2">{body}</svg>"##
    )
}

/// Minimize, maximize or restore, and close: 46 by 32, the close one
/// turning red.
pub(super) fn window_controls(s: &Screen<'_>) -> Option<El> {
    if !s.chrome {
        return None;
    }
    let max = s.ui.maximized();
    let buttons = [
        (Act::Minimize, "min", "Minimize"),
        (
            Act::ToggleMaximize,
            if max { "restore" } else { "max" },
            if max { "Restore" } else { "Maximize" },
        ),
        (Act::CloseWindow, "close", "Close"),
    ];
    let mut row = div().row().items_center().ml(8.0).mx(0.0);
    for (act, g, label) in buttons {
        let close = act == Act::CloseWindow;
        let id = s.acts.on("window", act);
        let hot = s.ui.hover(id);
        let bg = if close {
            white(0.0).mix(crate::ui::Rgba::hexa(look::RED_500, 0.8), hot)
        } else {
            white(0.0).mix(white(0.08), hot)
        };
        let fg = c(look::GRAY_400).mix(c(look::WHITE), hot);
        let b = div()
            .id(id)
            .on(Action::Click)
            .size(46.0, 32.0)
            .center()
            .bg(bg)
            .role(Role::Button, label)
            .child(icon(glyph(g), 10.0, fg));
        row = row.child(b);
    }
    Some(row)
}

/// What the toolbar shows on its right: grid options, recent sessions,
/// new session and the window controls. The tab strip ends with the same.
pub(super) fn right_cluster(s: &Screen<'_>, plus: bool) -> El {
    if s.st.view == MainView::Workflows {
        return super::workflows::header_tools(s);
    }
    let mut right = div().row().items_center().gap(4.0);
    if s.st.view == MainView::Tasks {
        return right
            .child(task_options(s))
            .child(tool(
                s,
                "toolbar",
                Act::Task(crate::tasks::TaskAct::New("todo".into())),
                "plus",
                2.0,
                "Add task",
                None,
                false,
            ))
            .child_if(window_controls(s));
    }
    if s.st.view == MainView::Sessions {
        let open = menu_open(s, &MenuKind::ViewOptions);
        let mut opts = tool(
            s,
            "toolbar",
            Act::Open(MenuKind::ViewOptions),
            "sliders-horizontal",
            1.5,
            "View options",
            Some(&format!("{MOD_LABEL}J")),
            open,
        );
        if s.st.options_changed() && !open {
            opts = div().child(opts).child(
                div()
                    .size(6.0, 6.0)
                    .rounded(3.0)
                    .bg(crate::ui::Rgba::hexa(look::INK, look::INK_SECONDARY))
                    .absolute_right(2.0, 2.0),
            );
        }
        right = right.child(opts).child(divider());
    }
    right = right.child(tool(
        s,
        "toolbar",
        Act::Open(MenuKind::Recent),
        "rotate-ccw",
        1.5,
        "Recent sessions",
        None,
        menu_open(s, &MenuKind::Recent),
    ));
    if plus {
        right = right.child(tool(
            s,
            "toolbar",
            Act::Open(MenuKind::Grid),
            "plus",
            2.0,
            "New session",
            None,
            menu_open(s, &MenuKind::Grid),
        ));
    }
    right.child_if(window_controls(s))
}

/// TaskToolbar's trigger: ink-secondary, white with a dot while a filter is on.
fn task_options(s: &Screen<'_>) -> El {
    let open = menu_open(s, &MenuKind::TaskOptions);
    let changed = s.st.tasks.options_changed();
    let id = s.acts.on("toolbar", Act::Open(MenuKind::TaskOptions));
    let hot = s.ui.hover(id);
    let secondary = crate::ui::Rgba::hexa(look::INK, look::INK_SECONDARY);
    let (fg, bg) = if open {
        (c(look::WHITE), white(0.10))
    } else if changed {
        (c(look::WHITE), white(0.08).mix(white(0.12), hot))
    } else {
        (
            secondary.mix(c(look::WHITE), hot),
            white(0.0).mix(white(0.06), hot),
        )
    };
    let mut b = div()
        .id(id)
        .on(Action::Click)
        .p(4.0)
        .rounded(6.0)
        .bg(bg)
        .role(Role::Button, "View options")
        .child(ico("sliders-horizontal", 16.0, 1.5, fg));
    if changed && !open {
        b = b.child(
            div()
                .size(6.0, 6.0)
                .rounded(3.0)
                .bg(secondary)
                .absolute_right(2.0, 2.0),
        );
    }
    tooltip(s.ui, b, id, "View options", Some(&format!("{MOD_LABEL}J")))
}

/// The left of the toolbar after the nav cluster: the dock and the
/// headless badge.
pub(super) fn left_extras(s: &Screen<'_>) -> El {
    div()
        .row()
        .items_center()
        .gap(8.0)
        .child_if(dock(s.store.approvals()))
        .child_if(headless_badge(s))
}

/// Makes `el` the window's title bar when the app draws its own frame.
pub(super) fn draggable(s: &Screen<'_>, el: El) -> El {
    if !s.chrome {
        return el;
    }
    s.acts.double(TITLE_BAR, Act::ToggleMaximize);
    el.id(TITLE_BAR).sense(Sense::WINDOW_DRAG | Sense::DOUBLE)
}

pub(super) fn top_bar(s: &Screen<'_>) -> El {
    let mut left = div().row().items_center().gap(8.0);
    if !s.st.sidebar {
        left = left.child(nav_cluster(s));
    }
    if s.st.view == MainView::Sessions {
        left = left.child(left_extras(s));
    }
    let crumb = s.st.project.as_deref().map(|p| {
        text(truncate(p, 40), 12.0, c(look::GRAY_400))
            .line_height(16.0)
            .role(Role::Label, p)
    });
    let bar = div()
        .row()
        .items_center()
        .justify_between()
        .h(look::TOP_BAR_H - 1.0)
        .pl(12.0)
        .pr(if s.chrome { 0.0 } else { 12.0 })
        .role(Role::Toolbar, "Toolbar")
        .child(left)
        .child_if(crumb)
        .child(right_cluster(s, true));
    draggable(
        s,
        div()
            .col()
            .shrink0()
            .bg(c(look::SURFACE_BASE))
            .child(bar)
            .child(div().h(1.0).w_full().bg(white(0.06))),
    )
}

/// SessionRestoredBanner: the sessions the last run left, offered back.
pub(super) fn banner(s: &Screen<'_>) -> Option<El> {
    let prev = &s.st.banner;
    if prev.is_empty() {
        return None;
    }
    let n = prev.len();
    let mut projects: Vec<&str> = prev.iter().map(|p| p.project_name.as_str()).collect();
    projects.sort_unstable();
    projects.dedup();
    let scope = if projects.len() == 1 {
        projects[0].to_owned()
    } else {
        format!("{} projects", projects.len())
    };
    let restore_id = s.acts.on("banner", Act::Restore);
    let label = if s.st.restoring {
        "Restoring..."
    } else {
        "Restore"
    };
    let restore = div()
        .id(restore_id)
        .on(Action::Click)
        .px(12.0)
        .py(4.0)
        .rounded(6.0)
        .bg(c(look::BRONZO))
        .role(Role::Button, label)
        .child(
            text(label, 12.0, c(0x000000))
                .weight(500)
                .line_height(16.0)
                .silent(),
        );
    let dismiss_id = s.acts.on("banner", Act::DismissBanner);
    let dismiss = div()
        .id(dismiss_id)
        .on(Action::Click)
        .p(4.0)
        .role(Role::Button, "Dismiss")
        .child(ico(
            "x",
            14.0,
            2.0,
            c(look::GRAY_500).mix(c(look::GRAY_300), s.ui.hover(dismiss_id)),
        ));
    let copy = format!(
        "{n} previous session{} can be restored",
        if n == 1 { "" } else { "s" }
    );
    Some(
        div()
            .row()
            .items_center()
            .justify_between()
            .mx(16.0)
            .mt(16.0)
            .px(16.0)
            .py(12.0)
            .border(1.0, white(0.08))
            .bg(white(0.03))
            .rounded(8.0)
            .role(Role::Status, copy.as_str())
            .child(
                div()
                    .row()
                    .items_center()
                    .gap(12.0)
                    .child(ico("rotate-ccw", 16.0, 2.0, c(look::GRAY_400)))
                    .child(
                        text(copy.as_str(), 14.0, c(look::GRAY_300))
                            .line_height(20.0)
                            .silent(),
                    )
                    .child(
                        text(format!(" \u{b7} {scope}"), 14.0, c(look::GRAY_500))
                            .line_height(20.0)
                            .silent(),
                    ),
            )
            .child(
                div()
                    .row()
                    .items_center()
                    .gap(8.0)
                    .ml(16.0)
                    .child(restore)
                    .child(dismiss),
            ),
    )
}

/// The toasts, newest at the bottom right.
pub(super) fn toasts(s: &Screen<'_>) -> Option<El> {
    if s.st.toasts.is_empty() {
        return None;
    }
    let mut col = div().col().gap(8.0);
    for t in &s.st.toasts {
        let (glyph, tint) = if t.error {
            ("x", look::RED_400)
        } else {
            ("check", look::GREEN_400)
        };
        let close = s.acts.on("toast", Act::DismissToast(t.text.clone()));
        let hover = s.ui.hover(close);
        col = col.child(
            div()
                .row()
                .items_center()
                .gap(10.0)
                .px(16.0)
                .py(10.0)
                .min_w(200.0)
                .max_w(360.0)
                .rounded(8.0)
                .border(1.0, c(tint).alpha(0.2))
                .bg(crate::ui::Rgba::hexa(0x1a1a1e, 0.92))
                .role(Role::Status, t.text.as_str())
                .child(ico(glyph, 15.0, 2.5, c(tint)))
                .child(
                    text(t.text.as_str(), 14.0, c(look::GRAY_200))
                        .line_height(20.0)
                        .truncate()
                        .silent(),
                )
                .child(div().grow())
                .child(
                    div()
                        .id(close)
                        .on(Action::Click)
                        .p(2.0)
                        .role(Role::Button, "Dismiss")
                        .child(ico(
                            "x",
                            12.0,
                            2.0,
                            c(look::GRAY_500).mix(c(look::GRAY_300), hover),
                        )),
                ),
        );
    }
    Some(
        div()
            .col()
            .absolute(0.0, 0.0)
            .size(s.size.0, s.size.1)
            .p(20.0)
            .justify_end()
            .items_end()
            .layered()
            .child(col),
    )
}
