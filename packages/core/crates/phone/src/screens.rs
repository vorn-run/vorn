//! The phone's screens: tabs in one column when compact, the list beside the session when wide.

use vorn_app::client::store::Session;
use vorn_app::points::Point;
use vorn_components::agent::status_icon;
use vorn_components::lucide;
use vorn_components::status::{status_dot, Status};
use vornui::accesskit::Role;
use vornui::native::Sem;
use vornui::theme::color::{
    BRONZO, GREEN_400, INK, INK_FAINT, INK_GHOST, INK_SECONDARY, SURFACE_BASE, SURFACE_OVERLAY,
    SURFACE_SUNKEN,
};
use vornui::theme::layout::{self, TOUCH};
use vornui::widgets::text_input;
use vornui::{custom, div, text, Action, El, Id, Rgba, Sense, Ui};

use crate::model::Link;
use crate::phone::{Phone, Tab};

const HAIRLINE: Rgba = Rgba::white(0.07);
const HAIRLINE_STRONG: Rgba = Rgba::white(0.12);
/// The regular layout's list column.
const LIST_W: f32 = 340.0;

/// The terminal's box, painted by [`Phone`].
pub const TERMINAL: u64 = 0x7e4d_0001;

pub fn id(name: &str) -> Id {
    Id::new(name)
}

pub fn row_id(session: &str) -> Id {
    Id::new("session-row").child(session)
}

pub fn settings_id(key: &str) -> Id {
    Id::new("settings-row").child(key)
}

pub fn tab_id(t: Tab) -> Id {
    Id::new("tab").child(t as u8)
}

/// The keys the bar above the keyboard sends: label, icon, DOM code, ctrl.
pub const KEYS: &[(&str, Option<&str>, &str, bool)] = &[
    ("esc", None, "Escape", false),
    ("tab", None, "Tab", false),
    ("ctrl", None, "", false),
    ("^C", None, "KeyC", true),
    ("^D", None, "KeyD", true),
    ("Up", Some("arrow-up"), "ArrowUp", false),
    ("Down", Some("arrow-down"), "ArrowDown", false),
    ("Left", Some("arrow-left"), "ArrowLeft", false),
    ("Right", Some("arrow-right"), "ArrowRight", false),
];

pub fn key_id(i: usize) -> Id {
    Id::new("key").child(i)
}

/// The whole drawn app at the phone's size.
pub fn root(p: &Phone, ui: &Ui) -> El {
    let (w, h) = p.size;
    let body =
        if p.terminal_only {
            terminal(p)
        } else if layout::compact(w) {
            match p.session.as_deref() {
                Some(sid) => session_screen(p, ui, sid, true),
                None => tabs_screen(p, ui),
            }
        } else {
            div()
                .row()
                .w_full()
                .h_full()
                .child(
                    div()
                        .col()
                        .w(LIST_W)
                        .h_full()
                        .border_right(1.0, HAIRLINE)
                        .child(tabs_screen(p, ui)),
                )
                .child(div().col().grow().h_full().child(
                    match p.session.as_deref() {
                        Some(sid) => session_screen(p, ui, sid, false),
                        None => div().grow().center().child(text(
                            "Pick a session",
                            15.0,
                            INK_FAINT,
                        )),
                    },
                ))
        };
    let mut root = div()
        .col()
        .size(w, h)
        .bg(SURFACE_BASE)
        .pl(p.insets.left)
        .pr(p.insets.right)
        .child(body);
    if let Some(key) = &p.settings {
        root = root.child(settings_sheet(p, ui, key.as_deref()));
    }
    root
}

/// Sessions, tasks or workflows, with the header above and the tabs below.
fn tabs_screen(p: &Phone, ui: &Ui) -> El {
    let (title, body) = match p.tab {
        Tab::Sessions => ("Sessions", sessions(p, ui)),
        Tab::Tasks => ("Tasks", tasks(p)),
        Tab::Workflows => ("Workflows", workflows(p)),
    };
    let mut col = div()
        .col()
        .w_full()
        .h_full()
        .pt(p.insets.top)
        .child(header(title));
    if p.tab == Tab::Sessions {
        col = col.child(controls(p));
    }
    div()
        .col()
        .w_full()
        .h_full()
        .child(
            col.child(
                div()
                    .id(id("list"))
                    .col()
                    .w_full()
                    .grow()
                    .scroll_y()
                    .child(body),
            ),
        )
        .child(tab_bar(p))
}

fn header(title: &str) -> El {
    div()
        .row()
        .items_center()
        .justify_between()
        .h(52.0)
        .pl(20.0)
        .pr(8.0)
        .child(text(title, 22.0, INK).weight(600))
        .child(icon_button(
            id("open-settings"),
            "settings",
            "Settings",
            INK_SECONDARY,
        ))
}

/// The computer the phone talks to, and how many sessions it runs.
fn controls(p: &Phone) -> El {
    let (dot, host) = match (&p.model.link, &p.model.target) {
        (Link::Online, Some(t)) => (GREEN_400, t.host.clone()),
        (Link::Connecting, Some(t)) => (INK_FAINT, t.host.clone()),
        (Link::Offline(_), Some(t)) => (INK_GHOST, t.host.clone()),
        _ => (INK_GHOST, "Not paired".to_owned()),
    };
    let count = p.model.store.sessions.len();
    div()
        .row()
        .items_center()
        .gap(10.0)
        .px(20.0)
        .pb(10.0)
        .child(
            div()
                .row()
                .items_center()
                .gap(8.0)
                .h(38.0)
                .px(14.0)
                .rounded(19.0)
                .bg(SURFACE_SUNKEN)
                .border(1.0, HAIRLINE_STRONG)
                .child(div().size(6.0, 6.0).rounded(3.0).bg(dot))
                .child(text(host, 14.0, INK_SECONDARY).truncate().min_w0()),
        )
        .child(text(count.to_string(), 13.0, INK_GHOST).mono())
}

fn sessions(p: &Phone, ui: &Ui) -> El {
    let s = &p.model.store.sessions;
    if s.is_empty() {
        let (title, line) = match &p.model.link {
            Link::Online => (
                "No agents running",
                "Start one on the computer and it appears here.",
            ),
            Link::Connecting => ("Connecting", "Reaching the computer."),
            Link::Offline(why) => ("Not connected", why.as_str()),
            Link::Unpaired => ("Not paired", "Pair the phone with a computer first."),
        };
        return empty(title, line);
    }
    div()
        .col()
        .w_full()
        .sem(Sem::List)
        .children(s.iter().map(|x| session_row(p, ui, x)))
}

fn empty(title: &str, line: &str) -> El {
    div()
        .col()
        .items_center()
        .gap(6.0)
        .px(32.0)
        .pt(96.0)
        .child(text(title, 17.0, INK).weight(600).sem(Sem::Text("title")))
        .child(
            text(line, 15.0, INK_FAINT)
                .text_center()
                .wrapping()
                .sem(Sem::Text("secondary")),
        )
}

fn meta(s: &Session) -> String {
    let mut parts = Vec::new();
    if s.title() != s.info.project_name && !s.info.project_name.is_empty() {
        parts.push(s.info.project_name.clone());
    }
    if let Some(b) = s.info.branch.as_deref().filter(|b| !b.is_empty()) {
        parts.push(b.to_owned());
    }
    parts.push(s.info.agent_type.to_string());
    parts.join(" · ")
}

fn session_row(p: &Phone, ui: &Ui, s: &Session) -> El {
    let sid = s.info.id.as_str();
    let status = Status::parse(s.status());
    let selected = p.session.as_deref() == Some(sid);
    div()
        .id(row_id(sid))
        .on(Action::Click)
        .role(Role::Link, s.title())
        .sem(Sem::Link(format!("session:{sid}")))
        .row()
        .items_center()
        .gap(12.0)
        .px(20.0)
        .py(14.0)
        .min_h(TOUCH)
        .border_bottom(1.0, HAIRLINE)
        .bg(if selected {
            Rgba::white(0.04)
        } else {
            Rgba::TRANSPARENT
        })
        .child(
            status_icon(ui, &p.marks, &s.info.agent_type, status, 22.0, 1.0).sem(Sem::Status {
                value: s.status().to_owned(),
                tone: tone(status),
            }),
        )
        .child(
            div()
                .col()
                .grow()
                .min_w0()
                .gap(2.0)
                .child(
                    text(s.title(), 17.0, INK)
                        .weight(600)
                        .truncate()
                        .sem(Sem::Text("title")),
                )
                .child(
                    div()
                        .row()
                        .items_center()
                        .gap(6.0)
                        .child(status_dot(ui, status).sem(Sem::Hidden))
                        .child(
                            text(meta(s), 13.0, INK_FAINT)
                                .truncate()
                                .min_w0()
                                .sem(Sem::Text("secondary")),
                        ),
                ),
        )
        .child(lucide("chevron-right", 16.0, 2.0, INK_FAINT.rgba()).sem(Sem::Hidden))
}

pub(crate) fn tone(s: Status) -> &'static str {
    match s {
        Status::Running => "busy",
        Status::Waiting => "warn",
        Status::Error => "bad",
        Status::Idle => "muted",
    }
}

fn plain_row(title: String, line: String) -> El {
    div()
        .row()
        .sem(Sem::Row)
        .items_center()
        .px(20.0)
        .py(14.0)
        .min_h(TOUCH)
        .border_bottom(1.0, HAIRLINE)
        .child(
            div()
                .col()
                .grow()
                .min_w0()
                .gap(2.0)
                .child(
                    text(title, 17.0, INK)
                        .weight(600)
                        .truncate()
                        .sem(Sem::Text("title")),
                )
                .child(
                    text(line, 13.0, INK_FAINT)
                        .truncate()
                        .sem(Sem::Text("secondary")),
                ),
        )
}

fn tasks(p: &Phone) -> El {
    let t = &p.model.store.tasks;
    if t.is_empty() {
        return empty("No tasks", "Tasks you add on the computer appear here.");
    }
    div().col().sem(Sem::List).children(t.iter().map(|t| {
        plain_row(
            t.title.clone(),
            format!(
                "{} · {}",
                t.project_name,
                t.status.to_string().replace('_', " ")
            ),
        )
    }))
}

fn workflows(p: &Phone) -> El {
    let w = &p.model.store.workflows;
    if w.is_empty() {
        return empty(
            "No workflows",
            "Workflows you make on the computer appear here.",
        );
    }
    div().col().sem(Sem::List).children(w.iter().map(|w| {
        plain_row(
            w.name.clone(),
            if w.enabled { "On" } else { "Off" }.to_owned(),
        )
    }))
}

fn tab_bar(p: &Phone) -> El {
    let tab = |t: Tab, icon: &str, name: &str| {
        let on = p.tab == t;
        div()
            .id(tab_id(t))
            .on(Action::Click)
            .role(Role::Tab, name)
            .selected(on)
            .grow()
            .h(49.0)
            .center()
            .child(lucide(
                icon,
                24.0,
                2.0,
                if on { INK.rgba() } else { INK_FAINT.rgba() },
            ))
    };
    div()
        .row()
        .w_full()
        .pb(p.insets.bottom)
        .border_top(1.0, HAIRLINE)
        .bg(SURFACE_BASE)
        .sem(Sem::Hidden)
        .child(tab(Tab::Sessions, "monitor", "Sessions"))
        .child(tab(Tab::Tasks, "list-todo", "Tasks"))
        .child(tab(Tab::Workflows, "workflow", "Workflows"))
}

/// One session: its name, its terminal, the composer and the key bar.
fn session_screen(p: &Phone, ui: &Ui, sid: &str, back: bool) -> El {
    let s = p.model.store.session(sid);
    let name = s.map_or_else(|| "Session".to_owned(), Session::title);
    let status = s.map_or(Status::Idle, |s| Status::parse(s.status()));
    let attached = p.model.views.contains_key(sid);
    let head = div()
        .row()
        .items_center()
        .gap(8.0)
        .h(TOUCH)
        .px(8.0)
        .sem(Sem::Hidden)
        .child_if(back.then(|| {
            div()
                .id(id("back"))
                .on(Action::Click)
                .role(Role::Button, "Sessions")
                .row()
                .items_center()
                .h(TOUCH)
                .pr(4.0)
                .child(lucide("chevron-left", 24.0, 2.0, BRONZO.rgba()))
                .child(text("Sessions", 17.0, BRONZO))
        }))
        .child(
            text(name, 17.0, INK)
                .weight(600)
                .truncate()
                .grow()
                .min_w0()
                .pl(if back { 0.0 } else { 12.0 }),
        )
        .child(status_dot(ui, status))
        .child_if((!attached).then(|| text("attaching", 13.0, INK_FAINT).pr(8.0)));
    div()
        .col()
        .w_full()
        .h_full()
        .pt(p.insets.top)
        .child(head)
        .child(terminal(p))
        .child(composer(p, ui))
        .child(key_bar(p))
        .child(div().h(if p.keyboard > 0.0 {
            p.keyboard
        } else {
            p.insets.bottom
        }))
}

/// The terminal's box; [`Phone`] paints the grid into it.
pub fn terminal(_p: &Phone) -> El {
    custom(TERMINAL)
        .id(Id(TERMINAL))
        .sense(Sense::CLICK | Sense::WHEEL)
        .on(Action::Click)
        .role(Role::Terminal, "Terminal")
        .sem(Sem::Drawn("terminal".into()))
        .w_full()
        .grow()
}

pub const COMPOSER: &str = "composer";
pub const SEND: &str = "send";

fn composer(_p: &Phone, ui: &Ui) -> El {
    div()
        .row()
        .items_center()
        .gap(8.0)
        .px(12.0)
        .py(6.0)
        .border_top(1.0, HAIRLINE)
        .child(
            text_input(ui, id(COMPOSER), "Message", "Message the session")
                .grow()
                .min_h(36.0),
        )
        .child(
            div()
                .id(id(SEND))
                .on(Action::Click)
                .role(Role::Button, "Send")
                .size(36.0, 36.0)
                .rounded(18.0)
                .bg(BRONZO)
                .center()
                .hit_slop(4.0, 4.0)
                .child(lucide("arrow-up", 18.0, 2.5, SURFACE_BASE.rgba())),
        )
}

fn key_bar(p: &Phone) -> El {
    div()
        .row()
        .items_center()
        .justify_between()
        .h(TOUCH)
        .px(4.0)
        .bg(SURFACE_SUNKEN)
        .children(KEYS.iter().enumerate().map(|(i, (label, icon, _, _))| {
            let latched = *label == "ctrl" && p.ctrl;
            let fg = if latched { SURFACE_BASE } else { INK_SECONDARY };
            let face = match icon {
                Some(name) => lucide(name, 16.0, 2.0, fg.rgba()),
                None => text(*label, 13.0, fg).mono(),
            };
            div()
                .id(key_id(i))
                .on(Action::Click)
                .role(Role::Button, *label)
                .min_w(36.0)
                .h(36.0)
                .px(6.0)
                .rounded(8.0)
                .bg(if latched {
                    INK.rgba()
                } else {
                    Rgba::TRANSPARENT
                })
                .center()
                .child(face)
        }))
        .child(icon_button(
            id("hide-keyboard"),
            "keyboard",
            "Hide keyboard",
            INK_SECONDARY,
        ))
}

fn icon_button(id: Id, icon: &str, label: &str, c: impl Into<Rgba>) -> El {
    let c = c.into();
    div()
        .id(id)
        .on(Action::Click)
        .role(Role::Button, label)
        .size(TOUCH, TOUCH)
        .center()
        .child(lucide(icon, 20.0, 2.0, c))
}

/// Extension settings over the screen: the pages, or the page `key`.
fn settings_sheet(p: &Phone, ui: &Ui, key: Option<&str>) -> El {
    let (w, h) = p.size;
    let page = key.and_then(|k| p.model.points.get(k));
    let title = page.map_or("Settings", |pt| pt.title.as_str());
    let body = match page {
        Some(pt) => settings_page(p, ui, pt),
        None => settings_list(p),
    };
    let top = p.insets.top + 12.0;
    let sheet_w = if layout::compact(w) { w } else { 540.0 };
    let head = div()
        .row()
        .items_center()
        .justify_between()
        .h(52.0)
        .px(8.0)
        .border_bottom(1.0, HAIRLINE)
        .child(div().w(88.0).child_if(page.is_some().then(|| {
            div()
                .id(id("sheet-back"))
                .on(Action::Click)
                .role(Role::Button, "Settings")
                .row()
                .items_center()
                .h(TOUCH)
                .child(lucide("chevron-left", 24.0, 2.0, BRONZO.rgba()))
                .child(text("Back", 17.0, BRONZO))
        })))
        .child(text(title, 17.0, INK).weight(600).truncate().min_w0())
        .child(
            div().w(88.0).row().justify_end().child(
                div()
                    .id(id("sheet-done"))
                    .on(Action::Click)
                    .role(Role::Button, "Done")
                    .h(TOUCH)
                    .px(12.0)
                    .center()
                    .child(text("Done", 17.0, BRONZO).weight(600)),
            ),
        );
    div()
        .absolute(0.0, 0.0)
        .size(w, h)
        .layered()
        .col()
        .items_center()
        .child(
            div()
                .id(id("sheet-scrim"))
                .on(Action::Click)
                .absolute(0.0, 0.0)
                .size(w, h)
                .bg(Rgba::hexa(0x000000, 0.5)),
        )
        .child(
            div()
                .col()
                .mt(top)
                .w(sheet_w)
                .h(h - top)
                .bg(SURFACE_OVERLAY)
                .rounded(14.0)
                .child(head)
                .child(
                    div()
                        .id(id("sheet-body"))
                        .col()
                        .grow()
                        .px(16.0)
                        .pt(8.0)
                        .pb(p.insets.bottom + p.keyboard + 16.0)
                        .scroll_y()
                        .child(body),
                ),
        )
}

pub fn settings_list(p: &Phone) -> El {
    let pages: Vec<&Point> = p.model.settings().collect();
    if pages.is_empty() {
        return empty("No settings", "Extensions with settings list them here.");
    }
    div()
        .col()
        .sem(Sem::List)
        .children(pages.into_iter().map(|pt| {
            div()
                .id(settings_id(&pt.key()))
                .on(Action::Click)
                .role(Role::Link, pt.title.as_str())
                .sem(Sem::Link(format!("settings:{}", pt.key())))
                .row()
                .items_center()
                .gap(12.0)
                .min_h(TOUCH)
                .py(12.0)
                .border_bottom(1.0, HAIRLINE)
                .child(
                    div()
                        .col()
                        .grow()
                        .min_w0()
                        .child(text(pt.title.as_str(), 17.0, INK).sem(Sem::Text("title")))
                        .child(
                            text(pt.extension_name.as_str(), 13.0, INK_FAINT)
                                .sem(Sem::Text("secondary")),
                        ),
                )
                .child(lucide("chevron-right", 16.0, 2.0, INK_FAINT.rgba()).sem(Sem::Hidden))
        }))
}

pub fn settings_page(p: &Phone, ui: &Ui, pt: &Point) -> El {
    let (acts, points) = (&p.model.acts, &p.model.points);
    let loading = || {
        if ui.is_native() {
            div().sem(Sem::Empty {
                title: "Loading".into(),
                body: String::new(),
            })
        } else {
            empty("Loading", "")
        }
    };
    vorn_app::screen::points::draw_tree(ui, acts, points, pt).unwrap_or_else(loading)
}

/// What a native host shows for `screen`, e.g. `sessions`, `settings:<key>` or `session:<id>`.
pub fn native(p: &Phone, ui: &Ui, screen: &str) -> El {
    p.model.acts.clear();
    if let Some(key) = screen.strip_prefix("settings:") {
        return match p.model.points.get(key) {
            Some(pt) => settings_page(p, ui, pt),
            None => empty("Not found", "This page is gone."),
        };
    }
    if let Some(sid) = screen.strip_prefix("session:") {
        let s = p.model.store.session(sid);
        let status = s.map_or(Status::Idle, |s| Status::parse(s.status()));
        return div()
            .col()
            .child(
                div()
                    .sem(Sem::Row)
                    .child(
                        text(
                            s.map_or_else(|| "Session".into(), Session::title),
                            17.0,
                            INK,
                        )
                        .sem(Sem::Text("title")),
                    )
                    .child(status_dot(ui, status).sem(Sem::Status {
                        value: s.map_or("idle", Session::status).to_owned(),
                        tone: tone(status),
                    })),
            )
            .child(terminal(p));
    }
    match screen {
        "tasks" => tasks(p),
        "workflows" => workflows(p),
        "settings" => settings_list(p),
        _ => div()
            .col()
            .child(controls(p).sem(Sem::Hidden))
            .child(sessions(p, ui)),
    }
}
