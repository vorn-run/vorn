//! A template's board, drawn by the kit: a column per value of the field
//! it groups by, each a scroll view that builds only the cards in or near
//! view, with its count, an add button and a drop target. The client keeps
//! the drag (which card, over which column) and answers the card's clicks;
//! the kit draws what the [`Bind`](crate::template::Bind) says of them.

use vorn_view::{Board, BoardColumn, Row};
use vornui::theme::color;
use vornui::{div, text, Action, El, Id, Place, Rgba, Sense, Ui};

use crate::template::{child_slot, Cx, Press};

use vornui::accesskit::Role;

/// What a column guesses a card's height is before it has measured one.
const CARD_GUESS: f32 = 92.0;

/// The column holding `value` on a board of `rows`.
pub fn column_id(rows: &str, value: &str) -> Id {
    Id::new("board-column").child(rows).child(value)
}

/// The scroll view of the cards of column `value`.
pub fn scroll_id(rows: &str, value: &str) -> Id {
    Id::new("board-column-scroll").child(rows).child(value)
}

/// The card of row `row`, before a client's [`Bind::card`](crate::template::Bind::card) names it.
pub fn card_id(rows: &str, row: &str) -> Id {
    Id::new("board-card").child(rows).child(row)
}

/// The column under `(x, y)` as last laid out, if any.
pub fn column_at<'b>(ui: &Ui, b: &'b Board, x: f32, y: f32) -> Option<&'b str> {
    b.columns
        .iter()
        .find(|c| {
            ui.rect_of(column_id(&b.rows, &c.value))
                .is_some_and(|r| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
        })
        .map(|c| c.value.as_str())
}

fn white(a: f32) -> Rgba {
    Rgba::white(a)
}

fn faint() -> Rgba {
    color::INK_FAINT
}

fn secondary() -> Rgba {
    color::INK_SECONDARY
}

/// The rows of each column, by index into the collection, in the order published.
fn grouped(b: &Board, rows: &[Row]) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = vec![Vec::new(); b.columns.len()];
    for (i, r) in rows.iter().enumerate() {
        let value = r.str(&b.by);
        if let Some(c) = value.and_then(|v| b.columns.iter().position(|c| c.value == v)) {
            groups[c].push(i);
        }
    }
    groups
}

/// The board, its columns side by side.
pub(crate) fn view(cx: &Cx<'_>, b: &Board, slot: u64) -> El {
    let rows = cx.src.rows(&b.rows);
    let groups = grouped(b, rows);
    let dragging = cx.bind.dragging(&b.rows);
    let keep: Vec<Id> = dragging
        .iter()
        .map(|(row, _)| cx.bind.card(&b.rows, row, None))
        .collect();
    let mut board = div().row().gap(8.0).grow().min_h(0.0).w_full();
    for (i, (c, group)) in b.columns.iter().zip(&groups).enumerate() {
        let over = dragging
            .as_ref()
            .is_some_and(|(_, col)| col.as_deref() == Some(c.value.as_str()));
        let at = Slots {
            card: child_slot(slot, 0),
            header: child_slot(slot, 1 + i * 2),
            foot: child_slot(slot, 2 + i * 2),
        };
        board = board.child(column(
            cx,
            b,
            c,
            at,
            group,
            rows,
            over,
            &keep,
            dragging.as_ref(),
        ));
    }
    board
}

/// The slots of one column's controls: its cards', and its two add buttons'.
#[derive(Clone, Copy)]
struct Slots {
    card: u64,
    header: u64,
    foot: u64,
}

/// One column: its header, its cards windowed, and its add button.
#[allow(clippy::too_many_arguments)] // One column's parts, each drawn from the frame's state.
fn column(
    cx: &Cx<'_>,
    b: &Board,
    c: &BoardColumn,
    at: Slots,
    group: &[usize],
    rows: &[Row],
    over: bool,
    keep: &[Id],
    dragging: Option<&(String, Option<String>)>,
) -> El {
    let scroll = scroll_id(&b.rows, &c.value);
    let mut cards = div()
        .id(scroll)
        .col()
        .gap(8.0)
        .grow()
        .min_h(0.0)
        .scroll_y()
        .px(8.0)
        .pb(8.0);
    if group.is_empty() {
        if let Some(empty) = &b.empty {
            let words = empty.fill_in(None, cx.src, Some(c));
            cards = cards.child(
                div().min_h(80.0).center().child(
                    div()
                        .w_full()
                        .px(16.0)
                        .py(20.0)
                        .rounded(8.0)
                        .border(1.0, white(0.08))
                        .dashed()
                        .center()
                        .child(text(words, 12.0, faint()).line_height(16.0).silent()),
                ),
            );
        }
    }
    let keys: Vec<Id> = group
        .iter()
        .map(|&i| cx.bind.card(&b.rows, &rows[i].id, None))
        .collect();
    let cards = cx
        .ui
        .windowed(scroll, &keys, CARD_GUESS, keep)
        .fill(cards, |k| {
            let i = group[k];
            match cx.src.row_at(&b.rows, i) {
                Some(row) => card(cx, b, c, &row, at.card, false, dragging),
                None => div().id(keys[k]),
            }
        });
    let id = column_id(&b.rows, &c.value);
    let mut col = div()
        .id(id)
        .sense(Sense::HOVER)
        .col()
        .flex(1.0)
        .min_w0()
        .min_h(0.0)
        .rounded(8.0)
        .role(Role::Group, c.label.as_str())
        .child(header(cx, b, c, group.len(), at.header))
        .child(cards);
    if let Some(add) = &b.add {
        let press = Press {
            call: Some(add.clone()),
            row: Some(c.value.clone()),
            ..Press::default()
        };
        let add = cx.bind.press(at.foot, press);
        let hot = cx.ui.hover(add);
        col = col.child(
            div().px(8.0).pb(8.0).child(
                div()
                    .id(add)
                    .on(Action::Click)
                    .w_full()
                    .py(8.0)
                    .rounded(8.0)
                    .bg(white(0.0).mix(white(0.04), hot))
                    .center()
                    .role(Role::Button, "Add")
                    .child(crate::lucide(
                        "plus",
                        12.0,
                        2.0,
                        faint().mix(secondary(), hot),
                    )),
            ),
        );
    }
    if over {
        col = col.bg(white(0.04)).border(1.0, white(0.10));
    }
    col
}

/// A column's header: its icon, label and count, and an add button shown on hover.
fn header(cx: &Cx<'_>, b: &Board, c: &BoardColumn, count: usize, slot: u64) -> El {
    let tone = c.tone.map_or(faint(), |t| cx.tone_of(t));
    let mut h = div().row().items_center().gap(8.0).px(12.0).py(12.0);
    if let Some(icon) = &c.icon {
        h = h.child(crate::lucide(icon, 14.0, 2.0, tone));
    }
    h = h
        .child(
            text(c.label.as_str(), 13.0, secondary())
                .weight(500)
                .line_height(18.0)
                .silent(),
        )
        .child(
            div().pl(2.0).child(
                text(count.to_string(), 11.0, faint())
                    .line_height(16.0)
                    .silent(),
            ),
        )
        .child(div().grow());
    let Some(add) = &b.add else {
        return h;
    };
    let press = Press {
        call: Some(add.clone()),
        row: Some(c.value.clone()),
        ..Press::default()
    };
    let add = cx.bind.press(slot, press);
    let hot = cx.ui.hover(add);
    let shown = cx.ui.hovered(column_id(&b.rows, &c.value)) || hot > 0.0;
    let button = div()
        .id(add)
        .on(Action::Click)
        .p(4.0)
        .rounded(4.0)
        .role(Role::Button, format!("Add to {}", c.label))
        .opacity(if shown { 1.0 } else { 0.0 })
        .child(crate::lucide(
            "plus",
            14.0,
            2.0,
            faint().mix(secondary(), hot),
        ));
    h.child(vornui::widgets::tooltip_at(
        cx.ui,
        button,
        add,
        "Add",
        None,
        Place::Below,
    ))
}

/// One card: the template's card in the kit's frame. A ghost is the copy
/// drawn at the pointer while the card is dragged.
fn card(
    cx: &Cx<'_>,
    b: &Board,
    c: &BoardColumn,
    row: &Row,
    slot: u64,
    ghost: bool,
    dragging: Option<&(String, Option<String>)>,
) -> El {
    let id = cx.bind.card(&b.rows, &row.id, b.open.as_deref());
    let hot = if ghost { 0.0 } else { cx.ui.hover(id) };
    let inner = Cx {
        ui: cx.ui,
        src: cx.src,
        bind: cx.bind,
        column: Some(c),
        soft: true,
        reveal: Some(hot),
    };
    let body = inner.node(&b.card, Some(row), slot).unwrap_or_else(div);
    let dim = b
        .dim
        .as_ref()
        .is_some_and(|d| d.holds_in(Some(row), cx.src, Some(c)));
    let border = if hot > 0.0 { white(0.16) } else { white(0.08) };
    let mut el = div()
        .col()
        .px(14.0)
        .py(12.0)
        .rounded(6.0)
        .bg(color::SURFACE_RAISED)
        .border(1.0, border)
        .child(body);
    if ghost {
        return if dim { el.opacity(0.6) } else { el };
    }
    let label = b
        .label
        .as_ref()
        .map_or_else(|| row.id.clone(), |l| l.fill_in(Some(row), cx.src, Some(c)));
    let dragged = dragging.is_some_and(|(d, _)| d == &row.id);
    el = el
        .id(id)
        .sense(Sense::CLICK | Sense::MOVE | Sense::HOVER)
        .on(Action::Click)
        .role(Role::Button, label.as_str());
    el.opacity(if dragged {
        0.4
    } else if dim {
        0.6
    } else {
        1.0
    })
}

/// The card of `row` drawn at the pointer, `grab` the point it was held
/// by and its width, while the client says it is dragged.
fn ghost(cx: &Cx<'_>, b: &Board, row: &str, grab: (f32, f32, f32)) -> Option<El> {
    // A board is the template's view in every place that draws one.
    let slot = child_slot(1, 0);
    let (gx, gy, w) = grab;
    let (px, py) = cx.ui.pointer()?;
    let rows = cx.src.rows(&b.rows);
    let i = rows.iter().position(|r| r.id == row)?;
    let full = cx.src.row_at(&b.rows, i)?;
    let value = full.text(&b.by);
    let c = b.columns.iter().find(|c| c.value == value)?;
    Some(
        div()
            .absolute(px - gx, py - gy)
            .w(w)
            .opacity(0.9)
            .silent()
            .child(card(cx, b, c, &full, slot, true, None)),
    )
}

/// The kit's context for a board drawn outside a template's view: the
/// ghost a client draws over everything while a card is dragged.
pub fn ghost_of(
    ui: &Ui,
    b: &Board,
    src: &dyn vorn_view::Source,
    bind: &dyn crate::template::Bind,
    row: &str,
    grab: (f32, f32, f32),
) -> Option<El> {
    let cx = Cx {
        ui,
        src,
        bind,
        column: None,
        soft: true,
        reveal: None,
    };
    ghost(&cx, b, row, grab)
}
