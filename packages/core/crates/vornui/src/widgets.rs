//! The controls the app is built from, with the look and behaviour of
//! today's components (`src/renderer/components`): tooltips, buttons, icon
//! buttons, pills, pickers and their menus, list rows, tab strips, split
//! panes, scroll views, text inputs and the prompt composer.
//!
//! Each is a function from the [`Ui`]'s interaction state to an [`El`]:
//! hover and focus are read while building, choices come back as
//! [`Event`](crate::Event)s. The returned element can be styled further
//! (`.w(...)`, `.ml(...)`) like any other.

use accesskit::Role;

use crate::element::{div, icon, text, Action, Content, EditLook, El, Id, Place, Sense};
use crate::scene::Rgba;
use crate::text::TextStyle;
use crate::theme::{color, radius};
use crate::Ui;

/// The icons the widgets draw (Lucide, ISC; see `icons/LICENSE`).
pub mod icons {
    pub const CHECK: &str = include_str!("../icons/check.svg");
    pub const CHEVRON_DOWN: &str = include_str!("../icons/chevron-down.svg");
    pub const PLUS: &str = include_str!("../icons/plus.svg");
    pub const X: &str = include_str!("../icons/x.svg");

    /// `svg` drawn with lines `w` units wide (Lucide's `strokeWidth`).
    pub fn stroke(svg: &str, w: f32) -> String {
        svg.replacen("stroke-width=\"2\"", &format!("stroke-width=\"{w}\""), 1)
    }

    /// `svg` turned `deg` degrees about its centre, to whole degrees so a
    /// turning icon rasterizes a bounded number of times.
    pub fn rotated(svg: &str, deg: f32) -> String {
        let deg = deg.round();
        if deg == 0.0 {
            return svg.to_owned();
        }
        match (svg.find('>'), svg.rfind("</svg>")) {
            (Some(open), Some(close)) if open < close => format!(
                "{}<g transform=\"rotate({deg} 12 12)\">{}</g>{}",
                &svg[..=open],
                &svg[open + 1..close],
                &svg[close..]
            ),
            _ => svg.to_owned(),
        }
    }
}

/// Entry `index` of a choice group (a pill, a menu entry, a list row).
pub fn item_id(group: Id, index: usize) -> Id {
    group.child(index)
}

/// `rest` at rest, `hot` when `id` is hovered, following its transition.
fn tint(ui: &Ui, id: Id, rest: Rgba, hot: Rgba) -> Rgba {
    rest.mix(hot, ui.hover(id))
}

/// A UI string at `size` with an explicit line height (Tailwind's
/// `text-xs` and `text-sm` set both).
fn label(s: &str, (size, line): (f32, f32), c: Rgba) -> El {
    text(s, size, c).line_height(line).silent()
}

/// Makes `owner` show `label` above it after the pointer rests on it, as
/// `Tooltip.tsx` does: hover only, never on focus, gone on scroll or press.
pub fn tooltip(ui: &Ui, owner: El, id: Id, label: &str, shortcut: Option<&str>) -> El {
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
        .border(1.0, Rgba::white(0.08))
        .bg(color::SURFACE_OVERLAY)
        .role(Role::Tooltip, label)
        .child(text(label, 11.0, color::GRAY_200).silent())
        .child_if(shortcut.map(|s| text(s, 10.0, color::GRAY_500).mono().ml(8.0).silent()));
    owner.overlay(tip, Place::Above, 6.0)
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
        .bg(tint(ui, id, Rgba::TRANSPARENT, Rgba::white(0.06)))
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
        .bg(tint(ui, id, Rgba::TRANSPARENT, Rgba::white(0.06)))
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
}

/// A segmented control: `bg-white/[0.04] rounded-lg p-0.5 gap-0.5`, the
/// chosen pill `bg-white/10 text-white`, the others gray. Choosing reports
/// `Event::Select { group, index }`; arrows move between pills.
pub fn pills(ui: &Ui, group: Id, name: &str, items: &[Pill<'_>], selected: usize) -> El {
    let pill = |(i, p): (usize, &Pill<'_>)| {
        let id = item_id(group, i);
        let on = i == selected;
        let fg = if on {
            color::WHITE
        } else {
            tint(ui, id, color::GRAY_500, color::GRAY_300)
        };
        div()
            .id(id)
            .row()
            .items_center()
            .gap(6.0)
            .px(10.0)
            .py(4.0)
            .rounded(radius::MD)
            .bg(if on {
                Rgba::white(0.1)
            } else {
                Rgba::TRANSPARENT
            })
            .on(Action::Select { group, index: i })
            .ring(radius::SM)
            .role(Role::RadioButton, p.label)
            .toggled(on)
            .child_if(p.icon.map(|svg| icon(svg, 14.0, fg)))
            .child(label(p.label, crate::theme::text::XS, fg))
    };
    div()
        .row()
        .items_center()
        .gap(2.0)
        .p(2.0)
        .rounded(radius::LG)
        .bg(Rgba::white(0.04))
        .role(Role::RadioGroup, name)
        .children(items.iter().enumerate().map(pill))
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
        .child(label(current, size, fg))
        .child(icon(chevron, 12.0, color::GRAY_500));
    let trigger = match look {
        PickerLook::Plain => trigger
            .px(6.0)
            .py(2.0)
            .mx(-6.0)
            .rounded(radius::SM)
            .bg(Rgba::TRANSPARENT.mix(Rgba::white(0.04), hot)),
        PickerLook::Compact => trigger
            .px(8.0)
            .py(4.0)
            .rounded(radius::MD)
            .bg(Rgba::TRANSPARENT.mix(Rgba::white(0.06), hot)),
        PickerLook::Chip => trigger
            .px(6.0)
            .py(2.0)
            .rounded(radius::SM)
            .border(1.0, Rgba::white(0.12).mix(Rgba::white(0.25), hot))
            .bg(Rgba::TRANSPARENT.mix(Rgba::white(0.04), hot)),
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
        .bg(color::SURFACE_OVERLAY)
        .border(1.0, Rgba::white(0.08))
        .rounded(radius::LG)
        .clip()
        .opacity(p)
        .transform(0.95 + 0.05 * p, (0.0, -4.0 * (1.0 - p)))
        .role(Role::ListBox, name)
        .child(list);
    trigger.dropdown(panel, Place::BelowStart, 4.0)
}

/// A choosable row (a menu entry, a list item): `px-3 py-1.5 text-[12px]`,
/// the chosen one white on `white/[0.06]` with a check, the others gray
/// turning white on hover. Reports `Event::Select { group, index }`.
pub fn option(ui: &Ui, group: Id, index: usize, selected: bool, c: Choice<'_>) -> El {
    let id = item_id(group, index);
    let fg = if selected {
        color::WHITE
    } else {
        tint(ui, id, color::GRAY_300, color::WHITE)
    };
    let bg = if selected {
        Rgba::white(0.06)
    } else {
        tint(ui, id, Rgba::TRANSPARENT, Rgba::white(0.04))
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
        .child(label(c.label, (12.0, 18.0), fg).truncate().grow())
        .child_if(c.hint.map(|h| label(h, (10.0, 15.0), color::GRAY_600)))
        .child_if(selected.then(|| icon(icons::stroke(icons::CHECK, 3.0), 11.0, fg)))
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
            color::GRAY_200
        } else {
            tint(ui, id, color::GRAY_500, color::GRAY_300)
        };
        let bg = if on {
            Rgba::white(0.06)
        } else {
            tint(ui, id, Rgba::TRANSPARENT, Rgba::white(0.03))
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
                .bg(tint(ui, add, Rgba::TRANSPARENT, Rgba::white(0.06)))
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
        .bg(Rgba::white(0.06).mix(color::BLUE_500.alpha(0.4), hot))
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

fn edit_look(style: TextStyle, color: Rgba, placeholder: &str, multiline: bool) -> Content {
    Content::Edit(Box::new(EditLook {
        style,
        color,
        placeholder: placeholder.to_owned(),
        placeholder_color: color::GRAY_600,
        multiline,
    }))
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
        .border(1.0, Rgba::white(0.12).mix(Rgba::white(0.25), hot))
        .bg(tint(ui, id, Rgba::TRANSPARENT, Rgba::white(0.04)))
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
        .border(1.0, Rgba::white(0.06))
        .bg(color::SURFACE_RAISED)
        .child(field)
        .child(
            div()
                .absolute_bottom()
                .px(12.0)
                .py(8.0)
                .border_top(1.0, Rgba::white(0.04))
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
