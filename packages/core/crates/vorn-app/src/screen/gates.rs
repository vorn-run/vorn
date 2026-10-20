//! A waiting approval gate: what it asks, and
//! the answers a reviewer reaches for, in the run pane, under the step in a
//! trace, and on the review page.

use serde_json::Value;

use super::{c, ico, white, Screen};
use crate::act::Act;
use crate::gates::{self, cell_id, edit_id, text_id, Editor, GateOp, Mode, Place};
use crate::look;
use crate::runs::{self, s as sv};
use crate::ui::{div, field, text, textarea, Action, El, Id, Rgba, Role, TextStyle};

fn ink(a: f32) -> Rgba {
    look::INK.alpha(a)
}

fn secondary() -> Rgba {
    ink(look::INK_SECONDARY)
}

fn faint() -> Rgba {
    ink(look::INK_FAINT)
}

fn bronzo(a: f32) -> Rgba {
    look::BRONZO.alpha(a)
}

fn danger(a: f32) -> Rgba {
    look::DANGER.alpha(a)
}

/// `GATE_APPROVE`, `GATE_NEUTRAL` and `GATE_REJECT`.
#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Approve,
    Neutral,
    Reject,
}

/// A gate button: small (`px-2 py-1 text-[11px]`) or large
/// (`px-3.5 py-2 text-[12.5px]`), in its kind's colours.
#[allow(clippy::too_many_arguments)]
pub fn button(
    s: &Screen<'_>,
    place: &str,
    act: Act,
    icon: &str,
    stroke: f32,
    label: &str,
    kind: Kind,
    large: bool,
    disabled: bool,
) -> El {
    let id = s.acts.on(place, act);
    let hot = if disabled { 0.0 } else { s.ui.hover(id) };
    let (fg, bg, border) = match kind {
        Kind::Approve => (
            bronzo(1.0),
            bronzo(0.10).mix(bronzo(0.20), hot),
            bronzo(0.4),
        ),
        Kind::Neutral => (
            secondary().mix(ink(1.0), hot),
            white(0.0).mix(white(0.04), hot),
            white(0.06),
        ),
        Kind::Reject => (
            secondary().mix(danger(1.0), hot),
            white(0.0).mix(danger(0.10), hot),
            white(0.06).mix(danger(0.30), hot),
        ),
    };
    let (size, line, isz) = if large {
        (12.5, 18.75, 13.0)
    } else {
        (11.0, 16.5, 11.0)
    };
    let mut b = div()
        .id(id)
        .row()
        .items_center()
        .gap(4.0)
        .px(if large { 14.0 } else { 8.0 })
        .py(if large { 8.0 } else { 4.0 })
        .rounded(6.0)
        .border(1.0, border)
        .bg(bg)
        .opacity(if disabled { 0.4 } else { 1.0 })
        .role(Role::Button, label)
        .disabled(disabled)
        .child(ico(icon, isz, stroke, fg))
        .child(text(label, size, fg).line_height(line).silent());
    if !disabled {
        b = b.on(Action::Click);
    }
    b
}

/// A plain text button: Cancel, Revert.
fn quiet(s: &Screen<'_>, place: &str, act: Act, label: &str, large: bool, disabled: bool) -> El {
    let id = s.acts.on(place, act);
    let hot = if disabled { 0.0 } else { s.ui.hover(id) };
    let (size, line) = if large { (12.5, 18.75) } else { (11.0, 16.5) };
    let mut b = div()
        .id(id)
        .px(if large { 14.0 } else { 8.0 })
        .py(if large { 8.0 } else { 4.0 })
        .opacity(if disabled { 0.4 } else { 1.0 })
        .role(Role::Button, label)
        .disabled(disabled)
        .child(
            text(label, size, faint().mix(secondary(), hot))
                .line_height(line)
                .silent(),
        );
    if !disabled {
        b = b.on(Action::Click);
    }
    b
}

/// The message as it was filled in, or that it waits, and the
/// last change asked for.
pub fn ask(state: &Value, config: Option<&Value>) -> El {
    let message = sv(state, "message")
        .or_else(|| config.and_then(|c| sv(c, "message")))
        .filter(|m| !m.is_empty());
    let mut col = div().col().gap(6.0);
    col = col.child(match message {
        Some(m) => div()
            .max_h(192.0)
            .clip()
            .rounded(6.0)
            .border(1.0, white(0.07))
            .bg(c(look::SURFACE_BASE))
            .px(10.0)
            .py(8.0)
            .child(
                text(m, 12.0, ink(1.0))
                    .wrapping()
                    .min_w0()
                    .line_height(18.0),
            ),
        None => text("Waiting for approval.", 11.5, c(look::BRONZO)).line_height(17.0),
    });
    if let Some(a) = gates::last_asked(state) {
        col = col.child(
            div()
                .row()
                .gap(6.0)
                .min_w0()
                .child(text("You asked", 11.5, faint()).line_height(16.7).silent())
                .child(
                    div()
                        .grow()
                        .min_w0()
                        .child(text(a, 11.5, secondary()).line_height(16.7).truncate()),
                ),
        );
    }
    col
}

/// What a trace or the review page shows at a waiting gate: the row of
/// answers, or the comment, note or editor one of them opened.
pub fn actions(
    s: &Screen<'_>,
    place: Place,
    run: &str,
    state: &Value,
    config: Option<&Value>,
    nodes: &Value,
) -> El {
    let nid = sv(state, "nodeId").unwrap_or("");
    let key = gates::key(place, run, nid);
    let g = s.st.runs.gates.get(&key);
    match g.and_then(|g| g.mode) {
        Some(Mode::Edit) => {
            if let Some(e) = g.and_then(|g| g.editor.as_ref()) {
                return editor(s, &key, e, false, true);
            }
        }
        Some(mode) => return composer(s, &key, mode, state, config, nodes, false),
        None => {}
    }
    let op = |o: GateOp| Act::Gate(o);
    let edited = g.is_some_and(|g| g.edited.is_some());
    let mut row = div().row().items_center().gap(6.0).wrap_children();
    if gates::editable(state).is_some() {
        row = row.child(button(
            s,
            "gate",
            op(GateOp::Open(key.clone(), Mode::Edit)),
            "pencil",
            1.75,
            if edited { "Edited" } else { "Edit" },
            Kind::Neutral,
            false,
            false,
        ));
    }
    if sv(state, "viewToken").is_some() && place != Place::Review {
        row = row.child(button(
            s,
            "gate",
            op(GateOp::Review(Some((run.to_owned(), nid.to_owned())))),
            "eye",
            1.75,
            "Open review",
            Kind::Neutral,
            false,
            false,
        ));
    }
    row = row.child(div().grow());
    if gates::can_request_changes(config, state) {
        row = row.child(button(
            s,
            "gate",
            op(GateOp::Open(key.clone(), Mode::Changes)),
            "message-square",
            1.75,
            "Request changes",
            Kind::Neutral,
            false,
            false,
        ));
    }
    let sending = g.is_some_and(|g| g.sending);
    row.child(button(
        s,
        "gate",
        op(GateOp::Open(key.clone(), Mode::Reject)),
        "x",
        2.5,
        "Reject",
        Kind::Reject,
        false,
        false,
    ))
    .child(button(
        s,
        "gate",
        op(GateOp::Approve(key)),
        "check",
        2.5,
        "Approve",
        Kind::Approve,
        false,
        sending,
    ))
}

/// The comment a request for changes needs, or the note a
/// rejection may carry.
pub fn composer(
    s: &Screen<'_>,
    key: &str,
    mode: Mode,
    state: &Value,
    config: Option<&Value>,
    nodes: &Value,
    large: bool,
) -> El {
    let changes = mode == Mode::Changes;
    let id = text_id(key);
    let typed = !s.ui.edit_text(id).trim().is_empty();
    let g = s.st.runs.gates.get(key);
    let edited = g.is_some_and(|g| g.edited.is_some());
    let sending = g.is_some_and(|g| g.sending);
    let style = TextStyle {
        line_height: 18.75,
        ..TextStyle::ui(12.5)
    };
    let field_el = textarea(
        id,
        if changes {
            "What should change"
        } else {
            "Why the run is rejected"
        },
        if changes {
            "What should change?"
        } else {
            "Why, if it should be kept with the run (optional)"
        },
        style,
        c(look::GRAY_200),
    )
    .h(3.0 * 18.75)
    .w_full()
    .min_w0();
    let boxed = div()
        .px(10.0)
        .py(8.0)
        .rounded(6.0)
        .bg(white(0.03))
        .border(1.0, white(0.2))
        .frames(id)
        .child(field_el);
    let from = config
        .and_then(|c| c.pointer("/feedback/from"))
        .and_then(Value::as_str)
        .and_then(|f| runs::node(nodes, f))
        .and_then(|n| sv(n, "label"))
        .filter(|l| !l.is_empty())
        .unwrap_or("the chosen step");
    let mut hint = div().row().wrap_children().min_w0();
    let small = |t: String, col: Rgba| text(t, 11.0, col).line_height(16.0).silent();
    if changes {
        let round = state.get("round").and_then(Value::as_u64).unwrap_or(1);
        hint = hint
            .child(small("Runs again from ".into(), faint()))
            .child(small(from.to_owned(), secondary()))
            .child(small(
                format!(
                    " with your comment, then asks you. Round {} of {}.{}",
                    round + 1,
                    gates::max_rounds(config),
                    if edited {
                        " Your edit goes back with it."
                    } else {
                        ""
                    }
                ),
                faint(),
            ));
    } else {
        hint = hint.child(small(
            format!(
                "Ends the run. A note is kept as its reason.{}",
                if edited {
                    " Your edit is discarded."
                } else {
                    ""
                }
            ),
            faint(),
        ));
    }
    let ready = !sending && (!changes || typed);
    let buttons = div()
        .row()
        .items_center()
        .justify_end()
        .gap(6.0)
        .child(quiet(
            s,
            "gate",
            Act::Gate(GateOp::Cancel(key.to_owned())),
            "Cancel",
            large,
            false,
        ))
        .child(if changes {
            button(
                s,
                "gate",
                Act::Gate(GateOp::Send(key.to_owned())),
                "rotate-ccw",
                2.0,
                "Send back",
                Kind::Approve,
                large,
                !ready,
            )
        } else {
            button(
                s,
                "gate",
                Act::Gate(GateOp::Send(key.to_owned())),
                "x",
                2.5,
                "Reject run",
                Kind::Reject,
                large,
                !ready,
            )
        });
    div().col().gap(8.0).child(boxed).child(hint).child(buttons)
}

/// Prose in a text area, or a list of records as a table.
pub fn editor(s: &Screen<'_>, key: &str, e: &Editor, large: bool, approve: bool) -> El {
    if e.is_json() {
        return json_editor(s, key, e, large, approve);
    }
    let style = TextStyle {
        line_height: 18.75,
        ..TextStyle::ui(12.5)
    };
    let area = div()
        .px(10.0)
        .py(8.0)
        .rounded(6.0)
        .bg(white(0.03))
        .frames(edit_id(key))
        .border(1.0, white(0.2))
        .child(
            textarea(
                edit_id(key),
                "The text to approve",
                "",
                style,
                c(look::GRAY_200),
            )
            .h(8.0 * 18.75)
            .w_full()
            .min_w0(),
        );
    let words = e.words();
    let changed = e.text != e.original;
    let info = div()
        .row()
        .items_center()
        .gap(8.0)
        .child(
            text(
                format!("{words} {}", if words == 1 { "word" } else { "words" }),
                11.0,
                faint(),
            )
            .line_height(16.0)
            .silent(),
        )
        .child_if(changed.then(|| text("Edited", 11.0, secondary()).line_height(16.0).silent()))
        .child(div().grow())
        .child(revert(s, key, !changed));
    div()
        .col()
        .gap(8.0)
        .child(area)
        .child(info)
        .child(edit_buttons(s, key, large, approve, None))
}

fn revert(s: &Screen<'_>, key: &str, disabled: bool) -> El {
    let id = s.acts.on("gate", Act::Gate(GateOp::Revert(key.to_owned())));
    let hot = if disabled { 0.0 } else { s.ui.hover(id) };
    let mut b = div()
        .id(id)
        .opacity(if disabled { 0.4 } else { 1.0 })
        .role(Role::Button, "Revert")
        .disabled(disabled)
        .child(
            text("Revert", 11.0, faint().mix(secondary(), hot))
                .line_height(16.0)
                .silent(),
        );
    if !disabled {
        b = b.on(Action::Click);
    }
    b
}

fn edit_buttons(
    s: &Screen<'_>,
    key: &str,
    large: bool,
    approve: bool,
    invalid: Option<&str>,
) -> El {
    let blocked = invalid.is_some();
    let mut row = div()
        .row()
        .items_center()
        .justify_end()
        .gap(6.0)
        .child(quiet(
            s,
            "gate",
            Act::Gate(GateOp::Cancel(key.to_owned())),
            "Cancel",
            large,
            false,
        ))
        .child(button(
            s,
            "gate",
            Act::Gate(GateOp::Save(key.to_owned())),
            "pencil",
            1.75,
            "Save",
            Kind::Neutral,
            large,
            blocked,
        ));
    if approve {
        row = row.child(button(
            s,
            "gate",
            Act::Gate(GateOp::Approve(key.to_owned())),
            "check",
            2.5,
            "Approve",
            Kind::Approve,
            large,
            blocked,
        ));
    }
    row
}

/// The list as a table to trim and correct, its source
/// one toggle away.
fn json_editor(s: &Screen<'_>, key: &str, e: &Editor, large: bool, approve: bool) -> El {
    let invalid = e.invalid();
    let edited = e.edited();
    let tab = |label: &str, on: bool, act: Act, disabled: bool| {
        let id = s.acts.on("gate-tab", act);
        let hot = if disabled { 0.0 } else { s.ui.hover(id) };
        let mut b = div()
            .id(id)
            .px(8.0)
            .py(2.0)
            .rounded(4.0)
            .bg(if on { white(0.08) } else { white(0.0) })
            .opacity(if disabled { 0.4 } else { 1.0 })
            .role(Role::Button, label)
            .toggled(on)
            .child(
                text(
                    label,
                    11.0,
                    if on {
                        ink(1.0)
                    } else {
                        faint().mix(secondary(), hot)
                    },
                )
                .line_height(16.5)
                .silent(),
            );
        if !disabled && !on {
            b = b.on(Action::Click);
        }
        b
    };
    let source_bad = e.source && invalid.is_some();
    let head = div()
        .row()
        .items_center()
        .gap(8.0)
        .px(12.0)
        .py(6.0)
        .border_bottom(1.0, white(0.06))
        .child(text("Items", 12.0, ink(1.0)).line_height(18.0).silent())
        .child(
            text(e.count(), 11.0, faint())
                .mono()
                .line_height(16.5)
                .silent(),
        )
        .child(div().grow())
        .child(
            div()
                .row()
                .items_center()
                .gap(2.0)
                .p(2.0)
                .rounded(6.0)
                .border(1.0, white(0.08))
                .role(Role::Group, "View")
                .child(tab(
                    "Table",
                    !e.source,
                    Act::Gate(GateOp::Source(key.to_owned(), false)),
                    source_bad,
                ))
                .child(tab(
                    "Source",
                    e.source,
                    Act::Gate(GateOp::Source(key.to_owned(), true)),
                    false,
                )),
        );
    let height = if large { 420.0 } else { 280.0 };
    let mut panel = div()
        .col()
        .rounded(6.0)
        .border(1.0, white(0.08))
        .bg(c(look::SURFACE_PANEL))
        .clip()
        .child(head);
    if e.source {
        let parse = e.source_parse();
        let status = match &parse {
            Some(Err(err)) => text(err.error.clone(), 11.0, danger(1.0)),
            Some(Ok(t)) => text(
                format!("Valid JSON \u{b7} {} items", t.rows.len()),
                11.0,
                c(crate::tasks::SAGE),
            ),
            None => text("", 11.0, faint()),
        };
        let text_now = s.ui.edit_text(edit_id(key));
        let lines = text_now.split('\n').count().max(1);
        let line = 21.0;
        let h = (lines as f32 * line).clamp(120.0 - 16.0, height - 16.0);
        let mut gutter = div().col().items_end().w(28.0).pr(8.0).shrink0();
        let bad_line = match &parse {
            Some(Err(e)) => e.line,
            _ => None,
        };
        for n in 1..=lines.min((h / line) as usize + 1) {
            let col = if bad_line == Some(n) {
                danger(1.0)
            } else {
                faint()
            };
            gutter = gutter.child(
                text(n.to_string(), 12.0, col)
                    .mono()
                    .line_height(line)
                    .silent(),
            );
        }
        panel = panel
            .child(
                div().row().py(8.0).h(h + 16.0).clip().child(gutter).child(
                    textarea(
                        edit_id(key),
                        "The text to approve",
                        "",
                        TextStyle {
                            line_height: line,
                            ..TextStyle::mono(12.0)
                        },
                        c(look::GRAY_200),
                    )
                    .h(h)
                    .grow()
                    .min_w0(),
                ),
            )
            .child(
                div()
                    .px(12.0)
                    .py(6.0)
                    .border_top(1.0, white(0.06))
                    .role(Role::Status, "")
                    .child(status.mono().line_height(16.5)),
            );
    } else if let Some(t) = &e.table {
        panel = panel.child(table(s, key, t, height));
    }
    let foot = div()
        .row()
        .items_center()
        .gap(8.0)
        .child_if(
            edited
                .is_some()
                .then(|| text("Edited", 11.0, secondary()).line_height(16.0).silent()),
        )
        .child_if(
            (!e.source)
                .then_some(invalid.as_deref())
                .flatten()
                .map(|i| text(i, 11.0, danger(1.0)).line_height(16.0).silent()),
        )
        .child(div().grow())
        .child(revert(s, key, edited.is_none() && invalid.is_none()));
    div()
        .col()
        .gap(8.0)
        .child(panel)
        .child(foot)
        .child(edit_buttons(s, key, large, approve, invalid.as_deref()))
}

fn table(s: &Screen<'_>, key: &str, t: &gates::Table, height: f32) -> El {
    if t.rows.is_empty() {
        return div()
            .px(12.0)
            .py(16.0)
            .child(text("No items.", 12.0, faint()).line_height(18.0));
    }
    let cols = t.columns();
    let mut head = div()
        .row()
        .items_center()
        .gap(8.0)
        .px(12.0)
        .py(6.0)
        .border_bottom(1.0, white(0.06))
        .child(div().w(20.0).shrink0());
    for col in &cols {
        head = head.child(
            div().flex(1.0).min_w(112.0).px(6.0).child(
                text(col.as_str(), 11.0, faint())
                    .mono()
                    .line_height(16.5)
                    .truncate()
                    .role(Role::ColumnHeader, col.as_str()),
            ),
        );
    }
    head = head.child(div().w(44.0).shrink0());
    let mut body = div().col().role(Role::Table, "Items").child(head);
    for (i, r) in t.rows.iter().enumerate() {
        let dim = if r.removed { 0.4 } else { 1.0 };
        let mut row = div()
            .row()
            .items_center()
            .gap(8.0)
            .px(12.0)
            .py(8.0)
            .border_bottom(1.0, white(0.04))
            .role_only(Role::Row)
            .child(
                div().w(20.0).shrink0().opacity(dim).child(
                    text((i + 1).to_string(), 11.0, faint())
                        .mono()
                        .line_height(16.5)
                        .silent(),
                ),
            );
        for col in &cols {
            row = row.child(
                div()
                    .flex(1.0)
                    .min_w(112.0)
                    .opacity(dim)
                    .child(cell(s, key, i, r, col)),
            );
        }
        let end = if r.removed {
            let id = s.acts.on(
                "gate-row",
                Act::Gate(GateOp::Remove(key.to_owned(), i, false)),
            );
            let hot = s.ui.hover(id);
            div()
                .id(id)
                .on(Action::Click)
                .px(6.0)
                .py(2.0)
                .rounded(4.0)
                .bg(white(0.0).mix(white(0.04), hot))
                .role(Role::Button, format!("Undo removing row {}", i + 1))
                .child(
                    text("Undo", 11.0, secondary().mix(ink(1.0), hot))
                        .line_height(16.5)
                        .silent(),
                )
        } else {
            let id = s.acts.on(
                "gate-row",
                Act::Gate(GateOp::Remove(key.to_owned(), i, true)),
            );
            let hot = s.ui.hover(id);
            div()
                .id(id)
                .on(Action::Click)
                .p(4.0)
                .rounded(4.0)
                .bg(white(0.0).mix(danger(0.10), hot))
                .role(Role::Button, format!("Remove row {}", i + 1))
                .child(ico("x", 12.0, 2.0, faint().mix(danger(1.0), hot)))
        };
        row = row.child(div().w(44.0).shrink0().row().justify_end().child(end));
        body = body.child(row);
    }
    div()
        .id(Id::new("gate-table").child(key))
        .w_full()
        .max_h(height)
        .scroll_y()
        .child(body.w_full())
}

fn cell(s: &Screen<'_>, key: &str, row: usize, r: &gates::Row, col: &str) -> El {
    let label = format!("{col}, row {}", row + 1);
    let value = r.values.get(col);
    match value {
        Some(Value::Bool(b)) => {
            let id = s.acts.on(
                "gate-cell",
                Act::Gate(GateOp::Flip(key.to_owned(), row, col.to_owned())),
            );
            let mut bx = div()
                .size(12.0, 12.0)
                .rounded(3.0)
                .center()
                .border(1.0, if *b { white(0.0) } else { c(look::GRAY_600) })
                .bg(if *b { secondary() } else { white(0.05) });
            if *b {
                bx = bx.child(ico("check", 9.0, 3.0, c(look::SURFACE_BASE)));
            }
            let mut el = div()
                .id(id)
                .row()
                .items_center()
                .gap(6.0)
                .px(6.0)
                .py(4.0)
                .role(Role::CheckBox, &label)
                .toggled(*b)
                .child(bx)
                .child(
                    text(b.to_string(), 12.0, secondary())
                        .mono()
                        .line_height(18.0)
                        .silent(),
                );
            if !r.removed {
                el = el.on(Action::Click);
            }
            el
        }
        Some(v @ (Value::Object(_) | Value::Array(_))) => div().px(6.0).py(4.0).child(
            text(v.to_string(), 12.0, faint())
                .mono()
                .line_height(18.0)
                .truncate(),
        ),
        _ => {
            let id = cell_id(key, row, col);
            let number = value.is_some_and(Value::is_number) || r.typed.contains_key(col);
            let bad = r
                .typed
                .get(col)
                .is_some_and(|t| gates::parse_number(t).is_none());
            let hot = if s.ui.focused(id) {
                1.0
            } else {
                s.ui.hover(id) * 0.4
            };
            let style = if number {
                TextStyle::mono(12.0)
            } else {
                TextStyle::ui(12.0)
            };
            let placeholder = if value.is_some_and(Value::is_null) {
                "null"
            } else {
                ""
            };
            div()
                .w_full()
                .px(6.0)
                .py(4.0)
                .rounded(4.0)
                .border(
                    1.0,
                    if bad {
                        danger(0.6)
                    } else {
                        white(0.0).mix(white(0.2), hot)
                    },
                )
                .frames(id)
                .child(
                    field(
                        id,
                        &label,
                        placeholder,
                        TextStyle {
                            line_height: 18.0,
                            ..style
                        },
                        if bad { danger(1.0) } else { ink(1.0) },
                    )
                    .w_full()
                    .min_w0(),
                )
        }
    }
}

/// The step, the run and its round over the review
/// page, with the answers in its footer. The page itself opens in the
/// browser until the app has a web view of its own.
pub fn review(s: &Screen<'_>) -> Option<El> {
    let (run_key, nid) = s.st.runs.review.as_ref()?;
    let run = s.store.runs.get(run_key)?;
    let state = runs::states(run).find(|n| sv(n, "nodeId") == Some(nid.as_str()))?;
    let wid = sv(run, "workflowId").unwrap_or("");
    let w = s
        .store
        .config
        .get("workflows")
        .and_then(Value::as_array)
        .and_then(|l| l.iter().find(|w| sv(w, "id") == Some(wid)));
    let nodes = w
        .and_then(|w| w.get("nodes"))
        .cloned()
        .unwrap_or(Value::Null);
    let node = runs::node(&nodes, nid);
    let config = node.and_then(|n| n.get("config"));
    let label = node.and_then(|n| sv(n, "label")).unwrap_or("Approval");
    let short: String = run_key.chars().take(8).collect();
    let meta: Vec<String> = [
        w.and_then(|w| sv(w, "name")).map(str::to_owned),
        Some(format!("run {short}")),
        gates::round_label(config, state),
    ]
    .into_iter()
    .flatten()
    .collect();
    let close_id = s.acts.on("gate-review", Act::Gate(GateOp::Review(None)));
    let close_hot = s.ui.hover(close_id);
    let head = div()
        .row()
        .items_center()
        .gap(10.0)
        .px(16.0)
        .h(44.0)
        .border_bottom(1.0, white(0.08))
        .shrink0()
        .child(ico("hand", 13.0, 1.5, faint()))
        .child(
            text(label, 13.0, ink(1.0))
                .weight(500)
                .line_height(19.5)
                .silent(),
        )
        .child(
            div().grow().min_w0().child(
                text(meta.join(" \u{b7} "), 11.5, faint())
                    .mono()
                    .line_height(17.0)
                    .truncate(),
            ),
        )
        .child(
            div()
                .id(close_id)
                .on(Action::Click)
                .p(4.0)
                .rounded(6.0)
                .role(Role::Button, "Close review")
                .child(ico("x", 14.0, 2.0, faint().mix(ink(1.0), close_hot))),
        );
    let open_id = s.acts.on(
        "gate-review",
        Act::Run(crate::runs::RunAct::OpenReview(
            run_key.clone(),
            nid.clone(),
        )),
    );
    let open_hot = s.ui.hover(open_id);
    let body = div()
        .col()
        .grow()
        .min_h(0.0)
        .center()
        .gap(10.0)
        .child(text("The review page opens in your browser.", 12.0, faint()).line_height(18.0))
        .child(
            div()
                .id(open_id)
                .on(Action::Click)
                .row()
                .items_center()
                .gap(6.0)
                .px(12.0)
                .py(6.0)
                .rounded(6.0)
                .border(1.0, white(0.08))
                .bg(white(0.0).mix(white(0.04), open_hot))
                .role(Role::Button, "Open the review page")
                .child(ico("external-link", 12.0, 1.75, secondary()))
                .child(
                    text(
                        "Open the review page",
                        12.0,
                        secondary().mix(ink(1.0), open_hot),
                    )
                    .line_height(18.0)
                    .silent(),
                ),
        );
    let producer = config.and_then(|c| sv(c, "view")).and_then(|v| {
        let at = v.find("{{")?;
        let rest = v[at + 2..].trim_start().strip_prefix("steps.")?;
        let slug: String = rest
            .chars()
            .take_while(|ch| ch.is_alphanumeric() || *ch == '_' || *ch == '-')
            .collect();
        nodes
            .as_array()?
            .iter()
            .find(|n| sv(n, "slug") == Some(slug.as_str()))
            .and_then(|n| sv(n, "label"))
            .map(str::to_owned)
    });
    let mut note = div().row().min_w0().items_center();
    if let Some(p) = &producer {
        note = note
            .child(text("Built by ", 11.0, faint()).line_height(16.5).silent())
            .child(
                text(p.as_str(), 11.0, secondary())
                    .line_height(16.5)
                    .silent(),
            )
            .child(text(". ", 11.0, faint()).line_height(16.5).silent());
    }
    note = note.child(
        div().grow().min_w0().child(
            text(
                "It runs sandboxed: nothing loads from the network and it can't reach Vorn.",
                11.0,
                faint(),
            )
            .line_height(16.5)
            .truncate(),
        ),
    );
    let foot = div()
        .row()
        .items_end()
        .gap(16.0)
        .px(16.0)
        .py(10.0)
        .border_top(1.0, white(0.08))
        .shrink0()
        .child(
            div()
                .grow()
                .min_w0()
                .row()
                .items_center()
                .gap(6.0)
                .pb(4.0)
                .child(ico("lock", 11.0, 2.0, faint()))
                .child(note),
        )
        .child(
            div()
                .w((s.size.0 * 0.9).min(960.0) * 0.55_f32.min(440.0 / (s.size.0 * 0.9).min(960.0)))
                .shrink0()
                .child(actions(s, Place::Review, run_key, state, config, &nodes)),
        );
    let (w, h) = ((s.size.0 * 0.9).min(960.0), s.size.1 * 0.84);
    let panel = div()
        .col()
        .size(w, h)
        .rounded(12.0)
        .border(1.0, white(0.1))
        .bg(c(look::SURFACE_PANEL))
        .clip()
        .role(Role::Dialog, format!("Review: {label}"))
        .child(head)
        .child(body)
        .child(foot);
    let panel = s.acts.click(panel, "gate-review-panel", Act::Nothing);
    Some(
        div()
            .absolute(0.0, 0.0)
            .size(s.size.0, s.size.1)
            .bg(crate::ui::theme::shadow(0.6))
            .layered()
            .center()
            .child(panel),
    )
}
