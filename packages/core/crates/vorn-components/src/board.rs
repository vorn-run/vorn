//! The board an element tree's `board` names when the client reads its cards
//! itself (`cards = tasks`): a column per lane, each a scroll view that builds
//! only the cards in or near view, with its count, an add button and a drop
//! target. The client keeps the cards, the drag (which card, over which
//! column) and what a card's click and menu do; the kit draws what the
//! [`BoardBind`] says of them.

use vorn_tree::Tone;
use vornui::accesskit::Role;
use vornui::native::Sem;
use vornui::theme::{color, radius, text as size};
use vornui::widgets::{self, Tone as ConfirmTone};
use vornui::{div, text, Action, El, Id, Place, Rgba, Sense, Ui};

/// What a column guesses a card's height is before it has measured one.
const CARD_GUESS: f32 = 92.0;

/// A column of the board: the value its cards hold, as the tree's column names it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lane<'a> {
    pub value: &'a str,
    pub title: &'a str,
    pub icon: Option<&'a str>,
    pub tone: Option<Tone>,
}

/// One card, filled for the cards in view only.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Card {
    pub id: String,
    /// A short key above the title, as `APP-1A2B`.
    pub key: String,
    pub title: String,
    /// Where it came from, as an imported item's number.
    pub link: Option<String>,
    pub archived: bool,
    /// Drawn faded, as a cancelled or archived task.
    pub dim: bool,
    /// The agent its icon shows, by key.
    pub agent: Option<String>,
    /// Its agent's session is open.
    pub live: bool,
    /// A quiet line at its foot, as when it was made.
    pub note: String,
    /// Its menu, in order.
    pub actions: Vec<CardAction>,
}

/// An entry of a card's menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CardAction {
    pub id: &'static str,
    pub label: &'static str,
    pub icon: &'static str,
    pub danger: bool,
    /// Asked before it runs.
    pub confirm: Option<&'static str>,
}

/// The client's cards.
pub trait Cards {
    /// Every card on the board, in its order: each one's id and the value of the lane it is in.
    fn placed(&self) -> &[(String, String)];
    /// The card at `at` in [`Cards::placed`], filled whole.
    fn card(&self, at: usize) -> Option<Card>;
}

/// How the client binds the board's controls.
pub trait BoardBind {
    /// The id of card `card`; a click on it opens it.
    fn card(&self, card: &str) -> Id;
    /// The card being dragged, and the lane it is over.
    fn dragging(&self) -> Option<(String, Option<String>)>;
    /// The id of a lane's add button: in its header, or at its foot.
    fn add(&self, lane: &str, foot: bool) -> Id;
    /// The id of a card's menu button, which opens and closes the menu.
    fn menu(&self, card: &str) -> Id;
    /// Whether menu `id` is open.
    fn menu_open(&self, id: Id) -> bool;
    /// The id of `action` in card `card`'s menu.
    fn action(&self, card: &str, action: &CardAction) -> Id;
    /// The confirm's buttons when menu entry `id` asks before it runs and is asking now.
    fn confirming(&self, id: Id) -> Option<(Id, Id)>;
    /// An icon the client draws for a name not in the kit's set, as
    /// `agent:<key>` for an agent's mark; `None` draws the kit's.
    fn icon(&self, name: &str, size: f32) -> Option<El> {
        let _ = (name, size);
        None
    }
}

/// The column holding `value` on board `board`.
pub fn column_id(board: &str, value: &str) -> Id {
    Id::new("board-column").child(board).child(value)
}

/// The scroll view of the cards of column `value`.
pub fn scroll_id(board: &str, value: &str) -> Id {
    Id::new("board-column-scroll").child(board).child(value)
}

/// The card `card` on board `board`, before a client's [`BoardBind::card`] names it.
pub fn card_id(board: &str, card: &str) -> Id {
    Id::new("board-card").child(board).child(card)
}

/// The lane under `(x, y)` as last laid out, if any.
pub fn column_at<'b>(
    ui: &Ui,
    board: &str,
    lanes: &'b [Lane<'b>],
    x: f32,
    y: f32,
) -> Option<&'b str> {
    lanes
        .iter()
        .find(|l| {
            ui.rect_of(column_id(board, l.value))
                .is_some_and(|r| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
        })
        .map(|l| l.value)
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

/// The colour `t` draws in on a board, where many show at once: good is muted.
pub fn tone(t: Tone) -> Rgba {
    match t {
        Tone::Good => color::STATUS_SAGE,
        other => crate::tree::tone(other),
    }
}

/// The cards of each lane, by index into [`Cards::placed`], in board order.
fn grouped(lanes: &[Lane<'_>], placed: &[(String, String)]) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = vec![Vec::new(); lanes.len()];
    for (i, (_, value)) in placed.iter().enumerate() {
        if let Some(l) = lanes.iter().position(|l| l.value == value) {
            groups[l].push(i);
        }
    }
    groups
}

/// What drawing a board needs: its name in ids, its lanes, the cards and the bindings.
pub struct Board<'a> {
    pub ui: &'a Ui,
    /// Tells two boards' ids apart.
    pub name: &'a str,
    pub lanes: &'a [Lane<'a>],
    /// What an empty column says.
    pub empty: Option<&'a str>,
    pub cards: &'a dyn Cards,
    pub bind: &'a dyn BoardBind,
}

impl Board<'_> {
    /// The board, its columns side by side; on a native backend a section per column.
    pub fn view(&self) -> El {
        let placed = self.cards.placed();
        let groups = grouped(self.lanes, placed);
        if self.ui.is_native() {
            return self.native(&groups);
        }
        let dragging = self.bind.dragging();
        let keep: Vec<Id> = dragging
            .iter()
            .map(|(card, _)| self.bind.card(card))
            .collect();
        let mut board = div().row().gap(8.0).grow().min_h(0.0).w_full();
        for (lane, group) in self.lanes.iter().zip(&groups) {
            let over = dragging
                .as_ref()
                .is_some_and(|(_, l)| l.as_deref() == Some(lane.value));
            board = board.child(self.column(lane, group, over, &keep, dragging.as_ref()));
        }
        board
    }

    /// Each lane as a titled section of its cards, as a platform lists them.
    fn native(&self, groups: &[Vec<usize>]) -> El {
        let mut col = div().col().sem(Sem::Column);
        for (lane, group) in self.lanes.iter().zip(groups) {
            let mut section = div().col().sem(Sem::Section(lane.title.to_owned()));
            if group.is_empty() {
                if let Some(empty) = self.empty {
                    section = section.child(text(empty, 12.0, faint()).sem(Sem::Text("secondary")));
                }
            }
            for &i in group {
                let Some(card) = self.cards.card(i) else {
                    continue;
                };
                let mut entry = div()
                    .id(self.bind.card(&card.id))
                    .on(Action::Click)
                    .row()
                    .sem(Sem::Entry {
                        title: card.title.clone(),
                        detail: card.key.clone(),
                    });
                if let Some(icon) = lane.icon {
                    entry = entry.child(crate::lucide(icon, 14.0, 2.0, faint()).sem(Sem::Icon {
                        name: icon.to_owned(),
                        tone: lane.tone.map_or("", crate::tree::native_tone),
                        svg: None,
                    }));
                }
                if card.live {
                    entry = entry.child(div().sem(Sem::Status {
                        value: "live".to_owned(),
                        tone: "good",
                    }));
                }
                if !card.note.is_empty() {
                    entry =
                        entry.child(text(&card.note, 11.0, faint()).sem(Sem::Text("secondary")));
                }
                section = section.child(entry);
            }
            col = col.child(section);
        }
        col
    }

    /// One column: its header, its cards windowed, and its add button.
    fn column(
        &self,
        lane: &Lane<'_>,
        group: &[usize],
        over: bool,
        keep: &[Id],
        dragging: Option<&(String, Option<String>)>,
    ) -> El {
        let placed = self.cards.placed();
        let scroll = scroll_id(self.name, lane.value);
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
            if let Some(empty) = self.empty {
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
                            .child(text(empty, 12.0, faint()).line_height(16.0).silent()),
                    ),
                );
            }
        }
        let keys: Vec<Id> = group
            .iter()
            .map(|&i| self.bind.card(&placed[i].0))
            .collect();
        let cards = self
            .ui
            .windowed(scroll, &keys, CARD_GUESS, keep)
            .fill(cards, |k| match self.cards.card(group[k]) {
                Some(card) => self.card(lane, &card, false, dragging),
                None => div().id(keys[k]),
            });
        let id = column_id(self.name, lane.value);
        let mut col = div()
            .id(id)
            .sense(Sense::HOVER)
            .col()
            .flex(1.0)
            .min_w0()
            .min_h(0.0)
            .rounded(8.0)
            .role(Role::Group, lane.title)
            .child(self.header(lane, group.len()))
            .child(cards);
        let add = self.bind.add(lane.value, true);
        let hot = self.ui.hover(add);
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
        if over {
            col = col.bg(white(0.04)).border(1.0, white(0.10));
        }
        col
    }

    /// A column's header: its icon, title and count, and an add button shown on hover.
    fn header(&self, lane: &Lane<'_>, count: usize) -> El {
        let ink = lane.tone.map_or(faint(), tone);
        let mut h = div().row().items_center().gap(8.0).px(12.0).py(12.0);
        if let Some(icon) = lane.icon {
            h = h.child(crate::lucide(icon, 14.0, 2.0, ink));
        }
        h = h
            .child(
                text(lane.title, 13.0, secondary())
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
        let add = self.bind.add(lane.value, false);
        let hot = self.ui.hover(add);
        let shown = self.ui.hovered(column_id(self.name, lane.value)) || hot > 0.0;
        let button = div()
            .id(add)
            .on(Action::Click)
            .p(4.0)
            .rounded(4.0)
            .role(Role::Button, format!("Add to {}", lane.title))
            .opacity(if shown { 1.0 } else { 0.0 })
            .child(crate::lucide(
                "plus",
                14.0,
                2.0,
                faint().mix(secondary(), hot),
            ));
        h.child(vornui::widgets::tooltip_at(
            self.ui,
            button,
            add,
            "Add",
            None,
            Place::Below,
        ))
    }

    /// One card in the kit's frame. A ghost is the copy drawn at the pointer
    /// while the card is dragged.
    fn card(
        &self,
        lane: &Lane<'_>,
        card: &Card,
        ghost: bool,
        dragging: Option<&(String, Option<String>)>,
    ) -> El {
        let id = self.bind.card(&card.id);
        let hot = if ghost { 0.0 } else { self.ui.hover(id) };
        let border = if hot > 0.0 { white(0.16) } else { white(0.08) };
        let el = div()
            .col()
            .px(14.0)
            .py(12.0)
            .rounded(6.0)
            .bg(color::SURFACE_RAISED)
            .border(1.0, border)
            .child(self.body(lane, card, hot, ghost));
        if ghost {
            return if card.dim { el.opacity(0.6) } else { el };
        }
        let dragged = dragging.is_some_and(|(d, _)| *d == card.id);
        el.id(id)
            .sense(Sense::CLICK | Sense::MOVE | Sense::HOVER)
            .on(Action::Click)
            .role(Role::Button, card.title.as_str())
            .opacity(if dragged {
                0.4
            } else if card.dim {
                0.6
            } else {
                1.0
            })
    }

    /// A card's three lines: its key, link and menu; its lane's icon and its
    /// title; its agent and when it was made.
    fn body(&self, lane: &Lane<'_>, card: &Card, reveal: f32, ghost: bool) -> El {
        let mut top = line().child(words(&card.key));
        if let Some(link) = card.link.as_deref() {
            top = top
                .child(words("·"))
                .child(self.icon("link", None))
                .child(words(&format!("#{link}")));
        }
        if card.archived {
            top = top.child(self.icon("archive", None));
        }
        top = top.child(div().grow().min_w0().child(words("")));
        if let Some(more) = self.more(card, reveal, ghost) {
            top = top.child(more);
        }
        let mut middle = line();
        if let Some(icon) = lane.icon {
            middle = middle.child(self.icon(icon, lane.tone));
        }
        middle = middle.child(
            div().grow().min_w0().child(
                text(&card.title, 13.0, color::GRAY_200)
                    .line_height(18.0)
                    .weight(size::SEMIBOLD)
                    .wrapping()
                    .silent(),
            ),
        );
        let mut foot = line();
        if let Some(agent) = card.agent.as_deref() {
            foot = foot.child(self.icon(&format!("agent:{agent}"), None));
            if card.live {
                let ink = tone(Tone::Good);
                let dot = if self.ui.is_cells() {
                    text("●", 12.0, ink).line_height(16.0).silent()
                } else {
                    div().size(6.0, 6.0).rounded(3.0).bg(ink)
                };
                foot = foot.child(dot.shrink0().role(Role::Status, "true"));
            }
        }
        foot = foot
            .child(div().grow().min_w0().child(words("")))
            .child(words(&card.note));
        div()
            .col()
            .gap(6.0)
            .min_w0()
            .child(top)
            .child(middle)
            .child(foot)
    }

    fn icon(&self, name: &str, t: Option<Tone>) -> El {
        let ink = t.map_or(color::GRAY_400, tone);
        match self.bind.icon(name, 14.0) {
            Some(drawn) => drawn.shrink0(),
            None => crate::lucide(name, 14.0, 2.0, ink).shrink0(),
        }
    }

    /// A card's menu button and, while open, its menu; while an entry asks
    /// first, its question under the button, as the menu closes on the press.
    fn more(&self, card: &Card, reveal: f32, ghost: bool) -> Option<El> {
        if card.actions.is_empty() {
            return None;
        }
        let menu = self.bind.menu(&card.id);
        let hot = self.ui.hover(menu);
        let open = !ghost && self.bind.menu_open(menu);
        let shown = if open { 1.0 } else { reveal.max(hot) };
        let mut b = div()
            .id(menu)
            .on(Action::Click)
            .opacity(shown)
            .px(6.0)
            .py(4.0)
            .shrink0()
            .rounded(radius::SM)
            .bg(Rgba::TRANSPARENT.mix(Rgba::white(0.06), hot))
            .ring(radius::SM)
            .role(Role::Button, "More")
            .child(crate::lucide(
                "ellipsis",
                12.0,
                2.0,
                color::GRAY_400.mix(color::WHITE, hot),
            ));
        if ghost {
            return Some(b);
        }
        let mut entries = Vec::with_capacity(card.actions.len());
        let mut asking = None;
        for a in &card.actions {
            let id = self.bind.action(&card.id, a);
            if let (Some(message), Some(ids)) = (a.confirm, self.bind.confirming(id)) {
                asking = Some((message, a, ids));
            }
            entries.push(self.entry(id, a));
        }
        if open {
            let panel = div()
                .col()
                .min_w(160.0)
                .py(4.0)
                .bg(color::SURFACE_OVERLAY)
                .border(1.0, Rgba::white(0.10))
                .rounded(radius::LG)
                .role(Role::Menu, "More")
                .children(entries);
            b = b.overlay(panel, Place::BelowEnd, 4.0);
        }
        Some(match asking {
            Some((message, a, (ok, cancel))) => {
                let t = if a.danger {
                    ConfirmTone::Danger
                } else {
                    ConfirmTone::Neutral
                };
                widgets::confirm(self.ui, b, message, (ok, a.label), cancel, t)
            }
            None => b,
        })
    }

    /// One entry of a card's menu.
    fn entry(&self, id: Id, a: &CardAction) -> El {
        let hot = self.ui.hover(id);
        let ink = if a.danger {
            color::RED_400.mix(color::RED_300, hot)
        } else {
            color::GRAY_300.mix(color::WHITE, hot)
        };
        div()
            .id(id)
            .on(Action::Click)
            .row()
            .items_center()
            .gap(8.0)
            .mx(4.0)
            .px(8.0)
            .py(5.0)
            .rounded(radius::SM)
            .bg(Rgba::TRANSPARENT.mix(Rgba::white(0.06), hot))
            .role(Role::MenuItem, a.label)
            .child(crate::lucide(a.icon, 12.0, 2.0, ink))
            .child(
                text(a.label, size::XS.0, ink)
                    .line_height(size::XS.1)
                    .truncate()
                    .silent(),
            )
    }

    /// The card `card` drawn at the pointer, `grab` the point it was held by
    /// and its width, while the client says it is dragged.
    pub fn ghost(&self, card: &str, grab: (f32, f32, f32)) -> Option<El> {
        let (gx, gy, w) = grab;
        let (px, py) = self.ui.pointer()?;
        let placed = self.cards.placed();
        let at = placed.iter().position(|(id, _)| id == card)?;
        let filled = self.cards.card(at)?;
        let lane = self.lanes.iter().find(|l| l.value == placed[at].1)?;
        Some(
            div()
                .absolute(px - gx, py - gy)
                .w(w)
                .opacity(0.9)
                .silent()
                .child(self.card(lane, &filled, true, None)),
        )
    }
}

fn line() -> El {
    div().row().items_center().gap(8.0).min_w0()
}

/// A quiet line of text, cut short when it does not fit.
fn words(s: &str) -> El {
    text(s, 11.0, color::GRAY_500)
        .line_height(16.0)
        .truncate()
        .silent()
        .shrink0()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use vornui::{Ui, UiConfig};

    use super::*;

    const LANES: [Lane<'static>; 2] = [
        Lane {
            value: "todo",
            title: "To do",
            icon: Some("circle"),
            tone: Some(Tone::Idle),
        },
        Lane {
            value: "done",
            title: "Done",
            icon: Some("circle-check"),
            tone: Some(Tone::Good),
        },
    ];

    struct Deck(Vec<(String, String)>);

    impl Cards for Deck {
        fn placed(&self) -> &[(String, String)] {
            &self.0
        }
        fn card(&self, at: usize) -> Option<Card> {
            let (id, _) = self.0.get(at)?;
            Some(Card {
                id: id.clone(),
                key: format!("APP-{at}"),
                title: format!("Card {at}"),
                note: "Created Mar 5".into(),
                live: at == 1,
                agent: (at == 1).then(|| "claude".to_owned()),
                actions: vec![CardAction {
                    id: "delete",
                    label: "Delete",
                    icon: "trash-2",
                    danger: true,
                    confirm: Some("Delete this task permanently?"),
                }],
                ..Card::default()
            })
        }
    }

    fn deck(n: usize) -> Deck {
        Deck(
            (0..n)
                .map(|i| {
                    let lane = if i % 3 == 0 { "done" } else { "todo" };
                    (format!("c{i}"), lane.to_owned())
                })
                .collect(),
        )
    }

    #[derive(Default)]
    struct Binds {
        dragging: Option<(String, Option<String>)>,
        added: RefCell<Vec<String>>,
    }

    impl BoardBind for Binds {
        fn card(&self, card: &str) -> Id {
            card_id("b", card)
        }
        fn dragging(&self) -> Option<(String, Option<String>)> {
            self.dragging.clone()
        }
        fn add(&self, lane: &str, foot: bool) -> Id {
            self.added.borrow_mut().push(lane.to_owned());
            Id::new("add").child(lane).child(foot)
        }
        fn menu(&self, card: &str) -> Id {
            Id::new("menu").child(card)
        }
        fn menu_open(&self, _: Id) -> bool {
            false
        }
        fn action(&self, card: &str, a: &CardAction) -> Id {
            Id::new("action").child(card).child(a.id)
        }
        fn confirming(&self, _: Id) -> Option<(Id, Id)> {
            None
        }
    }

    fn board<'a>(ui: &'a Ui, cards: &'a Deck, bind: &'a Binds) -> Board<'a> {
        Board {
            ui,
            name: "b",
            lanes: &LANES,
            empty: Some("Nothing here"),
            cards,
            bind,
        }
    }

    fn frame(ui: &mut Ui, cards: &Deck, bind: &Binds) {
        ui.begin(color::SURFACE_BASE);
        let root = div()
            .col()
            .w(180.0)
            .h(40.0)
            .child(board(ui, cards, bind).view());
        ui.layout(root, (180.0, 40.0));
    }

    #[test]
    fn builds_only_the_cards_in_view_in_each_column() {
        let mut ui = Ui::cells(&UiConfig::system(1.0, 13.0));
        let (cards, bind) = (deck(600), Binds::default());
        for _ in 0..3 {
            frame(&mut ui, &cards, &bind);
        }
        let built = |ui: &Ui| {
            (0..600)
                .filter(|i| ui.rect_of(card_id("b", &format!("c{i}"))).is_some())
                .count()
        };
        assert!(
            ui.rect_of(card_id("b", "c1")).is_some(),
            "the first To do card"
        );
        assert!(
            ui.rect_of(card_id("b", "c0")).is_some(),
            "the first Done card"
        );
        assert!(built(&ui) < 60, "{} built", built(&ui));
        assert!(ui.rect_of(card_id("b", "c599")).is_none());
        let added = bind.added.borrow();
        assert!(added.iter().any(|l| l == "todo") && added.iter().any(|l| l == "done"));
    }

    #[test]
    fn an_empty_column_says_so_and_a_point_finds_its_column() {
        let mut ui = Ui::cells(&UiConfig::system(1.0, 13.0));
        let (cards, bind) = (deck(2), Binds::default());
        frame(&mut ui, &cards, &bind);
        frame(&mut ui, &cards, &bind);
        let done = ui.rect_of(column_id("b", "done")).unwrap();
        assert_eq!(
            column_at(
                &ui,
                "b",
                &LANES,
                done.x + done.w / 2.0,
                done.y + done.h / 2.0
            ),
            Some("done")
        );
        assert_eq!(column_at(&ui, "b", &LANES, -10.0, -10.0), None);
        assert_eq!(grouped(&LANES, cards.placed()), vec![vec![1], vec![0]]);
    }

    #[test]
    fn a_dragged_card_stays_built() {
        let mut ui = Ui::cells(&UiConfig::system(1.0, 13.0));
        let cards = deck(600);
        let bind = Binds {
            dragging: Some(("c598".into(), Some("todo".into()))),
            ..Binds::default()
        };
        frame(&mut ui, &cards, &bind);
        frame(&mut ui, &cards, &bind);
        assert!(
            ui.rect_of(card_id("b", "c598")).is_some(),
            "kept while dragged, far below the fold"
        );
    }

    #[test]
    fn a_native_backend_lists_each_column_as_a_section_of_its_cards() {
        let mut ui = Ui::native(&UiConfig::system(3.0, 15.0));
        let (cards, bind) = (deck(4), Binds::default());
        let el = board(&ui, &cards, &bind).view();
        ui.begin(Rgba::TRANSPARENT);
        ui.layout(el, (390.0, 844.0));
        let json = ui.native_tree().unwrap().to_json();
        for s in [
            "To do",
            "Done",
            "Card 1",
            "APP-1",
            "Created Mar 5",
            "section",
        ] {
            assert!(json.contains(s), "{s} in {json}");
        }
    }
}
