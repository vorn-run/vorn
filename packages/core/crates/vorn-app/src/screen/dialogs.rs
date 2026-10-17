//! The modal dialogs: the keyboard shortcuts panel and the add or edit
//! project dialog.

use super::{alt_label, c, ico, mod_label, white, Screen};
use crate::act::Act;
use crate::look;
use crate::state::Dialog;
use crate::ui::widgets::kbd;
use crate::ui::{div, text, text_input, tooltip, Action, El, Id, Role};

/// The project dialog's folder field.
pub const PROJECT_PATH: Id = Id(0x7072_6f6a_7061_7468);
/// The project dialog's name field.
pub const PROJECT_NAME: Id = Id(0x7072_6f6a_6e61_6d65);
/// A new project's icon and colour until others are picked.
pub const PROJECT_ICON: &str = "Folder";
pub const PROJECT_COLOR: &str = "#6b7280";

/// The shortcuts panel's rows: its categories, each with description and
/// keys.
pub fn shortcut_groups() -> Vec<(&'static str, Vec<(&'static str, String)>)> {
    let m = mod_label();
    let a = alt_label();
    vec![
        (
            "Navigation",
            vec![
                ("Next card", format!("{m}]")),
                ("Previous card", format!("{m}[")),
                ("Close / deselect", "Esc".to_owned()),
                ("Jump to card by position", format!("{m}1-9")),
                (
                    "Previous / next command",
                    format!("{m}\u{21e7}\u{2191} / \u{2193}"),
                ),
            ],
        ),
        (
            "Sessions",
            vec![
                ("New session", format!("{m}N")),
                ("Rename session", "F2".to_owned()),
            ],
        ),
        (
            "View",
            vec![
                ("Command palette", format!("{m}K")),
                ("Keyboard shortcuts", format!("{m}/")),
                ("Settings", format!("{m},")),
                ("Toggle sidebar", format!("{m}B")),
                ("New terminal session", "Ctrl+`".to_owned()),
                ("Sessions view", format!("{m}S")),
                ("Tasks view", format!("{m}T")),
                ("Workflows view", format!("{m}\u{21e7}W")),
                ("View options", format!("{m}J")),
                ("Fit the whole workflow on the canvas", "1".to_owned()),
            ],
        ),
        (
            "Filters",
            vec![
                ("Show all", format!("{a}1")),
                ("Show running", format!("{a}2")),
                ("Show waiting", format!("{a}3")),
                ("Show idle", format!("{a}4")),
                ("Show errors", format!("{a}5")),
            ],
        ),
    ]
}

fn group_label(label: &str) -> El {
    text(label.to_uppercase(), 11.0, c(look::GRAY_500))
        .weight(500)
        .line_height(16.0)
        .role(Role::Heading, label)
}

fn shortcuts(s: &Screen<'_>) -> El {
    let header = div()
        .row()
        .items_center()
        .justify_between()
        .px(20.0)
        .py(16.0)
        .child(
            text("Keyboard Shortcuts", 14.0, c(look::WHITE))
                .weight(500)
                .line_height(20.0)
                .role(Role::Heading, "Keyboard Shortcuts"),
        )
        .child(kbd("ESC", (10.0, 15.0), c(look::GRAY_600)));
    let row = |desc: &str, keys: El| {
        div()
            .row()
            .items_center()
            .justify_between()
            .py(6.0)
            .px(8.0)
            .rounded(6.0)
            .role(Role::Label, desc)
            .child(
                text(desc, 13.0, c(look::GRAY_300))
                    .line_height(18.0)
                    .silent(),
            )
            .child(keys)
    };
    let mut body = div()
        .id(Id::new("shortcuts-scroll"))
        .col()
        .gap(20.0)
        .px(20.0)
        .py(16.0)
        .max_h(400.0)
        .scroll_y();
    for (label, items) in shortcut_groups() {
        let mut g = div()
            .col()
            .gap(2.0)
            .child(div().pb(8.0).child(group_label(label)));
        for (desc, keys) in items {
            g = g.child(row(
                desc,
                kbd(&keys, (11.0, 16.5), c(look::GRAY_400)).px(8.0),
            ));
        }
        body = body.child(g);
    }
    body = body.child(
        div()
            .col()
            .gap(2.0)
            .child(div().pb(8.0).child(group_label("Tips")))
            .child(row(
                "Rename inline",
                text("Double-click card title", 11.0, c(look::GRAY_500))
                    .mono()
                    .line_height(16.0)
                    .silent(),
            )),
    );
    let guide = s.acts.on("shortcuts", Act::Welcome);
    body = body.child(
        div()
            .col()
            .child(div().h(1.0).w_full().bg(white(0.06)))
            .child(
                div()
                    .id(guide)
                    .on(Action::Click)
                    .row()
                    .items_center()
                    .justify_between()
                    .mt(12.0)
                    .py(6.0)
                    .px(8.0)
                    .rounded(6.0)
                    .bg(white(0.0).mix(white(0.06), s.ui.hover(guide)))
                    .role(Role::Button, "Show Welcome Guide")
                    .child(
                        text("Show Welcome Guide", 13.0, c(look::BRONZO))
                            .line_height(18.0)
                            .silent(),
                    )
                    .child(
                        text("Feature tour", 11.0, c(look::GRAY_500))
                            .line_height(16.0)
                            .silent(),
                    ),
            ),
    );
    let panel = div()
        .col()
        .w(520.0)
        .rounded(12.0)
        .border(1.0, white(0.08))
        .bg(c(look::SURFACE_OVERLAY))
        .clip()
        .role(Role::Dialog, "Keyboard Shortcuts")
        .child(header)
        .child(div().h(1.0).w_full().bg(white(0.06)))
        .child(body);
    let panel = s.acts.click(panel, "dialog", Act::Nothing);
    let x = ((s.size.0 - 520.0) / 2.0).max(0.0);
    backdrop(s, 0.4).child(panel.absolute(x, s.size.1 * 0.15))
}

pub(in crate::screen) fn backdrop(s: &Screen<'_>, alpha: f32) -> El {
    let id = s.acts.on("dialog-backdrop", Act::CloseDialog);
    crate::ui::widgets::scrim(id, s.size, alpha)
}

/// The dialog's `text-xs uppercase tracking-wider` field label.
fn label(t: &str) -> El {
    div().pb(8.0).child(
        text(t.to_uppercase(), 12.0, c(look::GRAY_500))
            .weight(500)
            .tracking(0.05)
            .line_height(16.0)
            .silent(),
    )
}

/// The host selector's Local or remote host button.
fn host_button(s: &Screen<'_>, host: Option<&String>, glyph: &str, name: &str, on: bool) -> El {
    let id = s.acts.on("project-host", Act::ProjectHost(host.cloned()));
    let hot = s.ui.hover(id);
    let (border, bg, fg, icon) = match (on, host.is_some()) {
        (true, false) => (white(0.15), white(0.06), c(look::WHITE), c(look::WHITE)),
        (true, true) => (
            c(look::BLUE_500).alpha(0.2),
            c(look::BLUE_500).alpha(0.06),
            c(look::BLUE_300),
            c(look::BLUE_400),
        ),
        (false, _) => (
            white(0.04).mix(white(0.10), hot),
            white(0.02),
            c(look::GRAY_500),
            c(look::GRAY_500),
        ),
    };
    div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(8.0)
        .px(12.0)
        .py(8.0)
        .rounded(8.0)
        .border(1.0, border)
        .bg(bg)
        .role(Role::Button, name)
        .selected(on)
        .child(ico(glyph, 13.0, 1.5, icon))
        .child(text(name, 14.0, fg).line_height(20.0).silent())
}

/// `IconColorPicker` with its labels and preview, as the project dialog shows it.
fn icon_color(s: &Screen<'_>, icon: &str, color: &str) -> El {
    let tint = super::workflows::hex(Some(color)).unwrap_or(c(look::ICON_DEFAULT));
    // `grid-cols-8 gap-1.5` across the dialog's 472 px.
    let cell = ((472.0 - 7.0 * 6.0) / 8.0_f32).floor();
    let mut grid = div().row().wrap_children().gap(6.0);
    for &(name, glyph, tip) in &look::ICON_OPTIONS {
        let id = s.acts.on("project-icon", Act::ProjectIcon(name.to_owned()));
        let on = icon == name;
        let hot = s.ui.hover(id);
        let b = div()
            .id(id)
            .on(Action::Click)
            .size(cell, 34.0)
            .center()
            .rounded(8.0)
            .bg(if on {
                white(0.08)
            } else {
                white(0.0).mix(white(0.04), hot)
            })
            .border(1.0, if on { white(0.2) } else { white(0.0) })
            .role(Role::Button, tip)
            .selected(on)
            .child(ico(glyph, 16.0, 1.5, if on { tint } else { c(0x9ca3af) }));
        grid = grid.child(tooltip(s.ui, b, id, tip, None));
    }
    let mut swatches = div().row().wrap_children().items_center().gap(8.0);
    for rgb in look::ICON_COLORS {
        let hexs = format!("#{rgb:06x}");
        let id = s.acts.on("project-color", Act::ProjectColor(hexs.clone()));
        let on = color.eq_ignore_ascii_case(&hexs);
        let hot = s.ui.hover(id);
        let size = if on { 30.8 } else { 28.0 };
        swatches = swatches.child(
            div().size(28.0, 28.0).center().child(
                div()
                    .id(id)
                    .on(Action::Click)
                    .size(size, size)
                    .rounded(size / 2.0)
                    .bg(c(rgb))
                    .border(
                        2.0,
                        if on {
                            c(look::WHITE)
                        } else {
                            white(0.0).mix(white(0.3), hot)
                        },
                    )
                    .role(Role::Button, format!("Color {hexs}"))
                    .selected(on),
            ),
        );
    }
    let glyph = look::icon_map(icon).unwrap_or("folder");
    swatches = swatches.child(
        div()
            .row()
            .items_center()
            .gap(8.0)
            .ml(12.0)
            .child(ico(glyph, 20.0, 1.5, tint))
            .child(
                text("Preview", 12.0, c(look::GRAY_500))
                    .line_height(16.0)
                    .silent(),
            ),
    );
    div()
        .col()
        .gap(20.0)
        .child(div().col().child(label("Icon")).child(grid))
        .child(div().col().child(label("Color")).child(swatches))
}

/// The dialog's `px-4 py-2.5` field on `white/[0.03]`.
fn input(s: &Screen<'_>, id: Id, name: &str, placeholder: &str) -> El {
    let focus = if s.ui.focused(id) { 1.0 } else { 0.0 };
    text_input(s.ui, id, name, placeholder)
        .px(16.0)
        .py(10.0)
        .rounded(8.0)
        .bg(white(0.03))
        .border(1.0, white(0.06).mix(white(0.15), focus))
        .w_full()
}

fn project(s: &Screen<'_>, editing: Option<&str>) -> El {
    let (title, subtitle, submit) = if editing.is_some() {
        ("Edit Project", "Update your project settings", "Save")
    } else {
        (
            "Add Project",
            "Select a folder and customize your project",
            "Add Project",
        )
    };
    let header = div()
        .col()
        .px(24.0)
        .py(16.0)
        .child(
            text(title, 18.0, c(look::WHITE))
                .weight(500)
                .line_height(28.0)
                .role(Role::Heading, title),
        )
        .child(
            text(subtitle, 14.0, c(look::GRAY_500))
                .line_height(20.0)
                .mt(2.0)
                .silent(),
        );
    let form = &s.st.form;
    let hosts: Vec<(String, String)> = s
        .store
        .config
        .get("remoteHosts")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|h| {
            Some((
                h.get("id")?.as_str()?.to_owned(),
                h.get("label")?.as_str()?.to_owned(),
            ))
        })
        .collect();
    let remote = form.host.is_some();
    let mut body = div().col().gap(20.0).p(24.0);
    if !hosts.is_empty() {
        let mut row = div()
            .row()
            .wrap_children()
            .gap(8.0)
            .child(host_button(s, None, "monitor", "Local", !remote));
        for (id, name) in &hosts {
            let on = form.host.as_deref() == Some(id.as_str());
            row = row.child(host_button(s, Some(id), "server", name, on));
        }
        body = body.child(div().col().child(label("Host")).child(row));
    }
    let folder = if remote {
        div()
            .col()
            .child(label("Remote Path"))
            .child(input(s, PROJECT_PATH, "Remote Path", "/home/user/project").mono())
    } else {
        let browse = s.acts.on("project-dialog", Act::BrowseFolder);
        let path = s.ui.edit_text(PROJECT_PATH);
        let shown = if path.is_empty() {
            "No folder selected"
        } else {
            path
        };
        div().col().child(label("Project Folder")).child(
            div()
                .row()
                .gap(8.0)
                .child(
                    div()
                        .id(browse)
                        .on(Action::Click)
                        .row()
                        .items_center()
                        .gap(8.0)
                        .px(16.0)
                        .py(10.0)
                        .rounded(8.0)
                        .bg(white(0.06).mix(white(0.10), s.ui.hover(browse)))
                        .border(1.0, white(0.08))
                        .role(Role::Button, "Browse")
                        .child(ico("folder-open", 16.0, 1.5, c(look::GRAY_200)))
                        .child(
                            text("Browse", 14.0, c(look::GRAY_200))
                                .line_height(20.0)
                                .silent(),
                        ),
                )
                .child(
                    div()
                        .grow()
                        .min_w0()
                        .px(16.0)
                        .py(10.0)
                        .rounded(8.0)
                        .bg(white(0.03))
                        .border(1.0, white(0.06))
                        .child(
                            text(shown, 14.0, c(look::GRAY_400))
                                .line_height(20.0)
                                .truncate(),
                        ),
                ),
        )
    };
    let name = div().col().child(label("Project Name")).child(input(
        s,
        PROJECT_NAME,
        "Project Name",
        "my-project",
    ));
    let body = body.child(folder).child(name).child(icon_color(
        s,
        form.icon.as_deref().unwrap_or(PROJECT_ICON),
        form.icon_color.as_deref().unwrap_or(PROJECT_COLOR),
    ));
    let ready = !s.ui.edit_text(PROJECT_NAME).trim().is_empty()
        && !s.ui.edit_text(PROJECT_PATH).trim().is_empty();
    let cancel = s.acts.on("project-dialog", Act::CloseDialog);
    let save = s.acts.on("project-dialog", Act::SaveProject);
    let footer = div()
        .row()
        .items_center()
        .gap(12.0)
        .px(24.0)
        .py(16.0)
        .child(div().grow())
        .child(
            div()
                .id(cancel)
                .on(Action::Click)
                .px(16.0)
                .py(8.0)
                .rounded(8.0)
                .bg(white(0.04).mix(white(0.08), s.ui.hover(cancel)))
                .role(Role::Button, "Cancel")
                .child(
                    text("Cancel", 14.0, c(look::GRAY_400))
                        .line_height(20.0)
                        .silent(),
                ),
        )
        .child(
            div()
                .id(save)
                .on(Action::Click)
                .px(16.0)
                .py(8.0)
                .rounded(8.0)
                .bg(white(0.10).mix(white(0.15), s.ui.hover(save)))
                .opacity(if ready { 1.0 } else { 0.3 })
                .role(Role::Button, submit)
                .disabled(!ready)
                .child(
                    text(submit, 14.0, c(look::WHITE))
                        .weight(500)
                        .line_height(20.0)
                        .silent(),
                ),
        );
    let panel = div()
        .col()
        .w(520.0)
        .rounded(12.0)
        .border(1.0, white(0.08))
        .bg(c(look::SURFACE_OVERLAY))
        .clip()
        .role(Role::Dialog, title)
        .child(header)
        .child(div().h(1.0).w_full().bg(white(0.06)))
        .child(body)
        .child(div().h(1.0).w_full().bg(white(0.06)))
        .child(footer);
    let panel = s.acts.click(panel, "dialog", Act::Nothing);
    backdrop(s, 0.5).center().child(panel)
}

pub(super) fn dialog(s: &Screen<'_>) -> Option<El> {
    Some(match s.st.dialog.as_ref()? {
        Dialog::Shortcuts => shortcuts(s),
        Dialog::Project(editing) => project(s, editing.as_deref()),
        Dialog::AddTask => super::tasks::add_task(s),
        Dialog::Commit => super::tasks::commit(s, super::tasks::diff::Scope::Task),
        Dialog::GitCommit => super::tasks::commit(s, super::tasks::diff::Scope::Session),
        Dialog::Discard => super::side::discard(s),
        Dialog::Paste => paste(s)?,
        Dialog::Output => super::tasks::output(s),
        Dialog::RunPrompt | Dialog::Missed => super::workflows::dialog(s)?,
        Dialog::GateReview => super::gates::review(s)?,
        Dialog::Palette => super::palette::palette(s),
        Dialog::Welcome => super::onboarding::modal(s)?,
        Dialog::SignIn => super::signin::sheet(s)?,
        Dialog::ExtInstall => super::plugins::dialog(s)?,
    })
}

/// "Paste N lines?" before a paste the program would run line by line.
fn paste(s: &Screen<'_>) -> Option<El> {
    let (_, text) = s.st.paste.as_ref()?;
    let n = text.lines().count();
    let title = if n > 1 {
        format!("Paste {n} lines?")
    } else {
        "Paste this text?".to_owned()
    };
    Some(super::side::alert(
        s,
        "paste",
        &title,
        Some("The program runs each line as it arrives."),
        ("Paste", Act::ConfirmPaste(true)),
        Act::ConfirmPaste(false),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shortcuts_panel_lists_every_category() {
        let groups = shortcut_groups();
        let names: Vec<&str> = groups.iter().map(|g| g.0).collect();
        assert_eq!(names, ["Navigation", "Sessions", "View", "Filters"]);
        assert_eq!(groups[3].1.len(), 5);
    }
}
