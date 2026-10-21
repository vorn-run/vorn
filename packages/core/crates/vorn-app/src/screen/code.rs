//! A tree's `code` drawn: the strip with the file's path and its tools, the
//! find bar, the moved-file banner and the notices, and the gutter beside
//! the coloured text being edited.

use super::{c, ico, white, Screen};
use crate::act::Act;
use crate::code::{self, CodeAct, View, LINE_HEIGHT};
use crate::look;
use crate::ui::{
    code as code_input, div, field, text, tooltip, Action, El, Id, Rgba, Role, TextStyle,
};
use vorn_tree::{Code, CodeState};

pub fn editor_id(inst: &str) -> Id {
    Id::new("editor").child(inst)
}

pub fn find_id(inst: &str) -> Id {
    Id::new("editor-find").child(inst)
}

pub fn scroller_id(inst: &str) -> Id {
    Id::new("editor-scroll").child(inst)
}

fn pane_bg() -> Rgba {
    c(look::SURFACE_PANEL)
}

fn act(a: CodeAct) -> Act {
    Act::Code(a)
}

/// A file's Seti glyph in its colour.
pub(crate) fn file_icon(name: &str, size: f32) -> El {
    let (ch, color) = crate::file_icons::resolve(name);
    match crate::file_icons::svg(ch) {
        Some(svg) => crate::ui::icon(svg, size, c(color)),
        None => div().size(size, size),
    }
}

/// `ToolbarBtn`.
fn toolbar_btn(s: &Screen<'_>, a: Act, glyph: &str, label: &str, on: bool, off: bool) -> El {
    let id = s.acts.on("editor-tool", a);
    let mut b = div()
        .id(id)
        .p(4.0)
        .rounded(4.0)
        .shrink0()
        .bg(white(0.10 * if on { 1.0 } else { s.ui.hover(id) }))
        .role(Role::Button, label)
        .disabled(off)
        .opacity(if off { 0.4 } else { 1.0 })
        .child(ico(glyph, 14.0, 2.0, c(look::INK)));
    if !off {
        b = b.on(Action::Click);
    }
    tooltip(s.ui, b, id, label, None)
}

/// The path cut from the left, as `dir="rtl"` truncates it.
fn tail(path: &str, max: usize) -> String {
    let n = path.chars().count();
    if n <= max {
        return path.to_owned();
    }
    let rest: String = path.chars().skip(n - max + 1).collect();
    format!("\u{2026}{rest}")
}

/// Code `inst`, drawn from `node`: the path strip ending in `extra`, the
/// find bar, the notices and the body.
pub(crate) fn panel(s: &Screen<'_>, inst: &str, node: &Code, extra: Option<El>) -> El {
    let col = div().col().min_w0().min_h(0.0).bg(pane_bg());
    let view = s.st.codes.views.get(inst).cloned().unwrap_or_default();
    let name = crate::state::basename(&node.path);
    let rel = code::rel_path(node.root.as_deref().unwrap_or(""), &node.path);
    let taken = view.rev == Some(node.rev);
    let editing = node.state == CodeState::Edit && taken;
    let dirty = editing && view.text != node.text;
    let can_find = matches!(node.state, CodeState::Edit | CodeState::Read);
    let mut strip = div()
        .row()
        .items_center()
        .gap(4.0)
        .pl(8.0)
        .pr(4.0)
        .py(2.0)
        .shrink0()
        .bg(pane_bg())
        .child(file_icon(name, 12.0))
        .child(
            div().row().justify_end().grow().min_w0().ml(4.0).child(
                text(tail(rel, 48), 11.0, c(look::GRAY_400))
                    .mono()
                    .line_height(16.0)
                    .truncate()
                    .role(Role::Label, &node.path),
            ),
        );
    if dirty {
        strip = strip.child(
            div()
                .mx(4.0)
                .size(6.0, 6.0)
                .rounded(3.0)
                .bg(c(look::AMBER_400))
                .role(Role::Image, "Unsaved changes"),
        );
    }
    strip = strip.child(toolbar_btn(
        s,
        act(CodeAct::ToggleFind(inst.to_owned())),
        "search",
        "Find in file",
        view.find.is_some(),
        !can_find,
    ));
    if dirty {
        strip = strip
            .child(toolbar_btn(
                s,
                act(CodeAct::Save(inst.to_owned())),
                "save",
                if node.saving {
                    "Saving\u{2026}"
                } else {
                    "Save"
                },
                false,
                node.saving,
            ))
            .child(toolbar_btn(
                s,
                act(CodeAct::Tell(inst.to_owned(), "discard")),
                "undo-2",
                "Discard changes",
                false,
                false,
            ));
    }
    let mut col = col.child(strip.child_if(extra));
    if let (Some(f), true) = (&view.find, can_find) {
        let source = if editing {
            view.text.as_str()
        } else {
            node.text.as_str()
        };
        col = col.child(find_bar(s, inst, code::matches(source, &f.query).len(), f));
    }
    if node.conflict {
        col = col.child(conflict(s, inst));
    }
    if node.state == CodeState::Read {
        col = col.child(
            div()
                .px(12.0)
                .py(4.0)
                .border_top(1.0, white(0.06))
                .shrink0()
                .child(
                    text(
                        "Too large to edit here. Showing the first part, read-only.",
                        11.0,
                        c(look::INK).alpha(look::INK_FAINT),
                    )
                    .line_height(16.0)
                    .silent(),
                ),
        );
    }
    let body = match node.state {
        CodeState::Loading => {
            div()
                .col()
                .center()
                .grow()
                .child(ico("loader-circle", 16.0, 2.0, c(look::GRAY_500)))
        }
        CodeState::Binary | CodeState::Unreadable => {
            let msg = if node.state == CodeState::Binary {
                "Binary file \u{2014} preview unavailable"
            } else {
                "This file could not be read"
            };
            div().col().center().grow().child(
                text(msg, 13.0, c(look::GRAY_600))
                    .line_height(18.0)
                    .silent(),
            )
        }
        CodeState::Edit if taken => editor_view(inst, node, &view),
        _ => read_view(inst, &node.text),
    };
    col = col.child(body);
    if let Some(e) = &node.error {
        col = col.child(
            div()
                .px(12.0)
                .py(4.0)
                .bg(c(look::DANGER).alpha(0.1))
                .border_top(1.0, white(0.06))
                .shrink0()
                .child(text(e, 11.0, c(look::DANGER)).line_height(16.0).wrapping()),
        );
    }
    col
}

fn find_bar(s: &Screen<'_>, inst: &str, n: usize, f: &code::Find) -> El {
    let count = if n == 0 {
        "0/0".to_owned()
    } else {
        format!("{}/{n}", f.index % n + 1)
    };
    let arrow = |place: &str, d: i32, glyph: &str, label: &str| {
        let id = s.acts.on(place, act(CodeAct::FindStep(inst.to_owned(), d)));
        let off = n == 0;
        let mut el = div()
            .id(id)
            .px(4.0)
            .role(Role::Button, label)
            .disabled(off)
            .opacity(if off { 0.4 } else { 1.0 })
            .child(
                text(
                    glyph,
                    11.0,
                    c(look::GRAY_500).mix(c(look::WHITE), s.ui.hover(id)),
                )
                .line_height(16.0)
                .silent(),
            );
        if !off {
            el = el.on(Action::Click);
        }
        tooltip(s.ui, el, id, label, None)
    };
    let close = s
        .acts
        .on("find-close", act(CodeAct::CloseFind(inst.to_owned())));
    div()
        .row()
        .items_center()
        .gap(8.0)
        .px(12.0)
        .py(4.0)
        .shrink0()
        .bg(pane_bg())
        .child(ico("search", 12.0, 2.0, c(look::GRAY_500)))
        .child(
            div().grow().min_w0().frames(find_id(inst)).child(
                field(
                    find_id(inst),
                    "Find in file",
                    "Find in file",
                    TextStyle::mono(13.0),
                    c(look::GRAY_200),
                )
                .w_full(),
            ),
        )
        .child(
            text(count, 11.0, c(look::GRAY_500))
                .line_height(16.0)
                .silent(),
        )
        .child(arrow("find-prev", -1, "\u{2191}", "Previous (Shift+Enter)"))
        .child(arrow("find-next", 1, "\u{2193}", "Next (Enter)"))
        .child(tooltip(
            s.ui,
            div()
                .id(close)
                .on(Action::Click)
                .p(2.0)
                .role(Role::Button, "Close (Esc)")
                .child(ico(
                    "x",
                    12.0,
                    2.0,
                    c(look::GRAY_500).mix(c(look::WHITE), s.ui.hover(close)),
                )),
            close,
            "Close (Esc)",
            None,
        ))
}

/// The moved-file banner's three ways out.
fn conflict(s: &Screen<'_>, inst: &str) -> El {
    let link = |place: &str, label: &str, what: &'static str| {
        let id = s.acts.on(place, act(CodeAct::Tell(inst.to_owned(), what)));
        div()
            .id(id)
            .on(Action::Click)
            .role(Role::Button, label)
            .child(
                text(
                    label,
                    11.0,
                    c(look::GRAY_300).mix(c(look::GRAY_100), s.ui.hover(id)),
                )
                .line_height(16.0)
                .silent(),
            )
            .child(div().h(1.0).w_full().bg(c(look::GRAY_300).alpha(0.6)))
    };
    div()
        .row()
        .items_center()
        .gap(12.0)
        .px(12.0)
        .py(6.0)
        .bg(c(look::BRONZO).alpha(0.1))
        .border_top(1.0, white(0.06))
        .shrink0()
        .child(
            div().grow().min_w0().child(
                text(
                    "This file changed on disk while your edit was open.",
                    11.0,
                    c(look::BRONZO),
                )
                .line_height(16.0)
                .wrapping(),
            ),
        )
        .child(link("conflict-save", "Save mine anyway", "force"))
        .child(link("conflict-disk", "Discard mine", "disk"))
        .child(link("conflict-keep", "Keep editing", "keep"))
}

/// Right-aligned line numbers in a mono column, one per line.
fn gutter(lines: usize, from_top: f32) -> El {
    let w = lines.max(1).to_string().len();
    let numbers: Vec<String> = (1..=lines.max(1)).map(|n| format!("{n:>w$}")).collect();
    div()
        .pl(8.0)
        .pr(12.0)
        .pt(from_top)
        .shrink0()
        .bg(pane_bg())
        .child(
            text(numbers.join("\n"), 12.0, c(look::GRAY_600))
                .mono()
                .line_height(LINE_HEIGHT)
                .silent(),
        )
}

/// A span list with `range` laid over it in `color`, still sorted and
/// apart (the active find match's text).
fn overlay(
    spans: &[(std::ops::Range<usize>, Rgba)],
    range: std::ops::Range<usize>,
    color: Rgba,
) -> Vec<(std::ops::Range<usize>, Rgba)> {
    let mut out = Vec::with_capacity(spans.len() + 2);
    for (r, col) in spans {
        if r.end <= range.start || r.start >= range.end {
            out.push((r.clone(), *col));
            continue;
        }
        if r.start < range.start {
            out.push((r.start..range.start, *col));
        }
        if r.end > range.end {
            out.push((range.end..r.end, *col));
        }
    }
    out.push((range, color));
    out.sort_by_key(|(r, _)| r.start);
    out
}

/// The gutter beside the coloured, editable text, with the find marks.
fn editor_view(inst: &str, node: &Code, view: &View) -> El {
    let text_ = view.text.as_str();
    let lines = text_.split('\n').count();
    let mut spans: Vec<(std::ops::Range<usize>, Rgba)> = view
        .coloured
        .as_ref()
        .filter(|(t, _)| t == text_)
        .map(|(_, sp)| {
            sp.iter()
                .map(|(r, (token, a))| (r.clone(), c(*token).alpha(f32::from(*a) / 255.0)))
                .collect()
        })
        .unwrap_or_default();
    let mut marks = Vec::new();
    if let Some(f) = &view.find {
        let m = code::matches(text_, &f.query);
        let active = (!m.is_empty()).then(|| f.index % m.len());
        for (i, hit) in m.iter().enumerate() {
            let on = Some(i) == active;
            marks.push((
                hit.range.0..hit.range.1,
                c(look::AMBER_300).alpha(if on { 0.7 } else { 0.25 }),
            ));
            spans = overlay(
                &spans,
                hit.range.0..hit.range.1,
                if on {
                    crate::ui::theme::shadow(1.0)
                } else {
                    c(look::GRAY_100)
                },
            );
        }
    }
    let style = TextStyle {
        line_height: LINE_HEIGHT,
        ..TextStyle::mono(13.0)
    };
    let field = code_input(
        editor_id(inst),
        &format!("Edit {}", crate::state::basename(&node.path)),
        style,
        c(look::GRAY_300),
    )
    .edit_colors(spans, marks)
    .w_full()
    .h(lines as f32 * LINE_HEIGHT);
    div()
        .id(scroller_id(inst))
        .col()
        .grow()
        .min_h(0.0)
        .scroll_y()
        .child(
            div()
                .row()
                .child(gutter(lines, 4.0))
                .child(div().grow().min_w0().pt(4.0).pb(4.0).pr(12.0).child(field)),
        )
}

/// A file shown but not editable, capped at 2000 lines.
fn read_view(inst: &str, t: &str) -> El {
    let all: Vec<&str> = t.split('\n').collect();
    let capped = all.len() > code::MAX_HIGHLIGHT_LINES;
    let shown = &all[..all.len().min(code::MAX_HIGHLIGHT_LINES)];
    let mut col = div()
        .id(scroller_id(inst))
        .col()
        .grow()
        .min_h(0.0)
        .scroll_y()
        .child(
            div().row().child(gutter(shown.len(), 0.0)).child(
                div().grow().min_w0().px(4.0).child(
                    text(shown.join("\n"), 13.0, c(look::GRAY_400))
                        .mono()
                        .line_height(LINE_HEIGHT)
                        .silent(),
                ),
            ),
        );
    if capped {
        col = col.child(
            div().px(12.0).py(8.0).child(
                text(
                    format!(
                        "Showing first {} of {} lines",
                        code::MAX_HIGHLIGHT_LINES,
                        all.len()
                    ),
                    11.0,
                    c(look::GRAY_600),
                )
                .italic()
                .line_height(16.0)
                .silent(),
            ),
        );
    }
    col
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_active_match_is_laid_over_the_colours() {
        let a = Rgba::hex(0x111111);
        let b = Rgba::hex(0x222222);
        let m = Rgba::hex(0);
        let out = overlay(&[(0..4, a), (4..8, b)], 2..6, m);
        let ranges: Vec<_> = out.iter().map(|(r, _)| r.clone()).collect();
        assert_eq!(ranges, vec![0..2, 2..6, 6..8]);
    }

    #[test]
    fn long_paths_keep_their_ends() {
        assert_eq!(tail("abc", 5), "abc");
        assert_eq!(tail("abcdefgh", 5), "\u{2026}efgh");
    }
}
