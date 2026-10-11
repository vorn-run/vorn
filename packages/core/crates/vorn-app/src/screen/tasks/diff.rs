//! The detail panel's changes (`DiffFileList`, `DiffContent`): the files,
//! their lines with review comments, and the feedback bar.

use super::{comment_id, faint, ink, primary, secondary};
use crate::act::Act;
use crate::look;
use crate::screen::{c, ico, white, Screen, MOD_LABEL};
use crate::tasks::{DiffFile, TaskAct};
use crate::ui::{area, div, text, Action, El, Rgba, Role, TextStyle};

const ADD: u32 = 0x7ea96a;
const REMOVE: u32 = 0xc96f62;

/// One drawn line of a diff; `index` is its line in the raw diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Meta(usize, String),
    Hunk(usize, String),
    Add(usize, String, usize),
    Del(usize, String, usize),
    Ctx(usize, String, usize, usize),
}

/// `parseDiffRows`: header lines dropped, hunks numbered.
pub fn rows(diff: &str) -> Vec<Row> {
    let mut out = Vec::new();
    let (mut old, mut new, mut in_hunk) = (0usize, 0usize, false);
    for (i, line) in diff.split('\n').enumerate() {
        if ["diff --git", "index ", "--- ", "+++ "]
            .iter()
            .any(|p| line.starts_with(p))
        {
            continue;
        }
        if let Some((o, n)) = hunk(line) {
            (old, new, in_hunk) = (o, n, true);
            out.push(Row::Hunk(i, line.to_owned()));
            continue;
        }
        if !in_hunk {
            if !line.trim().is_empty() {
                out.push(Row::Meta(i, line.to_owned()));
            }
            continue;
        }
        if line.starts_with('+') {
            out.push(Row::Add(i, line.to_owned(), new));
            new += 1;
        } else if line.starts_with('-') {
            out.push(Row::Del(i, line.to_owned(), old));
            old += 1;
        } else if line.starts_with(' ') {
            out.push(Row::Ctx(i, line.to_owned(), old, new));
            old += 1;
            new += 1;
        }
    }
    out
}

/// The old and new start lines of an `@@ -a,b +c,d @@` header.
fn hunk(line: &str) -> Option<(usize, usize)> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(" +")?;
    let (new, _) = rest.split_once(" @@")?;
    let start = |s: &str| s.split(',').next()?.parse().ok();
    Some((start(old)?, start(new)?))
}

fn letter(status: &str) -> &'static str {
    match status {
        "added" => "A",
        "deleted" => "D",
        "renamed" => "R",
        _ => "M",
    }
}

/// `DiffFileList`.
pub(super) fn file_list(s: &Screen<'_>, files: &[DiffFile]) -> El {
    let selected = s.st.tasks.diff.file.as_deref();
    let mut col = div()
        .id(crate::ui::Id::new("task-diff-files"))
        .col()
        .max_h(200.0)
        .scroll_y()
        .border_top(1.0, white(0.0));
    for f in files {
        let id = s.acts.on(
            "task-diff-file",
            Act::Task(TaskAct::PickFile(Some(f.path.clone()))),
        );
        let on = selected == Some(f.path.as_str());
        let bg = if on {
            white(0.08)
        } else {
            white(0.0).mix(white(0.04), s.ui.hover(id))
        };
        let mut counts = div().row().gap(6.0).shrink0();
        if f.insertions > 0 {
            counts = counts.child(
                text(format!("+{}", f.insertions), 11.0, c(ADD))
                    .mono()
                    .line_height(16.0)
                    .silent(),
            );
        }
        if f.deletions > 0 {
            counts = counts.child(
                text(format!("-{}", f.deletions), 11.0, c(REMOVE))
                    .mono()
                    .line_height(16.0)
                    .silent(),
            );
        }
        col = col.child(
            div()
                .id(id)
                .on(Action::Click)
                .row()
                .items_center()
                .gap(8.0)
                .px(12.0)
                .py(6.0)
                .bg(bg)
                .role(Role::Button, &f.path)
                .selected(on)
                .child(ico("file-code", 15.0, 1.5, secondary()))
                .child(
                    div().grow().min_w0().child(
                        text(&f.path, 12.0, c(look::GRAY_300))
                            .mono()
                            .line_height(16.0)
                            .truncate()
                            .silent(),
                    ),
                )
                .child(counts)
                .child(
                    text(letter(&f.status), 10.0, secondary())
                        .weight(700)
                        .line_height(14.0)
                        .silent(),
                ),
        );
    }
    div()
        .col()
        .child(col)
        .child(div().h(1.0).w_full().bg(white(0.06)))
}

fn badge(s: &Screen<'_>, i: usize, comment: &str) -> El {
    let rm = s
        .acts
        .on("task-comment-remove", Act::Task(TaskAct::RemoveComment(i)));
    let hot = s.ui.hover(rm);
    div().mx(8.0).pt(2.0).pb(2.0).child(
        div()
            .row()
            .gap(8.0)
            .px(12.0)
            .py(6.0)
            .rounded(6.0)
            .bg(white(0.03))
            .border(1.0, white(0.08))
            .child(
                div()
                    .pt(2.0)
                    .child(ico("message-square", 11.0, 2.0, secondary())),
            )
            .child(
                div().grow().min_w0().child(
                    text(comment, 12.0, secondary())
                        .line_height(16.0)
                        .wrapping(),
                ),
            )
            .child(
                div()
                    .id(rm)
                    .on(Action::Click)
                    .p(2.0)
                    .role(Role::Button, "Remove comment")
                    .child(ico(
                        "x",
                        10.0,
                        2.0,
                        c(look::GRAY_600).mix(c(look::DANGER), hot),
                    )),
            ),
    )
}

/// `InlineCommentInput`.
fn comment_input(s: &Screen<'_>) -> El {
    let id = comment_id();
    let empty = s.ui.edit_text(id).trim().is_empty();
    let cancel = s
        .acts
        .on("task-comment-cancel", Act::Task(TaskAct::CancelComment));
    let add = s
        .acts
        .on("task-comment-add", Act::Task(TaskAct::AddComment));
    let focus = s.ui.focused(id);
    let mut add_btn = div()
        .id(add)
        .px(8.0)
        .py(4.0)
        .role(Role::Button, "Comment")
        .disabled(empty)
        .child(
            text(
                "Comment",
                10.0,
                primary().mix(c(look::WHITE), s.ui.hover(add)),
            )
            .weight(500)
            .line_height(14.0)
            .silent(),
        );
    if empty {
        add_btn = add_btn.opacity(0.3);
    } else {
        add_btn = add_btn.on(Action::Click);
    }
    div().mx(8.0).pt(4.0).pb(4.0).child(
        div()
            .col()
            .p(8.0)
            .rounded(6.0)
            .bg(white(0.04))
            .border(1.0, white(0.10))
            .child(
                div()
                    .px(8.0)
                    .py(6.0)
                    .h(48.0)
                    .rounded(4.0)
                    .bg(white(0.04))
                    .border(1.0, if focus { white(0.20) } else { white(0.08) })
                    .child(
                        area(
                            id,
                            "Review comment",
                            "Add review comment...",
                            TextStyle::mono(12.0),
                            c(look::GRAY_200),
                        )
                        .w_full()
                        .h_full(),
                    ),
            )
            .child(
                div()
                    .row()
                    .items_center()
                    .justify_between()
                    .pt(6.0)
                    .child(
                        text(
                            format!("{MOD_LABEL}Enter to submit"),
                            10.0,
                            c(look::GRAY_600),
                        )
                        .line_height(14.0)
                        .silent(),
                    )
                    .child(
                        div()
                            .row()
                            .gap(6.0)
                            .child(
                                div()
                                    .id(cancel)
                                    .on(Action::Click)
                                    .px(8.0)
                                    .py(4.0)
                                    .role(Role::Button, "Cancel")
                                    .child(
                                        text(
                                            "Cancel",
                                            10.0,
                                            c(look::GRAY_500)
                                                .mix(c(look::GRAY_300), s.ui.hover(cancel)),
                                        )
                                        .line_height(14.0)
                                        .silent(),
                                    ),
                            )
                            .child(add_btn),
                    ),
            ),
    )
}

fn num(n: Option<usize>, color: Rgba) -> El {
    div().w(35.0).pr(8.0).shrink0().row().justify_end().child(
        text(n.map_or(String::new(), |n| n.to_string()), 11.0, color)
            .mono()
            .line_height(19.0)
            .silent(),
    )
}

/// One file of `DiffContent`: its sticky header and lines.
fn file(s: &Screen<'_>, f: &DiffFile) -> El {
    let d = &s.st.tasks.diff;
    let count = d.comments.iter().filter(|c| c.file == f.path).count();
    let mut head = div()
        .row()
        .items_center()
        .gap(8.0)
        .px(12.0)
        .py(6.0)
        .bg(c(look::SURFACE_OVERLAY))
        .child(ico("file-code", 14.0, 1.5, secondary()))
        .child(
            div().grow().min_w0().child(
                text(&f.path, 12.0, c(look::GRAY_300))
                    .mono()
                    .line_height(16.0)
                    .truncate()
                    .silent(),
            ),
        )
        .child(
            text(letter(&f.status), 10.0, secondary())
                .weight(700)
                .line_height(14.0)
                .silent(),
        );
    if count > 0 {
        head = head.child(
            div().px(6.0).py(2.0).rounded(8.0).bg(white(0.06)).child(
                text(
                    format!("{count} comment{}", if count == 1 { "" } else { "s" }),
                    10.0,
                    secondary(),
                )
                .line_height(14.0)
                .silent(),
            ),
        );
    }
    let mut col = div()
        .col()
        .child(head)
        .child(div().h(1.0).w_full().bg(white(0.06)));
    let gray = c(look::GRAY_600);
    for row in rows(&f.diff) {
        let (index, line) = match row {
            Row::Hunk(_, t) => {
                col = col.child(
                    div()
                        .px(12.0)
                        .py(2.0)
                        .bg(white(0.05))
                        .child(text(t, 12.0, secondary()).mono().line_height(19.0).silent()),
                );
                continue;
            }
            Row::Meta(_, t) => {
                col = col.child(
                    div().px(12.0).child(
                        text(t, 12.0, c(look::GRAY_500))
                            .mono()
                            .line_height(19.0)
                            .silent(),
                    ),
                );
                continue;
            }
            Row::Ctx(_, t, o, n) => {
                col = col.child(
                    div()
                        .row()
                        .child(num(Some(o), gray))
                        .child(num(Some(n), gray))
                        .child(
                            div().px(4.0).grow().min_w0().child(
                                text(body(&t), 12.0, c(look::GRAY_400))
                                    .mono()
                                    .line_height(19.0)
                                    .silent(),
                            ),
                        ),
                );
                continue;
            }
            Row::Add(i, t, n) => {
                let id = s.acts.on(
                    "task-diff-line",
                    Act::Task(TaskAct::Line(f.path.clone(), i)),
                );
                let hot = s.ui.hover(id);
                (
                    i,
                    div()
                        .id(id)
                        .on(Action::Click)
                        .row()
                        .bg(Rgba::hexa(0x22c55e, 0.10 + 0.05 * hot))
                        .role(Role::Button, &t)
                        .child(num(None, gray))
                        .child(num(Some(n), c(0x16a34a)))
                        .child(
                            div().px(4.0).grow().min_w0().child(
                                text(body(&t), 12.0, c(0x86efac))
                                    .mono()
                                    .line_height(19.0)
                                    .silent(),
                            ),
                        )
                        .child(div().pr(8.0).center().opacity(hot).child(ico(
                            "message-square",
                            11.0,
                            2.0,
                            secondary(),
                        ))),
                )
            }
            Row::Del(i, t, o) => {
                let id = s.acts.on(
                    "task-diff-line",
                    Act::Task(TaskAct::Line(f.path.clone(), i)),
                );
                let hot = s.ui.hover(id);
                (
                    i,
                    div()
                        .id(id)
                        .on(Action::Click)
                        .row()
                        .bg(Rgba::hexa(0xef4444, 0.10 + 0.05 * hot))
                        .role(Role::Button, &t)
                        .child(num(Some(o), c(0xdc2626)))
                        .child(num(None, gray))
                        .child(
                            div().px(4.0).grow().min_w0().child(
                                text(body(&t), 12.0, c(0xfca5a5))
                                    .mono()
                                    .line_height(19.0)
                                    .silent(),
                            ),
                        )
                        .child(div().pr(8.0).center().opacity(hot).child(ico(
                            "message-square",
                            11.0,
                            2.0,
                            secondary(),
                        ))),
                )
            }
        };
        col = col.child(line);
        for (ci, cm) in d.comments.iter().enumerate() {
            if cm.file == f.path && cm.line == index {
                col = col.child(badge(s, ci, &cm.text));
            }
        }
        if d.commenting
            .as_ref()
            .is_some_and(|(p, l, _)| p == &f.path && *l == index)
        {
            col = col.child(comment_input(s));
        }
    }
    col
}

fn body(line: &str) -> String {
    let b: String = line.chars().skip(1).collect();
    if b.is_empty() {
        " ".to_owned()
    } else {
        b
    }
}

/// `DiffContent`: every file, or the selected one first.
pub(super) fn content(s: &Screen<'_>, files: &[DiffFile]) -> El {
    let mut col = div().col();
    let first = s.st.tasks.diff.file.as_deref();
    let ordered = first
        .and_then(|p| files.iter().find(|f| f.path == p))
        .into_iter()
        .chain(files.iter().filter(|f| Some(f.path.as_str()) != first));
    for f in ordered {
        col = col.child(file(s, f));
    }
    col
}

/// The review feedback bar: the count, Clear and Send.
pub(super) fn feedback_bar(s: &Screen<'_>, to_agent: bool) -> El {
    let n = s.st.tasks.diff.comments.len();
    let clear = s
        .acts
        .on("task-comments-clear", Act::Task(TaskAct::ClearComments));
    let send = s
        .acts
        .on("task-comments-send", Act::Task(TaskAct::SendFeedback));
    let send_hot = s.ui.hover(send);
    let label = if to_agent {
        "Send to Agent"
    } else {
        "Copy Feedback"
    };
    div()
        .col()
        .child(div().h(1.0).w_full().bg(white(0.08)))
        .child(
            div()
                .row()
                .items_center()
                .gap(8.0)
                .px(12.0)
                .py(8.0)
                .bg(white(0.02))
                .child(ico("message-square", 13.0, 2.0, secondary()))
                .child(
                    div().grow().child(
                        text(
                            format!("{n} comment{}", if n == 1 { "" } else { "s" }),
                            12.0,
                            secondary(),
                        )
                        .line_height(16.0)
                        .silent(),
                    ),
                )
                .child(
                    div()
                        .id(clear)
                        .on(Action::Click)
                        .role(Role::Button, "Clear")
                        .child(
                            text("Clear", 11.0, faint().mix(secondary(), s.ui.hover(clear)))
                                .line_height(16.0)
                                .silent(),
                        ),
                )
                .child(
                    div()
                        .id(send)
                        .on(Action::Click)
                        .row()
                        .items_center()
                        .gap(6.0)
                        .px(10.0)
                        .py(4.0)
                        .rounded(6.0)
                        .bg(ink(1.0).mix(c(look::WHITE), send_hot))
                        .role(Role::Button, label)
                        .child(ico("send", 11.0, 2.0, c(look::SURFACE_BASE)))
                        .child(
                            text(label, 11.0, c(look::SURFACE_BASE))
                                .weight(500)
                                .line_height(16.0)
                                .silent(),
                        ),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_drop_headers_and_number_hunks() {
        let diff = "diff --git a/x b/x\nindex 1..2\n--- a/x\n+++ b/x\n@@ -3,2 +3,3 @@ fn\n ctx\n-old\n+new\n+more";
        let r = rows(diff);
        assert_eq!(r[0], Row::Hunk(4, "@@ -3,2 +3,3 @@ fn".to_owned()));
        assert_eq!(r[1], Row::Ctx(5, " ctx".to_owned(), 3, 3));
        assert_eq!(r[2], Row::Del(6, "-old".to_owned(), 4));
        assert_eq!(r[3], Row::Add(7, "+new".to_owned(), 4));
        assert_eq!(r[4], Row::Add(8, "+more".to_owned(), 5));
        assert_eq!(
            rows("Binary files differ"),
            vec![Row::Meta(0, "Binary files differ".to_owned())]
        );
    }
}
