//! The settings page: the category sidebar beside the open panel, over the
//! whole window, and the rows, switches and segmented controls the panels
//! share.

mod agents;
mod pages;

pub use agents::agent_field_id;
pub use pages::theme_tile_id;
pub(crate) use pages::{agent_picker, font_picker, shell_panel, theme_info, theme_picker};
pub use pages::{rotate_id, AGENT_SEARCH, ROW_SLIDER, SHELL_PATH, THEME_SEARCH, VOLUME_SLIDER};

use super::{c, white, Screen};
use crate::act::Act;
use crate::look;
use crate::settings::{Category, SetAct, CONNECTIONS_PAGE, EXTENSIONS, SECTIONS};
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
        Category::Mcp => {
            r#"<path d="M12 2v4m0 12v4M2 12h4m12 0h4"/><circle cx="12" cy="12" r="4"/>"#
        }
        Category::Connectors => {
            r#"<path d="M15.39 4.39a1 1 0 001.68-.474 2.5 2.5 0 113.014 3.015 1 1 0 00-.474 1.68l1.683 1.682a2.414 2.414 0 010 3.414L19.61 15.39a1 1 0 01-1.68-.474 2.5 2.5 0 10-3.014 3.015 1 1 0 01.474 1.68l-1.683 1.682a2.414 2.414 0 01-3.414 0L8.61 19.61a1 1 0 00-1.68.474 2.5 2.5 0 11-3.014-3.015 1 1 0 00.474-1.68l-1.683-1.682a2.414 2.414 0 010-3.414L4.39 8.61a1 1 0 011.68.474 2.5 2.5 0 103.014-3.015 1 1 0 01-.474-1.68l1.683-1.682a2.414 2.414 0 013.414 0z"/>"#
        }
        Category::Connections => {
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

/// Back to app: a chevron and its label, on glass over the system's material.
fn back_button(s: &Screen<'_>, back: Id, fg: Rgba) -> El {
    let chevron = r#"<polyline points="15 18 9 12 15 6"/>"#;
    let b = div()
        .id(back)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(6.0)
        .role(Role::Button, "Back to app");
    let glass = super::chrome::glass(s)
        .then(|| crate::ui::icons::lucide("chevron-left", 2.0))
        .flatten();
    let Some(svg) = glass else {
        return b
            .gap(8.0)
            .child(icon(svg(chevron, 2.0), 16.0, fg))
            .child(text("Back to app", 14.0, fg).line_height(20.0).silent());
    };
    b.h(32.0)
        .pl(8.0)
        .pr(12.0)
        .rounded(16.0)
        .native_glass()
        .child(div().size(16.0, 16.0).native_icon(&svg, 16.0, fg))
        .child(
            text("Back to app", 13.0, white(0.0))
                .weight(500)
                .line_height(18.0)
                .native_label("Back to app", 13.0, fg)
                .silent(),
        )
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
        .child(back_button(s, back, back_fg));
    let mut list = div()
        .id(Id::new("settings-nav"))
        .col()
        .grow()
        .min_h(0.0)
        .scroll_y()
        .px(12.0)
        .pt(4.0);
    // A built-in's page that left the app keeps its place after the page it names.
    let pages: Vec<_> = s.st.points.at(crate::points::kind::SETTINGS).collect();
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
            for p in pages.iter().filter(|p| p.placed_after() == Some(*cat)) {
                items = items.child(ext_item(s, p));
            }
        }
        list = list.child(items);
    }
    // Connections, the Connectors built-in's page, leads the extensions' pages.
    let mut pages: Vec<_> = pages
        .into_iter()
        .filter(|p| p.placed_after().is_none_or(|c| !c.shown()))
        .collect();
    pages.sort_by_key(|p| p.key() != CONNECTIONS_PAGE);
    let pages = pages.into_iter().map(|p| ext_item(s, p));
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
        .child(
            div()
                .col()
                .gap(2.0)
                .children(EXTENSIONS.map(|cat| nav_item(s, cat, c(look::GRAY_400))))
                .children(pages),
        );
    div()
        .col()
        .w(224.0)
        .h_full()
        .shrink0()
        .bg(if s.ui.see_through() {
            crate::ui::theme::material::sidebar(true)
        } else {
            c(look::SURFACE_SUNKEN)
        })
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
        Category::Mcp => pages::mcp(s),
        Category::Keys => pages::keys(s),
        Category::About => pages::about(s),
        Category::Connectors => super::plugins::page(s),
        Category::Connections => super::points::settings_page(s, CONNECTIONS_PAGE),
        Category::Network => super::network::page(s),
    }
}

fn page_with(s: &Screen<'_>, body: El) -> El {
    div()
        .row()
        .w_full()
        .h_full()
        .bg(if s.ui.see_through() {
            crate::ui::theme::shadow(0.0)
        } else {
            c(look::SURFACE_OVERLAY)
        })
        .child(sidebar(s))
        .child(
            div()
                .id(CONTENT)
                .bg(crate::ui::theme::material::content(s.ui.see_through()))
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

/// The title, its description, and actions.
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
                .child_if((!description.is_empty()).then(|| {
                    text(description, 14.0, c(look::GRAY_500))
                        .line_height(20.0)
                        .wrapping()
                        .silent()
                })),
        )
        .child_if(actions)
}

/// The label and description beside `control`, a note
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

/// A title above a [`card`]: `text-[13px] font-semibold`, faint.
pub(in crate::screen) fn section(title: &str) -> El {
    text(title, 13.0, c(look::GRAY_500))
        .weight(600)
        .line_height(18.0)
        .pl(4.0)
        .mt(24.0)
        .mb(8.0)
        .role(Role::Heading, title)
}

/// Rows grouped on an inset card: a faint fill and border, a hairline
/// between rows.
pub(in crate::screen) fn card(rows: Vec<El>) -> El {
    let mut el = div()
        .col()
        .rounded(4.0)
        .bg(white(0.03))
        .border(1.0, white(0.06));
    for (i, r) in rows.into_iter().enumerate() {
        el = el.child(if i == 0 {
            r
        } else {
            r.border_top(1.0, white(0.06))
        });
    }
    el
}

/// One row of a [`card`]: an optional icon and the label at the left,
/// `control` at the right; `tip` explains the row while it is hovered, unless
/// a part of the control has a tooltip of its own.
pub(in crate::screen) fn card_row(
    s: &Screen<'_>,
    mark: Option<El>,
    label: &str,
    tip: Option<&str>,
    control: El,
) -> El {
    let name = text(label, 13.0, c(look::GRAY_200))
        .line_height(18.0)
        .role(Role::Label, label);
    let row = div()
        .row()
        .items_center()
        .justify_between()
        .gap(16.0)
        .min_h(48.0)
        .px(14.0)
        .py(8.0)
        .child(
            div()
                .row()
                .items_center()
                .gap(8.0)
                .min_w0()
                .child_if(mark)
                .child(name),
        )
        .child(control);
    match tip {
        Some(t) => crate::ui::tooltip(s.ui, row, row_tip_id(label), t, None),
        None => row,
    }
}

/// The id a [`card_row`] labelled `label` shows its tooltip under.
pub fn row_tip_id(label: &str) -> Id {
    Id::new("row-tip").child(label)
}

/// The one switch, [`widgets::toggle`](crate::ui::widgets::toggle).
pub(in crate::screen) fn toggle(
    s: &Screen<'_>,
    place: &str,
    on: bool,
    act: impl Into<Act>,
    disabled: bool,
    label: &str,
) -> El {
    let id = s.acts.on(place, act.into());
    crate::ui::widgets::toggle(s.ui, id, label, on, disabled)
}

/// One option of a [`segmented`] control.
pub(in crate::screen) struct Seg<'a> {
    pub label: &'a str,
    pub icon: Option<El>,
    pub act: Act,
    pub on: bool,
    pub disabled: bool,
}

/// `bg-white/[0.04] rounded-lg p-0.5 gap-0.5`.
pub(in crate::screen) fn segmented(s: &Screen<'_>, place: &str, opts: Vec<Seg<'_>>, px: f32) -> El {
    let mut row = div()
        .row()
        .shrink0()
        .gap(2.0)
        .p(2.0)
        .rounded(8.0)
        .bg(white(0.04))
        .role(Role::RadioGroup, place);
    for o in opts {
        let id = s.acts.on(place, o.act);
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

/// A form label: `text-[10px] text-gray-600 uppercase tracking-wider`.
pub(super) fn label(t: &str, mb: f32) -> El {
    text(t.to_uppercase(), 10.0, c(look::GRAY_600))
        .tracking(0.05)
        .line_height(15.0)
        .mb(mb)
        .role(Role::Label, t)
}
