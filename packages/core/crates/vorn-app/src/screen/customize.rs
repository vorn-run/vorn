//! Customize: the toolbar's items, the sidebar's tabs and a card's panes,
//! one list at a time in a sheet of one height. A filter narrows it; shown
//! items come in order, hidden ones after; a row drags into order and its
//! switch hides it.

use super::{c, ico, white, Screen};
use crate::act::Act;
use crate::arrange::List;
use crate::look;
use crate::points::Points;
use crate::ui::{div, text, tooltip, Action, El, Id, Role, Sense};

/// How tall each row is, which a drag counts places in.
pub const ROW_H: f32 = 36.0;

/// A card's own shells, the one pane the app adds rather than an extension.
pub const TERMINALS: &str = "terminals";

/// A pane as Customize lists it: its id (an extension pane's point key),
/// icon and name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pane {
    pub id: String,
    pub icon: Mark,
    pub title: String,
    /// The extension it comes from; none for the app's own.
    pub from: Option<String>,
}

/// An item's icon: a kit icon by name, or the vector mark an extension's
/// manifest declares (`view_box` and `paths`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mark {
    Kit(String),
    Drawn(serde_json::Value),
}

impl Mark {
    /// The icon `extension` declares for an item: the item's own mark, else
    /// the kit icon it names, else its extension's mark, else the generic one.
    pub fn of(
        st: &crate::state::State,
        extension: &str,
        own: Option<&serde_json::Value>,
        name: Option<&str>,
    ) -> Mark {
        let drawn = |v: &serde_json::Value| crate::library::parse_icon(v).is_some();
        if let Some(v) = own.filter(|v| drawn(v)) {
            return Mark::Drawn(v.clone());
        }
        if let Some(n) = name.filter(|n| crate::ui::icons::lucide(n, 1.5).is_some()) {
            return Mark::Kit(n.to_owned());
        }
        let id = |v: &&serde_json::Value| {
            v.get("id").and_then(serde_json::Value::as_str) == Some(extension)
        };
        let pack = (st.ext.packs.iter().chain(&st.points.builtins))
            .find(id)
            .and_then(|p| p.get("icon"))
            .filter(|v| drawn(v));
        match pack {
            Some(v) => Mark::Drawn(v.clone()),
            None => Mark::Kit("puzzle".to_owned()),
        }
    }

    pub fn kit(name: &str) -> Mark {
        Mark::Kit(name.to_owned())
    }

    /// The icon at `size` in `color`, a declared mark drawn as its vector.
    pub fn draw(&self, size: f32, color: crate::ui::Rgba) -> El {
        match self {
            Mark::Kit(n) => ico(n, size, 1.5, color),
            Mark::Drawn(v) => super::panes::extension_icon(Some(v), size, color),
        }
    }
}

/// The id an extension's pane is arranged by: its point key.
pub fn pane_id(extension: &str, pane: &str) -> String {
    format!("{extension}/{}/{pane}", crate::points::kind::PANES)
}

/// Every pane a card can show: its shells, then each extension's pane, from
/// the points listed and the installed packs, each once.
pub fn panes(st: &crate::state::State) -> Vec<Pane> {
    let mut out = vec![Pane {
        id: TERMINALS.to_owned(),
        icon: Mark::kit("square-terminal"),
        title: "Terminals".to_owned(),
        from: None,
    }];
    for p in st.points.at(crate::points::kind::PANES) {
        out.push(Pane {
            id: p.key(),
            icon: Mark::of(st, &p.extension, None, p.icon.as_deref()),
            title: if p.title.is_empty() {
                p.id.clone()
            } else {
                p.title.clone()
            },
            from: Some(p.extension_name.clone()),
        });
    }
    for (ext, pane, title, icon) in crate::extensions::pack_panes(&st.ext.packs) {
        let id = pane_id(&ext, &pane);
        if out.iter().all(|p| p.id != id) {
            let from = (st.ext.packs.iter())
                .find(|p| p.get("id").and_then(serde_json::Value::as_str) == Some(ext.as_str()))
                .and_then(|p| p.get("name").and_then(serde_json::Value::as_str))
                .unwrap_or(&ext)
                .to_owned();
            out.push(Pane {
                id,
                icon: Mark::of(st, &ext, icon.as_ref(), None),
                title,
                from: Some(from),
            });
        }
    }
    out
}

/// The sidebar tabs a person arranges: every view with a tab, by its point
/// key. Sessions is always first and always shown.
pub fn tab_ids(points: &Points) -> Vec<String> {
    points.top_bar().into_iter().map(|p| p.key()).collect()
}

/// The sheet's filter field.
pub const SEARCH: Id = Id(0x6375_7374_6f6d_7365);
/// The sheet's scrolling list, whose offset a drag adds to its travel.
pub const LIST: Id = Id(0x6375_7374_6f6d_6c73);
/// How tall the list is, whatever it holds.
const LIST_H: f32 = 360.0;

/// The ids `list` arranges, in their default order.
pub fn available(st: &crate::state::State, list: List) -> Vec<String> {
    match list {
        List::Toolbar => crate::toolbar::available(&st.points),
        List::Tabs => tab_ids(&st.points),
        List::Card => super::card_buttons::catalog(st)
            .into_iter()
            .map(|e| e.id)
            .collect(),
    }
}

/// One row as the list shows it.
struct Row {
    id: String,
    icon: Mark,
    label: String,
    from: Option<String>,
    tip: String,
}

fn row_of(st: &crate::state::State, list: List, id: &str) -> Option<Row> {
    let (icon, label, from, tip) = match list {
        List::Toolbar => {
            let it = crate::toolbar::Item::of(&st.points, id)?;
            let (from, icon) = match it {
                crate::toolbar::Item::Point(p) => (
                    Some(p.extension_name.clone()),
                    Mark::of(st, &p.extension, None, p.icon.as_deref()),
                ),
                _ => (None, Mark::kit(it.icon())),
            };
            (icon, it.label().to_owned(), from, it.tip().to_owned())
        }
        List::Tabs => {
            let p = st.points.get(id)?;
            let icon = Mark::of(st, &p.extension, None, p.icon.as_deref());
            let tip = format!("The {} tab", p.title);
            (icon, p.title.clone(), Some(p.extension_name.clone()), tip)
        }
        List::Card => {
            let e = super::card_buttons::catalog(st)
                .into_iter()
                .find(|e| e.id == id)?;
            let tip = format!("{} on a card's header, or in its More menu", e.title);
            (e.icon, e.title, e.from, tip)
        }
    };
    Some(Row {
        id: id.to_owned(),
        icon,
        label,
        from,
        tip,
    })
}

/// Whether `query` (lowercase) finds the row by its name or its extension's.
fn matches(r: &Row, query: &str) -> bool {
    query.is_empty()
        || r.label.to_lowercase().contains(query)
        || r.from
            .as_ref()
            .is_some_and(|f| f.to_lowercase().contains(query))
}

/// A small icon button: gray, white when hovered, with its tooltip.
fn small(s: &Screen<'_>, place: &str, act: Act, glyph: &str, tip: &str, off: bool) -> El {
    let id = s.acts.on(place, act);
    let hot = s.ui.hover(id);
    let b = div()
        .id(id)
        .p(4.0)
        .rounded(4.0)
        .bg(white(0.0).mix(white(0.06), hot))
        .ring(4.0)
        .role(Role::Button, tip)
        .child(ico(
            glyph,
            12.0,
            2.0,
            c(look::GRAY_500).mix(c(look::WHITE), hot),
        ));
    let b = if off {
        b.opacity(0.3).disabled(true)
    } else {
        b.on(Action::Click)
    };
    tooltip(s.ui, b, id, tip, None)
}

/// A muted section title inside the list.
fn caption(t: &str) -> El {
    text(t.to_uppercase(), 10.0, c(look::GRAY_500))
        .weight(500)
        .tracking(0.05)
        .line_height(16.0)
        .px(8.0)
        .pt(8.0)
        .pb(4.0)
        .role(Role::Heading, t)
}

/// One row: grip, icon, name, the extension it comes from, Move up and
/// down while shown, and its switch. `at` is its place in the list.
fn row(s: &Screen<'_>, list: List, r: &Row, at: usize, shown: bool, last: bool, drag: bool) -> El {
    let grip = s.acts.on(
        "arrange-drag",
        Act::ArrangeMove {
            list,
            id: r.id.clone(),
            to: at,
        },
    );
    let dy = match &s.st.arranging {
        Some((l, id, dy, from)) if *l == list && *id == r.id => {
            dy + s.ui.scroll_offset(LIST) - from
        }
        _ => 0.0,
    };
    let moving = |to: usize| Act::ArrangeMove {
        list,
        id: r.id.clone(),
        to,
    };
    let hot = s.ui.hover(grip);
    let mut grip_el = div()
        .id(grip)
        .size(16.0, 16.0)
        .center()
        .role(Role::Button, "Drag to reorder")
        .child(ico(
            "grip-vertical",
            12.0,
            1.5,
            c(look::GRAY_600).mix(c(look::GRAY_300), hot),
        ));
    if drag {
        grip_el = grip_el.sense(Sense::MOVE);
    } else {
        grip_el = grip_el.opacity(0.3);
    }
    let (eye, says) = if shown {
        ("eye", "Hide")
    } else {
        ("eye-off", "Show")
    };
    let flip = Act::ArrangeShow {
        list,
        id: r.id.clone(),
        shown: !shown,
    };
    let eye = small(s, "arrange-show", flip, eye, says, false);
    let ink = if shown {
        look::GRAY_200
    } else {
        look::GRAY_500
    };
    let mut el = div()
        .row()
        .items_center()
        .gap(8.0)
        .h(ROW_H)
        .px(8.0)
        .rounded(4.0)
        .bg(if dy != 0.0 {
            c(look::SURFACE_OVERLAY)
        } else {
            white(0.0)
        })
        .child(tooltip(s.ui, grip_el, grip, "Drag to reorder", None))
        .child(r.icon.draw(14.0, c(look::GRAY_400)))
        .child({
            let tip_id = Id::new("arrange-tip").child(&r.id);
            let name = div().id(tip_id).shrink0().child(
                text(&r.label, 13.0, c(ink))
                    .line_height(18.0)
                    .truncate()
                    .role(Role::Label, r.label.as_str()),
            );
            tooltip(s.ui, name, tip_id, &r.tip, None)
        })
        .child(
            div()
                .row()
                .grow()
                .min_w0()
                .child_if(r.from.as_ref().map(|f| {
                    text(f, 11.0, c(look::GRAY_600))
                        .line_height(16.0)
                        .truncate()
                        .silent()
                })),
        );
    if shown {
        el = el
            .child(small(
                s,
                "arrange-up",
                moving(at.saturating_sub(1)),
                "chevron-up",
                "Move up",
                at == 0,
            ))
            .child(small(
                s,
                "arrange-down",
                moving(at + 1),
                "chevron-down",
                "Move down",
                last,
            ));
    }
    el = el.child(eye);
    if dy != 0.0 {
        el = el.transform(1.0, (0.0, dy)).layered();
    }
    el
}

/// The list `list`, filtered by what the field holds: the shown items in
/// order, then the hidden ones under a caption. Tabs lead with Sessions,
/// always shown.
fn list_of(s: &Screen<'_>, list: List, query: &str) -> (El, usize, usize) {
    let ids = available(s.st, list);
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let a = s.st.arrange.get(list);
    let rows: Vec<Row> = a
        .display(&refs)
        .into_iter()
        .filter_map(|id| row_of(s.st, list, id))
        .collect();
    let total = rows.len() + usize::from(list == List::Tabs);
    let shown_n = rows.iter().filter(|r| a.is_shown(&r.id)).count();
    let filtering = !query.is_empty();
    let mut col = div().col().gap(2.0).role(Role::List, "Items");
    let mut found = 0;
    if list == List::Tabs && "sessions".contains(query) {
        found += 1;
        col = col.child(
            div()
                .row()
                .items_center()
                .gap(8.0)
                .h(ROW_H)
                .px(8.0)
                .child(div().size(16.0, 16.0))
                .child(ico("square-terminal", 14.0, 1.5, c(look::GRAY_400)))
                .child(
                    text("Sessions", 13.0, c(look::GRAY_200))
                        .line_height(18.0)
                        .grow()
                        .silent(),
                )
                .child(
                    text("Always", 12.0, c(look::GRAY_500))
                        .line_height(16.0)
                        .role(Role::Label, "Always shown"),
                ),
        );
    }
    let mut hidden_head = false;
    for (at, r) in rows.iter().enumerate() {
        if !matches(r, query) {
            continue;
        }
        found += 1;
        let shown = at < shown_n;
        if !shown && !hidden_head {
            hidden_head = true;
            col = col.child(caption("Hidden"));
        }
        let last = at + 1 >= shown_n;
        col = col.child(row(s, list, r, at, shown, last, shown && !filtering));
    }
    if found == 0 {
        col = col.child(
            text("Nothing matches", 12.0, c(look::GRAY_500))
                .line_height(16.0)
                .p(8.0)
                .role(Role::Label, "Nothing matches"),
        );
    }
    (col, found, total)
}

/// Buttons on the card: − n +, how many the header shows before More.
pub(super) fn card_stepper(s: &Screen<'_>) -> El {
    let n = s.st.arrange.card_shown;
    let total = super::card_buttons::catalog(s.st).len();
    let step = |to: usize, glyph: &str, tip: &str, off: bool| {
        small(s, "card-shown", Act::CardShown(to), glyph, tip, off)
    };
    div()
        .row()
        .items_center()
        .gap(4.0)
        .child(step(
            n.saturating_sub(1),
            "minus",
            "Show one fewer on the card",
            n == 0,
        ))
        .child(
            text(n.to_string(), 13.0, c(look::GRAY_200))
                .mono()
                .line_height(18.0)
                .w(24.0)
                .role(Role::Label, "Buttons on the card"),
        )
        .child(step(n + 1, "plus", "Show one more on the card", n >= total))
}

/// The sheet: Toolbar · Tabs · Card, the filter, one list that scrolls in
/// a sheet of one height, and Done.
pub(super) fn sheet(s: &Screen<'_>) -> El {
    use super::settings::{segmented, Seg};
    let list = s.st.customize_list;
    let w = 460.0_f32.min(s.size.0 - 32.0);
    let query = s.ui.edit_text(SEARCH).trim().to_lowercase();
    let (body, found, total) = list_of(s, list, &query);
    let what = match list {
        List::Toolbar => "items",
        List::Tabs => "tabs",
        List::Card => "buttons",
    };
    let count = if query.is_empty() {
        format!("{total} {what}")
    } else {
        format!("{found} of {total}")
    };
    let seg = |label, l: List, tip| Seg {
        label,
        icon: None,
        act: Act::Customize(l),
        on: list == l,
        disabled: false,
        tip: Some(tip),
    };
    let segments = segmented(
        s,
        "customize-list",
        vec![
            seg("Toolbar", List::Toolbar, "The toolbar's items"),
            seg("Tabs", List::Tabs, "The sidebar's tabs"),
            seg("Card", List::Card, "The buttons on a card's header"),
        ],
        16.0,
    );
    let focused = s.ui.focused(SEARCH);
    let search = div()
        .row()
        .items_center()
        .gap(8.0)
        .px(8.0)
        .py(6.0)
        .rounded(4.0)
        .bg(white(0.04))
        .border(1.0, if focused { white(0.2) } else { white(0.08) })
        .frames(SEARCH)
        .child(ico("search", 12.0, 2.0, c(look::GRAY_500)))
        .child(
            div().grow().min_w0().child(
                crate::ui::field(
                    SEARCH,
                    "Filter",
                    "Filter by name or extension",
                    crate::ui::TextStyle {
                        line_height: 18.0,
                        ..crate::ui::TextStyle::ui(13.0)
                    },
                    c(look::GRAY_200),
                )
                .w_full()
                .placeholder_color(c(look::GRAY_500)),
            ),
        );
    let done = s.acts.on("customize", Act::CloseDialog);
    let hot = s.ui.hover(done);
    let header = div()
        .row()
        .items_center()
        .justify_between()
        .px(20.0)
        .pt(16.0)
        .pb(12.0)
        .child(
            text("Customize", 14.0, c(look::WHITE))
                .weight(500)
                .line_height(20.0)
                .role(Role::Heading, "Customize"),
        )
        .child(
            text(count.as_str(), 12.0, c(look::GRAY_500))
                .line_height(16.0)
                .role(Role::Status, count.as_str()),
        );
    let footer = div()
        .row()
        .items_center()
        .justify_between()
        .px(20.0)
        .py(12.0)
        .child(
            text(
                "Drag to reorder \u{b7} the eye shows or hides",
                12.0,
                c(look::GRAY_500),
            )
            .line_height(16.0)
            .silent(),
        )
        .child(tooltip(
            s.ui,
            div()
                .id(done)
                .on(Action::Click)
                .px(12.0)
                .py(4.0)
                .rounded(4.0)
                .bg(white(0.06).mix(white(0.1), hot))
                .ring(4.0)
                .role(Role::Button, "Done")
                .child(
                    text("Done", 12.0, c(look::WHITE))
                        .line_height(16.0)
                        .silent(),
                ),
            done,
            "Closes the sheet; changes are already saved",
            None,
        ));
    let panel = div()
        .col()
        .w(w)
        .rounded(4.0)
        .border(1.0, white(0.08))
        .bg(c(look::SURFACE_OVERLAY))
        .role(Role::Dialog, "Customize")
        .child(header)
        .child(
            div()
                .col()
                .gap(8.0)
                .px(20.0)
                .pb(8.0)
                .child(segments)
                .child(search),
        )
        .child(
            div()
                .id(LIST)
                .col()
                .h(LIST_H)
                .px(12.0)
                .scroll_y()
                .child(body),
        )
        .child(div().h(1.0).w_full().bg(white(0.06)))
        .child(footer);
    let panel = s.acts.click(panel, "dialog", Act::Nothing);
    let x = ((s.size.0 - w) / 2.0).max(0.0);
    super::dialogs::backdrop(s, 0.4).child(panel.absolute(x, (s.size.1 * 0.12).min(80.0)))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn an_item_draws_the_icon_its_extension_declares() {
        let mut st = crate::state::State::default();
        let mark = json!({ "viewBox": "0 0 16 16", "paths": ["M0 0h16v16H0z"] });
        st.ext.packs = vec![json!({
            "id": "acme.notes",
            "icon": mark,
            "contributes": { "panes": [{ "id": "notes", "title": "Notes" }] },
        })];
        assert_eq!(
            Mark::of(&st, "acme.notes", None, None),
            Mark::Drawn(mark.clone())
        );
        let own = json!({ "paths": ["M1 1h2"] });
        assert_eq!(
            Mark::of(&st, "acme.notes", Some(&own), Some("clock")),
            Mark::Drawn(own)
        );
        assert_eq!(
            Mark::of(&st, "other", None, Some("clock")),
            Mark::kit("clock")
        );
        assert_eq!(
            Mark::of(&st, "other", None, Some("no-such")),
            Mark::kit("puzzle")
        );
        let notes = panes(&st)
            .into_iter()
            .find(|p| p.id == "acme.notes/panes/notes");
        assert_eq!(notes.map(|p| p.icon), Some(Mark::Drawn(mark)));
    }
}
