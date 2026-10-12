//! The modal dialogs: the keyboard shortcuts panel and the add or edit
//! project dialog.

use super::{c, ico, white, Screen, ALT_LABEL, MOD_LABEL};
use crate::act::Act;
use crate::look;
use crate::state::Dialog;
use crate::ui::widgets::kbd;
use crate::ui::{div, text, text_input, Action, El, Id, Role};

/// The project dialog's folder field.
pub const PROJECT_PATH: Id = Id(0x7072_6f6a_7061_7468);
/// The project dialog's name field.
pub const PROJECT_NAME: Id = Id(0x7072_6f6a_6e61_6d65);

/// The shortcuts panel's rows: its categories, each with description and
/// keys, as `keyboard-shortcuts.ts` lists them.
pub fn shortcut_groups() -> Vec<(&'static str, Vec<(&'static str, String)>)> {
    let m = MOD_LABEL;
    let a = ALT_LABEL;
    vec![
        (
            "Navigation",
            vec![
                ("Next card", format!("{m}]")),
                ("Previous card", format!("{m}[")),
                ("Close / deselect", "Esc".to_owned()),
                ("Jump to card by position", format!("{m}1-9")),
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

pub(super) fn backdrop(s: &Screen<'_>, alpha: f32) -> El {
    let id = s.acts.on("dialog-backdrop", Act::CloseDialog);
    crate::ui::widgets::scrim(id, s.size, alpha)
}

fn label(t: &str) -> El {
    div().pb(8.0).child(
        text(t, 13.0, c(look::GRAY_400))
            .weight(500)
            .line_height(18.0)
            .silent(),
    )
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
    let folder = div().col().child(label("Project Folder")).child(
        div()
            .row()
            .items_center()
            .gap(8.0)
            .child(ico("folder-open", 16.0, 1.5, c(look::GRAY_400)))
            .child(input(s, PROJECT_PATH, "Project Folder", "No folder selected").grow()),
    );
    let name = div().col().child(label("Project Name")).child(input(
        s,
        PROJECT_NAME,
        "Project Name",
        "my-project",
    ));
    let body = div().col().gap(20.0).p(24.0).child(folder).child(name);
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
        Dialog::RemoveWorktree => super::git::removal(s),
        Dialog::Discard => super::side::discard(s),
        Dialog::Output => super::tasks::output(s),
        Dialog::RunPrompt | Dialog::Missed | Dialog::WfFile => super::workflows::dialog(s)?,
        Dialog::GateReview => super::gates::review(s)?,
    })
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
