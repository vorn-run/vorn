//! The window's chrome: the 40 px toolbar, the window controls
//! drawn when there is no system title bar, the headless
//! badge and recent-sessions popovers, the restored banner and toasts.

use super::{c, ico, mod_label, truncate, white, MainView, Screen};
use crate::act::Act;
use crate::components::glyph;
use crate::headless::Headless;
use crate::look::{self, Color};
use crate::state::{now_ms, time_ago, MenuKind};
use crate::ui::widgets::{self, Pill, ToastKind};
use crate::ui::{div, icon, text, tooltip, tooltip_at, Action, El, Id, Place, Rgba, Role, Sense};
use crate::workflows::WfAct;

/// The toolbar's drag region; a double press maximizes.
pub const TITLE_BAR: Id = Id(0x7469_746c_6562_6172);

/// A toolbar button: gray, white on a faint white wash when hovered, and on
/// a stronger one while its popover is open.
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
        .bg(bg)
        .role(Role::Button, label);
    let b = match glass(s).then(|| look::icon(name, stroke)).flatten() {
        Some(svg) => b.size(28.0, 28.0).rounded(14.0).native_icon(&svg, 16.0, fg),
        None => b.p(4.0).rounded(6.0).child(ico(name, 16.0, stroke, fg)),
    };
    tooltip(s.ui, b, id, label, shortcut)
}

/// A thin vertical rule between toolbar groups.
pub(super) fn divider() -> El {
    div().w(1.0).h(16.0).bg(white(0.06)).mx(2.0).shrink0()
}

/// Whether toolbar buttons sit on glass: over the system's material.
pub(super) fn glass(s: &Screen<'_>) -> bool {
    s.ui.see_through() && crate::ui::palette::materials() != vorn_theme::Materials::Solid
}

/// Toolbar buttons that belong together: one glass capsule over the
/// system's material, a plain row otherwise.
pub(super) fn group(s: &Screen<'_>, buttons: impl IntoIterator<Item = El>) -> El {
    let row = div().row().items_center();
    let row = buttons.into_iter().fold(row, El::child);
    if !glass(s) {
        return row.gap(4.0);
    }
    row.gap(2.0)
        .h(32.0)
        .px(3.0)
        .rounded(16.0)
        .native_glass()
        .shrink0()
}

fn menu_open(s: &Screen<'_>, kind: &MenuKind) -> bool {
    s.st.menu.as_ref().is_some_and(|m| &m.kind == kind)
}

/// The main view pills; choosing one reports `Event::Select` on this group.
pub const VIEWS: Id = Id(0x7669_6577_7069_6c6c);

/// One top-bar button: Sessions, or a view an extension adds.
pub(super) struct TopButton {
    pub act: Act,
    pub view: MainView,
    pub icon: String,
    pub label: String,
    pub keys: Option<String>,
    pub on: bool,
}

/// The top bar's buttons in order, indexed by `Event::Select` on [`VIEWS`].
pub(super) fn top_buttons(s: &Screen<'_>) -> Vec<TopButton> {
    let sessions = TopButton {
        act: Act::View(MainView::Sessions),
        view: MainView::Sessions,
        icon: "square-terminal".to_owned(),
        label: "Sessions".to_owned(),
        keys: Some(format!("{m}S", m = mod_label())),
        on: s.st.view == MainView::Sessions,
    };
    let shown = super::view_point(&s.st.points, s.st.view, s.st.ext_view.as_deref())
        .map(crate::points::Point::key);
    let views = s.st.points.top_bar().into_iter().map(|p| TopButton {
        act: MainView::act(p),
        view: MainView::of(p),
        icon: p.icon.clone().unwrap_or_else(|| "puzzle".to_owned()),
        label: p.title.clone(),
        keys: p.key.as_deref().map(key_label),
        on: shown.as_deref() == Some(p.key().as_str()),
    });
    std::iter::once(sessions).chain(views).collect()
}

/// A manifest's key, `Mod+Shift+W`, as this platform writes it.
fn key_label(spec: &str) -> String {
    spec.split('+')
        .map(|part| match part {
            "Mod" => mod_label(),
            "Shift" => "\u{21e7}",
            "Alt" if crate::ui::mods::mac() => "\u{2325}",
            "Ctrl" if crate::ui::mods::mac() => "\u{2303}",
            "Alt" => "Alt+",
            "Ctrl" => "Ctrl+",
            key => key,
        })
        .collect()
}

/// Icon-only pills whose tooltips name the view.
fn pills(s: &Screen<'_>) -> El {
    let buttons = top_buttons(s);
    let icons: Vec<String> = buttons
        .iter()
        .map(|b| look::icon(&b.icon, 2.0).unwrap_or_default())
        .collect();
    let waiting = waiting(s).len();
    let count = if waiting > 9 {
        "9+".to_owned()
    } else {
        waiting.to_string()
    };
    let hint = format!("Workflows \u{b7} {waiting} awaiting review");
    let items: Vec<Pill<'_>> = buttons
        .iter()
        .zip(&icons)
        .map(|(b, svg)| {
            let pill = Pill::icon_only(&b.label, svg, b.keys.as_deref().unwrap_or_default());
            if b.view == MainView::Workflows && waiting > 0 {
                pill.badge(&count, &hint)
            } else {
                pill
            }
        })
        .collect();
    // A view whose button is hidden leaves no pill chosen.
    let on = buttons.iter().position(|b| b.on).unwrap_or(usize::MAX);
    widgets::pills(s.ui, VIEWS, "Main view", &items, on)
}

/// SidebarToggle, a divider and the view pills: what the toolbar starts
/// with while the sidebar is closed.
pub(super) fn nav_cluster(s: &Screen<'_>) -> El {
    let toggle = tool(
        s,
        "toolbar",
        Act::ToggleSidebar,
        "panel-left",
        2.0,
        "Toggle sidebar",
        Some(&format!("{m}B", m = mod_label())),
        false,
    );
    let row = div().row().items_center();
    if glass(s) {
        return row
            .gap(8.0)
            .ml(6.0)
            .child(group(s, [toggle]))
            .child(pills(s));
    }
    row.gap(4.0).child(toggle).child(divider()).child(pills(s))
}

/// The dock's popover, as the one menu open: a press outside closes it.
pub const DOCK: Id = Id(0x646f_636b_706f_7076);

/// One thing waiting for a person, as an extension's attention item says:
/// what opens it and answers it, and what its pill says.
struct Waiting<'a> {
    open: Act,
    approve: Act,
    reject: Act,
    title: &'a str,
    icon: &'a str,
    icon_color: Option<&'a str>,
    message: Option<&'a str>,
}

/// Everything waiting for a person in the active workspace, oldest first,
/// from the extensions' attention items. A Workflows gate opens its
/// run and is answered there, as the run pane answers it.
fn waiting<'a>(s: &'a Screen<'_>) -> Vec<Waiting<'a>> {
    use crate::points::PointAct;
    let ws = s.store.workspace();
    s.st.points
        .attention(ws)
        .into_iter()
        .map(|(p, row)| {
            let text = |k: &str| row.str(k).filter(|v| !v.is_empty());
            if p.extension == crate::workflows::ID {
                let run = text("run").unwrap_or_default().to_owned();
                let node = text("node").unwrap_or_default();
                let gate = crate::gates::key(crate::gates::Place::Pane, &run, node);
                let workflow =
                    text("workflow").and_then(|id| s.store.workflows.iter().find(|w| w.id == id));
                return Waiting {
                    open: Act::ShowRun(run),
                    approve: Act::Gate(crate::gates::GateOp::Approve(gate.clone())),
                    reject: Act::Gate(crate::gates::GateOp::Reject(gate)),
                    title: workflow
                        .map_or_else(|| text("subtitle").unwrap_or(""), |w| w.name.as_str()),
                    icon: workflow
                        .and_then(|w| look::icon_map(&w.icon))
                        .unwrap_or("workflow"),
                    icon_color: workflow.map(|w| w.icon_color.as_str()),
                    message: text("message"),
                };
            }
            let answer = |allow: bool| {
                Act::Point(PointAct::Answer {
                    key: p.key(),
                    row: row.id.clone(),
                    allow,
                })
            };
            Waiting {
                open: answer(true),
                approve: answer(true),
                reject: answer(false),
                title: text("title").unwrap_or(p.title.as_str()),
                icon: "puzzle",
                icon_color: None,
                message: text("message").or_else(|| text("subtitle")),
            }
        })
        .collect()
}

/// Waiting approvals, then minimized cards; nothing when
/// neither.
fn dock(s: &Screen<'_>) -> Option<El> {
    use crate::components::dock::{
        approval_pill, dock, minimized_pill, Approval, ApprovalIds, DockIds, DockKind, DockState,
        Minimized,
    };
    use crate::components::Status;
    let mut items = Vec::new();
    for w in waiting(s) {
        let ids = ApprovalIds {
            open: s.acts.on("dock", w.open),
            approve: s.acts.on("dock", w.approve),
            reject: s.acts.on("dock", w.reject),
        };
        let pill = approval_pill(
            s.ui,
            ids,
            &Approval {
                workflow: w.title,
                icon: w.icon,
                icon_color: super::workflows::hex(w.icon_color),
                message: w.message,
            },
        );
        items.push((DockKind::Waiting, pill));
    }
    for i in s.st.docked(s.store) {
        let session = &s.store.sessions[i];
        let info = &session.info;
        let name = session.title();
        let worktree = info.is_worktree == Some(true);
        let branch = match (worktree, info.worktree_name.as_deref()) {
            (true, Some(w)) if info.branch.is_some() => Some(w),
            _ => info.branch.as_deref(),
        };
        let id = s.acts.on("dock", Act::ToggleMinimized(info.id.clone()));
        let pill = minimized_pill(
            s.ui,
            s.agents,
            id,
            &Minimized {
                name: &name,
                agent: &info.agent_type,
                status: Status::parse(session.status()),
                branch,
                worktree,
            },
        );
        items.push((DockKind::Minimized, pill));
    }
    let ids = DockIds {
        chip: s.acts.on("dock", Act::DockPopover),
        overflow: s.acts.on("dock", Act::DockPopover),
        collapse: s.acts.on("dock", Act::ToggleDock),
        expand: s.acts.on("dock", Act::ToggleDock),
    };
    let state = DockState {
        collapsed: s.st.dock_collapsed,
        open: s.ui.menu_open(DOCK),
    };
    dock(s.ui, ids, state, items)
}

/// The sessions the badge counts and lists.
fn headless_visible<'a>(s: &Screen<'a>) -> Vec<&'a Headless> {
    crate::headless::shown(s.store, s.st)
}

/// Hidden without headless sessions; a zap and the count,
/// with a pulsing green dot while one runs, else a red one if one failed.
fn headless_badge(s: &Screen<'_>) -> Option<El> {
    let list = headless_visible(s);
    if list.is_empty() {
        return None;
    }
    let running = list.iter().filter(|h| h.running()).count();
    let errored = list.iter().filter(|h| h.errored()).count();
    let id = s.acts.on("toolbar", Act::Open(MenuKind::Headless));
    let hot = s.ui.hover(id);
    let mut tip = format!("{} headless", list.len());
    if running > 0 {
        tip.push_str(&format!(" \u{b7} {running} running"));
    } else if errored > 0 {
        tip.push_str(&format!(" \u{b7} {errored} errored"));
    }
    let fg = c(look::GRAY_300).mix(c(look::WHITE), hot);
    let mut b = div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(6.0)
        .h(26.0)
        .px(8.0)
        .rounded(6.0)
        .border(1.0, white(0.06).mix(white(0.12), hot))
        .bg(c(look::SURFACE_OVERLAY))
        .role(Role::Button, tip.as_str())
        .child(ico("zap", 12.0, 1.5, fg))
        .child(
            text(list.len().to_string(), 11.0, fg)
                .mono()
                .line_height(11.0)
                .silent(),
        );
    let dot = |color: Rgba| {
        div()
            .absolute_right(-2.0, -2.0)
            .size(6.0, 6.0)
            .rounded(3.0)
            .bg(color)
    };
    if running > 0 {
        b = b.child(dot(c(look::GREEN_500).alpha(glyph::pulse(s.ui))));
    } else if errored > 0 {
        b = b.child(dot(c(look::RED_500)));
    }
    Some(tooltip_at(s.ui, b, id, &tip, None, Place::Below))
}

/// The headless popover: a HeadlessPill per session.
pub(super) fn headless_panel(s: &Screen<'_>) -> El {
    let now = now_ms();
    let mut col = div()
        .id(Id::new("headless-list"))
        .col()
        .scroll_y()
        .min_w(280.0)
        .max_h(s.size.1 * 0.6)
        .p(6.0)
        .gap(6.0)
        .bg(c(look::SURFACE_OVERLAY))
        .border(1.0, white(0.08))
        .rounded(6.0)
        .role(Role::Dialog, "Headless sessions");
    for h in headless_visible(s) {
        col = col.child(headless_pill(s, h, now));
    }
    col
}

/// Status, agent, name, branch, the last line of output and
/// how long it ran; kill and hide on hover; a click unfolds its log.
fn headless_pill(s: &Screen<'_>, h: &Headless, now: f64) -> El {
    let id = s.acts.on("headless", Act::ToggleHeadless(h.id.clone()));
    let hot = s.ui.hovered(id) || s.ui.hover(id) > 0.0;
    let expanded = s.st.headless.expanded(&h.id);
    let running = h.running();
    let errored = h.errored();
    let mono = |t: String, size: f32, color: Color| {
        text(t, size, c(color)).mono().line_height(14.0).silent()
    };
    let dot_color = if running {
        look::GREEN_500
    } else if errored {
        look::RED_500
    } else {
        look::GRAY_500
    };
    let mut dot = div()
        .size(6.0, 6.0)
        .shrink0()
        .child(div().size(6.0, 6.0).rounded(3.0).bg(c(dot_color)));
    if running {
        if let Some((scale, left)) = glyph::ping(s.ui, 2.0) {
            dot = dot.child(
                div()
                    .absolute(-1.0, -1.0)
                    .size(8.0, 8.0)
                    .rounded(4.0)
                    .bg(c(look::GREEN_500).alpha(0.4 * left))
                    .transform(scale, (0.0, 0.0)),
            );
        }
    }
    let mut row = div()
        .row()
        .items_center()
        .gap(6.0)
        .child(dot)
        .child(super::agent_icon(s, &h.agent, 14.0))
        .child(
            text(h.name.as_str(), 11.0, c(look::GRAY_200))
                .weight(500)
                .line_height(14.0)
                .max_w(100.0)
                .truncate()
                .silent(),
        )
        .child(
            text("\u{b7}", 10.0, c(look::GRAY_600))
                .line_height(14.0)
                .silent(),
        );
    if let Some(b) = &h.branch {
        row = row.child(
            div()
                .row()
                .items_center()
                .gap(2.0)
                .max_w(90.0)
                .child(ico("git-branch", 9.0, 1.5, c(look::GRAY_500)))
                .child(mono(b.clone(), 10.0, look::GRAY_500).truncate()),
        );
    }
    if !expanded && !hot {
        if let Some(last) = s.st.headless.last(&h.id) {
            row = row
                .child(
                    text("\u{b7}", 10.0, c(look::GRAY_600))
                        .line_height(14.0)
                        .silent(),
                )
                .child(
                    mono(last.to_owned(), 10.0, look::GRAY_600)
                        .max_w(140.0)
                        .truncate(),
                );
        }
    }
    row = row.child(div().grow());
    if hot {
        let small = |act: Act, name: &str, label: &str, color: Color, over: Color, wash: Rgba| {
            let id = s.acts.on("headless", act);
            let t = s.ui.hover(id);
            let b = div()
                .id(id)
                .on(Action::Click)
                .p(2.0)
                .rounded(4.0)
                .bg(white(0.0).mix(wash, t))
                .role(Role::Button, label)
                .child(ico(name, 10.0, 2.0, c(color).mix(c(over), t)));
            tooltip_at(s.ui, b, id, label, None, Place::Below)
        };
        let mut acts = div().row().items_center().gap(4.0);
        if running {
            acts = acts.child(small(
                Act::KillHeadless(h.id.clone()),
                "square",
                "Kill process",
                look::RED_400,
                look::RED_300,
                c(look::RED_500).alpha(0.1),
            ));
        }
        row = row.child(acts.child(small(
            Act::HideHeadless(h.id.clone()),
            "x",
            "Hide",
            look::GRAY_500,
            look::GRAY_300,
            white(0.06),
        )));
    } else {
        row = row.child(mono(h.duration(now), 10.0, look::GRAY_500));
        if let Some(code) = h.exit {
            let (fg, bg, mark) = if code == 0 {
                (look::GREEN_400, look::GREEN_500, '\u{2713}')
            } else {
                (look::RED_400, look::RED_500, '\u{2717}')
            };
            row = row.child(
                div()
                    .px(4.0)
                    .py(1.0)
                    .rounded(4.0)
                    .bg(c(bg).alpha(0.1))
                    .child(mono(format!("{mark} {code}"), 9.0, fg).line_height(12.0)),
            );
        }
    }
    let border = if running {
        white(0.1).mix(white(0.28), glyph::border_pulse(s.ui))
    } else if errored {
        c(look::RED_500).alpha(0.25)
    } else {
        white(0.06)
    };
    let border = if hot { white(0.12) } else { border };
    let mut pill = div()
        .id(id)
        .on(Action::Click)
        .sense(Sense::HOVER)
        .col()
        .px(10.0)
        .py(4.0)
        .rounded(6.0)
        .border(1.0, border)
        .bg(c(look::SURFACE_OVERLAY))
        .opacity(if running { 1.0 } else { 0.65 })
        .role(Role::Button, h.name.as_str())
        .expanded(expanded)
        .child(row);
    if expanded {
        pill = pill.w(320.0);
    }
    let workflow = h
        .origin
        .as_ref()
        .filter(|o| o.extension == crate::headless::WORKFLOWS)
        .map(|o| (&o.id, &o.label));
    if let Some((wid, name)) = workflow {
        if let Some(w) = s.store.workflows.iter().find(|w| &w.id == wid) {
            let tag = s
                .acts
                .on("headless", Act::Workflow(WfAct::Edit(wid.clone())));
            let t = s.ui.hover(tag);
            let tint = super::sidebar::stored_color(Some(w.icon_color.as_str()))
                .unwrap_or(c(look::GRAY_500));
            pill = pill.child(
                div()
                    .id(tag)
                    .on(Action::Click)
                    .row()
                    .items_center()
                    .gap(4.0)
                    .mt(-2.0)
                    .ml(20.0)
                    .role(Role::Button, name.as_str())
                    .child(ico(
                        look::icon_map(&w.icon).unwrap_or("workflow"),
                        9.0,
                        1.5,
                        tint,
                    ))
                    .child(
                        text(
                            name.as_str(),
                            10.0,
                            c(look::GRAY_500).mix(c(look::GRAY_300), t),
                        )
                        .line_height(14.0)
                        .max_w(120.0)
                        .truncate()
                        .silent(),
                    ),
            );
        }
    }
    if expanded {
        let log = s.st.headless.log(&h.id);
        let body = if !log.is_empty() {
            log.to_owned()
        } else {
            s.st.headless
                .last(&h.id)
                .unwrap_or("Waiting for output...")
                .to_owned()
        };
        pill = pill.child(
            div()
                .id(crate::headless::log_id(&h.id))
                .scroll_y()
                .mt(6.0)
                .max_h(180.0)
                .p(8.0)
                .rounded(4.0)
                .bg(c(look::SURFACE_SUNKEN))
                .border(1.0, white(0.06))
                .child(
                    mono(body, 10.0, look::GRAY_400)
                        .line_height(16.25)
                        .wrapping(),
                ),
        );
    }
    pill
}

/// The sessions this machine ran here, newest
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
        let meta = format!(
            "{project} \u{b7} {} \u{b7} {}",
            time_ago(r.timestamp, now),
            r.activity_text()
        );
        let side = if r.running_in.is_some() {
            Some(
                text("running", 10.0, c(look::GREEN_400))
                    .line_height(14.0)
                    .silent(),
            )
        } else if r.can_resume {
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
            .child(super::agent_icon(s, &r.agent, 14.0))
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
            white(0.0).mix(look::RED_500.alpha(0.8), hot)
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
            Some(&format!("{m}J", m = mod_label())),
            open,
        );
        if s.st.options_changed() && !open {
            opts = div().child(opts).child(
                div()
                    .size(6.0, 6.0)
                    .rounded(3.0)
                    .bg(look::INK.alpha(look::INK_SECONDARY))
                    .absolute_right(2.0, 2.0),
            );
        }
        right = if glass(s) {
            right.gap(8.0).child(group(s, [opts]))
        } else {
            right.child(opts).child(divider())
        };
    }
    let recent = tool(
        s,
        "toolbar",
        Act::Open(MenuKind::Recent),
        "rotate-ccw",
        1.5,
        "Recent sessions",
        None,
        menu_open(s, &MenuKind::Recent),
    );
    let new = plus.then(|| {
        tool(
            s,
            "toolbar",
            Act::Open(MenuKind::Grid),
            "plus",
            2.0,
            "New session",
            None,
            menu_open(s, &MenuKind::Grid),
        )
    });
    right
        .child(group(s, std::iter::once(recent).chain(new)))
        .child_if(window_controls(s))
}

/// TaskToolbar's trigger: ink-secondary, white with a dot while a filter is on.
fn task_options(s: &Screen<'_>) -> El {
    let open = menu_open(s, &MenuKind::TaskOptions);
    let changed = s.st.tasks.options_changed();
    let id = s.acts.on("toolbar", Act::Open(MenuKind::TaskOptions));
    let hot = s.ui.hover(id);
    let secondary = look::INK.alpha(look::INK_SECONDARY);
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
    tooltip_at(
        s.ui,
        b,
        id,
        "View options",
        Some(&format!("{m}J", m = mod_label())),
        Place::Below,
    )
}

/// The left of the toolbar after the nav cluster: the dock and the
/// headless badge.
pub(super) fn left_extras(s: &Screen<'_>) -> El {
    div()
        .row()
        .items_center()
        .gap(8.0)
        .child_if(dock(s))
        .child_if(headless_badge(s))
}

/// Makes `el` the window's title bar when the app draws its own frame.
pub(super) fn draggable(s: &Screen<'_>, el: El) -> El {
    if !s.chrome && !s.traffic {
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
        .pl(s.lead())
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
            .bg(if s.ui.see_through() {
                crate::ui::theme::shadow(0.0)
            } else {
                c(look::SURFACE_BASE)
            })
            .child(bar)
            .child(div().h(1.0).w_full().bg(white(0.06))),
    )
}

/// The sessions the last run left, offered back.
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
            text(label, 12.0, crate::ui::theme::shadow(1.0))
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

/// What the launch step did with the `vorn` command, with its fix, until dismissed.
fn cli_notice(s: &Screen<'_>) -> Option<El> {
    use crate::cli_link::Fix;
    use crate::settings::SetAct;
    let n = s.st.cli_notice.as_ref()?;
    let kind = if n.error {
        ("triangle-alert", look::AMBER_400)
    } else {
        ("terminal", look::BLUE_400)
    };
    let actions = n
        .fix
        .iter()
        .map(|f| {
            let act = match f {
                Fix::Install(_) => Act::Set(SetAct::InstallCli),
                Fix::AddToPath(dir) => Act::Set(SetAct::CliAddToPath(dir.clone())),
                Fix::CopyPathLine(dir) => Act::Copy {
                    text: crate::cli_link::path_line(dir),
                    toast: "Copied. Add it to your shell's startup file.".to_owned(),
                },
            };
            (f.label(), act, false)
        })
        .collect();
    let dismiss = Act::Set(SetAct::DismissCliNotice);
    Some(super::git::toast(s, kind, &n.text, actions, Some(dismiss)))
}

/// The toasts, newest at the bottom right.
pub(super) fn toasts(s: &Screen<'_>) -> Option<El> {
    let notice = cli_notice(s);
    if s.st.toasts.is_empty() && notice.is_none() {
        return None;
    }
    let mut col = div().col().gap(8.0).child_if(notice);
    for t in &s.st.toasts {
        let kind = if t.error {
            ToastKind::Error
        } else {
            ToastKind::Success
        };
        let close = s.acts.on("toast", Act::DismissToast(t.text.clone()));
        col = col.child(widgets::toast(s.ui, close, &t.text, kind));
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
