//! An extension's template laid out with the kit: one renderer for every
//! surface, so a template reads the same on pixels and on terminal cells.
//! The caller binds each control to an [`Id`] and decides what it does.

use std::collections::BTreeMap;

use vorn_view::{
    Action as Do, BoardColumn, Cond, Group, Kind, Node, Role as TextRole, Row, Source, Style,
    Template, Tone,
};
use vornui::native::Sem;
use vornui::theme::{color, radius, text as size};
use vornui::widgets::{self, Tone as ConfirmTone};
use vornui::{div, text, Action, El, Id, Place, Rgba, Sense, Ui};

use vornui::accesskit::Role;

/// What pressing a template's button or toggle asks for, its bindings filled.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Press {
    /// The extension method to call.
    pub call: Option<String>,
    /// A URL to open in the browser.
    pub open: Option<String>,
    /// A session to show.
    pub focus: Option<String>,
    /// Bring the main window forward.
    pub show: bool,
    /// Close the dialog or window the template is in.
    pub close: bool,
    /// Open another of the extension's window points, by id.
    pub window: Option<String>,
    /// Open the client's sign-in window for this key.
    pub signin: Option<String>,
    /// The row the control sits in.
    pub row: Option<String>,
    /// A toggle's new value.
    pub value: Option<bool>,
    /// Asked inline before the press runs.
    pub confirm: Option<String>,
}

/// How the caller binds a template's controls.
pub trait Bind {
    /// The id of a button or toggle; `slot` tells controls of one template apart.
    fn press(&self, slot: u64, press: Press) -> Id;
    /// The id of a one-line field calling `call` on Return, which holds `value` until typed in.
    fn field(&self, slot: u64, row: Option<&str>, call: &str, value: &str) -> Id;
    /// The confirm's buttons when button `id` asks before it runs and is asking now.
    fn confirming(&self, id: Id) -> Option<(Id, Id)>;
    /// The id of a `more` node's button, which opens and closes its menu.
    fn menu(&self, slot: u64, row: Option<&str>) -> Id;
    /// Whether the more menu of button `id` is open.
    fn menu_open(&self, id: Id) -> bool;
    /// The id of row `row`'s card on a board of `rows`; a click on it opens
    /// the row, through the board's `open` method when it has one.
    fn card(&self, rows: &str, row: &str, open: Option<&str>) -> Id {
        let _ = open;
        crate::board::card_id(rows, row)
    }
    /// The row being dragged on a board of `rows`, and the column it is over.
    fn dragging(&self, rows: &str) -> Option<(String, Option<String>)> {
        let _ = rows;
        None
    }
    /// An icon the client draws for a name not in the kit's set, as
    /// `agent:<key>` for an agent's mark; `None` draws the kit's.
    fn icon(&self, name: &str, size: f32) -> Option<El> {
        let _ = (name, size);
        None
    }
    /// The id an entry's row hovers by, so its actions show.
    fn hover(&self, slot: u64, row: Option<&str>) -> Id {
        Id::new("template-entry").child(slot).child(row)
    }
}

/// How wide a `number` text's column is, so figures line up down a list.
pub const NUMBER_WIDTH: f32 = 80.0;

/// The colour a tone draws in.
pub fn tone(t: Tone) -> Rgba {
    match t {
        Tone::Good => color::GREEN_400,
        Tone::Bad => color::RED_400,
        Tone::Warn => color::AMBER_400,
        Tone::Busy => color::BRONZO,
        Tone::Muted => color::INK_FAINT,
        Tone::Idle => color::STATUS_SLATE,
        Tone::Active => color::STATUS_BLUE,
    }
}

/// A tone's name, as a native backend reads it.
pub fn tone_name(t: Tone) -> &'static str {
    match t {
        Tone::Good => "good",
        Tone::Bad => "bad",
        Tone::Warn => "warn",
        Tone::Busy => "busy",
        Tone::Muted => "muted",
        Tone::Idle => "idle",
        Tone::Active => "active",
    }
}

/// Template `t` with `row` in scope (a session's row, for a per-session place).
pub fn view(ui: &Ui, t: &Template, row: Option<&Row>, src: &dyn Source, bind: &dyn Bind) -> El {
    let cx = Cx {
        ui,
        src,
        bind,
        column: None,
        soft: false,
        reveal: None,
    };
    cx.node(&t.view, row, 1).unwrap_or_else(div)
}

/// Template `t` as a page: its toolbar, when the view is one or its root
/// column holds one, apart for the page's header, and the rest.
pub fn page(
    ui: &Ui,
    t: &Template,
    row: Option<&Row>,
    src: &dyn Source,
    bind: &dyn Bind,
) -> (Option<El>, Option<El>) {
    let cx = Cx {
        ui,
        src,
        bind,
        column: None,
        soft: false,
        reveal: None,
    };
    let root = &t.view;
    match &root.kind {
        Kind::Toolbar(_) => (cx.node(root, row, 1), None),
        Kind::Column(kids) if cx.holds(root.when.as_ref(), row) => {
            let at = kids.iter().position(|k| matches!(k.kind, Kind::Toolbar(_)));
            let Some(at) = at else {
                return (None, cx.node(root, row, 1));
            };
            let bar = cx.node(&kids[at], row, child_slot(1, at));
            let body = kids
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != at)
                .filter_map(|(i, k)| cx.node(k, row, child_slot(1, i)));
            (bar, Some(column().children(body)))
        }
        _ => (None, cx.node(root, row, 1)),
    }
}

fn column() -> El {
    div().col().gap(6.0).min_w0()
}

/// The box drawn in place of a template with errors: which place, and each error with its line.
pub fn invalid(place: &str, errors: &[String]) -> El {
    let mut b = div()
        .col()
        .gap(4.0)
        .px(8.0)
        .py(6.0)
        .rounded(radius::SM)
        .border(1.0, color::DANGER)
        .role(Role::Alert, place)
        .child(
            text(
                format!("{place} has template errors"),
                size::XS.0,
                color::RED_300,
            )
            .line_height(size::XS.1)
            .weight(600)
            .wrapping(),
        );
    for e in errors {
        b = b.child(
            text(e, 11.0, color::RED_300)
                .mono()
                .line_height(16.0)
                .wrapping(),
        );
    }
    b
}

pub(crate) struct Cx<'a> {
    pub(crate) ui: &'a Ui,
    pub(crate) src: &'a dyn Source,
    pub(crate) bind: &'a dyn Bind,
    /// In a board's card: the column it is in.
    pub(crate) column: Option<&'a BoardColumn>,
    /// Tones drawn muted, as a board shows many at once.
    pub(crate) soft: bool,
    /// In a card: how far its controls show, as they show on hover.
    pub(crate) reveal: Option<f32>,
}

pub(crate) fn child_slot(slot: u64, i: usize) -> u64 {
    slot.wrapping_mul(31).wrapping_add(i as u64 + 1)
}

/// Every slot a node of `t` takes, as [`Bind::press`] and [`Bind::field`] are given them.
pub fn slots(t: &Template) -> Vec<u64> {
    fn walk(n: &Node, slot: u64, out: &mut Vec<u64>) {
        out.push(slot);
        match &n.kind {
            Kind::Column(kids)
            | Kind::Row(kids)
            | Kind::More(kids)
            | Kind::Toolbar(kids)
            | Kind::Entry { children: kids, .. }
            | Kind::Section { children: kids, .. } => {
                for (i, k) in kids.iter().enumerate() {
                    walk(k, child_slot(slot, i), out);
                }
            }
            Kind::List { item, .. } => walk(item, child_slot(slot, 0), out),
            Kind::Board(b) => {
                walk(&b.card, child_slot(slot, 0), out);
                for i in 0..b.columns.len() * 2 {
                    out.push(child_slot(slot, i + 1));
                }
            }
            Kind::Choice { .. } => {
                for i in 0..vorn_view::MAX_LIST {
                    out.push(child_slot(slot, i));
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(&t.view, 1, &mut out);
    out
}

impl Cx<'_> {
    fn holds(&self, when: Option<&Cond>, row: Option<&Row>) -> bool {
        when.is_none_or(|c| c.holds_in(row, self.src, self.column))
    }

    /// The colour `t` draws in here.
    pub(crate) fn tone_of(&self, t: Tone) -> Rgba {
        match t {
            Tone::Good if self.soft => color::STATUS_SAGE,
            other => tone(other),
        }
    }

    fn kids(&self, kids: &[Node], row: Option<&Row>, slot: u64) -> Vec<El> {
        kids.iter()
            .enumerate()
            .filter_map(|(i, k)| self.node(k, row, child_slot(slot, i)))
            .collect()
    }

    pub(crate) fn node(&self, n: &Node, row: Option<&Row>, slot: u64) -> Option<El> {
        if !self.holds(n.when.as_ref(), row) {
            return None;
        }
        let fill = |t: &vorn_view::Text| t.fill_in(row, self.src, self.column);
        Some(match &n.kind {
            Kind::Column(kids) => column()
                .sem(Sem::Column)
                .children(self.kids(kids, row, slot)),
            Kind::Toolbar(kids) => div()
                .row()
                .items_center()
                .justify_end()
                .gap(8.0)
                .shrink0()
                .children(self.kids(kids, row, slot)),
            Kind::Entry {
                title,
                detail,
                children,
            } => self.entry_row(slot, &fill(title), detail.as_ref().map(fill), children, row),
            Kind::Notice { value, tone: t } => self.notice(&fill(value), *t),
            Kind::Row(kids) => div()
                .sem(Sem::Row)
                .row()
                .items_center()
                .gap(8.0)
                .min_w0()
                .children(self.kids(kids, row, slot)),
            Kind::More(items) => return self.more(slot, items, row),
            Kind::Section { title, children } => {
                let title = fill(title);
                div()
                    .sem(Sem::Section(title.clone()))
                    .col()
                    .gap(12.0)
                    .min_w0()
                    .child(
                        text(title.to_uppercase(), size::XS.0, color::GRAY_500)
                            .line_height(size::XS.1)
                            .weight(size::MEDIUM)
                            .tracking(0.05)
                            .truncate()
                            .role(Role::Heading, title.as_str())
                            .sem(Sem::Hidden),
                    )
                    .children(self.kids(children, row, slot))
            }
            Kind::Text {
                value,
                role,
                grow,
                wrap,
            } => {
                let t = if *wrap {
                    wrapped(&fill(value), *role)
                } else {
                    words(&fill(value), *role)
                };
                if *role == TextRole::Number {
                    div().row().justify_end().w(NUMBER_WIDTH).shrink0().child(t)
                } else if *grow {
                    div().grow().min_w0().child(t)
                } else {
                    t.shrink0()
                }
            }
            Kind::Icon {
                name,
                tone: t,
                column_tone,
            } => {
                let t = if *column_tone {
                    self.column.and_then(|c| c.tone)
                } else {
                    *t
                };
                let ink = t.map_or(color::GRAY_400, |t| self.tone_of(t));
                let name = fill(name);
                let drawn = match self.bind.icon(&name, 14.0) {
                    Some(drawn) => drawn.shrink0(),
                    None => crate::lucide(&name, 14.0, 2.0, ink).shrink0(),
                };
                drawn.sem(Sem::Icon {
                    name,
                    tone: t.map_or("", tone_name),
                    svg: None,
                })
            }
            Kind::Status { value, tones } => self.status(&fill(value), tones),
            Kind::Badge { value, tones } => {
                let v = fill(value);
                let (ink, name) = if tones.is_empty() {
                    (color::GRAY_300, "")
                } else {
                    let t = Tone::of(tones, &v);
                    (self.tone_of(t), tone_name(t))
                };
                widgets::badge(&v, ink, Rgba::white(0.06))
                    .role(Role::Label, v.as_str())
                    .sem(Sem::Badge {
                        value: v,
                        tone: name,
                    })
            }
            Kind::Button {
                label,
                action,
                confirm,
                style,
                icon,
            } => {
                let press = filled(action, row, self.src, confirm.as_ref().map(fill));
                self.button(slot, &fill(label), press, *style, icon.as_deref())
            }
            Kind::Toggle { value, label, call } => {
                let on = vorn_view::truthy(&fill(value));
                let name = label.as_ref().map(fill).unwrap_or_default();
                let id = self.bind.press(
                    slot,
                    Press {
                        call: Some(call.clone()),
                        row: row.map(|r| r.id.clone()),
                        value: Some(!on),
                        ..Press::default()
                    },
                );
                let switch = widgets::toggle(self.ui, id, &name, on, false).shrink0();
                if name.is_empty() {
                    switch
                } else {
                    div()
                        .row()
                        .items_center()
                        .gap(8.0)
                        .min_w0()
                        .child(
                            div()
                                .grow()
                                .min_w0()
                                .child(words(&name, TextRole::Body).sem(Sem::Hidden)),
                        )
                        .child(switch)
                }
            }
            Kind::Field {
                value,
                placeholder,
                call,
                secret,
            } => {
                let held = value.as_ref().map(fill).unwrap_or_default();
                let hint = placeholder.as_ref().map(fill).unwrap_or_default();
                let id = self
                    .bind
                    .field(slot, row.map(|r| r.id.as_str()), call, &held);
                let input = widgets::text_input(self.ui, id, &hint, &hint)
                    .grow()
                    .min_w0();
                if *secret {
                    input.secure()
                } else {
                    input
                }
            }
            Kind::Choice {
                rows,
                label,
                selected,
                call,
            } => self.choice(slot, rows, label, selected.as_ref(), call),
            Kind::List {
                rows,
                item,
                empty,
                only,
                group,
                loading,
            } => {
                let boxed = matches!(item.kind, Kind::Entry { .. });
                if let Some(l) = loading.as_deref() {
                    if l.0.holds_in(row, self.src, self.column) {
                        return Some(state(&fill(&l.1), boxed));
                    }
                }
                let only = only.as_ref().map(|(f, is)| (f.as_str(), fill(is)));
                let all: Vec<&Row> = self
                    .src
                    .rows(rows)
                    .iter()
                    .filter(|r| only.as_ref().is_none_or(|(f, is)| r.text(f) == *is))
                    .take(vorn_view::MAX_LIST)
                    .collect();
                if all.is_empty() {
                    let e = empty.as_ref().map(fill).unwrap_or_default();
                    if e.is_empty() {
                        return None;
                    }
                    return Some(state(&e, boxed));
                }
                match group.as_deref() {
                    None => self.items(all, item, slot, boxed),
                    Some(g) => self.groups(all, g, item, slot, boxed),
                }
            }
            Kind::Divider => div()
                .sem(Sem::Divider)
                .w_full()
                .h(0.0)
                .border_top(1.0, Rgba::white(0.08)),
            Kind::Board(b) => crate::board::view(self, b, slot),
        })
    }

    /// Each option of a choice as a chip, the chosen ones filled: on cells
    /// as on pixels, one press per option.
    fn choice(
        &self,
        slot: u64,
        rows: &str,
        label: &vorn_view::Text,
        selected: Option<&Cond>,
        call: &str,
    ) -> El {
        let mut line = div()
            .sem(Sem::Choice)
            .row()
            .wrap_children()
            .gap(4.0)
            .min_w0();
        for (i, r) in self
            .src
            .rows(rows)
            .iter()
            .take(vorn_view::MAX_LIST)
            .enumerate()
        {
            let name = label.fill(Some(r), self.src);
            let on = selected.is_some_and(|c| c.holds(Some(r), self.src));
            let id = self.bind.press(
                child_slot(slot, i),
                Press {
                    call: Some(call.to_owned()),
                    row: Some(r.id.clone()),
                    ..Press::default()
                },
            );
            let hot = self.ui.hover(id);
            let ink = if on {
                color::WHITE
            } else {
                color::GRAY_400.mix(color::WHITE, hot)
            };
            let fill = if on {
                Rgba::white(0.14)
            } else {
                Rgba::TRANSPARENT.mix(Rgba::white(0.06), hot)
            };
            line = line.child(
                div()
                    .id(id)
                    .on(Action::Click)
                    .px(8.0)
                    .py(3.0)
                    .rounded(radius::SM)
                    .border(1.0, Rgba::white(if on { 0.25 } else { 0.10 }))
                    .bg(fill)
                    .ring(radius::SM)
                    .role(Role::RadioButton, name.as_str())
                    .toggled(on)
                    .child(
                        text(&name, size::XS.0, ink)
                            .line_height(size::XS.1)
                            .silent(),
                    ),
            );
        }
        line
    }

    fn status(&self, value: &str, tones: &BTreeMap<String, Tone>) -> El {
        let t = Tone::of(tones, value);
        let ink = self.tone_of(t);
        let dot = if self.ui.is_cells() {
            text("●", 12.0, ink).line_height(16.0).silent()
        } else {
            div().size(6.0, 6.0).rounded(3.0).bg(ink)
        };
        dot.shrink0().role(Role::Status, value).sem(Sem::Status {
            value: value.to_owned(),
            tone: tone_name(t),
        })
    }

    /// A "more" button and, while open, its menu of the buttons that hold;
    /// nothing when none does. An entry that asks first asks under the
    /// button, as its menu closes on the press.
    fn more(&self, slot: u64, items: &[Node], row: Option<&Row>) -> Option<El> {
        let menu = self.bind.menu(slot, row.map(|r| r.id.as_str()));
        let mut entries = Vec::new();
        let mut asking = None;
        for (i, item) in items.iter().enumerate() {
            let Kind::Button {
                label,
                action,
                confirm,
                style,
                icon,
            } = &item.kind
            else {
                continue;
            };
            if !self.holds(item.when.as_ref(), row) {
                continue;
            }
            let label = label.fill(row, self.src);
            let ask = confirm.as_ref().map(|c| c.fill(row, self.src));
            let press = filled(action, row, self.src, ask.clone());
            let id = self.bind.press(child_slot(slot, i), press);
            let sem = Sem::Button {
                style: style_name(*style),
                confirm: ask.clone(),
                icon: icon.clone(),
            };
            if let (Some(message), Some(ids)) = (ask, self.bind.confirming(id)) {
                asking = Some((message, label.clone(), *style, ids));
            }
            entries.push(self.entry(id, &label, *style, icon.as_deref()).sem(sem));
        }
        if entries.is_empty() {
            return None;
        }
        if self.ui.is_native() {
            return Some(div().id(menu).sem(Sem::More).children(entries));
        }
        let hot = self.ui.hover(menu);
        let open = self.bind.menu_open(menu);
        let shown = match self.reveal {
            Some(r) if !open => r.max(hot),
            _ => 1.0,
        };
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
            Some((message, label, style, (ok, cancel))) => {
                let t = if style == Style::Danger {
                    ConfirmTone::Danger
                } else {
                    ConfirmTone::Neutral
                };
                widgets::confirm(self.ui, b, &message, (ok, &label), cancel, t)
            }
            None => b,
        })
    }

    /// One entry of a `more` menu.
    fn entry(&self, id: Id, label: &str, style: Style, icon: Option<&str>) -> El {
        let hot = self.ui.hover(id);
        let ink = match style {
            Style::Danger => color::RED_400.mix(color::RED_300, hot),
            _ => color::GRAY_300.mix(color::WHITE, hot),
        };
        let mut e = div()
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
            .role(Role::MenuItem, label);
        if let Some(name) = icon {
            e = e.child(crate::lucide(name, 12.0, 2.0, ink));
        }
        e.child(
            text(label, size::XS.0, ink)
                .line_height(size::XS.1)
                .truncate()
                .silent(),
        )
    }

    fn button(&self, slot: u64, label: &str, press: Press, style: Style, icon: Option<&str>) -> El {
        self.button_parts(slot, label, press, style, icon).1
    }

    /// A button and its id, as an entry needs to know when it is hovered or focused.
    fn button_parts(
        &self,
        slot: u64,
        label: &str,
        press: Press,
        style: Style,
        icon: Option<&str>,
    ) -> (Id, El) {
        let ask = press.confirm.clone();
        let sem = Sem::Button {
            style: style_name(style),
            confirm: ask.clone(),
            icon: icon.map(str::to_owned),
        };
        let id = self.bind.press(slot, press);
        let hot = self.ui.hover(id);
        let (rest, lit) = match style {
            Style::Danger => (color::RED_400, color::RED_300),
            Style::Primary => (color::WHITE, color::WHITE),
            Style::Plain => (color::GRAY_300, color::WHITE),
        };
        let ink = rest.mix(lit, hot);
        let fill = match style {
            Style::Primary => Rgba::white(0.10).mix(Rgba::white(0.15), hot),
            _ => Rgba::TRANSPARENT.mix(Rgba::white(0.06), hot),
        };
        let mut b = div()
            .id(id)
            .on(Action::Click)
            .row()
            .items_center()
            .gap(4.0)
            .px(8.0)
            .py(4.0)
            .shrink0()
            .rounded(radius::SM)
            .bg(fill)
            .ring(radius::SM)
            .role(Role::Button, label)
            .sem(sem);
        if let Some(name) = icon {
            b = b.child(crate::lucide(name, 12.0, 2.0, ink));
        }
        if !label.is_empty() {
            b = b.child(
                text(label, size::XS.0, ink)
                    .line_height(size::XS.1)
                    .weight(size::MEDIUM)
                    .silent(),
            );
        }
        let b = match (ask, self.bind.confirming(id)) {
            (Some(message), Some((ok, cancel))) => {
                let t = if style == Style::Danger {
                    ConfirmTone::Danger
                } else {
                    ConfirmTone::Neutral
                };
                widgets::confirm(self.ui, b, &message, (ok, label), cancel, t)
            }
            _ => b,
        };
        (id, b)
    }

    /// A list's items; entries share one box, a hairline between each.
    fn items(&self, rows: Vec<&Row>, item: &Node, slot: u64, boxed: bool) -> El {
        let els = rows
            .into_iter()
            .filter_map(|r| self.node(item, Some(r), child_slot(slot, 0)));
        if !boxed {
            return div().sem(Sem::List).col().gap(4.0).min_w0().children(els);
        }
        div()
            .sem(Sem::List)
            .col()
            .min_w0()
            .rounded(radius::SM)
            .border(1.0, Rgba::white(0.06))
            .bg(Rgba::white(0.02))
            .clip()
            .children(els.enumerate().map(|(i, e)| {
                if i == 0 {
                    e
                } else {
                    e.border_top(1.0, Rgba::white(0.06))
                }
            }))
    }

    /// A list's rows in groups by `g.by`, in the order first published, each under its heading.
    fn groups(&self, rows: Vec<&Row>, g: &Group, item: &Node, slot: u64, boxed: bool) -> El {
        let mut groups: Vec<(String, Vec<&Row>)> = Vec::new();
        for r in rows {
            let key = r.text(&g.by);
            match groups.iter_mut().find(|(k, _)| *k == key) {
                Some((_, rs)) => rs.push(r),
                None => groups.push((key, vec![r])),
            }
        }
        div()
            .col()
            .gap(24.0)
            .min_w0()
            .children(groups.into_iter().map(|(_, rs)| {
                let first = rs.first().copied();
                let heading = g.heading.fill_in(first, self.src, self.column);
                let aside = g
                    .aside
                    .as_ref()
                    .map(|a| a.fill_in(first, self.src, self.column))
                    .filter(|a| !a.is_empty());
                let head = div()
                    .row()
                    .items_center()
                    .gap(8.0)
                    .px(if boxed { 12.0 } else { 0.0 })
                    .min_w0()
                    .child(
                        div().grow().min_w0().child(
                            text(&heading, size::XS.0, color::GRAY_400)
                                .line_height(size::XS.1)
                                .weight(size::MEDIUM)
                                .truncate()
                                .role(Role::Heading, heading.as_str()),
                        ),
                    )
                    .child_if(aside.map(|a| {
                        text(a, size::XS.0, color::GRAY_500)
                            .line_height(size::XS.1)
                            .shrink0()
                            .silent()
                    }));
                div()
                    .col()
                    .gap(8.0)
                    .min_w0()
                    .role(Role::Group, heading.as_str())
                    .child(head)
                    .child(self.items(rs, item, slot, boxed))
            }))
    }

    /// One entry: icon, title over detail, trailing text, and actions that
    /// show while the row is hovered or one of them has focus.
    fn entry_row(
        &self,
        slot: u64,
        title: &str,
        detail: Option<String>,
        kids: &[Node],
        row: Option<&Row>,
    ) -> El {
        let id = self.bind.hover(slot, row.map(|r| r.id.as_str()));
        let hot = self.ui.hover(id);
        let cells = self.ui.is_cells();
        let (mut lead, mut dots, mut badges, mut trail, mut acts, mut last) = (
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        for (i, k) in kids.iter().enumerate() {
            if !self.holds(k.when.as_ref(), row) {
                continue;
            }
            let at = child_slot(slot, i);
            match &k.kind {
                Kind::Button {
                    label,
                    action,
                    confirm,
                    style,
                    icon,
                } => {
                    let fill = |t: &vorn_view::Text| t.fill_in(row, self.src, self.column);
                    let press = filled(action, row, self.src, confirm.as_ref().map(fill));
                    let (b, el) =
                        self.button_parts(at, &fill(label), press, *style, icon.as_deref());
                    let pinned = *style == Style::Primary
                        || self.ui.key_focused(b)
                        || self.bind.confirming(b).is_some();
                    // A terminal has no hover to reveal it, so there the
                    // shortcut gives its room back; the more menu still has it.
                    if pinned {
                        acts.push(el);
                    } else if !cells {
                        acts.push(el.opacity(hot.max(self.ui.hover(b))));
                    }
                }
                Kind::Icon { .. } => {
                    lead = self.node(k, row, at).map(|icon| {
                        div()
                            .size(28.0, 28.0)
                            .shrink0()
                            .center()
                            .rounded(radius::SM)
                            .bg(Rgba::white(0.04))
                            .child(icon)
                    });
                }
                Kind::Status { .. } => dots.extend(self.node(k, row, at)),
                Kind::Badge { .. } => badges.extend(self.node(k, row, at)),
                Kind::Text { .. } => trail.extend(self.node(k, row, at)),
                _ => last.extend(self.node(k, row, at)),
            }
        }
        let two = detail.as_deref().is_some_and(|d| !d.is_empty());
        let line = div()
            .row()
            .items_center()
            .gap(8.0)
            .min_w0()
            .children(dots)
            .child(
                text(title, 13.0, color::GRAY_100)
                    .line_height(18.0)
                    .weight(size::MEDIUM)
                    .truncate()
                    .silent(),
            )
            .children(badges);
        let middle = div()
            .col()
            .grow()
            .min_w0()
            .child(line)
            .child_if(detail.filter(|_| two).map(|d| {
                text(d, size::XS.0, color::GRAY_500)
                    .line_height(size::XS.1)
                    .truncate()
                    .silent()
            }));
        let mut e = div()
            .id(id)
            .sense(Sense::HOVER)
            .row()
            .items_center()
            .gap(12.0)
            .px(12.0)
            .min_w0()
            .bg(Rgba::TRANSPARENT.mix(Rgba::white(0.03), hot))
            .role(Role::ListItem, title)
            .child_if(lead.filter(|_| !cells))
            .child(middle);
        if !cells {
            e = e.min_h(if two { 52.0 } else { 40.0 }).py(8.0);
        }
        // Actions that only show on hover sit before the numbers, so a row
        // without one keeps its number in the same column as its neighbours.
        let group = |els: Vec<El>| div().row().items_center().gap(4.0).shrink0().children(els);
        if !acts.is_empty() {
            e = e.child(group(acts));
        }
        e = e.children(trail);
        if !last.is_empty() {
            e = e.child(group(last));
        }
        e
    }

    /// A line that stands out, as an error says what happened.
    fn notice(&self, value: &str, t: Option<Tone>) -> El {
        let (icon, ink) = match t {
            Some(Tone::Bad) => ("circle-alert", color::RED_400),
            Some(Tone::Warn) => ("triangle-alert", color::AMBER_400),
            Some(other) => ("info", self.tone_of(other)),
            None => ("info", color::GRAY_500),
        };
        div()
            .row()
            .items_start()
            .gap(8.0)
            .px(12.0)
            .py(8.0)
            .min_w0()
            .rounded(radius::SM)
            .border(1.0, Rgba::white(0.08))
            .bg(color::SURFACE_SUNKEN)
            .role(Role::Status, value)
            .child(crate::lucide(icon, 14.0, 2.0, ink).mt(1.0).shrink0())
            .child(
                div().grow().min_w0().child(
                    text(value, size::XS.0, color::GRAY_300)
                        .line_height(size::XS.1)
                        .wrapping()
                        .silent(),
                ),
            )
    }
}

/// What a list says in place of its rows: empty, or still loading.
fn state(copy: &str, boxed: bool) -> El {
    let t = text(copy, size::XS.0, color::GRAY_500)
        .line_height(size::XS.1)
        .wrapping()
        .silent();
    if !boxed {
        return t;
    }
    div()
        .px(12.0)
        .py(16.0)
        .min_w0()
        .rounded(radius::SM)
        .border(1.0, Rgba::white(0.06))
        .role(Role::Status, copy)
        .child(t)
}

fn style_name(s: Style) -> &'static str {
    match s {
        Style::Plain => "plain",
        Style::Primary => "primary",
        Style::Danger => "danger",
    }
}

fn role_name(role: TextRole) -> &'static str {
    match role {
        TextRole::Title => "title",
        TextRole::Body => "body",
        TextRole::Secondary => "secondary",
        TextRole::Mono => "mono",
        TextRole::Number => "number",
    }
}

/// Text in a role, on as many lines as it takes.
fn wrapped(s: &str, role: TextRole) -> El {
    styled(s, role)
        .wrapping()
        .silent()
        .sem(Sem::Text(role_name(role)))
}

fn styled(s: &str, role: TextRole) -> El {
    match role {
        TextRole::Title => text(s, 13.0, color::GRAY_200)
            .line_height(18.0)
            .weight(size::SEMIBOLD),
        TextRole::Body => text(s, size::XS.0, color::GRAY_300).line_height(size::XS.1),
        TextRole::Secondary => text(s, 11.0, color::GRAY_500).line_height(16.0),
        TextRole::Mono => text(s, 11.0, color::GRAY_400).mono().line_height(16.0),
        TextRole::Number => text(s, size::XS.0, color::GRAY_300).line_height(size::XS.1),
    }
}

fn words(s: &str, role: TextRole) -> El {
    styled(s, role)
        .truncate()
        .silent()
        .sem(Sem::Text(role_name(role)))
}

fn filled(a: &Do, row: Option<&Row>, src: &dyn Source, confirm: Option<String>) -> Press {
    Press {
        call: a.call.clone(),
        open: a.open.as_ref().map(|t| t.fill(row, src)),
        focus: a.focus.as_ref().map(|t| t.fill(row, src)),
        show: a.show,
        close: a.close,
        window: a.window.clone(),
        signin: a.signin.as_ref().map(|t| t.fill(row, src)),
        row: row.map(|r| r.id.clone()),
        value: None,
        confirm,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use vorn_view::{file, Options};
    use vornui::UiConfig;

    const EVERY: &str = r#"schema "vorn:view@1"
rows "items" {
    name string
    on bool
    state enum pass fail
}
column {
    section "All" {
        row {
            icon "check" tone=good
            badge "3"
            text "Every node" role=secondary
        }
        list "items" {
            row {
                status "{row.state}" pass=good fail=bad
                toggle "{row.on}" label="{row.name}" call="flip"
                button "Run" call="run" style=primary
                more {
                    button "Drop" call="drop" style=danger confirm="Drop {row.name}?"
                }
            }
        }
        divider
        choice "items" label="{row.name}" selected="{row.on}" call="pick"
        field placeholder="Key" call="key" secret=#true
    }
}
"#;

    struct Rows(Vec<Row>);

    impl Source for Rows {
        fn rows(&self, _: &str) -> &[Row] {
            &self.0
        }
    }

    #[derive(Default)]
    struct Ids(Cell<u64>);

    impl Ids {
        fn next(&self) -> Id {
            self.0.set(self.0.get() + 1);
            Id::new("bind").child(self.0.get())
        }
    }

    impl Bind for Ids {
        fn press(&self, _: u64, _: Press) -> Id {
            self.next()
        }
        fn field(&self, _: u64, _: Option<&str>, _: &str, _: &str) -> Id {
            self.next()
        }
        fn confirming(&self, _: Id) -> Option<(Id, Id)> {
            None
        }
        fn menu(&self, _: u64, _: Option<&str>) -> Id {
            self.next()
        }
        fn menu_open(&self, _: Id) -> bool {
            false
        }
    }

    #[test]
    fn the_native_tree_has_a_node_for_every_template_node() {
        let t = file::parse("views/every.kdl", EVERY, Options::default()).unwrap();
        let row = |id: &str, on: bool| {
            Row::from_value(&serde_json::json!({ "id": id, "name": id, "on": on, "state": "pass" }))
                .unwrap()
        };
        let src = Rows(vec![row("a", true), row("b", false)]);
        let mut ui = Ui::native(&UiConfig::system(3.0, 15.0));
        let el = view(&ui, &t, None, &src, &Ids::default());
        ui.begin(Rgba::TRANSPARENT);
        ui.layout(el, (390.0, 844.0));
        let tree = ui.native_tree().unwrap();
        let all = tree.walk();
        let kinds: Vec<_> = all.iter().map(|n| n.kind).collect();
        // A board is drawn whole: native platforms show tasks on their own screens.
        for k in vorn_view::NODES.iter().filter(|k| **k != "board") {
            assert!(kinds.contains(k), "no {k} node in {kinds:?}");
        }
        let toggle = all.iter().find(|n| n.kind == "toggle").unwrap();
        assert_eq!((toggle.label.as_str(), toggle.on), ("a", Some(true)));
        let field = all.iter().find(|n| n.kind == "field").unwrap();
        assert!(field.secret && field.label == "Key");
        let drop = all.iter().find(|n| n.label == "Drop").unwrap();
        assert_eq!(
            (drop.style, drop.confirm.as_deref()),
            ("danger", Some("Drop a?"))
        );
        let options: Vec<_> = all.iter().filter(|n| n.kind == "option").collect();
        assert_eq!(options.len(), 2);
        assert_eq!(options[0].on, Some(true));
        let section = all.iter().find(|n| n.kind == "section").unwrap();
        assert_eq!(section.label, "All");
        assert!(
            !all.iter().any(|n| n.kind == "text" && n.label == "All"),
            "a section's title is the section's, not a text of its own"
        );
    }
}
