//! An extension's template laid out with the kit: one renderer for every
//! surface, so a template reads the same on pixels and on terminal cells.
//! The caller binds each control to an [`Id`] and decides what it does.

use std::collections::BTreeMap;

use vorn_view::{
    Action as Do, Cond, Kind, Node, Role as TextRole, Row, Source, Style, Template, Tone,
};
use vornui::theme::{color, radius, text as size};
use vornui::widgets::{self, Tone as ConfirmTone};
use vornui::{div, text, Action, El, Id, Place, Rgba, Ui};

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
}

/// The colour a tone draws in.
pub fn tone(t: Tone) -> Rgba {
    match t {
        Tone::Good => color::GREEN_400,
        Tone::Bad => color::RED_400,
        Tone::Warn => color::AMBER_400,
        Tone::Busy => color::BRONZO,
        Tone::Muted => color::INK_FAINT,
    }
}

/// Template `t` with `row` in scope (a session's row, for a per-session place).
pub fn view(ui: &Ui, t: &Template, row: Option<&Row>, src: &dyn Source, bind: &dyn Bind) -> El {
    let cx = Cx { ui, src, bind };
    cx.node(&t.view, row, 1).unwrap_or_else(div)
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

struct Cx<'a> {
    ui: &'a Ui,
    src: &'a dyn Source,
    bind: &'a dyn Bind,
}

fn child_slot(slot: u64, i: usize) -> u64 {
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
            | Kind::Section { children: kids, .. } => {
                for (i, k) in kids.iter().enumerate() {
                    walk(k, child_slot(slot, i), out);
                }
            }
            Kind::List { item, .. } => walk(item, child_slot(slot, 0), out),
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
        when.is_none_or(|c| c.holds(row, self.src))
    }

    fn kids(&self, kids: &[Node], row: Option<&Row>, slot: u64) -> Vec<El> {
        kids.iter()
            .enumerate()
            .filter_map(|(i, k)| self.node(k, row, child_slot(slot, i)))
            .collect()
    }

    fn node(&self, n: &Node, row: Option<&Row>, slot: u64) -> Option<El> {
        if !self.holds(n.when.as_ref(), row) {
            return None;
        }
        let fill = |t: &vorn_view::Text| t.fill(row, self.src);
        Some(match &n.kind {
            Kind::Column(kids) => div()
                .col()
                .gap(6.0)
                .min_w0()
                .children(self.kids(kids, row, slot)),
            Kind::Row(kids) => div()
                .row()
                .items_center()
                .gap(8.0)
                .min_w0()
                .children(self.kids(kids, row, slot)),
            Kind::More(items) => return self.more(slot, items, row),
            Kind::Section { title, children } => div()
                .col()
                .gap(8.0)
                .min_w0()
                .child(
                    text(fill(title), size::SM.0, color::GRAY_200)
                        .line_height(size::SM.1)
                        .weight(size::SEMIBOLD)
                        .truncate()
                        .silent(),
                )
                .children(self.kids(children, row, slot)),
            Kind::Text { value, role, grow } => {
                let t = words(&fill(value), *role);
                if *grow {
                    div().grow().min_w0().child(t)
                } else {
                    t.shrink0()
                }
            }
            Kind::Icon { name, tone: t } => {
                let ink = t.map_or(color::GRAY_400, tone);
                crate::lucide(&fill(name), 14.0, 2.0, ink).shrink0()
            }
            Kind::Status { value, tones } => self.status(&fill(value), tones),
            Kind::Badge { value, tones } => {
                let v = fill(value);
                let ink = if tones.is_empty() {
                    color::GRAY_300
                } else {
                    tone(Tone::of(tones, &v))
                };
                widgets::badge(&v, ink, Rgba::white(0.06)).role(Role::Label, v.as_str())
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
                        .child(div().grow().min_w0().child(words(&name, TextRole::Body)))
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
            } => {
                let only = only.as_ref().map(|(f, is)| (f.as_str(), fill(is)));
                let all: Vec<&Row> = self
                    .src
                    .rows(rows)
                    .iter()
                    .filter(|r| only.as_ref().is_none_or(|(f, is)| r.text(f) == *is))
                    .collect();
                if all.is_empty() {
                    let e = empty.as_ref().map(fill).unwrap_or_default();
                    if e.is_empty() {
                        return None;
                    }
                    return Some(words(&e, TextRole::Secondary));
                }
                div().col().gap(4.0).min_w0().children(
                    all.into_iter()
                        .take(vorn_view::MAX_LIST)
                        .filter_map(|r| self.node(item, Some(r), child_slot(slot, 0))),
                )
            }
            Kind::Divider => div().w_full().h(0.0).border_top(1.0, Rgba::white(0.08)),
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
        let mut line = div().row().wrap_children().gap(4.0).min_w0();
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
        let ink = tone(Tone::of(tones, value));
        let dot = if self.ui.is_cells() {
            text("●", 12.0, ink).line_height(16.0).silent()
        } else {
            div().size(6.0, 6.0).rounded(3.0).bg(ink)
        };
        dot.shrink0().role(Role::Status, value)
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
            if let (Some(message), Some(ids)) = (ask, self.bind.confirming(id)) {
                asking = Some((message, label.clone(), *style, ids));
            }
            entries.push(self.entry(id, &label, *style, icon.as_deref()));
        }
        if entries.is_empty() {
            return None;
        }
        let hot = self.ui.hover(menu);
        let mut b = div()
            .id(menu)
            .on(Action::Click)
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
        if self.bind.menu_open(menu) {
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
        let ask = press.confirm.clone();
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
            .role(Role::Button, label);
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
        match (ask, self.bind.confirming(id)) {
            (Some(message), Some((ok, cancel))) => {
                let t = if style == Style::Danger {
                    ConfirmTone::Danger
                } else {
                    ConfirmTone::Neutral
                };
                widgets::confirm(self.ui, b, &message, (ok, label), cancel, t)
            }
            _ => b,
        }
    }
}

fn words(s: &str, role: TextRole) -> El {
    let el = match role {
        TextRole::Title => text(s, 13.0, color::GRAY_200)
            .line_height(18.0)
            .weight(size::SEMIBOLD),
        TextRole::Body => text(s, size::XS.0, color::GRAY_300).line_height(size::XS.1),
        TextRole::Secondary => text(s, 11.0, color::GRAY_500).line_height(16.0),
        TextRole::Mono => text(s, 11.0, color::GRAY_400).mono().line_height(16.0),
    };
    el.truncate().silent()
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
