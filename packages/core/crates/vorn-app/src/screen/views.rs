//! A declared card view: the sessions in columns by status, project, worktree, your groups or an extension's grouping, each card full, mini or one line, and the agents' permission requests drawn over any of them.

use vorn_tree::view::{CardSize, CardStatus};

use super::{c, cards, ico, white, Screen};
use crate::act::{Act, Acts};
use crate::card_view::{self, Column, ColumnsBy, Spec};
use crate::client::Session;
use crate::look;
use crate::points::{Point, PointAct, Row};
use crate::ui::{custom, div, text, tooltip, Action, El, Id, Role, Sense};

/// Custom ids from here are mini cards' terminals: `MINI_ID_BASE + i` draws `store.sessions[i]` smaller.
pub const MINI_ID_BASE: u64 = 3_000_000;
/// Past the mini range: the shell blocks' ids start here.
const MINI_ID_END: u64 = super::cards::BLOCK_ID_BASE;

/// The session a mini card's custom box draws, if it is one.
pub fn mini_index(id: u64) -> Option<usize> {
    (MINI_ID_BASE..MINI_ID_END)
        .contains(&id)
        .then(|| usize::try_from(id - MINI_ID_BASE).ok())
        .flatten()
}

/// A column's box, which a dragged card is dropped on.
pub fn column_id(key: &str) -> Id {
    Id::new("view-column").child(key)
}

/// What a card is clicked and dragged by.
pub fn card_id(session: &str) -> Id {
    Acts::id("view-card", &Act::Select(session.to_owned()))
}

/// The view's own bar over its columns.
const BAR_H: f32 = 36.0;
const COLUMN_HEAD_H: f32 = 28.0;
const GAP: f32 = 8.0;
const COLUMN_PAD: f32 = 6.0;
const FULL_H: f32 = 300.0;
const MINI_HEAD_H: f32 = 28.0;
const MINI_TERM_H: f32 = 132.0;
const LINE_H: f32 = 32.0;
/// A request drawn over a one-line card takes this much room instead.
const ASK_H: f32 = 104.0;

/// How tall a card of `size` is in a column.
fn height(size: CardSize, asking: bool) -> f32 {
    match size {
        CardSize::Full => FULL_H,
        CardSize::Mini => MINI_HEAD_H + MINI_TERM_H,
        CardSize::Line | CardSize::Tile if asking => ASK_H,
        CardSize::Line | CardSize::Tile => LINE_H,
    }
}

/// The view's cards in their columns, or the grid where a view cannot draw.
pub(super) fn view(s: &Screen<'_>) -> El {
    let Some((p, spec)) = s.st.layout.view().and_then(|k| s.st.points.card_view(k)) else {
        return cards::grid(s);
    };
    if s.ui.is_cells() {
        return div()
            .col()
            .grow()
            .min_h(0.0)
            .child(
                text(
                    format!("{} draws in the desktop window; showing Grid", p.title),
                    12.0,
                    c(look::GRAY_500),
                )
                .line_height(16.0)
                .px(12.0)
                .role(Role::Status, "Card views draw in the desktop window"),
            )
            .child(cards::grid(s));
    }
    let now = crate::state::now_ms().max(0.0) as u64;
    let shown = s.st.visible(s.store);
    let rows = grouping_rows(s, &spec.columns);
    let attention = s.st.points.attention(s.store.workspace());
    let status = |x: &Session| {
        let id = x.info.id.as_str();
        let asking = s.st.asks.iter().any(|a| a.session == id)
            || attention.iter().any(|(_, r)| r.str("session") == Some(id));
        card_view::status_of(x, s.st.cards.printed_at(id), now, asking)
    };
    let cols = card_view::columns(&spec.columns, &shown, s.store, status, &rows);
    let (w, h) = s.grid_size();
    let room = (h - BAR_H - COLUMN_HEAD_H - 2.0 * GAP).max(LINE_H);
    let n = cols.len().max(1) as f32;
    // A column's inside: the board's gaps and the column's own padding and border taken off.
    let inside = ((w - GAP * (n + 1.0)) / n - 2.0 * COLUMN_PAD - 2.0).max(120.0);
    let mut board = div()
        .row()
        .grow()
        .min_h(0.0)
        .gap(GAP)
        .p(GAP)
        .role(Role::Grid, p.title.as_str());
    for col in &cols {
        board = board.child(column(s, spec, col, (inside, room), &status, &attention).flex(1.0));
    }
    div()
        .col()
        .grow()
        .min_h(0.0)
        .bg(c(look::SURFACE_BASE))
        .child(bar(s, p, spec))
        .child(board)
}

/// The rows of the grouping a view's columns follow, those of the active workspace.
fn grouping_rows<'a>(s: &'a Screen<'_>, by: &ColumnsBy) -> Vec<&'a Row> {
    let ColumnsBy::Grouping(key) = by else {
        return Vec::new();
    };
    s.st.points
        .get(key)
        .map(|g| s.st.points.listed_rows(g, s.store.workspace()))
        .unwrap_or_default()
}

/// The view's name and what its columns follow.
fn bar(s: &Screen<'_>, p: &Point, spec: &Spec) -> El {
    let by = match &spec.columns {
        ColumnsBy::Grouping(key) => {
            s.st.points
                .get(key)
                .map_or(key.as_str(), |g| g.title.as_str())
        }
        other => other.label(),
    };
    let drags = if spec.columns.moves() {
        "Drag a card to another column to move it"
    } else {
        "Columns follow what each session does; dragging moves nothing"
    };
    let label = format!("Columns by {by}");
    let id = Id::new("view-bar").child(p.key().as_str());
    let el = div()
        .id(id)
        .sense(Sense::HOVER)
        .row()
        .items_center()
        .gap(8.0)
        .h(BAR_H)
        .px(12.0)
        .shrink0()
        .border_bottom(1.0, white(0.06))
        .child(ico(
            p.icon.as_deref().unwrap_or("columns-3"),
            14.0,
            2.0,
            c(look::GRAY_300),
        ))
        .child(
            text(p.title.as_str(), 13.0, c(look::WHITE))
                .line_height(18.0)
                .silent(),
        )
        .child(
            text(label.as_str(), 12.0, c(look::GRAY_500))
                .line_height(18.0)
                .silent(),
        );
    tooltip(s.ui, el, id, drags, None)
}

/// One column: its heading and count, then as many cards as fit in `room`, then how many more there are.
fn column(
    s: &Screen<'_>,
    spec: &Spec,
    col: &Column,
    (inside, room): (f32, f32),
    status: &dyn Fn(&Session) -> CardStatus,
    attention: &[(&Point, &Row)],
) -> El {
    let id = column_id(&col.key);
    let over =
        s.st.view_drag
            .as_ref()
            .is_some_and(|(_, to)| to.as_deref() == Some(col.key.as_str()));
    let dot = CardStatus::parse(&col.key).map(|st| {
        div()
            .size(8.0, 8.0)
            .rounded(4.0)
            .shrink0()
            .bg(c(status_color(st)))
    });
    let head = div()
        .row()
        .items_center()
        .gap(6.0)
        .h(COLUMN_HEAD_H)
        .px(4.0)
        .shrink0()
        .child_if(dot)
        .child(
            div().grow().min_w0().child(
                text(col.title.to_uppercase(), 11.0, c(look::GRAY_300))
                    .weight(600)
                    .line_height(14.0)
                    .truncate()
                    .role(Role::Heading, col.title.as_str()),
            ),
        )
        .child(
            text(col.sessions.len().to_string(), 11.0, c(look::GRAY_500))
                .line_height(14.0)
                .silent(),
        );
    let mut list = div().col().gap(GAP).min_w0();
    let sized: Vec<(usize, f32, bool)> = col
        .sessions
        .iter()
        .map(|&i| {
            let session = &s.store.sessions[i];
            let asking = ask_of(s, attention, &session.info.id).is_some();
            (i, height(spec.size(status(session)), asking), asking)
        })
        .collect();
    let (shown, hidden) = card_view::fit(&sized, room, GAP);
    for i in shown {
        let session = &s.store.sessions[i];
        let ask = ask_of(s, attention, &session.info.id);
        let size = spec.size(status(session));
        list = list.child(card(s, i, session, size, ask, inside));
    }
    if hidden > 0 {
        list = list.child(
            text(format!("{hidden} more"), 11.0, c(look::GRAY_500))
                .line_height(16.0)
                .px(4.0)
                .silent(),
        );
    }
    div()
        .id(id)
        .sense(Sense::HOVER)
        .col()
        .min_w0()
        .min_h(0.0)
        .p(COLUMN_PAD)
        .rounded(4.0)
        .clip()
        .bg(if over {
            white(0.06)
        } else {
            c(look::SURFACE_PANEL)
        })
        .border(1.0, if over { white(0.18) } else { white(0.0) })
        .role(Role::Group, col.title.as_str())
        .child(head)
        .child(list)
}

/// The colour of a status column's dot.
fn status_color(st: CardStatus) -> look::Color {
    match st {
        CardStatus::Started | CardStatus::Printing => look::GREEN_500,
        CardStatus::Waiting => look::BRONZO,
        CardStatus::Finished => look::GRAY_400,
        CardStatus::Idle => look::GRAY_600,
    }
}

/// A request waiting on a person, about one session: an agent's, or an attention row naming it.
enum Asking<'a> {
    Agent(&'a card_view::Ask),
    Row(&'a Point, &'a Row),
}

/// The request waiting about session `sid`: an agent's first, else an attention row naming it.
fn ask_of<'a>(
    s: &'a Screen<'_>,
    attention: &[(&'a Point, &'a Row)],
    sid: &str,
) -> Option<Asking<'a>> {
    if let Some(a) = s.st.asks.iter().find(|a| a.session == sid) {
        return Some(Asking::Agent(a));
    }
    attention
        .iter()
        .find(|(_, r)| r.str("session") == Some(sid))
        .map(|&(p, r)| Asking::Row(p, r))
}

/// One session's card at `size`, with any request about it on top.
fn card(
    s: &Screen<'_>,
    i: usize,
    session: &Session,
    size: CardSize,
    ask: Option<Asking<'_>>,
    w: f32,
) -> El {
    let sid = session.info.id.as_str();
    let selected = s.st.selected.as_deref() == Some(sid);
    let dragged = s.st.view_drag.as_ref().is_some_and(|(d, _)| d == sid);
    let el = match size {
        CardSize::Full => cards::card(s, i, i, session).h(FULL_H).shrink0(),
        CardSize::Mini => mini(s, i, session, selected),
        CardSize::Line | CardSize::Tile => {
            if let Some(ask) = ask {
                return ask_panel(s, session, &ask, w);
            }
            line(s, session, selected)
        }
    };
    let el = if dragged { el.opacity(0.5) } else { el };
    match ask {
        Some(ask) => el.overlay(
            ask_panel(s, session, &ask, w - 12.0),
            crate::ui::Place::InsideEnd,
            6.0,
        ),
        None => el,
    }
}

/// The card's frame: raised, its border brighter while selected or hovered.
fn frame(s: &Screen<'_>, el: El, sid: &str, selected: bool) -> El {
    let id = card_id(sid);
    let hot = s.ui.hover(id);
    let border = if selected {
        white(0.24)
    } else {
        white(0.06).mix(white(0.12), hot)
    };
    s.acts
        .click(el, "view-card", Act::Select(sid.to_owned()))
        .sense(Sense::MOVE | Sense::HOVER)
        .bg(c(look::SURFACE_RAISED))
        .border(1.0, border)
        .rounded(4.0)
        .clip()
        .selected(selected)
}

/// A mini card: its header, and its terminal drawn smaller, clipped to the newest rows; the session keeps its size.
fn mini(s: &Screen<'_>, i: usize, session: &Session, selected: bool) -> El {
    let sid = session.info.id.as_str();
    let expand = s.acts.on("view-expand", Act::Expand(sid.to_owned()));
    let hot = s.ui.hover(expand);
    let open = div()
        .id(expand)
        .on(Action::Click)
        .size(20.0, 20.0)
        .center()
        .rounded(4.0)
        .shrink0()
        .role(Role::Button, "Open full")
        .child(ico(
            "maximize-2",
            12.0,
            2.0,
            c(look::GRAY_500).mix(c(look::WHITE), hot),
        ));
    let head = div()
        .row()
        .items_center()
        .gap(6.0)
        .h(MINI_HEAD_H)
        .px(8.0)
        .shrink0()
        .border_bottom(1.0, white(0.04))
        .child(cards::status_icon(s, session, 14.0, 1.0))
        .child(
            div().grow().min_w0().child(
                text(session.title(), 12.0, c(look::GRAY_200))
                    .line_height(16.0)
                    .truncate()
                    .silent(),
            ),
        )
        .child(tooltip(s.ui, open, expand, "Open full", None));
    let el = div()
        .col()
        .min_w0()
        .shrink0()
        .h(MINI_HEAD_H + MINI_TERM_H)
        .role(Role::Group, session.title())
        .child(head)
        .child(
            custom(MINI_ID_BASE + i as u64)
                .h(MINI_TERM_H - 1.0)
                .w_full()
                .bg(c(look::SURFACE_SUNKEN)),
        );
    frame(s, el, sid, selected)
}

/// A one-line card: its status, title and the last line it printed.
fn line(s: &Screen<'_>, session: &Session, selected: bool) -> El {
    let sid = session.info.id.as_str();
    let last =
        s.st.cards
            .lines
            .get(sid)
            .and_then(|l| l.text.as_deref())
            .unwrap_or("");
    let el = div()
        .row()
        .items_center()
        .gap(8.0)
        .h(LINE_H)
        .px(8.0)
        .min_w0()
        .shrink0()
        .role(Role::Group, session.title())
        .child(cards::status_icon(s, session, 14.0, 1.0))
        .child(
            text(session.title(), 12.0, c(look::GRAY_200))
                .line_height(16.0)
                .truncate()
                .shrink0()
                .max_w(160.0)
                .silent(),
        )
        .child(
            div().grow().min_w0().child(
                text(last, 11.0, c(look::GRAY_500))
                    .mono()
                    .line_height(16.0)
                    .truncate()
                    .silent(),
            ),
        );
    frame(s, el, sid, selected)
}

/// A request to use a tool, drawn by Vorn over the card it is about: the tool, what it would run, and Allow and Deny.
fn ask_panel(s: &Screen<'_>, session: &Session, ask: &Asking<'_>, w: f32) -> El {
    let (key, title, detail, allow, deny) = match ask {
        Asking::Agent(a) => (
            a.request.clone(),
            format!("Allow {}?", a.tool),
            a.description.clone(),
            Act::Permission {
                request: a.request.clone(),
                allow: true,
            },
            Act::Permission {
                request: a.request.clone(),
                allow: false,
            },
        ),
        Asking::Row(p, r) => {
            let answer = |allow| {
                Act::Point(PointAct::Answer {
                    key: p.key(),
                    row: r.id.clone(),
                    allow,
                })
            };
            (
                format!("{}#{}", p.key(), r.id),
                r.str("title").unwrap_or(p.title.as_str()).to_owned(),
                r.str("message")
                    .or_else(|| r.str("subtitle"))
                    .map(str::to_owned),
                answer(true),
                answer(false),
            )
        }
    };
    let button = |label: &str, act: Act, strong: bool| {
        let id = s.acts.on("view-ask", act);
        let hot = s.ui.hover(id);
        div()
            .id(id)
            .on(Action::Click)
            .px(10.0)
            .py(3.0)
            .rounded(4.0)
            .border(1.0, if strong { white(0.24) } else { white(0.1) })
            .bg(white(0.0).mix(white(0.06), hot))
            .role(Role::Button, label)
            .child(
                text(
                    label,
                    11.0,
                    c(if strong { look::WHITE } else { look::GRAY_300 }),
                )
                .line_height(14.0)
                .silent(),
            )
    };
    let shift = if crate::ui::mods::mac() {
        "\u{21e7}"
    } else {
        "Shift+"
    };
    let keys = format!("{shift}{}A", super::mod_label());
    div()
        .id(Id::new("view-ask-panel").child(key.as_str()))
        .sense(Sense::HOVER)
        .col()
        .gap(6.0)
        .p(8.0)
        .w(w)
        .rounded(4.0)
        .bg(c(look::SURFACE_OVERLAY))
        .border(1.0, c(look::BRONZO))
        .role(Role::Alert, title.as_str())
        .child(
            div()
                .row()
                .items_center()
                .gap(6.0)
                .child(cards::status_icon(s, session, 14.0, 1.0))
                .child(
                    div().grow().min_w0().child(
                        text(session.title(), 12.0, c(look::GRAY_200))
                            .line_height(16.0)
                            .truncate()
                            .silent(),
                    ),
                )
                .child(
                    text(title.as_str(), 11.0, c(look::BRONZO))
                        .line_height(14.0)
                        .silent(),
                ),
        )
        .child_if(detail.map(|d| {
            div()
                .px(6.0)
                .py(4.0)
                .rounded(4.0)
                .bg(c(look::SURFACE_BASE))
                .child(
                    text(d, 10.0, c(look::GRAY_300))
                        .mono()
                        .line_height(14.0)
                        .truncate()
                        .silent(),
                )
        }))
        .child(
            div()
                .row()
                .items_center()
                .gap(6.0)
                .child(button("Allow", allow, true))
                .child(button("Deny", deny, false))
                .child(div().grow())
                .child(
                    text(keys, 10.0, c(look::GRAY_500))
                        .mono()
                        .line_height(14.0)
                        .silent(),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mini_ids_map_back_to_sessions_and_leave_the_blocks_alone() {
        assert_eq!(mini_index(MINI_ID_BASE + 4), Some(4));
        assert_eq!(mini_index(MINI_ID_BASE - 1), None);
        assert_eq!(mini_index(super::super::cards::BLOCK_ID_BASE), None);
    }

    #[test]
    fn a_request_over_a_one_line_card_takes_the_room_it_needs() {
        assert!(height(CardSize::Line, true) > height(CardSize::Line, false));
        assert_eq!(height(CardSize::Mini, true), height(CardSize::Mini, false));
    }
}
