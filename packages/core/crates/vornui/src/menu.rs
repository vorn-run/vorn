//! Menus: a panel of entries drawn at a point (context menus, a
//! toolbar's popovers, a picker's options), with a submenu beside the
//! hovered entry. Entries are data carrying the caller's choice `T`; a
//! chosen entry comes back as a `Select` in the panel's group.

use std::time::Duration;
use web_time::Instant;

use crate::element::{div, icon, text, Action, El, Id};
use crate::scene::{Rect, Rgba};
use crate::theme::rgb;
use crate::widgets::item_id;
use crate::{accesskit::Role, Ui};

fn ico(name: &str, size: f32, stroke: f32, color: Rgba) -> El {
    icon(
        crate::icons::lucide(name, stroke).unwrap_or_default(),
        size,
        color,
    )
}

/// A rule between sections: a `white/[0.06]` hairline with `margin` above
/// and below.
pub fn rule(margin: f32) -> El {
    div()
        .py(margin)
        .w_full()
        .child(div().h(1.0).w_full().bg(Rgba::white(0.06)))
}

/// One entry of a menu.
pub struct Item<T> {
    pub icon: Option<(&'static str, u32)>,
    /// A coloured dot instead of an icon (the status filter).
    pub dot: Option<u32>,
    pub label: String,
    pub detail: Option<String>,
    pub shortcut: Option<String>,
    pub act: Option<T>,
    pub sub: Vec<Item<T>>,
    /// A rule above it.
    pub separator: bool,
    /// A section title rather than a choice.
    pub header: bool,
    pub disabled: bool,
    pub checked: bool,
    pub danger: bool,
    pub mono: bool,
    /// A row drawn by the caller (the layout buttons); not a choice.
    pub row: Option<El>,
    /// A `SelectPicker` row: the check on the left, 30 px tall.
    pub picker: bool,
    /// A glyph drawn by the caller (a connector's or a project's mark).
    pub mark: Option<El>,
    /// A plain `px-3 py-2` button (the launcher's agent picker): the
    /// checked one white on `white/[0.04]`, the rest `text-gray-400`.
    pub plain: bool,
    /// An `OptionRow` menu's entry: check on the left, sections of `py-1.5`.
    pub option_row: bool,
    /// A `KanbanCardMenu` entry: 28 px, the label always secondary.
    pub card_row: bool,
    /// Drawn as a sidebar menu's `text-[13px]` button (`side_row`).
    pub side: Option<Inset>,
    /// A scoped submenu's entry under its project title: `pl-7`, mono.
    pub nested: bool,
    /// The detail's colour, when not gray-600.
    pub detail_color: Option<Rgba>,
    /// Its submenu's width, when not 200.
    pub sub_w: Option<f32>,
}

/// A sidebar menu row's padding: its left edge and above and below.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Inset {
    pub left: f32,
    pub y: f32,
}

impl Inset {
    /// `px-3 py-2.5`: the project and group menus.
    pub const MENU: Inset = Inset {
        left: 12.0,
        y: 10.0,
    };
    /// `pl-5 pr-3 py-2`: a session's "Move to group" entries.
    pub const NESTED: Inset = Inset { left: 20.0, y: 8.0 };
}

impl<T> Item<T> {
    pub fn new(icon: &'static str, label: impl Into<String>, act: T) -> Item<T> {
        Item {
            icon: Some((icon, rgb::GRAY_500)),
            label: label.into(),
            act: Some(act),
            ..Item::default()
        }
    }

    /// Drawn as a sidebar menu button (`side_row`).
    pub fn side(mut self, inset: Inset) -> Item<T> {
        self.side = Some(inset);
        self
    }

    /// Drawn as a plain menu button (`plain_row`).
    pub fn plain(mut self) -> Item<T> {
        self.plain = true;
        self
    }

    pub fn header(label: impl Into<String>) -> Item<T> {
        Item {
            label: label.into(),
            header: true,
            ..Item::default()
        }
    }

    pub fn option(label: impl Into<String>, checked: bool, act: T) -> Item<T> {
        Item {
            label: label.into(),
            act: Some(act),
            checked,
            ..Item::default()
        }
    }

    /// A `SelectPicker` option.
    pub fn pick(label: impl Into<String>, checked: bool, act: T) -> Item<T> {
        Item {
            label: label.into(),
            act: Some(act),
            checked,
            picker: true,
            ..Item::default()
        }
    }

    pub fn mark(mut self, el: El) -> Item<T> {
        self.mark = Some(el);
        self
    }

    pub fn sep(mut self) -> Item<T> {
        self.separator = true;
        self
    }

    pub fn submenu(mut self, items: Vec<Item<T>>) -> Item<T> {
        self.sub = items;
        self
    }

    pub fn detail(mut self, d: impl Into<String>) -> Item<T> {
        self.detail = Some(d.into());
        self
    }

    pub fn tint(mut self, color: u32) -> Item<T> {
        if let Some((name, _)) = self.icon {
            self.icon = Some((name, color));
        }
        self
    }

    pub fn disabled(mut self, d: bool) -> Item<T> {
        self.disabled = d;
        self
    }

    /// Whether choosing it does something: not a title or a drawn row.
    pub fn choosable(&self) -> bool {
        !self.header && self.row.is_none()
    }
}

impl<T> Default for Item<T> {
    fn default() -> Self {
        Item {
            icon: Default::default(),
            dot: Default::default(),
            label: Default::default(),
            detail: Default::default(),
            shortcut: Default::default(),
            act: Default::default(),
            sub: Default::default(),
            separator: Default::default(),
            header: Default::default(),
            disabled: Default::default(),
            checked: Default::default(),
            danger: Default::default(),
            mono: Default::default(),
            row: Default::default(),
            picker: Default::default(),
            mark: Default::default(),
            plain: Default::default(),
            option_row: Default::default(),
            card_row: Default::default(),
            side: Default::default(),
            nested: Default::default(),
            detail_color: Default::default(),
            sub_w: Default::default(),
        }
    }
}

/// How lit an entry is: the pointer over it, or keys on it.
fn hot(ui: &Ui, id: Id) -> f32 {
    if ui.key_focused(id) {
        1.0
    } else {
        ui.hover(id)
    }
}

fn row<T>(ui: &Ui, group: Id, i: usize, mut item: Item<T>, sub_open: Option<El>) -> El {
    if let Some(r) = item.row {
        return r;
    }
    if item.header && item.side.is_some() {
        return div().px(12.0).py(4.0).child(
            text(item.label.to_uppercase(), 10.0, Rgba::hex(rgb::GRAY_500))
                .tracking(0.05)
                .line_height(15.0)
                .role(Role::Label, item.label.as_str()),
        );
    }
    if item.header && item.option_row {
        return div().px(12.0).py(4.0).child(
            text(item.label.to_uppercase(), 10.0, Rgba::hex(rgb::GRAY_500))
                .tracking(0.05)
                .line_height(15.0)
                .role(Role::Label, item.label.as_str()),
        );
    }
    if let Some(m) = item.header.then(|| item.mark.take()).flatten() {
        // A scoped submenu's project title: its icon, `font-medium tracking-wider`.
        return div()
            .row()
            .items_center()
            .gap(8.0)
            .px(12.0)
            .pt(8.0)
            .pb(4.0)
            .child(m)
            .child(
                text(item.label.to_uppercase(), 10.0, Rgba::hex(rgb::GRAY_500))
                    .weight(500)
                    .tracking(0.05)
                    .line_height(15.0)
                    .role(Role::Label, item.label.as_str()),
            );
    }
    if item.header {
        return div().px(12.0).pt(6.0).pb(4.0).child(
            text(item.label.to_uppercase(), 10.0, Rgba::hex(rgb::GRAY_500))
                .line_height(14.0)
                .role(Role::Label, item.label.as_str()),
        );
    }
    let id = item_id(group, i);
    if item.picker {
        return picker_row(ui, group, i, id, item);
    }
    if let Some(inset) = item.side {
        return side_row(ui, group, i, id, inset, item);
    }
    if item.plain {
        return plain_row(ui, group, i, id, item);
    }
    let hot = hot(ui, id).max(if sub_open.is_some() { 1.0 } else { 0.0 });
    if item.card_row {
        return card_row(group, i, id, hot, item);
    }
    let fg = if item.danger {
        Rgba::hex(rgb::DANGER)
    } else if item.checked {
        Rgba::hex(rgb::WHITE)
    } else {
        Rgba::hex(rgb::GRAY_300).mix(Rgba::hex(rgb::WHITE), hot)
    };
    let bg = if item.checked {
        Rgba::white(0.06)
    } else {
        Rgba::white(0.0).mix(Rgba::white(0.06), hot)
    };
    let mut r = div()
        .id(id)
        .row()
        .items_center()
        .gap(10.0)
        .px(12.0)
        .pl(if item.nested { 28.0 } else { 12.0 })
        .h(32.0)
        .bg(bg)
        .role(Role::MenuItem, item.label.as_str())
        .selected(item.checked);
    if item.disabled {
        r = r.opacity(0.4).disabled(true);
    } else {
        r = r.on(Action::Select { group, index: i });
    }
    if let Some(dot) = item.dot {
        r = r.child(
            div()
                .size(8.0, 8.0)
                .rounded(4.0)
                .bg(Rgba::hex(dot))
                .shrink0(),
        );
    }
    if let Some(m) = item.mark {
        r = r.child(div().shrink0().child(m));
    } else if let Some((name, color)) = item.icon {
        let color = if item.danger { rgb::DANGER } else { color };
        r = r.child(ico(name, 14.0, 1.5, Rgba::hex(color)).shrink0());
    }
    let mut label = text(&item.label, 12.0, fg)
        .line_height(16.0)
        .truncate()
        .silent();
    if item.mono || item.nested {
        label = label.mono();
    }
    r = r.child(div().row().grow().min_w(0.0).child(label));
    if let Some(d) = &item.detail {
        r = r.child(
            text(
                d,
                10.0,
                item.detail_color.unwrap_or(Rgba::hex(rgb::GRAY_600)),
            )
            .line_height(14.0)
            .silent(),
        );
    }
    if let Some(s) = &item.shortcut {
        r = r.child(
            text(s, 10.0, Rgba::hex(rgb::GRAY_600))
                .mono()
                .line_height(14.0)
                .silent(),
        );
    }
    if !item.sub.is_empty() {
        r = r.has_popup().expanded(sub_open.is_some()).child(ico(
            "chevron-right",
            11.0,
            2.0,
            Rgba::hex(rgb::GRAY_500),
        ));
    }
    if item.checked {
        r = r.child(ico("check", 11.0, 3.0, fg));
    }
    match sub_open {
        // The submenu's first entry level with this one, a little over the
        // menu's edge.
        Some(panel) => r.overlay_beside(panel, OVERLAP, SUB_LIFT),
        None => r,
    }
}

/// `KanbanCardMenu`'s button: `px-3 py-1.5 text-xs text-ink-secondary`,
/// only the icon in the action's colour.
fn card_row<T>(group: Id, i: usize, id: Id, hot: f32, item: Item<T>) -> El {
    let mut r = div()
        .id(id)
        .row()
        .items_center()
        .gap(10.0)
        .px(12.0)
        .py(6.0)
        .bg(Rgba::white(0.0).mix(Rgba::white(0.06), hot))
        .role(Role::MenuItem, item.label.as_str())
        .on(Action::Select { group, index: i });
    if let Some((name, color)) = item.icon {
        let color = if item.danger { rgb::DANGER } else { color };
        r = r.child(ico(name, 14.0, 2.0, Rgba::hex(color)).shrink0());
    }
    r.child(
        text(&item.label, 12.0, crate::theme::color::INK_SECONDARY)
            .line_height(16.0)
            .silent(),
    )
}

/// A plain menu button: `text-[12px] text-gray-300` (danger at 80 %), the
/// icon in the text's colour.
fn plain_row<T>(ui: &Ui, group: Id, i: usize, id: Id, item: Item<T>) -> El {
    let hot = hot(ui, id);
    let (fg, bg) = if item.danger {
        let fg = Rgba::hexa(rgb::DANGER, 0.8).mix(Rgba::hex(rgb::DANGER), hot);
        (fg, Rgba::white(0.0).mix(Rgba::white(0.06), hot))
    } else if item.checked {
        (Rgba::hex(rgb::WHITE), Rgba::white(0.04))
    } else {
        let fg = Rgba::hex(rgb::GRAY_400);
        (fg, Rgba::white(0.0).mix(Rgba::white(0.06), hot))
    };
    let mut r = div()
        .id(id)
        .row()
        .items_center()
        .gap(8.0)
        .px(12.0)
        .py(8.0)
        .bg(bg)
        .role(Role::MenuItem, item.label.as_str());
    if item.disabled {
        r = r.opacity(0.35).disabled(true);
    } else {
        r = r.on(Action::Select { group, index: i });
    }
    if let Some(m) = item.mark {
        r = r.child(div().shrink0().child(m));
    } else if let Some((name, _)) = item.icon {
        r = r.child(ico(name, 12.0, 1.5, fg).shrink0());
    }
    r = r.child(
        div().row().grow().min_w(0.0).child(
            text(&item.label, 12.0, fg)
                .line_height(16.0)
                .truncate()
                .silent(),
        ),
    );
    if let Some(d) = &item.detail {
        r = r.child(
            text(d, 9.0, Rgba::hex(rgb::GRAY_600))
                .line_height(12.0)
                .silent(),
        );
    }
    r
}

/// A sidebar menu's button (`ProjectContextMenu`, `GroupContextMenu`,
/// `SessionContextMenu`): `text-[13px] text-gray-300`, a 12 px icon; a
/// checked danger entry is the armed "Confirm delete?".
fn side_row<T>(ui: &Ui, group: Id, i: usize, id: Id, inset: Inset, item: Item<T>) -> El {
    let hot = hot(ui, id);
    let (fg, bg) = match (item.danger, item.checked) {
        (true, true) => (
            Rgba::hex(rgb::DANGER),
            Rgba::hexa(rgb::DANGER, 0.10).mix(Rgba::hexa(rgb::DANGER, 0.20), hot),
        ),
        (true, false) => (
            Rgba::hexa(rgb::DANGER, 0.8).mix(Rgba::hex(rgb::DANGER), hot),
            Rgba::white(0.0).mix(Rgba::white(0.06), hot),
        ),
        (false, true) => (Rgba::hex(rgb::WHITE), Rgba::white(0.06)),
        (false, false) => (
            Rgba::hex(rgb::GRAY_300).mix(Rgba::hex(rgb::WHITE), hot),
            Rgba::white(0.0).mix(Rgba::white(0.06), hot),
        ),
    };
    let mut r = div()
        .id(id)
        .row()
        .items_center()
        .gap(8.0)
        .pl(inset.left)
        .pr(12.0)
        .py(inset.y)
        .bg(bg)
        .role(Role::MenuItem, item.label.as_str())
        .selected(item.checked && !item.danger);
    if item.disabled {
        r = r.opacity(0.4).disabled(true);
    } else {
        r = r.on(Action::Select { group, index: i });
    }
    if let Some(m) = item.mark {
        r = r.child(div().shrink0().child(m));
    } else if let Some((name, _)) = item.icon {
        r = r.child(ico(name, 12.0, 1.5, fg).shrink0());
    }
    r = r.child(
        div().row().grow().min_w(0.0).child(
            text(&item.label, 13.0, fg)
                .line_height(19.5)
                .truncate()
                .silent(),
        ),
    );
    if item.checked && !item.danger {
        r = r.child(ico("check", 12.0, 2.5, fg).shrink0());
    }
    r
}

/// `SelectPicker`'s option: the check (or its room), the mark, the label
/// and a hint.
fn picker_row<T>(ui: &Ui, group: Id, i: usize, id: Id, item: Item<T>) -> El {
    let hot = hot(ui, id);
    let fg = if item.checked {
        Rgba::hex(rgb::WHITE)
    } else {
        Rgba::hex(rgb::GRAY_300).mix(Rgba::hex(rgb::WHITE), hot)
    };
    let bg = if item.checked {
        Rgba::white(0.06)
    } else {
        Rgba::white(0.0).mix(Rgba::white(0.04), hot)
    };
    let mut r = div()
        .id(id)
        .row()
        .items_center()
        .gap(8.0)
        .px(12.0)
        .py(6.0)
        .bg(bg)
        .role(Role::MenuItem, item.label.as_str())
        .selected(item.checked)
        .child(if item.checked {
            ico("check", 11.0, 3.0, fg)
        } else {
            div().w(11.0).shrink0()
        });
    if let Some(d) = item.dot.filter(|_| item.option_row) {
        r = r.child(div().size(6.0, 6.0).rounded(3.0).bg(Rgba::hex(d)).shrink0());
    }
    if item.disabled {
        r = r.opacity(0.4).disabled(true);
    } else {
        r = r.on(Action::Select { group, index: i });
    }
    if let Some(m) = item.mark {
        r = r.child(div().shrink0().child(m));
    } else if let Some((name, color)) = item.icon {
        r = r.child(ico(name, 12.0, 2.0, Rgba::hex(color)).shrink0());
    }
    r = r.child(
        div().row().grow().min_w(0.0).child(
            text(&item.label, 12.0, fg)
                .line_height(18.0)
                .truncate()
                .silent(),
        ),
    );
    if let Some(d) = &item.detail {
        r = r.child(
            text(d, 10.0, Rgba::hex(rgb::GRAY_600))
                .line_height(15.0)
                .silent(),
        );
    }
    r
}

/// How far a submenu lies over its menu's edge.
pub const OVERLAP: f32 = 3.0;
/// A submenu's `py-1` and border: how far above its entry it starts.
const SUB_LIFT: f32 = 5.0;
/// How long a change of submenu waits while the pointer heads for the open
/// one, and after it stops.
pub const HOLD: Duration = Duration::from_millis(250);
/// How long a submenu takes to fade in.
const SUB_IN: Duration = Duration::from_millis(120);

/// The id of the submenu panel whose entries choose in `sub_group`.
pub fn sub_panel_id(sub_group: Id) -> Id {
    sub_group.child("scroll")
}

/// Which entry's submenu is open, and where the pointer is heading: the
/// open one stays while the pointer crosses other entries on its way
/// there (the triangle between the pointer and the submenu's near edge).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Cascade {
    pub open: Option<usize>,
    /// When `open` last changed; the submenu fades in from it.
    pub since: Option<Instant>,
    /// Focus the submenu's first entry once it is laid out.
    pub focus_sub: bool,
    /// The entry the pointer was last on.
    row: Option<usize>,
    /// The pointer now, and where it was before its last move.
    at: Option<(f32, f32)>,
    from: Option<(f32, f32)>,
    /// A change held back while the pointer heads for the submenu, and
    /// when it lands anyway.
    held: Option<(Option<usize>, Instant)>,
}

impl Cascade {
    fn set(&mut self, open: Option<usize>, now: Instant) {
        if self.open != open {
            self.open = open;
            self.since = Some(now);
        }
        self.focus_sub = false;
        self.held = None;
    }

    /// The entry `i`, which has a submenu, clicked: toggles it.
    pub fn click(&mut self, i: usize, now: Instant) {
        let open = (self.open != Some(i)).then_some(i);
        self.set(open, now);
        self.row = Some(i);
    }

    /// Opens entry `i`'s submenu from the keyboard, focus going into it.
    pub fn open_from_keys(&mut self, i: usize, now: Instant) {
        self.set(Some(i), now);
        self.focus_sub = true;
    }

    pub fn close(&mut self, now: Instant) {
        self.set(None, now);
    }

    /// One frame of pointer: `row` the entry under it and whether that has
    /// a submenu, `in_sub` whether it is over the submenu at `sub`. The
    /// hovered entry's submenu opens and another's closes, unless the
    /// pointer is heading for the open one. Returns when to look again.
    pub fn step(
        &mut self,
        now: Instant,
        pointer: Option<(f32, f32)>,
        row: Option<(usize, bool)>,
        in_sub: bool,
        sub: Option<Rect>,
    ) -> Option<Instant> {
        let moved = pointer.is_some() && pointer != self.at;
        if moved {
            self.from = self.at;
            self.at = pointer;
        }
        if in_sub {
            self.row = None;
            self.held = None;
            return None;
        }
        let aiming = moved && heading(self.from, self.at, sub.filter(|_| self.open.is_some()));
        let on = row.map(|r| r.0);
        if on != self.row {
            self.row = on;
            match row {
                // Off the entries: what is open stays.
                None => self.held = None,
                Some((i, has_sub)) => {
                    let want = has_sub.then_some(i);
                    if want == self.open {
                        self.held = None;
                    } else if aiming {
                        self.held = Some((want, now + HOLD));
                    } else {
                        self.set(want, now);
                    }
                }
            }
        } else if let Some((want, until)) = self.held {
            if aiming {
                self.held = Some((want, now + HOLD));
            } else if moved || now >= until {
                self.set(want, now);
            }
        }
        self.held.map(|h| h.1)
    }
}

/// Whether the pointer, moving `from` → `to`, heads for the near edge of
/// `sub`: `to` inside the triangle of `from` and that edge's corners.
fn heading(from: Option<(f32, f32)>, to: Option<(f32, f32)>, sub: Option<Rect>) -> bool {
    let (Some(a), Some(p), Some(r)) = (from, to, sub) else {
        return false;
    };
    let x = if r.x >= a.0 { r.x } else { r.right() };
    let (b, c) = ((x, r.y - 4.0), (x, r.bottom() + 4.0));
    let side = |p: (f32, f32), q: (f32, f32), s: (f32, f32)| {
        (q.0 - p.0) * (s.1 - p.1) - (q.1 - p.1) * (s.0 - p.0)
    };
    let (d1, d2, d3) = (side(a, b, p), side(b, c, p), side(c, a, p));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

/// Follows the pointer over a menu's entries in `group` (`subs[i]`:
/// whether entry `i` has a submenu) and its submenu in `sub_group`, and
/// focuses the submenu's first entry when keys opened it.
pub fn track(ui: &mut Ui, c: &mut Cascade, group: Id, sub_group: Id, subs: &[bool]) {
    let row = subs
        .iter()
        .enumerate()
        .find(|(i, _)| ui.hovered(crate::widgets::item_id(group, *i)))
        .map(|(i, &s)| (i, s));
    let panel = sub_panel_id(sub_group);
    let sub = ui.rect_of(panel);
    if let Some(at) = c.step(ui.now(), ui.pointer(), row, ui.hovered(panel), sub) {
        ui.wake_at(at);
    }
    if c.focus_sub && c.open.is_some() {
        if ui.focus_first_in(sub_group) {
            c.focus_sub = false;
        } else {
            ui.wake_at(ui.now());
        }
    }
}

/// A key while a menu with submenus is open: Right, Enter or Space on an
/// entry with one opens it and focuses its first entry; Left or Escape
/// closes the open submenu (focus back on its entry); Up and Down in the
/// menu close it. True when the key was used.
pub fn key(
    ui: &mut Ui,
    c: &mut Cascade,
    group: Id,
    sub_group: Id,
    subs: &[bool],
    code: &str,
) -> bool {
    let now = ui.now();
    let at = ui.focus().and_then(|f| ui.select_of(f));
    let in_menu = at.is_some_and(|(g, _)| g == group);
    let in_sub = at.is_some_and(|(g, _)| g == sub_group);
    match code {
        "ArrowRight" | "Enter" | "NumpadEnter" | "Space" if in_menu => {
            let Some((_, i)) = at.filter(|(_, i)| subs.get(*i) == Some(&true)) else {
                return code == "ArrowRight";
            };
            c.open_from_keys(i, now);
            true
        }
        "ArrowLeft" | "Escape" if c.open.is_some() && (in_sub || code == "Escape") => {
            let parent = c.open.map(|i| crate::widgets::item_id(group, i));
            c.close(now);
            if let Some(p) = parent.filter(|_| in_sub || in_menu) {
                ui.focus_from_keys(p);
            }
            true
        }
        "ArrowLeft" | "ArrowRight" => in_menu || in_sub,
        "ArrowUp" | "ArrowDown" | "Home" | "End" if in_menu && c.open.is_some() => {
            c.close(now);
            false
        }
        _ => false,
    }
}

/// A panel of `items`: `bg-surface-overlay border-white/10 rounded-lg
/// py-1`, rows of 32 px, separators as rules; `cascade`'s open submenu
/// beside its entry.
#[allow(clippy::too_many_arguments)]
pub fn panel<T>(
    ui: &Ui,
    menu: Id,
    group: Id,
    sub_group: Id,
    w: f32,
    name: &str,
    items: Vec<Item<T>>,
    cascade: Cascade,
) -> El {
    let sub = cascade.open;
    let p = ui.menu_progress(menu);
    let edge = if items
        .iter()
        .any(|i| i.plain || i.option_row || i.side.is_some())
    {
        0.08
    } else {
        0.10
    };
    // An `OptionRow` menu is sections of `py-1.5` with a rule between.
    let pad = if items.iter().any(|i| i.option_row) {
        6.0
    } else {
        4.0
    };
    let mut col = div()
        .col()
        .w(w)
        .py(pad)
        .bg(Rgba::hex(rgb::SURFACE_OVERLAY))
        .border(1.0, Rgba::white(edge))
        .rounded(8.0)
        .opacity(p)
        .transform(0.96 + 0.04 * p, (0.0, -4.0 * (1.0 - p)))
        .role(Role::Menu, name);
    for (i, mut item) in items.into_iter().enumerate() {
        if item.separator && i > 0 {
            col = col.child(rule(pad));
        }
        let sub_items = std::mem::take(&mut item.sub);
        let open = sub == Some(i) && !sub_items.is_empty();
        let has_sub = !sub_items.is_empty();
        let sub_w = item.sub_w.unwrap_or(200.0);
        let sub_panel = open.then(|| {
            div()
                .id(sub_panel_id(sub_group))
                .col()
                .scroll_y()
                .max_h(320.0)
                .opacity(sub_progress(ui, cascade.since))
                .child(panel_inner(ui, sub_group, sub_w, sub_items))
        });
        if has_sub {
            // Keeps the chevron while the entries live in the submenu.
            item.sub.push(Item::default());
        }
        col = col.child(row(ui, group, i, item, sub_panel));
    }
    col
}

/// How far a submenu opened at `since` has faded in, eased out.
fn sub_progress(ui: &Ui, since: Option<Instant>) -> f32 {
    let Some(since) = since else { return 1.0 };
    let t = ui.now().saturating_duration_since(since);
    if t >= SUB_IN {
        return 1.0;
    }
    ui.wake_at(ui.now());
    let x = t.as_secs_f32() / SUB_IN.as_secs_f32();
    1.0 - (1.0 - x) * (1.0 - x)
}

fn panel_inner<T>(ui: &Ui, group: Id, w: f32, items: Vec<Item<T>>) -> El {
    let mut col = div()
        .col()
        .w(w)
        .py(4.0)
        .bg(Rgba::hex(rgb::SURFACE_OVERLAY))
        .border(1.0, Rgba::white(0.10))
        .rounded(8.0)
        .role(Role::Menu, "Submenu");
    for (i, item) in items.into_iter().enumerate() {
        if item.separator && i > 0 {
            col = col.child(rule(4.0));
        }
        col = col.child(row(ui, group, i, item, None));
    }
    col
}

/// The entry `index` of a menu's items, or of the open submenu's when
/// `in_sub`; `None` for an entry that chooses nothing.
pub fn chosen<T>(
    mut items: Vec<Item<T>>,
    sub: Option<usize>,
    in_sub: bool,
    index: usize,
) -> Option<Item<T>> {
    if in_sub {
        let parent = items.get_mut(sub?)?;
        let mut subs = std::mem::take(&mut parent.sub);
        return (index < subs.len()).then(|| subs.swap_remove(index));
    }
    (index < items.len())
        .then(|| items.swap_remove(index))
        .filter(Item::choosable)
}
