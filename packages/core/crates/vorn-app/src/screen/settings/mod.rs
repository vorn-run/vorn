//! `SettingsPage`: the category sidebar beside the open panel, over the
//! whole window, and the rows, switches and segmented controls the panels
//! share (`SettingRow`, `ToggleSwitch`, `SegmentedControl`).

mod agents;
mod pages;
mod ssh;

pub use agents::agent_field_id;
pub use pages::{rotate_id, ROW_SLIDER, SHELL_PATH, VOLUME_SLIDER};
pub(crate) use pages::{select_items, shell_panel};
pub(crate) use ssh::ssh_key_items;
pub use ssh::{host_id, key_field_id};

use super::{c, ico, white, Screen};
use crate::act::Act;
use crate::look;
use crate::settings::{Category, SetAct, SECTIONS};
use crate::ui::{div, icon, text, Action, El, Id, Rgba, Role, Sense};

pub(super) fn set(a: SetAct) -> Act {
    Act::Set(a)
}

/// The content column's scroll view.
pub const CONTENT: Id = Id(0x7365_7474_696e_6773);

/// A sidebar icon from `SIDEBAR_SECTIONS`: 16 px, stroke 1.5.
fn nav_svg(cat: Category) -> String {
    let body = match cat {
        Category::Appearance => {
            r#"<circle cx="12" cy="12" r="5"/><path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42"/>"#
        }
        Category::General => {
            r#"<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 010 2.83 2 2 0 01-2.83 0l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83 0 2 2 0 010-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 112.83-2.83l.06.06a1.65 1.65 0 001.82.33H9a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 112.83 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z"/>"#
        }
        Category::Updates => r#"<path d="M12 3v12"/><path d="M7 10l5 5 5-5"/><path d="M4 20h16"/>"#,
        Category::Notifications => {
            r#"<path d="M18 8A6 6 0 006 8c0 7-3 9-3 9h18s-3-2-3-9"/><path d="M13.73 21a2 2 0 01-3.46 0"/>"#
        }
        Category::Agents => {
            r#"<polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/>"#
        }
        Category::Ssh => {
            r##"<rect x="2" y="2" width="20" height="8" rx="2"/><rect x="2" y="14" width="20" height="8" rx="2"/><circle cx="6" cy="6" r="1" fill="#ffffff"/><circle cx="6" cy="18" r="1" fill="#ffffff"/>"##
        }
        Category::Mcp => {
            r#"<path d="M12 2v4m0 12v4M2 12h4m12 0h4"/><circle cx="12" cy="12" r="4"/>"#
        }
        Category::Connectors => {
            r#"<path d="M9 17H7A5 5 0 017 7h2"/><path d="M15 7h2a5 5 0 010 10h-2"/><line x1="8" y1="12" x2="16" y2="12"/>"#
        }
        Category::Keys => {
            r#"<circle cx="8" cy="12" r="4"/><line x1="12" y1="12" x2="21" y2="12"/><line x1="18" y1="12" x2="18" y2="16"/>"#
        }
        Category::Network => {
            r#"<circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 014 10 15.3 15.3 0 01-4 10 15.3 15.3 0 01-4-10 15.3 15.3 0 014-10z"/>"#
        }
        Category::About => {
            r#"<circle cx="12" cy="12" r="10"/><line x1="12" y1="16" x2="12" y2="12"/><line x1="12" y1="8" x2="12.01" y2="8"/>"#
        }
    };
    svg(body, 1.5)
}

/// A 24-unit stroked SVG with `body`, white for tinting.
pub(super) fn svg(body: &str, stroke: f32) -> String {
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="#ffffff" stroke-width="{stroke}">{body}</svg>"##
    )
}

/// One category button: `px-3 py-2 rounded-md text-[13px] gap-2.5`.
fn nav_item(s: &Screen<'_>, cat: Category, rest: Rgba) -> El {
    let id = s.acts.on("settings-nav", set(SetAct::Open(cat)));
    let on = s.st.settings.category == cat && s.st.settings.ext_page.is_none();
    nav_button(
        s,
        id,
        cat.label(),
        icon(nav_svg(cat), 16.0, nav_ink(s, id, on, rest)),
        on,
        rest,
    )
}

fn nav_ink(s: &Screen<'_>, id: Id, on: bool, rest: Rgba) -> Rgba {
    if on {
        c(look::WHITE)
    } else {
        rest.mix(c(look::WHITE), s.ui.hover(id))
    }
}

/// An extension's settings page in the nav, under its own heading.
fn ext_item(s: &Screen<'_>, p: &crate::points::Point) -> El {
    let key = p.key();
    let id = s.acts.on(
        "settings-nav-ext",
        Act::Point(crate::points::PointAct::Settings(key.clone())),
    );
    let on = s.st.settings.ext_page.as_deref() == Some(key.as_str());
    let rest = c(look::GRAY_400);
    let glyph = vorn_components::lucide(
        p.icon.as_deref().unwrap_or("puzzle"),
        16.0,
        1.5,
        nav_ink(s, id, on, rest),
    );
    nav_button(s, id, &p.title, glyph, on, rest)
}

fn nav_button(s: &Screen<'_>, id: Id, label: &str, glyph: El, on: bool, rest: Rgba) -> El {
    let hot = s.ui.hover(id);
    let (fg, bg) = if on {
        (c(look::WHITE), white(0.08))
    } else {
        (
            rest.mix(c(look::WHITE), hot),
            white(0.0).mix(white(0.04), hot),
        )
    };
    div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(10.0)
        .px(12.0)
        .py(8.0)
        .rounded(6.0)
        .bg(bg)
        .role(Role::Button, label)
        .selected(on)
        .child(glyph)
        .child(text(label, 13.0, fg).line_height(19.5).silent())
}

fn sidebar(s: &Screen<'_>) -> El {
    let back = s.acts.on("settings", set(SetAct::Close));
    let back_fg = c(look::GRAY_400).mix(c(look::WHITE), s.ui.hover(back));
    let header = div()
        .id(super::chrome::TITLE_BAR)
        .sense(Sense::WINDOW_DRAG)
        .row()
        .items_center()
        .h(52.0)
        .pl(78.0)
        .pr(12.0)
        .shrink0()
        .border_bottom(1.0, white(0.06))
        .child(
            div()
                .id(back)
                .on(Action::Click)
                .row()
                .items_center()
                .gap(8.0)
                .role(Role::Button, "Back to app")
                .child(icon(
                    svg(r#"<polyline points="15 18 9 12 15 6"/>"#, 2.0),
                    16.0,
                    back_fg,
                ))
                .child(
                    text("Back to app", 14.0, back_fg)
                        .line_height(20.0)
                        .silent(),
                ),
        );
    let mut list = div()
        .id(Id::new("settings-nav"))
        .col()
        .grow()
        .min_h(0.0)
        .scroll_y()
        .px(12.0)
        .pt(4.0);
    for (title, cats) in SECTIONS {
        list = list.child(
            div().px(12.0).pt(20.0).pb(6.0).child(
                text(title, 10.0, c(look::GRAY_600))
                    .weight(500)
                    .tracking(0.05)
                    .line_height(15.0)
                    .role(Role::Heading, title),
            ),
        );
        let mut items = div().col().gap(2.0);
        for cat in cats.iter().filter(|c| c.shown()) {
            items = items.child(nav_item(s, *cat, c(look::GRAY_400)));
        }
        list = list.child(items);
    }
    let pages: Vec<El> =
        s.st.points
            .at(crate::points::kind::SETTINGS)
            .map(|p| ext_item(s, p))
            .collect();
    if !pages.is_empty() {
        list = list
            .child(
                div().px(12.0).pt(20.0).pb(6.0).child(
                    text("EXTENSIONS", 10.0, c(look::GRAY_600))
                        .weight(500)
                        .tracking(0.05)
                        .line_height(15.0)
                        .role(Role::Heading, "Extensions"),
                ),
            )
            .child(div().col().gap(2.0).children(pages));
    }
    div()
        .col()
        .w(224.0)
        .h_full()
        .shrink0()
        .bg(c(look::SURFACE_SUNKEN))
        .border_right(1.0, white(0.06))
        .role(Role::Navigation, "Settings")
        .child(header)
        .child(list)
        .child(
            div()
                .px(12.0)
                .pb(16.0)
                .pt(8.0)
                .border_top(1.0, white(0.06))
                .child(nav_item(s, Category::About, c(look::GRAY_500))),
        )
}

/// The whole page, over the window.
pub(super) fn page(s: &Screen<'_>) -> El {
    let body = match (&s.st.settings.ext_page, s.st.settings.category) {
        (Some(key), _) => super::points::settings_page(s, key),
        (None, cat) => category_page(s, cat),
    };
    page_with(s, body)
}

fn category_page(s: &Screen<'_>, cat: Category) -> El {
    match cat {
        Category::Appearance => pages::appearance(s),
        Category::General => pages::general(s),
        Category::Updates => pages::updates(s),
        Category::Notifications => pages::notifications(s),
        Category::Agents => agents::page(s),
        Category::Ssh => ssh::page(s),
        Category::Mcp => pages::mcp(s),
        Category::Keys => pages::keys(s),
        Category::About => pages::about(s),
        Category::Connectors => super::plugins::page(s),
        Category::Network => super::network::page(s),
    }
}

fn page_with(s: &Screen<'_>, body: El) -> El {
    div()
        .row()
        .w_full()
        .h_full()
        .bg(c(look::SURFACE_OVERLAY))
        .child(sidebar(s))
        .child(
            div()
                .id(CONTENT)
                .col()
                .items_center()
                .grow()
                .min_w0()
                .h_full()
                .scroll_y()
                .child(
                    div()
                        .col()
                        .shrink0()
                        .w_full()
                        .max_w(672.0)
                        .px(32.0)
                        .py(32.0)
                        .child(body),
                ),
        )
}

/// `SettingsPageHeader`: the title, its description, and actions.
pub(in crate::screen) fn header(title: &str, description: &str, actions: Option<El>) -> El {
    div()
        .row()
        .items_start()
        .justify_between()
        .mb(24.0)
        .child(
            div()
                .col()
                .min_w0()
                .child(
                    text(title, 20.0, c(look::WHITE))
                        .weight(600)
                        .line_height(28.0)
                        .mb(4.0)
                        .role(Role::Heading, title),
                )
                .child(
                    text(description, 14.0, c(look::GRAY_500))
                        .line_height(20.0)
                        .wrapping()
                        .silent(),
                ),
        )
        .child_if(actions)
}

/// `SettingRow`: the label and description beside `control`, a note
/// under the description, dimmed when `disabled`.
pub(in crate::screen) fn row(
    label: &str,
    description: &str,
    note: Option<&str>,
    control: El,
    disabled: bool,
) -> El {
    let mut left = div()
        .col()
        .child(
            text(label, 14.0, c(look::GRAY_200))
                .weight(500)
                .line_height(20.0)
                .role(Role::Label, label),
        )
        .child(
            text(description, 12.0, c(look::GRAY_500))
                .line_height(16.0)
                .mt(2.0)
                .wrapping()
                .silent(),
        );
    if let Some(n) = note {
        left = left.child(
            div().mt(6.0).pl(8.0).border_left(1.0, white(0.12)).child(
                text(n, 12.0, c(look::GRAY_400))
                    .line_height(16.0)
                    .wrapping()
                    .silent(),
            ),
        );
    }
    let mut el = div()
        .row()
        .items_center()
        .justify_between()
        .py(16.0)
        .border_bottom(1.0, white(0.06))
        .child(left)
        .child(control);
    if disabled {
        el = el.opacity(0.4);
    }
    el
}

/// `ToggleSwitch`: 40 by 24, blue when on, its 16 px knob sliding.
pub(in crate::screen) fn toggle(
    s: &Screen<'_>,
    place: &str,
    on: bool,
    act: impl Into<Act>,
    disabled: bool,
    label: &str,
) -> El {
    let id = s.acts.on(place, act.into());
    let mut el = div()
        .id(id)
        .w(40.0)
        .h(24.0)
        .rounded(12.0)
        .bg(if on { c(look::BLUE_500) } else { white(0.1) })
        .role(Role::Switch, label)
        .toggled(on)
        .disabled(disabled)
        .child(
            div()
                .absolute(if on { 20.0 } else { 4.0 }, 4.0)
                .size(16.0, 16.0)
                .rounded(8.0)
                .bg(c(look::WHITE)),
        );
    if !disabled {
        el = el.on(Action::Click);
    }
    el
}

/// One option of a [`segmented`] control.
pub(super) struct Seg<'a> {
    pub label: &'a str,
    pub icon: Option<El>,
    pub act: SetAct,
    pub on: bool,
    pub disabled: bool,
}

/// `SegmentedControl`: `bg-white/[0.04] rounded-lg p-0.5 gap-0.5`.
pub(super) fn segmented(s: &Screen<'_>, place: &str, opts: Vec<Seg<'_>>, px: f32) -> El {
    let mut row = div()
        .row()
        .shrink0()
        .gap(2.0)
        .p(2.0)
        .rounded(8.0)
        .bg(white(0.04))
        .role(Role::RadioGroup, place);
    for o in opts {
        let id = s.acts.on(place, set(o.act));
        let hot = s.ui.hover(id);
        let fg = if o.disabled {
            c(look::GRAY_600)
        } else if o.on {
            c(look::WHITE)
        } else {
            c(look::GRAY_500).mix(c(look::GRAY_300), hot)
        };
        let mut b = div()
            .id(id)
            .row()
            .items_center()
            .gap(6.0)
            .px(px)
            .py(4.0)
            .rounded(6.0)
            .role(Role::RadioButton, o.label)
            .selected(o.on);
        if o.on && !o.disabled {
            b = b.bg(white(0.1));
        }
        if let Some(i) = o.icon {
            b = b.child(i);
        }
        b = b.child(text(o.label, 12.0, fg).line_height(16.0).silent());
        b = if o.disabled {
            b.opacity(0.3)
        } else {
            b.on(Action::Click)
        };
        row = row.child(b);
    }
    row
}

/// A small text button: `px-3 py-1.5 text-xs` on a faint wash.
pub(super) fn button(s: &Screen<'_>, place: &str, label: &str, act: SetAct, enabled: bool) -> El {
    let id = s.acts.on(place, set(act));
    let hot = s.ui.hover(id);
    let mut b = div()
        .id(id)
        .px(12.0)
        .py(6.0)
        .rounded(6.0)
        .shrink0()
        .bg(white(0.04).mix(white(0.08), hot))
        .border(1.0, white(0.08))
        .role(Role::Button, label)
        .disabled(!enabled)
        .child(
            text(label, 12.0, c(look::GRAY_300))
                .line_height(16.0)
                .silent(),
        );
    b = if enabled {
        b.on(Action::Click)
    } else {
        b.opacity(0.4)
    };
    b
}

/// A section title: an icon, the label in small caps, and a count.
pub(super) fn section(name: &str, label: &str, count: Option<usize>) -> El {
    div()
        .row()
        .items_center()
        .gap(8.0)
        .mb(12.0)
        .child(ico(name, 12.0, 2.0, c(look::GRAY_600)))
        .child(
            text(label.to_uppercase(), 12.0, c(look::GRAY_500))
                .weight(500)
                .tracking(0.05)
                .line_height(16.0)
                .role(Role::Heading, label),
        )
        .child_if(count.map(|n| {
            text(n.to_string(), 10.0, c(look::GRAY_600))
                .line_height(15.0)
                .silent()
        }))
}

/// A form label: `text-[10px] text-gray-600 uppercase tracking-wider`.
pub(super) fn label(t: &str, mb: f32) -> El {
    text(t.to_uppercase(), 10.0, c(look::GRAY_600))
        .tracking(0.05)
        .line_height(15.0)
        .mb(mb)
        .role(Role::Label, t)
}
