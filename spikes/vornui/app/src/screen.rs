//! Today's main screen and the terminal grid, as vornui elements. Sizes and
//! colors are the renderer's Tailwind classes (see `spike_shared::look`).

use spike_shared::look::{self, Rgb};
use vornui::accesskit::Role;
use vornui::{custom, div, icon, image, text, El, Rgba};

fn c(rgb: Rgb) -> Rgba {
    Rgba::hex(rgb)
}

/// Tailwind's `white/[a]`.
fn white(a: f32) -> Rgba {
    Rgba::hexa(look::WHITE, a)
}

fn ico(name: &str, size: f32, stroke: f32, color: Rgba) -> El {
    icon(look::icon(name, stroke).unwrap_or_default(), size, color)
}

/// A `p-1 rounded-md text-gray-400` icon button.
fn icon_button(name: &str, label: &str, stroke: f32) -> El {
    div()
        .p(4.0)
        .rounded(6.0)
        .role(Role::Button, label)
        .child(ico(name, 16.0, stroke, c(look::GRAY_400)))
}

fn divider() -> El {
    div().w(1.0).h(16.0).bg(white(0.06)).ml(2.0).shrink0()
}

fn pills() -> El {
    let mut row = div()
        .row()
        .bg(white(0.04))
        .rounded(8.0)
        .p(2.0)
        .gap(2.0)
        .role(Role::TabList, "Main view");
    for (i, (name, label)) in look::PILLS.iter().enumerate() {
        let active = i == 0;
        let mut pill = div()
            .px(10.0)
            .py(4.0)
            .rounded(6.0)
            .role(Role::Tab, *label)
            .selected(active)
            .child(ico(
                name,
                14.0,
                2.0,
                c(if active { look::WHITE } else { look::GRAY_500 }),
            ));
        if active {
            pill = pill.bg(white(0.1));
        }
        row = row.child(pill);
    }
    row
}

/// SessionDock's chip: `h-[26px] px-2 rounded-md border bg-surface-raised`.
fn sessions_chip() -> El {
    div()
        .row()
        .items_center()
        .gap(6.0)
        .h(26.0)
        .px(8.0)
        .ml(4.0)
        .rounded(6.0)
        .border(1.0, white(0.06))
        .bg(c(look::SURFACE_RAISED))
        .role(Role::Button, format!("{} sessions", look::SESSIONS))
        .child(ico("layers", 11.0, 1.5, c(look::GRAY_300)))
        .child(
            text(look::SESSIONS, 11.0, c(look::GRAY_300))
                .weight(500)
                .line_height(11.0)
                .silent(),
        )
}

/// The 40 px bar, its `border-b white/[0.06]` included.
pub fn top_bar() -> El {
    let left = div()
        .row()
        .items_center()
        .gap(4.0)
        .child(icon_button("panel-left", "Toggle sidebar", 2.0))
        .child(divider())
        .child(pills())
        .child(sessions_chip());
    let right = div()
        .row()
        .items_center()
        .gap(4.0)
        .child(icon_button("sliders-horizontal", "Grid options", 1.5))
        .child(divider())
        .child(icon_button("rotate-ccw", "Recent sessions", 1.5))
        .child(icon_button("plus", "New session", 2.0));
    let bar = div()
        .row()
        .items_center()
        .justify_between()
        .h(look::TOP_BAR_H - 1.0)
        .px(12.0)
        .role(Role::Toolbar, "Toolbar")
        .child(left)
        .child(right);
    div()
        .col()
        .shrink0()
        .bg(c(look::SURFACE_BASE))
        .child(bar)
        .child(div().h(1.0).w_full().bg(white(0.06)))
}

/// One of the composer's pickers: `px-2 py-1 rounded-md gap-1.5 text-xs`.
fn chip(name: &str, label: &str, size: f32, dim: bool) -> El {
    let color = c(if dim { look::GRAY_500 } else { look::GRAY_400 });
    let stroke = if name == "folder-git-2" { 1.5 } else { 2.0 };
    let mut e = div()
        .row()
        .items_center()
        .gap(6.0)
        .px(8.0)
        .py(4.0)
        .rounded(6.0)
        .role(
            Role::Button,
            if label.is_empty() { "Worktree" } else { label },
        )
        .child(ico(name, size, stroke, color));
    if !label.is_empty() {
        e = e.child(text(label, 12.0, color).line_height(16.0).silent());
    }
    e.child(ico("chevron-down", 10.0, 2.0, color))
}

fn composer() -> El {
    let send = div()
        .p(6.0)
        .rounded(999.0)
        .bg(white(0.06))
        .role(Role::Button, "Launch")
        .child(ico("arrow-up", 14.0, 2.5, c(look::GRAY_600)));
    let bar = div()
        .row()
        .items_center()
        .gap(4.0)
        .px(12.0)
        .py(8.0)
        .children(look::CHIPS.iter().map(|(n, l, s, d)| chip(n, l, *s, *d)))
        .child(div().grow())
        .child(send);
    // The textarea's border-t white/[0.04] line above the settings bar.
    let rule = div().h(1.0).w_full().bg(white(0.04));
    let input = div()
        .px(16.0)
        .h(83.0)
        .py(16.0)
        .role(Role::MultilineTextInput, "Describe your task")
        .child(
            text(look::PLACEHOLDER, 14.0, c(look::GRAY_600))
                .line_height(20.0)
                .silent(),
        );
    div()
        .col()
        .w_full()
        .rounded(12.0)
        .border(1.0, white(0.06))
        .bg(c(look::SURFACE_RAISED))
        .child(input)
        .child(rule)
        .child(bar)
}

pub fn main_screen(logo: u32) -> El {
    let column = div()
        .col()
        .items_center()
        .w_full()
        .max_w(look::COMPOSER_MAX_W)
        .child(
            image(logo, 32.0 * 400.0 / 185.0, 32.0)
                .role(Role::Image, "Vorn")
                .alpha(0.5),
        )
        .child(div().h(24.0).shrink0())
        .child(composer())
        .child(
            text(look::TIP, 11.0, c(look::GRAY_600))
                .line_height(16.0)
                .mt(8.0),
        );
    div()
        .col()
        .w_full()
        .h_full()
        .bg(c(look::SURFACE_BASE))
        .child(top_bar())
        .child(div().col().center().grow().px(16.0).child(column))
}

/// The top bar over `n` panes in the grid's rows and columns; pane `i` is
/// custom box `i`, painted by [`crate::term`].
pub fn grid_screen(n: usize) -> El {
    let (cols, rows) = spike_shared::layout(n);
    let mut body = div().col().grow().gap(look::GAP).bg(c(look::PANE_BORDER));
    for r in 0..rows {
        let mut row = div().row().grow().gap(look::GAP);
        for col in 0..cols {
            let i = r * cols + col;
            let pane = if i < n { custom(i as u64) } else { div() };
            row = row.child(pane.grow());
        }
        body = body.child(row);
    }
    div()
        .col()
        .w_full()
        .h_full()
        .bg(c(look::SURFACE_BASE))
        .child(top_bar())
        .child(body)
}
