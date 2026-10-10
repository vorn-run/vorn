//! The empty grid's composer (`PromptLauncher` inline): the logo, the
//! prompt box with its pickers and send button, and the line under it.

use super::{c, ico, truncate, white, Screen, MOD_LABEL};
use crate::look;
use crate::ui::{div, image, text, El, Role};

/// One of the composer's pickers: `px-2 py-1 rounded-md gap-1.5 text-xs`.
fn chip(name: &str, label: &str, size: f32, color: u32, a11y: &str) -> El {
    let color = c(color);
    let stroke = if name == "folder-git-2" { 1.5 } else { 2.0 };
    let mut e = div()
        .row()
        .items_center()
        .gap(6.0)
        .px(8.0)
        .py(4.0)
        .rounded(6.0)
        .role(Role::Button, a11y)
        .child(ico(name, size, stroke, color));
    if !label.is_empty() {
        e = e.child(text(label, 12.0, color).line_height(16.0).silent());
    }
    e.child(ico("chevron-down", 10.0, 2.0, color))
}

fn prompt_box(s: &Screen<'_>) -> El {
    let composer = s.composer;
    let can_launch = composer.project.is_some() && !composer.launching;
    let project = match &composer.project {
        Some(p) => chip("folder", &truncate(p, 24), 13.0, look::GRAY_300, p),
        None => chip(
            "folder",
            "Select project",
            13.0,
            look::GRAY_500,
            "Select project",
        ),
    };
    let agent = s.store.default_agent();
    let send = div()
        .p(6.0)
        .rounded(999.0)
        .bg(if can_launch {
            c(look::INK)
        } else {
            white(0.06)
        })
        .role(Role::Button, "Launch (Enter)")
        .child(ico(
            "arrow-up",
            14.0,
            2.5,
            c(if can_launch {
                look::SURFACE_BASE
            } else {
                look::GRAY_600
            }),
        ));
    let bar = div()
        .row()
        .items_center()
        .gap(4.0)
        .px(12.0)
        .py(8.0)
        .child(project)
        .child(chip("bot", agent, 14.0, look::GRAY_400, "Agent"))
        .child(chip(
            "sparkles",
            "Default model",
            13.0,
            look::GRAY_400,
            "Model",
        ))
        .child(chip("folder-git-2", "", 13.0, look::GRAY_500, "Worktree"))
        .child(div().grow())
        .child(send);
    // The textarea's border-t white/[0.04] line above the settings bar.
    let rule = div().h(1.0).w_full().bg(white(0.04));
    let typed = !composer.text.is_empty();
    let shown = if typed {
        text(&composer.text, 14.0, c(look::GRAY_200))
            .line_height(20.0)
            .wrapping()
            .silent()
    } else {
        text(look::PLACEHOLDER, 14.0, c(look::GRAY_600))
            .line_height(20.0)
            .silent()
    };
    let input = div()
        .px(16.0)
        .h(83.0)
        .py(16.0)
        .role(Role::MultilineTextInput, "Describe your task")
        .value(composer.text.as_str())
        .child(shown);
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

/// The line under the box: what to do first, why the launch failed, or a
/// tip.
fn hint(s: &Screen<'_>) -> El {
    let line = |t: &str, color: u32| text(t, 11.0, c(color)).line_height(16.0).mt(8.0);
    if s.composer.project.is_none() {
        return line(look::PICK_PROJECT, look::GRAY_600);
    }
    if let Some(e) = &s.composer.error {
        // text-xs text-red-400
        return text(truncate(e, 120), 12.0, c(0xff6467))
            .line_height(16.0)
            .mt(8.0)
            .role(Role::Alert, e.as_str());
    }
    let kbd = div().px(4.0).py(2.0).rounded(4.0).bg(white(0.06)).child(
        text(
            format!("{MOD_LABEL}1\u{2013}{MOD_LABEL}9"),
            10.0,
            c(look::GRAY_500),
        )
        .line_height(12.0)
        .silent(),
    );
    div()
        .row()
        .items_center()
        .gap(6.0)
        .mt(8.0)
        .role(Role::Label, "Jump directly to any card by its position")
        .child(kbd)
        .child(
            text(
                "Jump directly to any card by its position",
                11.0,
                c(look::GRAY_600),
            )
            .line_height(16.0)
            .silent(),
        )
}

pub(super) fn empty_state(s: &Screen<'_>) -> El {
    let mut column = div()
        .col()
        .items_center()
        .w_full()
        .max_w(look::COMPOSER_MAX_W);
    if let Some(logo) = s.logo {
        column = column.child(
            image(logo, 32.0 * look::LOGO_ASPECT, 32.0)
                .role(Role::Image, "Vorn")
                .alpha(0.5),
        );
    }
    column = column
        .child(div().h(24.0).shrink0())
        .child(prompt_box(s))
        .child(hint(s));
    div().col().center().grow().px(16.0).child(column)
}
