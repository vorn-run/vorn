//! The empty grid's composer (`PromptLauncher` inline): the logo, the
//! prompt box with its pickers and send button, and the line under it.

use super::menus::launch_name;
use super::{agent_icon, c, ico, truncate, white, Screen, MOD_LABEL};
use crate::act::Act;
use crate::look;
use crate::state::{basename, MenuKind};
use crate::ui::{div, image, text, Action, El, Rgba, Role};

/// One of the composer's pickers: `px-2 py-1 rounded-md gap-1.5 text-xs`,
/// opening its menu above it.
#[allow(clippy::too_many_arguments)]
fn chip(
    s: &Screen<'_>,
    kind: MenuKind,
    name: &str,
    label: &str,
    size: f32,
    color: u32,
    a11y: &str,
    tint: Option<Rgba>,
) -> El {
    let open = s.st.menu.as_ref().is_some_and(|m| m.kind == kind);
    let glyph = if kind == MenuKind::LaunchAgent {
        agent_icon(s, name, size)
    } else {
        ico(
            name,
            size,
            if name == "folder-git-2" { 1.5 } else { 2.0 },
            c(color),
        )
    };
    let id = s.acts.on("composer", Act::Open(kind));
    let hot = if open { 1.0 } else { s.ui.hover(id) };
    let color = c(color);
    let bg = tint.unwrap_or(white(0.0).mix(white(0.06), hot));
    let mut e = div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(6.0)
        .px(8.0)
        .py(4.0)
        .rounded(6.0)
        .bg(bg)
        .role(Role::Button, a11y)
        .has_popup()
        .expanded(open)
        .child(glyph);
    if !label.is_empty() {
        e = e.child(text(label, 12.0, color).line_height(16.0).silent());
    }
    e.child(ico("chevron-down", 10.0, 2.0, color))
}

fn prompt_box(s: &Screen<'_>) -> El {
    let composer = &s.st.composer;
    let can_launch = composer.project.is_some() && !composer.launching;
    let project = match &composer.project {
        Some(p) => chip(
            s,
            MenuKind::LaunchProject,
            "folder",
            &truncate(p, 24),
            13.0,
            look::GRAY_300,
            p,
            None,
        ),
        None => chip(
            s,
            MenuKind::LaunchProject,
            "folder",
            "Select project",
            13.0,
            look::GRAY_500,
            "Select project",
            None,
        ),
    };
    let agent =
        s.st.launch
            .agent
            .as_deref()
            .unwrap_or(s.store.default_agent());
    let send_id = s.acts.on("composer", Act::Launch);
    let send = div()
        .id(send_id)
        .on(Action::Click)
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
        .child(chip(
            s,
            MenuKind::LaunchAgent,
            agent,
            launch_name(agent),
            14.0,
            look::GRAY_400,
            "Agent",
            None,
        ))
        .child(model_chip())
        .children(git_chips(s))
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

/// The model trigger: the agent's default; listing models is not ported.
fn model_chip() -> El {
    div()
        .row()
        .items_center()
        .gap(6.0)
        .px(8.0)
        .py(4.0)
        .rounded(6.0)
        .role(Role::Button, "Model")
        .disabled(true)
        .child(ico("cpu", 13.0, 1.75, c(look::GRAY_400)))
        .child(
            text("Default", 12.0, c(look::GRAY_400))
                .line_height(16.0)
                .silent(),
        )
        .child(ico("chevron-down", 10.0, 2.0, c(look::GRAY_400)))
}

/// The worktree and branch triggers, once the project is known to be a
/// git repository.
fn git_chips(s: &Screen<'_>) -> Vec<El> {
    let Some(path) =
        s.st.composer
            .project
            .as_deref()
            .and_then(|n| s.store.projects.iter().find(|p| p.name == n))
            .map(|p| p.path.as_str())
    else {
        return Vec::new();
    };
    let Some(branches) = s.st.branches.get(path) else {
        return Vec::new();
    };
    let l = &s.st.launch;
    let (label, color, tint) = if l.new_worktree {
        (
            "New worktree".to_owned(),
            look::AMBER_400,
            Some(Rgba::hexa(look::AMBER_400, 0.1)),
        )
    } else if let Some(w) = &l.worktree {
        (
            truncate(basename(w), 16),
            look::AMBER_400,
            Some(Rgba::hexa(look::AMBER_400, 0.1)),
        )
    } else {
        (String::new(), look::GRAY_600, None)
    };
    let worktree = chip(
        s,
        MenuKind::LaunchWorktree,
        "folder-git-2",
        &label,
        13.0,
        color,
        "Worktree",
        tint,
    );
    let branch = l
        .branch
        .clone()
        .or_else(|| branches.first().cloned())
        .unwrap_or_else(|| "branch".to_owned());
    let branch = chip(
        s,
        MenuKind::LaunchBranch,
        "git-branch",
        &truncate(&branch, 14),
        12.0,
        look::GRAY_400,
        "Branch",
        None,
    );
    vec![worktree, branch]
}

/// The line under the box: what to do first, why the launch failed, or a
/// tip.
fn hint(s: &Screen<'_>) -> El {
    let line = |t: &str, color: u32| text(t, 11.0, c(color)).line_height(16.0).mt(8.0);
    if s.st.composer.project.is_none() {
        return line(look::PICK_PROJECT, look::GRAY_600);
    }
    if let Some(e) = &s.st.composer.error {
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

/// The composer over the cards (Mod+N): a dimmed backdrop and the box.
pub(super) fn overlay(s: &Screen<'_>) -> Option<El> {
    if !s.st.composer_open || s.st.view != crate::screen::MainView::Sessions {
        return None;
    }
    let backdrop = s.acts.on("composer", Act::CloseDialog);
    let w = (s.size.0 * 0.8).min(look::COMPOSER_MAX_W);
    let modal = div()
        .col()
        .w(w)
        .p(24.0)
        .rounded(12.0)
        .border(1.0, white(0.08))
        .bg(c(look::SURFACE_OVERLAY))
        .role(Role::Dialog, "New session")
        .child(prompt_box(s))
        .child(hint(s));
    let modal = s.acts.click(modal, "composer", Act::Nothing);
    Some(
        div()
            .id(backdrop)
            .on(Action::Click)
            .absolute(0.0, 0.0)
            .size(s.size.0, s.size.1)
            .bg(Rgba::hexa(0, 0.5))
            .layered()
            .center()
            .child(modal),
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
