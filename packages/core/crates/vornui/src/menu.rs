//! Menus: a panel of entries drawn at a point (context menus, a
//! toolbar's popovers, a picker's options), with a submenu beside the
//! hovered entry. Entries are data carrying the caller's choice `T`; a
//! chosen entry comes back as a `Select` in the panel's group.

use crate::element::{div, icon, text, Action, El, Id, Place};
use crate::scene::Rgba;
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
        }
    }
}

fn row<T>(ui: &Ui, group: Id, i: usize, item: Item<T>, sub_open: Option<El>) -> El {
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
    let hot = ui.hover(id).max(if sub_open.is_some() { 1.0 } else { 0.0 });
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
    if let Some((name, color)) = item.icon {
        let color = if item.danger { rgb::DANGER } else { color };
        r = r.child(ico(name, 14.0, 1.5, Rgba::hex(color)).shrink0());
    }
    let mut label = text(&item.label, 12.0, fg)
        .line_height(16.0)
        .truncate()
        .silent();
    if item.mono {
        label = label.mono();
    }
    r = r.child(div().row().grow().min_w(0.0).child(label));
    if let Some(d) = &item.detail {
        r = r.child(
            text(d, 10.0, Rgba::hex(rgb::GRAY_600))
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
        Some(panel) => r.overlay(panel, Place::Right, 4.0),
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
    let hot = ui.hover(id);
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
    let hot = ui.hover(id);
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
    let hot = ui.hover(id);
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

/// A panel of `items`: `bg-surface-overlay border-white/10 rounded-lg
/// py-1`, rows of 32 px, separators as rules.
#[allow(clippy::too_many_arguments)]
pub fn panel<T>(
    ui: &Ui,
    menu: Id,
    group: Id,
    sub_group: Id,
    w: f32,
    name: &str,
    items: Vec<Item<T>>,
    sub: Option<usize>,
) -> El {
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
        let sub_panel = open.then(|| {
            div()
                .id(sub_group.child("scroll"))
                .col()
                .scroll_y()
                .max_h(320.0)
                .child(panel_inner(ui, sub_group, sub_items))
        });
        if has_sub {
            // Keeps the chevron while the entries live in the submenu.
            item.sub.push(Item::default());
        }
        col = col.child(row(ui, group, i, item, sub_panel));
    }
    col
}

fn panel_inner<T>(ui: &Ui, group: Id, items: Vec<Item<T>>) -> El {
    let mut col = div()
        .col()
        .w(200.0)
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
