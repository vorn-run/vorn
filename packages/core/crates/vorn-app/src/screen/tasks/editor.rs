//! The description editor (`RichMarkdownEditor`): markdown in a framed
//! field, the slash menu at the caret and the floating format toolbar over
//! a selection.

use super::{desc_form, desc_id, link_id};
use crate::act::Act;
use crate::look;
use crate::screen::{c, ico, white, Screen};
use crate::tasks::{self, Form, Mark, TaskAct, SLASH};
use crate::ui::{area, div, field, text, tooltip, Action, El, Role, TextStyle};

const LINE: f32 = 22.4;

/// The framed field, grown to its text; `cols` is about how many
/// characters fit a line, for wrapped lines.
pub(super) fn editor(s: &Screen<'_>, form: Form, placeholder: &str, min_h: f32, cols: usize) -> El {
    let id = desc_id(form);
    let body = s.ui.edit_text(id);
    let lines: usize = body
        .split('\n')
        .map(|l| l.chars().count().div_ceil(cols.max(1)).max(1))
        .sum();
    let h = (lines as f32 * LINE + 24.0).max(min_h);
    let focus = s.ui.focused(id);
    div()
        .w_full()
        .h(h)
        .px(16.0)
        .py(12.0)
        .rounded(8.0)
        .bg(white(0.03))
        .border(1.0, if focus { white(0.15) } else { white(0.06) })
        .child(
            area(
                id,
                "Description",
                placeholder,
                TextStyle {
                    line_height: LINE,
                    ..TextStyle::ui(14.0)
                },
                c(look::GRAY_200),
            )
            .w_full()
            .h_full(),
        )
}

/// The form whose description is focused, with its text and caret.
fn focused<'a>(s: &'a Screen<'_>) -> Option<(Form, &'a crate::ui::TextEdit)> {
    [Form::Dialog, Form::Detail].into_iter().find_map(|f| {
        let id = desc_id(f);
        s.ui.focused(id)
            .then(|| s.ui.edit(id))
            .flatten()
            .map(|e| (f, e))
    })
}

/// The slash menu's entries for the focused editor, when it shows one.
pub fn slash_open(ui: &crate::ui::Ui, closed: Option<usize>) -> Option<(Form, usize, Vec<usize>)> {
    let (form, edit) = [Form::Dialog, Form::Detail].into_iter().find_map(|f| {
        let id = desc_id(f);
        ui.focused(id)
            .then(|| ui.edit(id))
            .flatten()
            .map(|e| (f, e))
    })?;
    if edit.anchor() != edit.caret() {
        return None;
    }
    let (start, query) = tasks::slash_query(edit.text(), edit.caret())?;
    if closed == Some(start) {
        return None;
    }
    let matches = tasks::slash_matches(&query);
    (!matches.is_empty()).then_some((form, start, matches))
}

/// What floats over the editor: the slash menu or the format toolbar.
pub(in crate::screen) fn overlay(s: &Screen<'_>) -> Option<El> {
    let caret = s.ui.ime_area();
    if let Some((form, _, matches)) = slash_open(s.ui, s.st.tasks.slash_closed) {
        let r = caret?;
        return Some(slash_menu(s, form, &matches).absolute(r.x, r.y + r.h + 6.0));
    }
    if s.st.tasks.linking.is_some() {
        let (x, y) = s.st.tasks.link_pos;
        return Some(link_bar(s).absolute((x - 120.0).max(0.0), (y - 44.0).max(0.0)));
    }
    let (form, edit) = focused(s)?;
    if edit.selection().is_empty() || in_code(edit.text(), edit.selection().start) {
        return None;
    }
    let r = caret?;
    let unlink = tasks::link_at(edit.text(), &edit.selection()).is_some();
    Some(toolbar(s, form, unlink).absolute((r.x - 90.0).max(0.0), (r.y - 8.0 - 34.0).max(0.0)))
}

/// Whether `at` lies in a fenced code block.
fn in_code(text: &str, at: usize) -> bool {
    text[..at.min(text.len())]
        .lines()
        .filter(|l| l.trim_start().starts_with("```"))
        .count()
        % 2
        == 1
}

fn slash_menu(s: &Screen<'_>, form: Form, matches: &[usize]) -> El {
    let sel = s.st.tasks.slash.min(matches.len().saturating_sub(1));
    let mut col = div()
        .id(crate::ui::Id::new("task-slash"))
        .col()
        .min_w(220.0)
        .max_h(280.0)
        .scroll_y()
        .py(4.0)
        .rounded(8.0)
        .border(1.0, white(0.08))
        .bg(c(look::SURFACE_OVERLAY))
        .role(Role::Menu, "Commands");
    for (i, &m) in matches.iter().enumerate() {
        let (title, desc, icon, _) = SLASH[m];
        let id = s.acts.on("task-slash", Act::Task(TaskAct::Slash(form, m)));
        let on = i == sel || s.ui.hover(id) > 0.5;
        col = col.child(
            div()
                .id(id)
                .on(Action::Click)
                .row()
                .items_center()
                .gap(12.0)
                .px(12.0)
                .py(8.0)
                .bg(if on { white(0.08) } else { white(0.0) })
                .role(Role::MenuItem, title)
                .selected(i == sel)
                .child(
                    div()
                        .size(28.0, 28.0)
                        .rounded(6.0)
                        .bg(if on { white(0.08) } else { white(0.04) })
                        .center()
                        .child(ico(
                            icon,
                            15.0,
                            1.8,
                            if on {
                                c(look::WHITE)
                            } else {
                                c(look::GRAY_400)
                            },
                        )),
                )
                .child(
                    div()
                        .col()
                        .child(
                            text(
                                title,
                                14.0,
                                if on {
                                    c(look::WHITE)
                                } else {
                                    c(look::GRAY_200)
                                },
                            )
                            .weight(500)
                            .line_height(18.0)
                            .silent(),
                        )
                        .child(
                            text(desc, 11.0, c(look::GRAY_500))
                                .line_height(14.0)
                                .silent(),
                        ),
                ),
        );
    }
    col
}

fn tool(s: &Screen<'_>, act: TaskAct, icon: &str, label: &str) -> El {
    let id = s.acts.on("task-format", Act::Task(act));
    let hot = s.ui.hover(id);
    let b = div()
        .id(id)
        .on(Action::Click)
        .p(6.0)
        .rounded(4.0)
        .bg(white(0.0).mix(white(0.08), hot))
        .role(Role::Button, label)
        .child(ico(
            icon,
            14.0,
            2.0,
            c(look::GRAY_400).mix(c(look::WHITE), hot),
        ));
    tooltip(s.ui, b, id, label, None)
}

fn toolbar(s: &Screen<'_>, form: Form, unlink: bool) -> El {
    div()
        .row()
        .items_center()
        .gap(2.0)
        .p(4.0)
        .rounded(8.0)
        .border(1.0, white(0.10))
        .bg(c(look::SURFACE_OVERLAY))
        .role(Role::Toolbar, "Format")
        .child(tool(s, TaskAct::Format(form, Mark::Bold), "bold", "Bold"))
        .child(tool(
            s,
            TaskAct::Format(form, Mark::Italic),
            "italic",
            "Italic",
        ))
        .child(tool(
            s,
            TaskAct::Format(form, Mark::Strike),
            "strikethrough",
            "Strikethrough",
        ))
        .child(tool(s, TaskAct::Format(form, Mark::Code), "code", "Code"))
        .child(div().w(1.0).h(16.0).bg(white(0.08)).mx(2.0))
        .child(if unlink {
            tool(
                s,
                TaskAct::Format(form, Mark::Link),
                "unlink",
                "Remove link",
            )
        } else {
            tool(s, TaskAct::StartLink(form), "link", "Add link")
        })
}

fn link_bar(s: &Screen<'_>) -> El {
    let apply = s.acts.on("task-link-apply", Act::Task(TaskAct::ApplyLink));
    let hot = s.ui.hover(apply);
    div()
        .row()
        .items_center()
        .gap(4.0)
        .p(4.0)
        .rounded(8.0)
        .border(1.0, white(0.10))
        .bg(c(look::SURFACE_OVERLAY))
        .child(
            div().w(180.0).px(8.0).py(4.0).child(
                field(
                    link_id(),
                    "Link",
                    "Paste link...",
                    TextStyle::ui(12.0),
                    c(look::GRAY_200),
                )
                .w_full(),
            ),
        )
        .child(
            div()
                .id(apply)
                .on(Action::Click)
                .p(6.0)
                .rounded(4.0)
                .bg(white(0.0).mix(white(0.08), hot))
                .role(Role::Button, "Apply")
                .child(ico(
                    "link",
                    14.0,
                    2.0,
                    c(look::GRAY_400).mix(c(look::WHITE), hot),
                )),
        )
}

/// The form whose description `id` is, re-exported for the app.
pub fn form_of(id: crate::ui::Id) -> Option<Form> {
    desc_form(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_fences_hide_the_toolbar() {
        let t = "a\n```\ncode\n```\nb";
        assert!(!in_code(t, 1));
        assert!(in_code(t, 8));
        assert!(!in_code(t, t.len()));
    }
}
