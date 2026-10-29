//! The panes beside a session's terminal (their column, dividers and tab
//! strips), its Files pane drawn from the Files extension's tree, its
//! Terminals panel, and the cards an extension's rows make, as a file popped
//! out of its session.

use super::cards;
use super::{c, ico, truncate, white, Screen};
use crate::act::Act;
use crate::client::Session;
use crate::git::GitAct;
use crate::look;
use crate::side::{pane_id, SideAct, SideKind};
use crate::state::State;
use crate::ui::{custom, div, text, tooltip, Action, El, Id, Rgba, Role, Sense};

/// The pane grey (`PANE_SURFACE`).
fn pane_bg() -> Rgba {
    c(look::SURFACE_PANEL)
}

fn pane(a: SideAct) -> Act {
    Act::Side(a)
}

/// The box a session's Files pane draws its tree in.
pub fn files_body_id(owner: &str) -> Id {
    Id::new("files-body").child(owner)
}

fn card_divider_id(owner: &str) -> Id {
    Id::new("card-divider").child(owner)
}

fn column_divider_id(owner: &str, i: usize) -> Id {
    Id::new("pane-divider").child((owner, i))
}

/// What a dragged divider resizes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneDivider {
    /// The terminal against `owner`'s pane column.
    Card(String),
    /// Between pane `i` and the next in `owner`'s column.
    Column(String, usize),
    /// The Changes panel's width.
    Changes,
}

/// Which divider `id` is.
pub fn divider_of(st: &State, id: Id) -> Option<PaneDivider> {
    if id == super::git::changes_resize_id() {
        return Some(PaneDivider::Changes);
    }
    for owner in st.side.files.iter().chain(st.side.saved.panels.keys()) {
        if id == card_divider_id(owner) {
            return Some(PaneDivider::Card(owner.clone()));
        }
        for i in 0..2 {
            if id == column_divider_id(owner, i) {
                return Some(PaneDivider::Column(owner.clone(), i));
            }
        }
    }
    None
}

/// A 1px seam with a 3px grab either side.
fn divider(s: &Screen<'_>, id: Id, side_by_side: bool, label: &str) -> El {
    let hot = if s.ui.pressed(id) {
        1.0
    } else {
        s.ui.hover(id)
    };
    let line = div()
        .id(id)
        .shrink0()
        .sense(Sense::DRAG)
        .bg(white(0.06).mix(look::BLUE_500.alpha(0.4), hot))
        .role(Role::Splitter, label)
        .vertical(side_by_side);
    if side_by_side {
        line.w(1.0).h_full().hit_slop(3.0, 0.0)
    } else {
        line.h(1.0).w_full().hit_slop(0.0, 3.0)
    }
}

/// `ICON_BUTTON`: `p-1 rounded hover:bg-white/10 text-ink`.
fn icon_button(s: &Screen<'_>, place: &str, act: Act, name: &str, label: &str, on: bool) -> El {
    let id = s.acts.on(place, act);
    icon_button_on(s, id, name, label, on)
}

/// An `ICON_BUTTON` whose control is already bound as `id`.
pub(crate) fn icon_button_at(s: &Screen<'_>, id: Id, name: &str, label: &str) -> El {
    icon_button_on(s, id, name, label, false)
}

fn icon_button_on(s: &Screen<'_>, id: Id, name: &str, label: &str, on: bool) -> El {
    let b = div()
        .id(id)
        .on(Action::Click)
        .p(4.0)
        .rounded(4.0)
        .shrink0()
        .bg(white(0.10 * s.ui.hover(id).max(if on { 1.0 } else { 0.0 })))
        .role(Role::Button, label)
        .child(ico(name, 14.0, 2.0, c(look::INK)));
    tooltip(s.ui, b, id, label, None)
}

/// Maximize or restore, and close.
fn controls(s: &Screen<'_>, kind: SideKind, owner: &str) -> El {
    controls_after(s, kind, owner, Vec::new())
}

/// The pane's maximize and close after `tools`, an extension's own.
fn controls_after(s: &Screen<'_>, kind: SideKind, owner: &str, tools: Vec<El>) -> El {
    let id = pane_id(kind, owner);
    let max = s.st.side.saved.maximized.as_deref() == Some(id.as_str());
    let row = div()
        .row()
        .items_center()
        .gap(2.0)
        .shrink0()
        .children(tools);
    let (glyph, label) = if max {
        ("minimize-2", "Restore")
    } else {
        ("maximize-2", "Maximize")
    };
    row.child(icon_button(
        s,
        "pane-max",
        pane(SideAct::Maximize(id)),
        glyph,
        label,
        false,
    ))
    .child(icon_button(
        s,
        "pane-close",
        pane(SideAct::Close(kind, owner.to_owned())),
        "x",
        "Close",
        false,
    ))
}

/// One tab of a `PaneTabStrip`.
struct Tab {
    name: String,
    /// The parent folder, when another tab has the same name.
    parent: Option<String>,
    icon: Option<El>,
    italic: bool,
    dirty: bool,
    active: bool,
    select: Act,
    close: Act,
    double: Option<Act>,
    extract: Option<Act>,
}

/// The pane's title bar. What a strip carries besides its tabs.
struct Strip<'a> {
    leading: Option<El>,
    /// The title shown while there are no tabs.
    empty: Option<&'a str>,
    add: Option<(Act, &'a str)>,
    overflow: Option<Act>,
    trailing: El,
}

fn tab_strip(s: &Screen<'_>, place: &str, tabs: Vec<Tab>, strip_: Strip<'_>) -> El {
    let Strip {
        leading,
        empty,
        add,
        overflow,
        trailing,
    } = strip_;
    let mut list = div()
        .row()
        .gap(2.0)
        .grow()
        .min_w0()
        .clip()
        .role(Role::TabList, place);
    if tabs.is_empty() {
        if let Some(t) = empty {
            list = list.child(
                div().pl(4.0).row().items_center().child(
                    text(t, 12.0, c(look::INK))
                        .weight(500)
                        .line_height(16.0)
                        .silent(),
                ),
            );
        }
    }
    for t in tabs {
        let id = s.acts.on(&format!("{place}-tab"), t.select.clone());
        if let Some(d) = &t.double {
            s.acts.double(id, d.clone());
        }
        let close_id = Acts_id(&format!("{place}-tab-close"), &t.close);
        let hot = s.ui.hovered(id) || s.ui.hovered(close_id);
        let fg = if t.active {
            c(look::GRAY_200)
        } else {
            c(look::GRAY_500).mix(c(look::GRAY_300), s.ui.hover(id))
        };
        let mut name = text(truncate(&t.name, 22), 11.0, fg)
            .line_height(16.0)
            .silent();
        if t.italic {
            name = name.italic();
        }
        let mut tab = div()
            .id(id)
            .on(Action::Click)
            .sense(Sense::DOUBLE)
            .row()
            .items_center()
            .gap(4.0)
            .pl(10.0)
            .pr(4.0)
            .py(4.0)
            .rounded(4.0)
            .max_w(170.0)
            .shrink0()
            .bg(if t.active {
                white(0.06)
            } else {
                white(0.03 * s.ui.hover(id))
            })
            .role(Role::Tab, &t.name)
            .selected(t.active)
            .child_if(t.icon)
            .child(name);
        if let Some(p) = &t.parent {
            tab = tab.child(
                text(p, 11.0, c(look::INK).alpha(look::INK_FAINT))
                    .line_height(16.0)
                    .silent(),
            );
        }
        if let Some(x) = t.extract {
            let xid = s.acts.on(&format!("{place}-tab-extract"), x);
            tab = tab.child(tooltip(
                s.ui,
                div()
                    .id(xid)
                    .on(Action::Click)
                    .p(2.0)
                    .rounded(4.0)
                    .bg(white(0.08 * s.ui.hover(xid)))
                    .role(Role::Button, format!("Open {} as its own terminal", t.name))
                    .child(ico(
                        "square-arrow-out-up-right",
                        10.0,
                        2.5,
                        c(look::GRAY_600).mix(c(look::WHITE), s.ui.hover(xid)),
                    )),
                xid,
                "Open as its own terminal",
                None,
            ));
        }
        let close_id = s.acts.on(&format!("{place}-tab-close"), t.close);
        let slot = if t.dirty && !hot {
            div()
                .size(14.0, 14.0)
                .center()
                .role(Role::Image, "Unsaved changes")
                .child(div().size(6.0, 6.0).rounded(3.0).bg(c(look::AMBER_400)))
        } else {
            div()
                .id(close_id)
                .on(Action::Click)
                .p(2.0)
                .rounded(4.0)
                .opacity(if hot { 1.0 } else { 0.0 })
                .role(Role::Button, format!("Close {}", t.name))
                .child(ico(
                    "x",
                    10.0,
                    2.5,
                    c(look::GRAY_600).mix(c(look::WHITE), s.ui.hover(close_id)),
                ))
        };
        list = list.child(tab.child(slot));
    }
    let mut strip = div()
        .row()
        .items_center()
        .gap(4.0)
        .pl(6.0)
        .pr(4.0)
        .pt(4.0)
        .shrink0()
        .child_if(leading)
        .child(list);
    if let Some((act, label)) = add {
        let id = s.acts.on(&format!("{place}-add"), act);
        strip = strip.child(tooltip(
            s.ui,
            div()
                .id(id)
                .on(Action::Click)
                .p(4.0)
                .rounded(4.0)
                .bg(white(0.06 * s.ui.hover(id)))
                .role(Role::Button, label)
                .child(ico(
                    "plus",
                    13.0,
                    2.0,
                    c(look::GRAY_600).mix(c(look::GRAY_200), s.ui.hover(id)),
                )),
            id,
            label,
            None,
        ));
    }
    if let Some(act) = overflow {
        strip = strip.child(icon_button(
            s,
            &format!("{place}-all"),
            act,
            "chevron-down",
            "All open tabs",
            false,
        ));
    }
    strip.child(trailing)
}

#[allow(non_snake_case)]
fn Acts_id(place: &str, act: &Act) -> Id {
    crate::act::Acts::id(place, act)
}

/// The Files pane's tree for `owner`, when Files is on.
fn files_key(s: &Screen<'_>, owner: &str) -> Option<String> {
    let p =
        s.st.points
            .find(crate::app::FILES, crate::points::kind::PANES, "files")?;
    Some(crate::points::scoped(&p.key(), owner))
}

/// The row of `card` among the cards points list: the point's key and the row.
fn card_row<'a>(s: &'a Screen<'_>, card: &str) -> Option<(String, &'a crate::points::Row)> {
    s.st.points.at(crate::points::kind::CARDS).find_map(|p| {
        let row =
            s.st.points
                .listed_rows(p, "")
                .into_iter()
                .find(|r| r.id == card)?;
        Some((p.key(), row))
    })
}

/// Card `card`'s tree key, and the session it sits beside.
fn card_of(s: &Screen<'_>, card: &str) -> Option<(String, String)> {
    let (key, row) = card_row(s, card)?;
    Some((
        crate::points::scoped(&key, card),
        row.str("session")?.to_owned(),
    ))
}

/// What card `card` is called.
fn card_title(s: &Screen<'_>, card: &str) -> String {
    card_row(s, card)
        .map(|(_, r)| r.text("title"))
        .unwrap_or_default()
}

/// The cards each session's points list, by the session they sit beside, in their order.
pub(crate) fn cards_of<'a>(st: &'a State, owner: &str) -> Vec<&'a str> {
    st.points
        .at(crate::points::kind::CARDS)
        .flat_map(|p| st.points.listed_rows(p, ""))
        .filter(|r| r.str("session") == Some(owner))
        .map(|r| r.id.as_str())
        .collect()
}

/// The Files extension's pane for `owner`: its tree, ending its strip with the pane's own controls.
fn files_card(s: &Screen<'_>, owner: &str) -> El {
    let body = files_key(s, owner)
        .and_then(|key| {
            let chrome = |tools: Vec<El>| controls_after(s, SideKind::Files, owner, tools);
            super::points::draw_scoped(s, &key, Some(&chrome), None)
        })
        .unwrap_or_else(div);
    div()
        .id(files_body_id(owner))
        .sense(Sense::HOVER)
        .col()
        .grow()
        .min_h(0.0)
        .min_w0()
        .bg(pane_bg())
        .role(Role::Group, "Files")
        .child(body)
}

/// The shells a session holds, one in front.
fn terminals_card(s: &Screen<'_>, owner: &str) -> El {
    let Some(panel) = s.st.side.saved.panels.get(owner) else {
        return div();
    };
    let active = panel.active();
    let tabs: Vec<Tab> = panel
        .terminals
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let name = s
                .store
                .session(id)
                .map_or_else(|| id.clone(), Session::title);
            Tab {
                name,
                parent: None,
                icon: None,
                italic: false,
                dirty: false,
                active: i == active,
                select: pane(SideAct::PanelTab(owner.to_owned(), i)),
                close: pane(SideAct::CloseTerminal(id.clone())),
                double: None,
                extract: Some(pane(SideAct::Extract(owner.to_owned(), id.clone()))),
            }
        })
        .collect();
    let strip = tab_strip(
        s,
        "terminals",
        tabs,
        Strip {
            leading: None,
            empty: None,
            add: Some((
                pane(SideAct::AddTerminal(owner.to_owned())),
                "New terminal in this session",
            )),
            overflow: None,
            trailing: controls(s, SideKind::Terminals, owner),
        },
    );
    let shell = panel.terminals.get(active).and_then(|id| {
        s.store
            .sessions
            .iter()
            .enumerate()
            .find(|(_, x)| x.info.id == *id)
    });
    let mut col = div()
        .col()
        .grow()
        .min_h(0.0)
        .min_w0()
        .bg(pane_bg())
        .role(Role::Group, "Terminals")
        .child(strip);
    if let Some((i, session)) = shell {
        let sid = session.info.id.as_str();
        let id = cards::term_id(sid);
        // A shell showing blocks shows them here too: its live rows alone would hide every finished command.
        let blocks = cards::split(s, session);
        let body = match blocks {
            Some(sh) => cards::split_body(s, i, sh),
            None => custom(cards::CARD_ID_BASE + i as u64).grow(),
        };
        let surface = div()
            .row()
            .grow()
            .min_h(0.0)
            .pt(2.0)
            .bg(c(look::SURFACE_SUNKEN))
            .child_if(blocks.is_some().then(|| div().w(8.0).shrink0()))
            .child(body)
            .child_if(blocks.is_none().then(|| cards::scrollbar(s, sid)).flatten());
        s.acts
            .bind(id, pane(SideAct::FocusTerminal(sid.to_owned())));
        let surface = s
            .acts
            .context(
                surface,
                id,
                Act::Open(crate::state::MenuKind::Terminal(sid.to_owned())),
            )
            .on(Action::Click);
        let surface = if blocks.is_some() {
            surface
        } else {
            surface.sense(Sense::WHEEL | Sense::MOVE | Sense::DOUBLE)
        };
        col = col
            .child(surface)
            .child_if(cards::composer(s, session, true));
    }
    col
}

/// `owner`'s panes stacked, resizable, one maximized over
/// the rest.
fn column(s: &Screen<'_>, owner: &str) -> Option<El> {
    let entries = s.st.side.entries(owner);
    if entries.is_empty() {
        return None;
    }
    let max = s.st.side.maximized_in(owner);
    let weights = s.st.side.weights(owner);
    let mut col = div().col().grow().min_h(0.0).min_w0();
    for (i, kind) in entries.iter().enumerate() {
        if max.is_some_and(|m| m != *kind) {
            continue;
        }
        if i > 0 && max.is_none() {
            col = col.child(divider(
                s,
                column_divider_id(owner, i - 1),
                false,
                &format!("Resize {} panel", kind.key()),
            ));
        }
        let el = match kind {
            SideKind::Files => files_card(s, owner),
            SideKind::Terminals => terminals_card(s, owner),
        };
        let share = if max.is_some() {
            1.0
        } else {
            weights.get(i).copied().unwrap_or(1.0)
        };
        col = col.child(div().col().flex(share).min_h(0.0).min_w0().child(el));
    }
    Some(col)
}

/// The whole `PaneColumn`: the browser and device over the files and
/// terminals, or the one maximized pane alone, and whether one is.
fn all_panes(s: &Screen<'_>, owner: &str, session: &Session) -> Option<(El, bool)> {
    let web = super::panes::column(s, session);
    if web.is_some() && super::panes::maximized(s, owner) {
        return web.map(|w| (w, true));
    }
    let own = column(s, owner);
    if s.st.side.maximized_in(owner).is_some() {
        return own.map(|o| (o, true));
    }
    let both = match (web, own) {
        (Some(w), Some(o)) => div()
            .col()
            .grow()
            .min_h(0.0)
            .min_w0()
            .child(div().col().flex(1.0).min_h(0.0).min_w0().child(w))
            .child(div().h(1.0).w_full().shrink0().bg(white(0.06)))
            .child(div().col().flex(1.0).min_h(0.0).min_w0().child(o)),
        (w, o) => w.or(o)?,
    };
    Some((both, false))
}

/// A session's card body: its terminal column (header, terminal, status
/// bar) beside its pane column, split by the stored ratio; a maximized
/// pane takes the body and the header and status bar span the card.
pub(super) fn frame(
    s: &Screen<'_>,
    session: &Session,
    header: Option<El>,
    term: El,
    status: El,
) -> El {
    let owner = session.info.id.as_str();
    let Some((panes, maximized)) = all_panes(s, owner, session) else {
        return div()
            .col()
            .grow()
            .min_h(0.0)
            .child_if(header)
            .child(term)
            .child(status);
    };
    if maximized {
        return div()
            .col()
            .grow()
            .min_h(0.0)
            .child_if(header)
            .child(div().row().grow().min_h(0.0).child(panes))
            .child(status);
    }
    let r = s.st.side.terminal_ratio(owner);
    let left = div()
        .col()
        .flex(r)
        .min_w0()
        .min_h(0.0)
        .child_if(header)
        .child(term)
        .child(status);
    div()
        .row()
        .grow()
        .min_h(0.0)
        .child(left)
        .child(divider(
            s,
            card_divider_id(owner),
            true,
            "Resize terminal and panels",
        ))
        .child(div().row().flex(1.0 - r).min_w0().min_h(0.0).child(panes))
}

/// A card an extension's rows make, as a file popped out of a session: its
/// tree drawn for the row, with the session's project and branch before the
/// card's own buttons.
pub(super) fn ext_card(s: &Screen<'_>, card: &str) -> El {
    let Some((key, owner)) = card_of(s, card) else {
        return div();
    };
    let Some(owner) = s.store.session(&owner) else {
        return div();
    };
    let selected = s.st.selected.as_deref() == Some(card);
    let label = div()
        .row()
        .items_center()
        .gap(4.0)
        .min_w0()
        .child(
            text("\u{b7}", 11.0, c(look::INK).alpha(0.2))
                .line_height(16.0)
                .silent(),
        )
        .child(
            text(
                truncate(&owner.info.project_name, 20),
                11.0,
                c(look::INK).alpha(look::INK_SECONDARY),
            )
            .line_height(16.0)
            .silent(),
        )
        .child_if(cards::branch_chip(s, owner, 1.0));
    let name = card_title(s, card);
    let id = crate::act::Acts::id("editor-card", &pane(SideAct::SelectCard(card.to_owned())));
    let body = super::points::draw_scoped(s, &key, None, Some(label)).unwrap_or_else(div);
    let el = div()
        .col()
        .grow()
        .min_w0()
        .min_h(0.0)
        .bg(c(look::SURFACE_RAISED))
        .border(
            1.0,
            if selected {
                white(0.4)
            } else {
                white(0.06).mix(white(0.12), s.ui.hover(id))
            },
        )
        .role(Role::Group, name.as_str())
        .selected(selected)
        .child(body.grow());
    s.acts
        .click(
            el,
            "editor-card",
            pane(SideAct::SelectCard(card.to_owned())),
        )
        .sense(Sense::HOVER)
}

/// The question an extension's tree asks, as `window.confirm` asks it.
pub(super) fn ask(s: &Screen<'_>) -> El {
    let Some((_, q)) = &s.st.ask else {
        return div();
    };
    alert(
        s,
        "discard",
        &q.title,
        None,
        (q.ok.as_deref().unwrap_or("OK"), Act::Answer(true)),
        Act::Answer(false),
    )
}

/// A small alert: a title, an optional line under it, Cancel and the
/// default button. `place` keeps its controls' ids apart from other alerts'.
pub(super) fn alert(
    s: &Screen<'_>,
    place: &str,
    title: &str,
    info: Option<&str>,
    (label, ok): (&str, Act),
    cancel: Act,
) -> El {
    let cancel_id = s.acts.on(&format!("{place}-cancel"), cancel.clone());
    let ok = s.acts.on(&format!("{place}-ok"), ok);
    let button = |id: Id, label: &str, strong: bool| {
        div()
            .id(id)
            .on(Action::Click)
            .px(14.0)
            .py(6.0)
            .rounded(6.0)
            .bg(if strong {
                c(look::BLUE_400).alpha(0.85 + 0.15 * s.ui.hover(id))
            } else {
                white(0.06).mix(white(0.10), s.ui.hover(id))
            })
            .role(Role::Button, label)
            .child(
                text(
                    label,
                    13.0,
                    if strong {
                        c(look::WHITE)
                    } else {
                        c(look::GRAY_200)
                    },
                )
                .line_height(18.0)
                .silent(),
            )
    };
    let panel = div()
        .col()
        .w(300.0)
        .p(16.0)
        .gap(16.0)
        .rounded(10.0)
        .border(1.0, white(0.1))
        .bg(c(look::SURFACE_OVERLAY))
        .role(Role::AlertDialog, title)
        .child(
            text(title, 13.0, c(look::GRAY_200))
                .weight(500)
                .line_height(18.0)
                .silent(),
        )
        .child_if(info.map(|info| {
            text(info, 12.0, c(look::GRAY_400))
                .line_height(16.0)
                .silent()
        }))
        .child(
            div()
                .row()
                .justify_end()
                .gap(8.0)
                .child(button(cancel_id, "Cancel", false))
                .child(button(ok, label, true)),
        );
    let panel = s.acts.click(panel, place, Act::Nothing);
    let bg = s.acts.on(&format!("{place}-backdrop"), cancel);
    div()
        .id(bg)
        .on(Action::Click)
        .absolute(0.0, 0.0)
        .size(s.size.0, s.size.1)
        .bg(crate::ui::theme::shadow(0.3))
        .layered()
        .center()
        .child(panel)
}

/// The card menu's folder and shells entries: act, icon, label and their Mod+Shift letter, if any.
pub(super) fn pane_entries(
    s: &Screen<'_>,
    session: &Session,
) -> Vec<(Act, &'static str, &'static str, Option<char>)> {
    let sid = session.info.id.clone();
    let mut out = Vec::new();
    if files_key(s, &sid).is_some() {
        let open = s.st.side.files.contains(&sid);
        out.push((
            pane(SideAct::ToggleFiles(sid.clone())),
            "folder-open",
            if open { "Hide folder" } else { "Show folder" },
            Some('O'),
        ));
    }
    let has = s.st.side.saved.panels.contains_key(&sid);
    if !session.is_shell() || has {
        out.push((
            pane(SideAct::ToggleTerminals(sid)),
            "square-terminal",
            if has {
                "Close shells"
            } else {
                "New shell here"
            },
            super::SHELLS_KEY.then_some('T'),
        ));
    }
    out
}

/// The git chips of a card's status bar: changes, and Open in.
pub(super) fn status_items(s: &Screen<'_>, session: &Session, dim: f32) -> Vec<El> {
    let mut out = Vec::new();
    if let Some(i) = super::git::indicator(s, session, dim) {
        out.push(i);
    }
    if let Some(o) = super::git::open_in(s, session, dim) {
        out.push(o);
    }
    out
}

/// What a Git act on a branch chip looks like, for the cards.
pub(super) fn branch_act(session: &Session) -> Act {
    Act::Git(GitAct::Branches(session.info.id.clone()))
}
