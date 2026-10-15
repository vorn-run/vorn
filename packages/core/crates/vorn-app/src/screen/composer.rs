//! The empty grid's composer (`PromptLauncher` inline): the logo, the
//! prompt box with its pickers and send button, and the line under it.

use super::menus::launch_name;
use super::{agent_icon, c, ico, mod_label, truncate, white, Screen};
use crate::act::Act;
use crate::app::MODEL_ID;
use crate::look;
use crate::models::{footer, rows, selects_models, Listing, Row, Status};
use crate::state::{basename, MenuKind};
use crate::ui::{area, div, field, image, text, tooltip, Action, El, Id, Rgba, Role, TextStyle};

/// The launcher's prompt field.
pub const COMPOSER_INPUT: Id = Id(0x636f_6d70_6f73_6572);

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
        .children(model_chip(s, agent))
        .children(git_chips(s))
        .child(div().grow())
        .child(send);
    // The textarea's border-t white/[0.04] line above the settings bar.
    let rule = div().h(1.0).w_full().bg(white(0.04));
    // PromptLauncher's textarea: `px-4 py-4 text-sm`, three rows, no focus outline.
    let input = area(
        COMPOSER_INPUT,
        "Describe your task",
        look::PLACEHOLDER,
        TextStyle {
            line_height: 20.0,
            ..TextStyle::ui(14.0)
        },
        c(look::GRAY_200),
    )
    .placeholder_color(c(look::GRAY_600))
    .w_full()
    .px(16.0)
    .h(83.0)
    .py(16.0);
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

/// What the model picker shows: the launcher's agent, its listing for the
/// launcher's project, and the model picked for it.
fn model_state<'a>(s: &'a Screen<'_>) -> (&'a str, Option<&'a Listing>, Option<&'a str>) {
    let agent =
        s.st.launch
            .agent
            .as_deref()
            .unwrap_or(s.store.default_agent());
    let path =
        s.st.composer
            .project
            .as_deref()
            .and_then(|n| s.store.projects.iter().find(|p| p.name == n))
            .map_or("", |p| p.path.as_str());
    let listing = s.st.models.lists.get(&(agent.to_owned(), path.to_owned()));
    let value = s.st.models.picked.get(agent).map(String::as_str);
    (agent, listing, value)
}

/// `ModelPicker`'s compact trigger, for the agents that take a model.
fn model_chip(s: &Screen<'_>, agent: &str) -> Option<El> {
    if !selects_models(agent) {
        return None;
    }
    let (_, listing, value) = model_state(s);
    let label = value.map(|v| listing.map_or(v, |l| l.label(v)));
    let open =
        s.st.menu
            .as_ref()
            .is_some_and(|m| m.kind == MenuKind::LaunchModel);
    let id = s.acts.on("composer", Act::Open(MenuKind::LaunchModel));
    let hot = if open { 1.0 } else { s.ui.hover(id) };
    let a11y = label.map_or("Model: agent default".to_owned(), |l| format!("Model: {l}"));
    let tone = |set: u32| c(if label.is_some() { set } else { look::GRAY_500 });
    Some(
        div()
            .id(id)
            .on(Action::Click)
            .row()
            .items_center()
            .gap(6.0)
            .px(8.0)
            .py(4.0)
            .rounded(6.0)
            .bg(white(0.0).mix(white(0.06), hot))
            .role(Role::Button, &a11y)
            .has_popup()
            .expanded(open)
            .child(ico("cpu", 13.0, 1.75, tone(look::GRAY_300)))
            .child(
                div().max_w(170.0).min_w0().child(
                    text(label.unwrap_or("Default"), 12.0, tone(look::GRAY_400))
                        .line_height(16.0)
                        .truncate()
                        .silent(),
                ),
            )
            .child(ico("chevron-down", 10.0, 2.0, c(look::GRAY_500))),
    )
}

/// One of the model menu's choices; `id` is none for the agent's default.
fn model_row(
    s: &Screen<'_>,
    id: Option<&str>,
    value: Option<&str>,
    label: &str,
    tip: Option<&str>,
) -> El {
    let current = id == value;
    let act = s
        .acts
        .on("model-pick", Act::PickModel(id.map(str::to_owned)));
    let hot = s.ui.hover(act);
    let color = if current {
        c(look::WHITE)
    } else {
        c(look::GRAY_300).mix(c(look::WHITE), hot)
    };
    let check = if current {
        ico("check", 11.0, 3.0, color)
    } else {
        div().w(11.0).h(11.0)
    };
    let mut row = div()
        .id(act)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(8.0)
        .px(12.0)
        .py(6.0)
        .bg(if current {
            white(0.06)
        } else {
            white(0.04 * hot)
        })
        .role(Role::ListBoxOption, label)
        .selected(current)
        .child(check.shrink0())
        .child(
            div().grow().min_w0().child(
                text(label, 12.0, color)
                    .line_height(16.0)
                    .truncate()
                    .silent(),
            ),
        );
    if let Some(id) = id.filter(|id| *id != label) {
        row = row.child(
            div().max_w(120.0).min_w0().shrink0().child(
                text(id, 10.0, c(look::GRAY_600))
                    .mono()
                    .line_height(15.0)
                    .truncate()
                    .silent(),
            ),
        );
    }
    match tip {
        Some(t) => tooltip(s.ui, row, act, t, None),
        None => row,
    }
}

/// `ModelPicker`'s menu: the agent's default, its listed models, the id
/// field, and where the list came from.
pub(super) fn model_panel(s: &Screen<'_>) -> El {
    let (agent, listing, value) = model_state(s);
    let empty = Listing::default();
    let listing = listing.unwrap_or(&empty);
    let mut list = div()
        .id(Id::new("model-list"))
        .col()
        .py(4.0)
        .max_h(240.0)
        .scroll_y();
    for r in rows(agent, listing, value, s.ui.edit_text(MODEL_ID)) {
        list = list.child(match r {
            Row::Asking => div()
                .row()
                .items_center()
                .gap(8.0)
                .px(12.0)
                .py(6.0)
                .role(Role::Status, "Asking for models")
                .child(ico("loader-circle", 11.0, 2.0, c(look::GRAY_500)))
                .child(
                    text(
                        format!("Asking {agent} for its models\u{2026}"),
                        12.0,
                        c(look::GRAY_500),
                    )
                    .line_height(16.0)
                    .silent(),
                ),
            Row::Default => model_row(s, None, value, "Agent default", None),
            Row::Provider(p) => div().px(12.0).pt(8.0).pb(2.0).child(
                text(p.to_uppercase(), 10.0, c(look::GRAY_600))
                    .tracking(0.025)
                    .line_height(15.0)
                    .silent(),
            ),
            Row::Model {
                id,
                label,
                description,
            } => model_row(s, Some(id), value, label, description),
            Row::NoMatch => div().px(12.0).py(6.0).child(
                text("No listed model matches", 12.0, c(look::GRAY_600))
                    .line_height(16.0)
                    .silent(),
            ),
        });
    }
    let invalid = s.st.models.invalid;
    let border = if invalid.is_some() {
        c(look::DANGER)
    } else if s.ui.focused(MODEL_ID) {
        white(0.2)
    } else {
        white(0.08)
    };
    let mut id_box = div()
        .col()
        .px(12.0)
        .py(8.0)
        .border_top(1.0, white(0.06))
        .child(
            div()
                .row()
                .items_center()
                .gap(8.0)
                .child(
                    text("ID", 10.0, c(look::GRAY_600))
                        .tracking(0.025)
                        .line_height(15.0)
                        .silent(),
                )
                .child(
                    div()
                        .grow()
                        .min_w0()
                        .px(8.0)
                        .py(2.0)
                        .rounded(4.0)
                        .bg(white(0.04))
                        .border(1.0, border)
                        .frames(MODEL_ID)
                        .child(
                            field(
                                MODEL_ID,
                                "Model id",
                                "type a model id",
                                TextStyle {
                                    line_height: 16.5,
                                    ..TextStyle::mono(11.0)
                                },
                                c(look::GRAY_200),
                            )
                            .w_full(),
                        ),
                ),
        );
    if let Some(why) = invalid {
        id_box = id_box.child(
            text(why, 11.0, c(look::DANGER))
                .line_height(16.5)
                .wrapping()
                .mt(4.0)
                .role(Role::Alert, why),
        );
    }
    let (line, spinning) = footer(agent, listing, s.st.models.opened_at);
    let mut foot = div().row().items_center().gap(8.0).px(12.0).pb(8.0);
    if spinning {
        foot = foot.child(ico("loader-circle", 10.0, 2.0, c(look::GRAY_600)));
    }
    foot = foot.child(
        div().grow().min_w0().child(
            text(line, 10.0, c(look::GRAY_600))
                .line_height(15.0)
                .truncate()
                .silent(),
        ),
    );
    let can_refresh = !listing.loading
        && listing
            .catalog
            .as_ref()
            .is_some_and(|c| c.status != Status::Unavailable);
    if can_refresh {
        let id = s.acts.on("model-refresh", Act::RefreshModels);
        let hot = s.ui.hover(id);
        let b = div()
            .id(id)
            .on(Action::Click)
            .w(20.0)
            .h(20.0)
            .rounded(4.0)
            .items_center()
            .justify_center()
            .bg(white(0.06 * hot))
            .role(Role::Button, "Refresh models")
            .child(ico(
                "refresh-cw",
                11.0,
                2.0,
                c(look::GRAY_500).mix(c(look::GRAY_300), hot),
            ));
        foot = foot.child(tooltip(s.ui, b, id, &format!("Ask {agent} again"), None));
    }
    div()
        .col()
        .w(288.0)
        .rounded(8.0)
        .border(1.0, white(0.08))
        .bg(c(look::SURFACE_OVERLAY))
        .role(Role::ListBox, "Model")
        .child(list)
        .child(id_box)
        .child(foot)
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
        return text(truncate(e, 120), 12.0, c(look::RED_400))
            .line_height(16.0)
            .mt(8.0)
            .role(Role::Alert, e.as_str());
    }
    let kbd = div().px(4.0).py(2.0).rounded(4.0).bg(white(0.06)).child(
        text(
            format!("{m}1\u{2013}{m}9", m = mod_label()),
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
        .child_if(super::points::composer(s))
        .child(prompt_box(s))
        .child(hint(s))
        .child_if(super::points::launchers(s));
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
        .child_if(super::points::composer(s))
        .child(prompt_box(s))
        .child(hint(s))
        .child_if(super::points::launchers(s));
    div().col().center().grow().px(16.0).child(column)
}
