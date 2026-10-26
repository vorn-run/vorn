//! The controls the app is built from: tooltips, buttons, icon buttons,
//! pills, pickers and their menus, list rows, tab strips, split panes, scroll
//! views, text inputs and the prompt composer.
//!
//! Each is a function from the [`Ui`]'s interaction state to an [`El`]:
//! hover and focus are read while building, choices come back as
//! [`Event`](crate::Event)s. The returned element can be styled further
//! (`.w(...)`, `.ml(...)`) like any other.

use accesskit::Role;

use crate::element::{div, icon, text, Action, Content, EditLook, El, Id, Place, Sense};
use crate::scene::Rgba;
use crate::text::TextStyle;
use crate::theme::{color, radius, shadow, Color};
use crate::Ui;

pub use crate::icons;

/// Entry `index` of a choice group (a pill, a menu entry, a list row).
pub fn item_id(group: Id, index: usize) -> Id {
    group.child(index)
}

/// `rest` at rest, `hot` when `id` is hovered, following its transition.
fn tint(ui: &Ui, id: Id, rest: impl Into<Rgba>, hot: impl Into<Rgba>) -> Rgba {
    let rest = rest.into();
    let hot = hot.into();
    rest.mix(hot, ui.hover(id))
}

/// A UI string at `size` with an explicit line height (Tailwind's
/// `text-xs` and `text-sm` set both).
fn label(s: &str, (size, line): (f32, f32), c: impl Into<Rgba>) -> El {
    let c = c.into();
    text(s, size, c).line_height(line).silent()
}

/// Makes `owner` show `label` above it after the pointer rests on it: hover
/// only, never on focus, gone on scroll or press.
pub fn tooltip(ui: &Ui, owner: El, id: Id, label: &str, shortcut: Option<&str>) -> El {
    tooltip_at(ui, owner, id, label, shortcut, Place::Above)
}

/// [`tooltip`] on side `place` (`position="bottom"` is [`Place::Below`]);
/// it still flips when that side has no room.
pub fn tooltip_at(
    ui: &Ui,
    owner: El,
    id: Id,
    label: &str,
    shortcut: Option<&str>,
    place: Place,
) -> El {
    let owner = owner.id(id).tooltip_owner();
    if !ui.tooltip_shown(id) {
        return owner;
    }
    let tip = div()
        .row()
        .items_center()
        .px(8.0)
        .py(4.0)
        .rounded(radius::MD)
        .border(1.0, crate::theme::bright(0.08))
        .floating()
        .shadow(&shadow::LG)
        .role(Role::Tooltip, label)
        .child(text(label, 11.0, color::GRAY_200).silent())
        .child_if(shortcut.map(|s| text(s, 10.0, color::GRAY_500).mono().ml(8.0).silent()));
    owner.overlay(tip, place, 6.0)
}

/// A text button: `px-2.5 py-1 rounded-md text-xs`, gray turning white
/// on a faint white wash when hovered.
pub fn button(ui: &Ui, id: Id, text: &str) -> El {
    div()
        .id(id)
        .row()
        .items_center()
        .px(10.0)
        .py(4.0)
        .rounded(radius::MD)
        .bg(tint(ui, id, Rgba::TRANSPARENT, crate::theme::bright(0.06)))
        .on(Action::Click)
        .ring(radius::SM)
        .role(Role::Button, text)
        .child(label(
            text,
            crate::theme::text::XS,
            tint(ui, id, color::GRAY_300, color::WHITE),
        ))
}

/// An icon-only button (`IconButton`): `p-1 rounded`, faint ink turning
/// full ink on `white/[0.06]`, with `label` as its tooltip and name. Chain
/// `.disabled(true).opacity(0.4)` for the disabled look.
pub fn icon_button(ui: &Ui, id: Id, svg: &str, size: f32, label: &str) -> El {
    let b = div()
        .p(4.0)
        .rounded(radius::SM)
        .bg(tint(ui, id, Rgba::TRANSPARENT, crate::theme::bright(0.06)))
        .on(Action::Click)
        .ring(radius::SM)
        .role(Role::Button, label)
        .child(icon(svg, size, tint(ui, id, color::INK_FAINT, color::INK)));
    tooltip(ui, b, id, label, None)
}

/// One segment of [`pills`].
#[derive(Debug, Clone, Copy)]
pub struct Pill<'a> {
    pub label: &'a str,
    pub icon: Option<&'a str>,
    /// Draws the icon alone and moves the label into a tooltip below, with
    /// this shortcut: `MainViewPills`.
    pub tip: Option<&'a str>,
    /// A count over the pill's corner; the tooltip then reads `hint`.
    pub badge: Option<(&'a str, &'a str)>,
    /// A tooltip for a labelled pill, saying what choosing it shows.
    pub hint: Option<&'a str>,
}

impl<'a> Pill<'a> {
    /// A labelled pill, with `icon` before the label when there is one.
    pub fn new(label: &'a str, icon: Option<&'a str>) -> Self {
        Pill {
            label,
            icon,
            tip: None,
            badge: None,
            hint: None,
        }
    }

    /// An icon-only pill whose tooltip shows `label` and `shortcut`.
    pub fn icon_only(label: &'a str, icon: &'a str, shortcut: &'a str) -> Self {
        Pill {
            label,
            icon: Some(icon),
            tip: Some(shortcut),
            badge: None,
            hint: None,
        }
    }

    /// Adds `count` over the corner (`bg-bronzo`, 9 px mono), and `hint`
    /// as the tooltip's text in place of the label.
    pub fn badge(mut self, count: &'a str, hint: &'a str) -> Self {
        self.badge = Some((count, hint));
        self
    }

    /// Adds a tooltip to a labelled pill.
    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = Some(hint);
        self
    }
}

/// A segmented control: `bg-white/[0.04] rounded-lg p-0.5 gap-0.5`, the
/// chosen pill `bg-white/10 text-white`, the others gray. Choosing reports
/// `Event::Select { group, index }`; arrows move between pills.
pub fn pills(ui: &Ui, group: Id, name: &str, items: &[Pill<'_>], selected: usize) -> El {
    let pill = |(i, p): (usize, &Pill<'_>)| {
        let id = item_id(group, i);
        let on = i == selected;
        let fg = if on {
            color::WHITE.rgba()
        } else {
            tint(ui, id, color::GRAY_500, color::GRAY_300)
        };
        let pill = div()
            .id(id)
            .row()
            .items_center()
            .gap(6.0)
            .px(10.0)
            .py(4.0)
            .rounded(radius::MD)
            .bg(if on {
                crate::theme::bright(0.1)
            } else {
                Rgba::TRANSPARENT
            })
            .on(Action::Select { group, index: i })
            .ring(radius::SM)
            .role(Role::RadioButton, p.label)
            .toggled(on)
            .child_if(p.icon.map(|svg| icon(svg, 14.0, fg)))
            .child_if(p.badge.map(|(count, _)| {
                div()
                    .absolute_right(-2.0, -2.0)
                    .row()
                    .items_center()
                    .justify_center()
                    .min_w(12.0)
                    .h(12.0)
                    .px(3.0)
                    .rounded(6.0)
                    .bg(color::BRONZO)
                    .child(
                        text(count, 9.0, color::SURFACE_BASE)
                            .mono()
                            .line_height(9.0)
                            .silent(),
                    )
            }));
        let tip = p.badge.map_or(p.label, |(_, hint)| hint);
        match p.tip {
            Some(keys) => tooltip_at(ui, pill, id, tip, Some(keys), Place::Below),
            None => {
                let pill = pill.child(label(p.label, crate::theme::text::XS, fg));
                match p.hint {
                    Some(hint) => tooltip_at(ui, pill, id, hint, None, Place::Below),
                    None => pill,
                }
            }
        }
    };
    div()
        .row()
        .items_center()
        .gap(2.0)
        .p(2.0)
        .rounded(radius::LG)
        .bg(crate::theme::bright(0.04))
        .role(Role::RadioGroup, name)
        .children(items.iter().enumerate().map(pill))
}

/// A full-width segmented row (the workflow trigger form's `Segmented`):
/// equal `flex-1` buttons in a `white/[0.06]` outline, the chosen one
/// `bg-white/[0.1] text-gray-200`. Each segment is `(id, label, chosen)`
/// and reports `Click` on its id, so the caller maps each to its own value.
pub fn segmented<'a>(ui: &Ui, segments: impl IntoIterator<Item = (Id, &'a str, bool)>) -> El {
    let segment = |(id, text, on): (Id, &str, bool)| {
        let (fg, bg) = if on {
            (color::GRAY_200.rgba(), crate::theme::bright(0.1))
        } else {
            (
                tint(ui, id, color::GRAY_500, color::GRAY_300),
                tint(ui, id, Rgba::TRANSPARENT, crate::theme::bright(0.04)),
            )
        };
        div()
            .id(id)
            .on(Action::Click)
            .flex(1.0)
            .row()
            .justify_center()
            .px(8.0)
            .py(6.0)
            .bg(bg)
            .role(Role::Button, text)
            .toggled(on)
            .child(label(text, (12.0, 18.0), fg))
    };
    div()
        .row()
        .w_full()
        .rounded(radius::LG)
        .border(1.0, crate::theme::bright(0.06))
        .clip()
        .children(segments.into_iter().map(segment))
}

/// One entry of a [`picker`]'s menu or a [`option`] row.
#[derive(Debug, Clone, Copy)]
pub struct Choice<'a> {
    pub label: &'a str,
    pub hint: Option<&'a str>,
}

/// How a [`picker`]'s trigger looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerLook {
    /// `SelectPicker`: bare 12px text and chevron, a faint wash on hover.
    Plain,
    /// `ModelPicker` compact: `text-xs text-gray-400`, `px-2 py-1 rounded-md`.
    Compact,
    /// `ModelPicker` bordered: a chip with a `white/[0.12]` outline.
    Chip,
    /// A native `<select>` in a filter row: `py-1.5 px-2 text-xs` on
    /// `white/[0.05]`, as wide as its longest choice.
    Select,
    /// A form's `<select>`: full width, `px-3 py-1.5 text-sm` on
    /// `white/[0.05]` with a `white/[0.1]` border, the chevron at the end.
    Field,
}

/// The menu a picker opens; its entries report `Event::Select` with it as
/// the group.
pub fn menu_id(picker: Id) -> Id {
    picker.child("menu")
}

/// A dropdown chooser (`SelectPicker`, `ModelPicker`): a trigger showing
/// the chosen entry and a chevron that turns over when open, and a menu
/// under it (at least as wide as the trigger, min 180px, then flipped
/// above if there is no room). Entries choose on press; Escape or a press
/// elsewhere closes it.
pub fn picker(
    ui: &Ui,
    id: Id,
    name: &str,
    look: PickerLook,
    choices: &[Choice<'_>],
    selected: usize,
) -> El {
    let menu = menu_id(id);
    let open = ui.menu_open(menu);
    let current = choices.get(selected).map_or("", |c| c.label);
    let hot = ui.hover(id);
    let (fg, size) = match look {
        PickerLook::Compact => (color::GRAY_400, crate::theme::text::XS),
        PickerLook::Plain | PickerLook::Chip => (color::GRAY_300, (12.0, 18.0)),
        PickerLook::Field => (color::GRAY_200, crate::theme::text::SM),
        PickerLook::Select => (color::GRAY_300, crate::theme::text::XS),
    };
    let chevron = icons::rotated(icons::CHEVRON_DOWN, 180.0 * ui.menu_progress(menu));
    let trigger = div()
        .id(id)
        .row()
        .items_center()
        .gap(6.0)
        .on(Action::ToggleMenu(menu))
        .ring(radius::SM)
        .role(Role::ComboBox, name)
        .value(current)
        .has_popup()
        .expanded(open)
        .child(match look {
            PickerLook::Field => div().grow().min_w0().child(label(current, size, fg)),
            // Every choice laid out at no height, so the box is as wide as the longest.
            PickerLook::Select => {
                div()
                    .col()
                    .child(label(current, size, fg))
                    .children(choices.iter().map(|c| {
                        div()
                            .h(0.0)
                            .clip()
                            .opacity(0.0)
                            .child(label(c.label, size, fg))
                    }))
            }
            _ => label(current, size, fg),
        })
        .child(icon(chevron, 12.0, color::GRAY_500));
    let trigger = match look {
        PickerLook::Plain => trigger
            .px(6.0)
            .py(2.0)
            .mx(-6.0)
            .rounded(radius::SM)
            .bg(Rgba::TRANSPARENT.mix(crate::theme::bright(0.04), hot)),
        PickerLook::Compact => trigger
            .px(8.0)
            .py(4.0)
            .rounded(radius::MD)
            .bg(Rgba::TRANSPARENT.mix(crate::theme::bright(0.06), hot)),
        PickerLook::Chip => trigger
            .px(6.0)
            .py(2.0)
            .rounded(radius::SM)
            .border(
                1.0,
                crate::theme::bright(0.12).mix(crate::theme::bright(0.25), hot),
            )
            .bg(Rgba::TRANSPARENT.mix(crate::theme::bright(0.04), hot)),
        PickerLook::Select => trigger
            .gap(8.0)
            .px(8.0)
            .py(6.0)
            .rounded(radius::SM)
            .bg(crate::theme::bright(0.05))
            .border(
                1.0,
                crate::theme::bright(0.1)
                    .mix(crate::theme::bright(0.2), if open { 1.0 } else { 0.0 }),
            ),
        PickerLook::Field => trigger
            .w_full()
            .px(12.0)
            .py(6.0)
            .rounded(radius::SM)
            .bg(crate::theme::bright(0.05))
            .border(
                1.0,
                crate::theme::bright(0.1)
                    .mix(crate::theme::bright(0.2), if open { 1.0 } else { 0.0 }),
            ),
    };
    if !open {
        return trigger;
    }
    let max_h = if look == PickerLook::Plain {
        280.0
    } else {
        240.0
    };
    let p = ui.menu_progress(menu);
    let list = div()
        .id(menu.child("list"))
        .col()
        .scroll_y()
        .max_h(max_h)
        .py(4.0)
        .children(
            choices
                .iter()
                .enumerate()
                .map(|(i, c)| option(ui, menu, i, i == selected, *c)),
        );
    let panel = div()
        .id(menu)
        .col()
        .min_w(180.0)
        .floating()
        .border(1.0, crate::theme::bright(0.08))
        .rounded(radius::LG)
        .clip()
        .opacity(p)
        .transform(0.95 + 0.05 * p, (0.0, -4.0 * (1.0 - p)))
        .role(Role::ListBox, name)
        .child(list);
    trigger.dropdown(panel, Place::BelowStart, 4.0)
}

/// A choosable row (a menu entry, a list item): `px-3 py-1.5 text-[12px]`,
/// the chosen one white on `white/[0.06]` behind a leading check, the others
/// gray turning white on hover with the check's column kept empty so labels
/// line up. Reports `Event::Select { group, index }`.
pub fn option(ui: &Ui, group: Id, index: usize, selected: bool, c: Choice<'_>) -> El {
    let id = item_id(group, index);
    let fg = if selected {
        color::WHITE.rgba()
    } else {
        tint(ui, id, color::GRAY_300, color::WHITE)
    };
    let bg = if selected {
        crate::theme::bright(0.06)
    } else {
        tint(ui, id, Rgba::TRANSPARENT, crate::theme::bright(0.04))
    };
    div()
        .id(id)
        .row()
        .items_center()
        .gap(8.0)
        .px(12.0)
        .py(6.0)
        .bg(bg)
        .on(Action::Select { group, index })
        .ring(radius::SM)
        .role(Role::ListBoxOption, c.label)
        .selected(selected)
        .child(if selected {
            icon(icons::stroke(icons::CHECK, 3.0), 11.0, fg)
        } else {
            div().w(11.0).shrink0()
        })
        .child(label(c.label, (12.0, 18.0), fg).truncate().grow())
        .child_if(c.hint.map(|h| label(h, (10.0, 15.0), color::GRAY_600)))
}

/// A scrolling list of [`option`] rows: a listbox named `name`.
pub fn list(id: Id, name: &str, rows: impl IntoIterator<Item = El>) -> El {
    scroll_view(id).role(Role::ListBox, name).children(rows)
}

/// A box that scrolls its children vertically, with the app's 6px
/// overlay-colored scrollbar when they overflow.
pub fn scroll_view(id: Id) -> El {
    div().id(id).col().scroll_y()
}

/// One tab of a [`tab_strip`].
#[derive(Debug, Clone, Copy)]
pub struct Tab<'a> {
    pub label: &'a str,
    pub closable: bool,
}

/// Tab `index` of strip `strip`; choosing it reports
/// `Event::Select { group: strip, index }`.
pub fn tab_id(strip: Id, index: usize) -> Id {
    strip.child(("tab", index))
}

/// The close button of tab `index`; it reports `Event::Click`.
pub fn tab_close_id(strip: Id, index: usize) -> Id {
    strip.child(("close", index))
}

/// The strip's new-tab button; it reports `Event::Click`.
pub fn tab_add_id(strip: Id) -> Id {
    strip.child("add")
}

/// A pane's tab strip (`PaneTabStrip`): tabs that truncate at 170px, the
/// active one on `white/[0.06]`, each with a close button that shows while
/// the tab is hovered, then a new-tab button. Append more buttons with
/// `.child(...)`.
pub fn tab_strip(ui: &Ui, strip: Id, tabs: &[Tab<'_>], active: usize) -> El {
    let tab = |(i, t): (usize, &Tab<'_>)| {
        let id = tab_id(strip, i);
        let on = i == active;
        let fg = if on {
            color::GRAY_200.rgba()
        } else {
            tint(ui, id, color::GRAY_500, color::GRAY_300)
        };
        let bg = if on {
            crate::theme::bright(0.06)
        } else {
            tint(ui, id, Rgba::TRANSPARENT, crate::theme::bright(0.03))
        };
        let close = t.closable.then(|| {
            let cid = tab_close_id(strip, i);
            let shown = if ui.focused(cid) { 1.0 } else { ui.hover(id) };
            div()
                .id(cid)
                .p(2.0)
                .rounded(radius::SM)
                .opacity(shown)
                .on(Action::Click)
                .ring(radius::SM)
                .role(Role::Button, format!("Close {}", t.label))
                .child(icon(
                    icons::stroke(icons::X, 2.5),
                    10.0,
                    tint(ui, cid, color::GRAY_600, color::WHITE),
                ))
        });
        div()
            .id(id)
            .row()
            .items_center()
            .gap(4.0)
            .pl(10.0)
            .pr(4.0)
            .py(4.0)
            .rounded(radius::SM)
            .max_w(170.0)
            .shrink0()
            .bg(bg)
            .on(Action::Select {
                group: strip,
                index: i,
            })
            .ring(radius::SM)
            .role(Role::Tab, t.label)
            .selected(on)
            .child(text(t.label, 11.0, fg).silent().truncate())
            .child_if(close)
    };
    let add = tab_add_id(strip);
    div()
        .row()
        .items_center()
        .gap(4.0)
        .pl(6.0)
        .pr(4.0)
        .pt(4.0)
        .child(
            div()
                .row()
                .items_stretch()
                .gap(2.0)
                .grow()
                .min_w0()
                .clip()
                .role_only(Role::TabList)
                .children(tabs.iter().enumerate().map(tab)),
        )
        .child(
            div()
                .id(add)
                .p(4.0)
                .rounded(radius::SM)
                .bg(tint(ui, add, Rgba::TRANSPARENT, crate::theme::bright(0.06)))
                .on(Action::Click)
                .ring(radius::SM)
                .role(Role::Button, "New tab")
                .child(icon(
                    icons::PLUS,
                    13.0,
                    tint(ui, add, color::GRAY_600, color::GRAY_200),
                )),
        )
}

/// Two panes and the 1px divider between them (`SplitDivider`). Dragging
/// the divider (grabbable 3px either side) reports `Event::Resized`, and
/// `Event::ResizeEnd` on release; each side keeps at least 15%.
/// `side_by_side` puts the panes in a row with a vertical divider.
pub fn split(ui: &Ui, id: Id, side_by_side: bool, default: f32, first: El, second: El) -> El {
    let r = ui.split_ratio(id, default);
    let hot = if ui.pressed(id) { 1.0 } else { ui.hover(id) };
    let line = div()
        .id(id)
        .shrink0()
        .sense(Sense::DRAG)
        .bg(crate::theme::bright(0.06).mix(color::BLUE_500.alpha(0.4), hot))
        .role(Role::Splitter, "Resize")
        .value(format!("{}%", (r * 100.0).round()))
        .vertical(side_by_side);
    let (line, container) = if side_by_side {
        (line.w(1.0).h_full().hit_slop(3.0, 0.0), div().row())
    } else {
        (line.h(1.0).w_full().hit_slop(0.0, 3.0), div().col())
    };
    container
        .w_full()
        .h_full()
        .child(first.flex(r))
        .child(line)
        .child(second.flex(1.0 - r))
}

fn edit_look(
    style: TextStyle,
    color: impl Into<Rgba>,
    placeholder: &str,
    multiline: bool,
) -> Content {
    let color = color.into();
    Content::Edit(Box::new(EditLook {
        style,
        color,
        placeholder: placeholder.to_owned(),
        placeholder_color: color::GRAY_600.rgba(),
        multiline,
        textarea: false,
        nowrap: false,
        spans: Vec::new(),
        marks: Vec::new(),
        centered: false,
        secure: false,
    }))
}

/// A code field: a `<textarea>` whose lines do not wrap, as tall as the
/// caller sizes it, coloured with [`El::edit_colors`].
pub fn code(id: Id, name: &str, style: TextStyle, color: impl Into<Rgba>) -> El {
    let color = color.into();
    let mut look = edit_look(style, color, "", true);
    if let Content::Edit(l) = &mut look {
        l.textarea = true;
        l.nowrap = true;
    }
    div()
        .id(id)
        .sense(Sense::TEXT)
        .content(look)
        .role(Role::MultilineTextInput, name)
}

/// A bare one-line text field in `style`, for callers that draw their own
/// frame around it.
pub fn field(
    id: Id,
    name: &str,
    placeholder: &str,
    style: TextStyle,
    color: impl Into<Rgba>,
) -> El {
    let color = color.into();
    div()
        .id(id)
        .sense(Sense::TEXT)
        .content(edit_look(style, color, placeholder, false))
        .role(Role::TextInput, name)
}

/// A bare `<textarea>` in `style`: Enter starts a new line and Cmd+Enter
/// (Ctrl+Enter off macOS) reports `Event::Submit`.
pub fn textarea(
    id: Id,
    name: &str,
    placeholder: &str,
    style: TextStyle,
    color: impl Into<Rgba>,
) -> El {
    let color = color.into();
    let mut look = edit_look(style, color, placeholder, true);
    if let Content::Edit(l) = &mut look {
        l.textarea = true;
    }
    div()
        .id(id)
        .sense(Sense::TEXT)
        .content(look)
        .role(Role::MultilineTextInput, name)
}

/// A bare multiline text field in `style`; Enter reports `Event::Submit`,
/// Shift+Enter starts a new line. It is as tall as the caller sizes it.
pub fn area(id: Id, name: &str, placeholder: &str, style: TextStyle, color: impl Into<Rgba>) -> El {
    let color = color.into();
    div()
        .id(id)
        .sense(Sense::TEXT)
        .content(edit_look(style, color, placeholder, true))
        .role(Role::MultilineTextInput, name)
}

/// A one-line text field in the bordered-chip style: 12px text, the
/// outline brightening on hover or focus. Typing reports `Event::Changed`,
/// Enter `Event::Submit`; IME composition shows underlined in place.
pub fn text_input(ui: &Ui, id: Id, name: &str, placeholder: &str) -> El {
    let hot = if ui.focused(id) { 1.0 } else { ui.hover(id) };
    div()
        .id(id)
        .sense(Sense::TEXT)
        .px(6.0)
        .py(2.0)
        .rounded(radius::SM)
        .border(
            1.0,
            crate::theme::bright(0.12).mix(crate::theme::bright(0.25), hot),
        )
        .bg(tint(ui, id, Rgba::TRANSPARENT, crate::theme::bright(0.04)))
        .content(edit_look(
            TextStyle {
                line_height: 18.0,
                ..TextStyle::ui(12.0)
            },
            color::GRAY_300,
            placeholder,
            false,
        ))
        .role(Role::TextInput, name)
}

/// [`text_input`] of several lines, `lines` tall: Return starts a new line
/// and Cmd+Return (Ctrl+Return off macOS) reports `Event::Submit`.
pub fn text_area_input(ui: &Ui, id: Id, name: &str, placeholder: &str, lines: u8) -> El {
    let hot = if ui.focused(id) { 1.0 } else { ui.hover(id) };
    let style = TextStyle {
        line_height: 18.0,
        ..TextStyle::ui(12.0)
    };
    let mut look = edit_look(style, color::GRAY_300, placeholder, true);
    if let Content::Edit(l) = &mut look {
        l.textarea = true;
    }
    div()
        .id(id)
        .sense(Sense::TEXT)
        .px(6.0)
        .py(4.0)
        .h(f32::from(lines) * 18.0 + 8.0)
        .rounded(radius::SM)
        .border(
            1.0,
            crate::theme::bright(0.12).mix(crate::theme::bright(0.25), hot),
        )
        .bg(tint(ui, id, Rgba::TRANSPARENT, crate::theme::bright(0.04)))
        .content(look)
        .role(Role::MultilineTextInput, name)
}

/// The prompt composer (`PromptLauncher`): a three-row textarea, `text-sm`
/// on the raised surface in a `rounded-xl` outline, with `bar` (pickers,
/// buttons) pinned along its bottom over a faint rule. Enter reports
/// `Event::Submit`; Shift+Enter starts a new line.
pub fn composer(id: Id, placeholder: &str, bar: impl IntoIterator<Item = El>) -> El {
    let (size, line) = crate::theme::text::SM;
    let field = div()
        .id(id)
        .sense(Sense::TEXT)
        .px(16.0)
        .pt(16.0)
        .pb(48.0)
        .h(16.0 + 3.0 * line + 48.0)
        .content(edit_look(
            TextStyle {
                line_height: line,
                ..TextStyle::ui(size)
            },
            color::GRAY_200,
            placeholder,
            true,
        ))
        .role(Role::MultilineTextInput, placeholder);
    div()
        .col()
        .rounded(radius::XL)
        .border(1.0, crate::theme::bright(0.06))
        .bg(color::SURFACE_RAISED)
        .child(field)
        .child(
            div()
                .absolute_bottom()
                .px(12.0)
                .py(8.0)
                .border_top(1.0, crate::theme::bright(0.04))
                .child(div().row().items_center().gap(4.0).children(bar)),
        )
}

#[cfg(test)]
mod tests {
    use super::icons;

    #[test]
    fn icon_variants_stay_valid_svg() {
        let thick = icons::stroke(icons::X, 2.5);
        assert!(thick.contains("stroke-width=\"2.5\""));
        let turned = icons::rotated(icons::CHEVRON_DOWN, 179.6);
        assert!(turned.contains("rotate(180 12 12)"));
        assert!(turned.ends_with("</svg>\n") || turned.ends_with("</svg>"));
        assert_eq!(icons::rotated(icons::CHECK, 0.2), icons::CHECK);
        for svg in [thick, turned] {
            assert!(crate::rasterize_svg(&svg, 16).is_some_and(|m| m.iter().any(|&a| a > 0)));
        }
    }
}

/// A [`toggle`]'s look alone, for a row that is the switch itself (a menu
/// entry): 40 by 24, Vorn's accent when on, a 16 px knob.
pub fn switch_track(on: bool) -> El {
    div()
        .w(40.0)
        .h(24.0)
        .shrink0()
        .rounded(radius::FULL)
        .bg(if on {
            color::BRONZO.rgba()
        } else {
            crate::theme::bright(0.1)
        })
        .child(
            div()
                .absolute(if on { 20.0 } else { 4.0 }, 4.0)
                .size(16.0, 16.0)
                .rounded(radius::FULL)
                // literal-colour: a knob is white on every theme, as system switches are.
                .bg(Rgba::WHITE)
                .border(1.0, crate::palette::shadow(0.12)),
        )
}

/// A switch on a terminal: the word "on" or "off" in its track's colour.
fn switch_cells(on: bool) -> El {
    let (fg, bg) = if on {
        (color::WHITE.rgba(), color::BRONZO.rgba())
    } else {
        (color::GRAY_400.rgba(), crate::theme::bright(0.1))
    };
    div()
        .row()
        .justify_center()
        .w(40.0)
        .shrink0()
        .bg(bg)
        .child(text(if on { "on" } else { "off" }, 12.0, fg).silent())
}

/// `ToggleSwitch`: a `w-10 h-6` track, Vorn's accent when on, its knob sliding
/// from `translate-x-1` to `translate-x-5`; on a terminal it reads "on" or
/// "off". Clicking reports `Click` on `id`; the caller flips the value.
pub fn toggle(ui: &Ui, id: Id, name: &str, on: bool, disabled: bool) -> El {
    let look = if ui.is_cells() {
        switch_cells(on)
    } else {
        switch_track(on)
    };
    let track = look
        .id(id)
        .ring(radius::FULL)
        .role(Role::Switch, name)
        .toggled(on);
    if disabled {
        track.disabled(true)
    } else {
        track.on(Action::Click)
    }
}

/// What a [`confirm`]'s button does: a destructive one is red.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Danger,
}

/// `ConfirmPopover`'s panel under `trigger`: the question, Cancel and the
/// confirming button. Both report `Click` on their ids.
pub fn confirm(
    ui: &Ui,
    trigger: El,
    message: &str,
    (ok, ok_label): (Id, &str),
    cancel: Id,
    tone: Tone,
) -> El {
    let down = |id: Id| if ui.pressed(id) { 1.0 } else { 0.0 };
    let wash = |id: Id, rest: f32, hot: f32, pressed: f32| {
        crate::theme::bright(rest)
            .mix(crate::theme::bright(hot), ui.hover(id))
            .mix(crate::theme::bright(pressed), down(id))
    };
    let cancel_btn = div()
        .id(cancel)
        .on(Action::Click)
        .px(12.0)
        .py(8.0)
        .rounded(radius::MD)
        .bg(wash(cancel, 0.04, 0.08, 0.12))
        .ring(radius::MD)
        .role(Role::Button, "Cancel")
        .child(label(
            "Cancel",
            crate::theme::text::XS,
            tint(ui, cancel, color::GRAY_400, color::GRAY_200),
        ));
    let (bg, edge, fg) = match tone {
        Tone::Danger => {
            let red = |a| crate::theme::color::RED_500.alpha(a);
            (
                red(0.1).mix(red(0.2), ui.hover(ok)).mix(red(0.3), down(ok)),
                Some(red(0.2)),
                color::RED_400,
            )
        }
        Tone::Neutral => (wash(ok, 0.1, 0.15, 0.2), None, color::WHITE),
    };
    let mut ok_btn = div()
        .id(ok)
        .on(Action::Click)
        .px(12.0)
        .py(8.0)
        .rounded(radius::MD)
        .bg(bg)
        .ring(radius::MD)
        .role(Role::Button, ok_label)
        .child(label(ok_label, crate::theme::text::XS, fg).weight(500));
    if let Some(e) = edge {
        ok_btn = ok_btn.border(1.0, e);
    }
    let panel = div()
        .col()
        .min_w(180.0)
        .max_w(280.0)
        .p(12.0)
        .rounded(radius::LG)
        .border(1.0, crate::theme::bright(0.1))
        .floating()
        .shadow(&shadow::XL2)
        .role(Role::AlertDialog, message)
        .child(label(message, crate::theme::text::XS, color::GRAY_300).mb(12.0))
        .child(
            div()
                .row()
                .justify_end()
                .gap(8.0)
                .child(cancel_btn)
                .child(ok_btn),
        );
    // `POPOVER_GAP`.
    trigger.overlay(panel, Place::Below, 6.0)
}

/// What a [`toast`] reports, and the colour it reports it in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Success,
    Error,
    Warning,
    Info,
}

impl ToastKind {
    /// The icon and the Tailwind 400/500 pair it is tinted with.
    fn look(self) -> (&'static str, Color, Color) {
        match self {
            ToastKind::Success => (
                "check",
                crate::theme::color::GREEN_400,
                crate::theme::color::GREEN_500,
            ),
            ToastKind::Error => (
                "x",
                crate::theme::color::RED_400,
                crate::theme::color::RED_500,
            ),
            ToastKind::Warning => (
                "triangle-alert",
                crate::theme::color::AMBER_400,
                crate::theme::color::AMBER_500,
            ),
            ToastKind::Info => (
                "info",
                crate::theme::color::BLUE_400,
                crate::theme::color::BLUE_500,
            ),
        }
    }
}

/// One `Toast`: icon, message and a dismiss button (`close` reports
/// `Click`), on the near-opaque toast glass.
pub fn toast(ui: &Ui, close: Id, message: &str, kind: ToastKind) -> El {
    let (glyph, fg, edge) = kind.look();
    let lucide = |name, stroke| crate::icons::lucide(name, stroke).unwrap_or_default();
    div()
        .row()
        .items_center()
        .gap(10.0)
        .px(16.0)
        .py(10.0)
        .min_w(200.0)
        .max_w(360.0)
        .rounded(radius::LG)
        .border(1.0, edge.alpha(0.2))
        .bg(color::SURFACE_OVERLAY.alpha(0.92))
        .shadow(&shadow::XL)
        .role(Role::Status, message)
        .child(icon(lucide(glyph, 2.5), 15.0, fg))
        .child(
            div()
                .row()
                .grow()
                .min_w(0.0)
                .child(label(message, crate::theme::text::SM, color::GRAY_200).truncate()),
        )
        .child(
            div()
                .id(close)
                .on(Action::Click)
                .p(2.0)
                .role(Role::Button, "Dismiss")
                .child(icon(
                    lucide("x", 2.0),
                    12.0,
                    tint(ui, close, color::GRAY_500, color::GRAY_300),
                )),
        )
}

/// A popover's panel: `bg-surface-raised border-white/[0.08] rounded-md
/// shadow-lg`. The caller adds padding and places it with `overlay`.
pub fn popover(name: &str) -> El {
    div()
        .col()
        .rounded(radius::MD)
        .border(1.0, crate::theme::bright(0.08))
        .bg(color::SURFACE_RAISED)
        .shadow(&shadow::LG)
        .role(Role::Dialog, name)
}

/// A modal: a `bg-black/[alpha]` backdrop over a `size` window with
/// `panel` at `at`. Clicking the backdrop reports `Click` on `id`.
pub fn dialog(id: Id, size: (f32, f32), alpha: f32, panel: El, at: (f32, f32)) -> El {
    scrim(id, size, alpha).child(panel.absolute(at.0, at.1))
}

/// A modal's backdrop: `bg-black/{alpha}` over a `size` window on its own
/// layer, reporting `Click` on `id` (click-outside closes). The caller
/// places the panel in it.
pub fn scrim(id: Id, size: (f32, f32), alpha: f32) -> El {
    div()
        .id(id)
        .on(Action::Click)
        .absolute(0.0, 0.0)
        .size(size.0, size.1)
        .bg(Rgba::hexa(0, alpha))
        .layered()
}

/// A count or status badge: `text` at 10 px, medium, on a rounded tint.
pub fn badge(text: &str, fg: impl Into<Rgba>, bg: impl Into<Rgba>) -> El {
    let fg = fg.into();
    let bg = bg.into();
    div()
        .row()
        .items_center()
        .px(6.0)
        .h(16.0)
        .rounded(radius::FULL)
        .bg(bg)
        .shrink0()
        .child(label(text, (10.0, 14.0), fg).weight(500))
}

/// A chip: `h-[26px] px-2 gap-1.5 rounded-md` on the raised surface, its
/// `white/[0.06]` hairline brightening on hover. `body` is its content.
pub fn chip(ui: &Ui, id: Id, name: &str, body: impl IntoIterator<Item = El>) -> El {
    div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(6.0)
        .h(26.0)
        .px(8.0)
        .rounded(radius::MD)
        .border(
            1.0,
            tint(
                ui,
                id,
                crate::theme::bright(0.06),
                crate::theme::bright(0.12),
            ),
        )
        .bg(color::SURFACE_RAISED)
        .ring(radius::MD)
        .role(Role::Button, name)
        .children(body)
}

/// A `<kbd>`: mono text on `white/[0.04]` in a `white/[0.06]` outline,
/// `(size, line height)` as the caller's `text-*` and `leading-*` set it
/// (`KbdHint` is `leading-none`; a shortcut list inherits its line's).
pub fn kbd(text: &str, (size, line): (f32, f32), fg: impl Into<Rgba>) -> El {
    let fg = fg.into();
    div()
        .row()
        .items_center()
        .justify_center()
        .px(6.0)
        .py(2.0)
        .rounded(radius::SM)
        .bg(crate::theme::bright(0.04))
        .border(1.0, crate::theme::bright(0.06))
        .shrink0()
        .child(
            crate::element::text(text, size, fg)
                .mono()
                .line_height(line)
                .silent(),
        )
}
