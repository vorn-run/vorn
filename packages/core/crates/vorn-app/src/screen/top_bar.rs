//! The 40 px top bar: the sidebar toggle, the view pills and the session
//! dock on the left; grid options, recent sessions and new session on the
//! right (`App.tsx`'s toolbar).

use super::{c, ico, white, MainView, Screen, MOD_LABEL};
use crate::look;
use crate::ui::{div, text, El, Role};

/// A `p-1 rounded-md text-gray-400` icon button.
fn icon_button(name: &str, label: &str, stroke: f32) -> El {
    div()
        .p(4.0)
        .rounded(6.0)
        .role(Role::Button, label)
        .child(ico(name, 16.0, stroke, c(look::GRAY_400)))
}

/// `w-px h-4 bg-white/[0.06] mx-0.5`.
fn divider() -> El {
    div().w(1.0).h(16.0).bg(white(0.06)).ml(2.0).shrink0()
}

fn pills(active: MainView) -> El {
    let views = [MainView::Sessions, MainView::Tasks, MainView::Workflows];
    let mut row = div()
        .row()
        .bg(white(0.04))
        .rounded(8.0)
        .p(2.0)
        .gap(2.0)
        .role(Role::TabList, "Main view");
    for ((name, label), view) in look::PILLS.iter().zip(views) {
        let on = view == active;
        let mut pill = div()
            .px(10.0)
            .py(4.0)
            .rounded(6.0)
            .role(Role::Tab, *label)
            .selected(on)
            .child(ico(
                name,
                14.0,
                2.0,
                c(if on { look::WHITE } else { look::GRAY_500 }),
            ));
        if on {
            pill = pill.bg(white(0.1));
        }
        row = row.child(pill);
    }
    row
}

/// SessionDock, collapsed to its chip: `h-[26px] px-2 rounded-md border
/// bg-surface-raised`, the count in mono and a bronzo dot for approvals.
/// It is not there when nothing waits, as in the renderer.
fn dock(approvals: usize) -> Option<El> {
    if approvals == 0 {
        return None;
    }
    let label = format!(
        "{approvals} waiting approval{}",
        if approvals == 1 { "" } else { "s" }
    );
    let count = approvals.to_string();
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
            text(count, 11.0, c(look::GRAY_300))
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

pub(super) fn bar(s: &Screen<'_>) -> El {
    let mut left = div()
        .row()
        .items_center()
        .gap(4.0)
        .child(icon_button(
            "panel-left",
            &format!("Toggle sidebar ({MOD_LABEL}B)"),
            2.0,
        ))
        .child(divider())
        .child(pills(s.view));
    if let Some(dock) = dock(s.store.approvals()) {
        left = left.child(dock);
    }
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
