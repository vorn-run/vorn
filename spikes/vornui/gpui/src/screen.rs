//! Today's main screen and the terminal grid as GPUI elements, built from
//! the same Tailwind values as the vornui prototype (`spike_shared::look`).

use std::sync::Arc;

use gpui::{
    div, hsla, img, prelude::*, px, rgb, svg, AnyElement, Div, FontWeight, Hsla, Image, Role,
    SharedString, Stateful,
};
use spike_shared::look::{self, Rgb};

fn c(v: Rgb) -> Hsla {
    rgb(v).into()
}

/// Tailwind's `white/[a]`.
fn white(a: f32) -> Hsla {
    hsla(0.0, 0.0, 1.0, a)
}

/// An icon from [`crate::Icons`]: `name@stroke`.
fn ico(name: &str, size: f32, stroke: f32, color: Hsla) -> impl IntoElement {
    svg()
        .path(SharedString::from(format!("{name}@{stroke}")))
        .size(px(size))
        .flex_shrink_0()
        .text_color(color)
}

fn node(id: impl Into<SharedString>, role: Role) -> Stateful<Div> {
    let id: SharedString = id.into();
    div().id(id.clone()).role(role).aria_label(id)
}

/// A `p-1 rounded-md text-gray-400` icon button.
fn icon_button(name: &str, label: &'static str, stroke: f32) -> impl IntoElement {
    node(label, Role::Button)
        .p(px(4.0))
        .rounded(px(6.0))
        .child(ico(name, 16.0, stroke, c(look::GRAY_400)))
}

fn divider() -> impl IntoElement {
    div()
        .w(px(1.0))
        .h(px(16.0))
        .ml(px(2.0))
        .flex_shrink_0()
        .bg(white(0.06))
}

fn pills() -> impl IntoElement {
    let mut row = node("Main view", Role::TabList)
        .flex()
        .flex_row()
        .bg(white(0.04))
        .rounded(px(8.0))
        .p(px(2.0))
        .gap(px(2.0));
    for (i, (name, label)) in look::PILLS.iter().enumerate() {
        let active = i == 0;
        let color = c(if active { look::WHITE } else { look::GRAY_500 });
        let mut pill = node(*label, Role::Tab)
            .aria_selected(active)
            .px(px(10.0))
            .py(px(4.0))
            .rounded(px(6.0))
            .child(ico(name, 14.0, 2.0, color));
        if active {
            pill = pill.bg(white(0.1));
        }
        row = row.child(pill);
    }
    row
}

/// SessionDock's chip: `h-[26px] px-2 rounded-md border bg-surface-raised`.
fn sessions_chip() -> impl IntoElement {
    node(format!("{} sessions", look::SESSIONS), Role::Button)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .h(px(26.0))
        .px(px(8.0))
        .ml(px(4.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(white(0.06))
        .bg(c(look::SURFACE_RAISED))
        .child(ico("layers", 11.0, 1.5, c(look::GRAY_300)))
        .child(
            div()
                .text_size(px(11.0))
                .line_height(px(11.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(c(look::GRAY_300))
                .child(look::SESSIONS),
        )
}

/// The 40 px bar, its `border-b white/[0.06]` included.
pub fn top_bar() -> impl IntoElement {
    let left = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .child(icon_button("panel-left", "Toggle sidebar", 2.0))
        .child(divider())
        .child(pills())
        .child(sessions_chip());
    let right = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .child(icon_button("sliders-horizontal", "Grid options", 1.5))
        .child(divider())
        .child(icon_button("rotate-ccw", "Recent sessions", 1.5))
        .child(icon_button("plus", "New session", 2.0));
    let bar = node("Toolbar", Role::Toolbar)
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .h(px(look::TOP_BAR_H - 1.0))
        .px(px(12.0))
        .child(left)
        .child(right);
    div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .bg(c(look::SURFACE_BASE))
        .child(bar)
        .child(div().h(px(1.0)).w_full().bg(white(0.06)))
}

/// One of the composer's pickers: `px-2 py-1 rounded-md gap-1.5 text-xs`.
fn chip(name: &str, label: &'static str, size: f32, dim: bool) -> impl IntoElement {
    let color = c(if dim { look::GRAY_500 } else { look::GRAY_400 });
    let stroke = if name == "folder-git-2" { 1.5 } else { 2.0 };
    let mut e = node(
        if label.is_empty() { "Worktree" } else { label },
        Role::Button,
    )
    .flex()
    .flex_row()
    .items_center()
    .gap(px(6.0))
    .px(px(8.0))
    .py(px(4.0))
    .rounded(px(6.0))
    .child(ico(name, size, stroke, color));
    if !label.is_empty() {
        e = e.child(
            div()
                .text_size(px(12.0))
                .line_height(px(16.0))
                .text_color(color)
                .child(label),
        );
    }
    e.child(ico("chevron-down", 10.0, 2.0, color))
}

fn composer() -> impl IntoElement {
    let send = node("Launch", Role::Button)
        .p(px(6.0))
        .rounded_full()
        .bg(white(0.06))
        .child(ico("arrow-up", 14.0, 2.5, c(look::GRAY_600)));
    let bar = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .px(px(12.0))
        .py(px(8.0))
        .children(look::CHIPS.iter().map(|(n, l, s, d)| chip(n, l, *s, *d)))
        .child(div().flex_grow(1.))
        .child(send);
    let input = node("Describe your task", Role::MultilineTextInput)
        .px(px(16.0))
        .h(px(83.0))
        .py(px(16.0))
        .text_size(px(14.0))
        .line_height(px(20.0))
        .text_color(c(look::GRAY_600))
        .child(look::PLACEHOLDER);
    div()
        .flex()
        .flex_col()
        .w_full()
        .rounded(px(12.0))
        .border_1()
        .border_color(white(0.06))
        .bg(c(look::SURFACE_RAISED))
        .child(input)
        .child(div().h(px(1.0)).w_full().bg(white(0.04)))
        .child(bar)
}

pub fn main_screen(logo: Arc<Image>) -> AnyElement {
    let column = div()
        .flex()
        .flex_col()
        .items_center()
        .w_full()
        .max_w(px(look::COMPOSER_MAX_W))
        .child(
            node("Vorn", Role::Image).child(
                img(logo)
                    .w(px(32.0 * 400.0 / 185.0))
                    .h(px(32.0))
                    .opacity(0.5),
            ),
        )
        .child(div().h(px(24.0)).flex_shrink_0())
        .child(composer())
        .child(
            div()
                .mt(px(8.0))
                .text_size(px(11.0))
                .line_height(px(16.0))
                .text_color(c(look::GRAY_600))
                .child(look::TIP),
        );
    div()
        .flex()
        .flex_col()
        .size_full()
        .bg(c(look::SURFACE_BASE))
        .child(top_bar())
        .child(
            div()
                .flex()
                .flex_col()
                .flex_grow(1.)
                .items_center()
                .justify_center()
                .px(px(16.0))
                .child(column),
        )
        .into_any_element()
}

/// The top bar over the panes, in the grid's rows and columns.
pub fn grid_screen(panes: Vec<AnyElement>) -> AnyElement {
    let n = panes.len();
    let (cols, rows) = spike_shared::layout(n);
    let mut panes = panes.into_iter();
    let mut body = div()
        .flex()
        .flex_col()
        .flex_grow(1.)
        .gap(px(look::GAP))
        .bg(c(look::PANE_BORDER));
    for _ in 0..rows {
        let mut row = div().flex().flex_row().flex_1().gap(px(look::GAP));
        for _ in 0..cols {
            row = match panes.next() {
                Some(p) => row.child(p),
                None => row.child(div().flex_1()),
            };
        }
        body = body.child(row);
    }
    div()
        .flex()
        .flex_col()
        .size_full()
        .bg(c(look::SURFACE_BASE))
        .child(top_bar())
        .child(body)
        .into_any_element()
}
