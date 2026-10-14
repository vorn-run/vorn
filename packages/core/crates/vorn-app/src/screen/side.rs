//! The panes beside a session's terminal (`PaneColumn`, `SplitDivider`,
//! `PaneCard`, `PaneTabStrip`), its Files pane (`FilesCard`,
//! `FileTreeExplorer`) and Terminals panel (`TerminalsCard`), an open file
//! (`FilePanel`, `CodeEditor`), and a file popped out into a card of its
//! own (`EditorCard`).

use super::cards;
use super::{c, ico, truncate, white, Screen};
use crate::act::Act;
use crate::client::Session;
use crate::editor::{self, Buffer, EditAct, ReadOnly, LINE_HEIGHT};
use crate::git::GitAct;
use crate::look;
use crate::side::{pane_id, tab_key, SideAct, SideKind, TreeRow};
use crate::state::State;
use crate::ui::{
    code, custom, div, field, icon, text, tooltip, Action, El, Id, Rgba, Role, Sense, TextStyle,
};

/// The pane grey (`PANE_SURFACE`).
fn pane_bg() -> Rgba {
    c(look::SURFACE_PANEL)
}

fn pane(a: SideAct) -> Act {
    Act::Side(a)
}

fn edit(a: EditAct) -> Act {
    Act::Edit(a)
}

pub fn editor_id(key: &str) -> Id {
    Id::new("editor").child(key)
}

pub fn find_id(key: &str) -> Id {
    Id::new("editor-find").child(key)
}

pub fn scroller_id(key: &str) -> Id {
    Id::new("editor-scroll").child(key)
}

pub fn files_body_id(owner: &str) -> Id {
    Id::new("files-body").child(owner)
}

pub fn filter_id(owner: &str) -> Id {
    Id::new("files-filter").child(owner)
}

fn card_divider_id(owner: &str) -> Id {
    Id::new("card-divider").child(owner)
}

fn column_divider_id(owner: &str, i: usize) -> Id {
    Id::new("pane-divider").child((owner, i))
}

fn tree_divider_id(owner: &str) -> Id {
    Id::new("tree-divider").child(owner)
}

/// What a dragged divider resizes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneDivider {
    /// The terminal against `owner`'s pane column.
    Card(String),
    /// Between pane `i` and the next in `owner`'s column.
    Column(String, usize),
    /// A Files pane's tree against its file.
    Tree,
    /// The Changes panel's width.
    Changes,
}

/// Which divider `id` is.
pub fn divider_of(st: &State, id: Id) -> Option<PaneDivider> {
    if id == super::git::changes_resize_id() {
        return Some(PaneDivider::Changes);
    }
    for owner in st
        .side
        .saved
        .files
        .keys()
        .chain(st.side.saved.panels.keys())
    {
        if id == card_divider_id(owner) {
            return Some(PaneDivider::Card(owner.clone()));
        }
        if id == tree_divider_id(owner) {
            return Some(PaneDivider::Tree);
        }
        for i in 0..2 {
            if id == column_divider_id(owner, i) {
                return Some(PaneDivider::Column(owner.clone(), i));
            }
        }
    }
    None
}

/// `SplitDivider`: a 1px seam with a 3px grab either side.
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
        .bg(white(0.06).mix(Rgba::hexa(look::BLUE_500, 0.4), hot))
        .role(Role::Splitter, label)
        .vertical(side_by_side);
    if side_by_side {
        line.w(1.0).h_full().hit_slop(3.0, 0.0)
    } else {
        line.h(1.0).w_full().hit_slop(0.0, 3.0)
    }
}

/// `FileTypeIcon`: a file's Seti glyph in its colour.
pub(crate) fn file_icon(name: &str, size: f32) -> El {
    let (ch, color) = crate::file_icons::resolve(name);
    match crate::file_icons::svg(ch) {
        Some(svg) => icon(svg, size, c(color)),
        None => div().size(size, size),
    }
}

/// `ICON_BUTTON`: `p-1 rounded hover:bg-white/10 text-ink`.
fn icon_button(s: &Screen<'_>, place: &str, act: Act, name: &str, label: &str, on: bool) -> El {
    let id = s.acts.on(place, act);
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

/// `PaneControls`: pop out, maximize or restore, close.
fn controls(s: &Screen<'_>, kind: SideKind, owner: &str, pop: Option<Act>) -> El {
    let id = pane_id(kind, owner);
    let max = s.st.side.saved.maximized.as_deref() == Some(id.as_str());
    let mut row = div().row().items_center().gap(2.0).shrink0();
    if let Some(p) = pop {
        row = row.child(icon_button(
            s,
            "pane-pop",
            p,
            "square-arrow-out-up-right",
            "Open as its own card",
            false,
        ));
    }
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

/// `PaneTabStrip`: the pane's title bar.
/// What a strip carries besides its tabs.
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

/// The base name of a path.
fn base(path: &str) -> &str {
    crate::state::basename(path)
}

fn parent(path: &str) -> String {
    let t = path.trim_end_matches(['/', '\\']);
    let dir = t.rsplit_once(['/', '\\']).map_or("", |(d, _)| d);
    base(dir).to_owned()
}

/// Whether `tabs` would overflow a strip `w` wide, as the strip measures.
fn overflows(names: &[&str], w: f32) -> bool {
    let need: f32 = names
        .iter()
        .map(|n| (n.chars().count().min(22) as f32 * 6.2 + 44.0).min(170.0) + 2.0)
        .sum();
    need > (w - 120.0).max(0.0)
}

/// `FilesCard`: the tree and the open files as tabs.
fn files_card(s: &Screen<'_>, owner: &str, session: &Session) -> El {
    let Some(fp) = s.st.side.saved.files.get(owner) else {
        return div();
    };
    let names: Vec<&str> = fp.tabs.iter().map(|p| base(p)).collect();
    let tabs: Vec<Tab> = fp
        .tabs
        .iter()
        .map(|p| {
            let n = base(p);
            let shared = names.iter().filter(|x| **x == n).count() > 1;
            Tab {
                name: n.to_owned(),
                parent: shared.then(|| parent(p)),
                icon: Some(file_icon(n, 12.0)),
                italic: fp.preview.as_deref() == Some(p.as_str()),
                dirty: s.st.editors.dirty(&tab_key(owner, p)),
                active: fp.active.as_deref() == Some(p.as_str()),
                select: pane(SideAct::Tab(owner.to_owned(), p.clone())),
                close: pane(SideAct::CloseTab(owner.to_owned(), p.clone())),
                double: Some(pane(SideAct::PinTab(owner.to_owned(), p.clone()))),
                extract: None,
            }
        })
        .collect();
    let has_tabs = !fp.tabs.is_empty();
    let narrow = s.st.side.narrow.contains(owner);
    let tree_shown = !has_tabs || fp.tree_visible;
    let file_shown = has_tabs && !(narrow && fp.tree_visible);
    let toggle_label = if tree_shown {
        "Hide file tree"
    } else {
        "Show file tree"
    };
    let toggle = icon_button(
        s,
        "files-tree",
        pane(SideAct::ToggleTree(owner.to_owned())),
        "list-tree",
        toggle_label,
        tree_shown,
    );
    let width = s.ui.rect_of(files_body_id(owner)).map_or(400.0, |r| r.w);
    let overflow =
        (has_tabs && overflows(&names, width)).then(|| pane(SideAct::TabsMenu(owner.to_owned())));
    let pop = fp
        .active
        .is_some()
        .then(|| pane(SideAct::PopOutActive(owner.to_owned())));
    let strip = tab_strip(
        s,
        "files",
        tabs,
        Strip {
            leading: Some(toggle),
            empty: Some("Files"),
            add: None,
            overflow,
            trailing: controls(s, SideKind::Files, owner, pop),
        },
    );
    let ratio = s.st.side.tree_ratio();
    let mut body = div()
        .id(files_body_id(owner))
        .sense(Sense::HOVER)
        .row()
        .grow()
        .min_h(0.0)
        .pt(2.0);
    if tree_shown {
        let col = tree(s, owner);
        body = body.child(if file_shown {
            col.flex(ratio)
        } else {
            col.flex(1.0)
        });
    }
    if tree_shown && file_shown {
        body = body.child(divider(s, tree_divider_id(owner), true, "Resize file tree"));
    }
    if file_shown {
        if let Some(path) = &fp.active {
            let key = tab_key(owner, path);
            let cwd = crate::git::session_cwd(&session.info);
            let panel = file_panel(s, &key, &cwd, None);
            body = body.child(if tree_shown {
                panel.flex(1.0 - ratio)
            } else {
                panel.flex(1.0)
            });
        }
    }
    div()
        .col()
        .grow()
        .min_h(0.0)
        .min_w0()
        .bg(pane_bg())
        .role(Role::Group, "Files")
        .child(strip)
        .child(body)
}

/// `FileTreePane` and its filter.
fn tree(s: &Screen<'_>, owner: &str) -> El {
    let col = div().col().min_w0().min_h(0.0);
    let Some(t) = s.st.side.trees.get(owner) else {
        return col;
    };
    if t.loading {
        return col
            .center()
            .child(ico("loader-circle", 20.0, 2.0, c(look::GRAY_500)));
    }
    if t.root.as_ref().is_none_or(Vec::is_empty) {
        return col.center().child(
            text("Empty directory", 14.0, c(look::GRAY_500))
                .line_height(20.0)
                .silent(),
        );
    }
    let fid = filter_id(owner);
    let focus = s.ui.focused(fid);
    let mut search = div()
        .row()
        .items_center()
        .gap(6.0)
        .grow()
        .min_w0()
        .px(8.0)
        .py(4.0)
        .rounded(4.0)
        .frames(fid)
        .bg(white(if focus { 0.13 } else { 0.09 }))
        .child(ico("search", 13.0, 2.0, c(look::GRAY_600)))
        .child(
            div().grow().min_w0().child(
                field(
                    fid,
                    "Filter files",
                    "Filter files\u{2026}",
                    TextStyle::ui(13.0),
                    c(look::GRAY_200),
                )
                .w_full(),
            ),
        );
    if !t.filter.is_empty() {
        let id = s.acts.on(
            "files-filter-clear",
            pane(SideAct::ClearFilter(owner.to_owned())),
        );
        search = search.child(
            div()
                .id(id)
                .on(Action::Click)
                .p(2.0)
                .role(Role::Button, "Clear filter")
                .child(ico(
                    "x",
                    11.0,
                    2.0,
                    c(look::GRAY_600).mix(c(look::WHITE), s.ui.hover(id)),
                )),
        );
    }
    let selected =
        s.st.side
            .saved
            .files
            .get(owner)
            .and_then(|f| f.active.as_deref());
    let mut list = div()
        .id(Id::new("files-tree").child(owner))
        .col()
        .grow()
        .min_h(0.0)
        .py(2.0)
        .scroll_y();
    for row in t.rows() {
        list = list.child(tree_row(s, owner, &row, selected));
    }
    if t.no_match() {
        list = list.child(
            div().px(12.0).py(8.0).child(
                text("No matching files loaded", 11.0, c(look::GRAY_600))
                    .italic()
                    .line_height(16.0)
                    .silent(),
            ),
        );
    }
    col.child(div().row().px(6.0).py(6.0).shrink0().child(search))
        .child(list)
}

const ROW_H: f32 = 22.0;
const INDENT: f32 = 16.0;
const BASE_LEFT: f32 = 8.0;

/// The indent guides of a row at `depth`.
fn guides(depth: usize) -> Vec<El> {
    (0..depth)
        .map(|i| {
            div()
                .absolute(BASE_LEFT + 7.0 + i as f32 * INDENT, 0.0)
                .w(1.0)
                .h(ROW_H)
                .bg(white(0.06))
        })
        .collect()
}

fn tree_row(s: &Screen<'_>, owner: &str, row: &TreeRow, selected: Option<&str>) -> El {
    match row {
        TreeRow::Empty { depth } => {
            let mut r = div()
                .row()
                .items_center()
                .h(ROW_H)
                .pl(BASE_LEFT + *depth as f32 * INDENT + 16.0)
                .child(
                    text("empty", 11.0, c(look::GRAY_600))
                        .italic()
                        .line_height(14.0)
                        .silent(),
                );
            for g in guides(*depth) {
                r = r.child(g);
            }
            r
        }
        TreeRow::Dir {
            entry,
            depth,
            open,
            loading,
        } => {
            let id = s.acts.on(
                "tree-dir",
                pane(SideAct::Dir(owner.to_owned(), entry.path.clone())),
            );
            let glyph = if *loading {
                ico("loader-circle", 14.0, 2.0, c(look::GRAY_600))
            } else {
                ico(
                    if *open {
                        "chevron-down"
                    } else {
                        "chevron-right"
                    },
                    14.0,
                    2.0,
                    c(look::GRAY_500),
                )
            };
            let mut r = div()
                .id(id)
                .on(Action::Click)
                .row()
                .items_center()
                .gap(5.0)
                .h(ROW_H)
                .pl(BASE_LEFT + *depth as f32 * INDENT)
                .pr(8.0)
                .bg(white(0.05 * s.ui.hover(id)))
                .role(Role::TreeItem, &entry.name)
                .expanded(*open)
                .child(glyph)
                .child(
                    text(&entry.name, 13.5, c(look::GRAY_300))
                        .line_height(16.0)
                        .truncate()
                        .silent(),
                );
            for g in guides(*depth) {
                r = r.child(g);
            }
            r
        }
        TreeRow::File { entry, depth } => {
            let act = pane(SideAct::File(owner.to_owned(), entry.path.clone()));
            let id = s.acts.on("tree-file", act);
            s.acts.double(
                id,
                pane(SideAct::PinFile(owner.to_owned(), entry.path.clone())),
            );
            let on = selected == Some(entry.path.as_str());
            let pop = s.acts.on(
                "tree-pop",
                pane(SideAct::PopOut(owner.to_owned(), entry.path.clone())),
            );
            let hot = s.ui.hovered(id) || s.ui.hovered(pop);
            let fg = if on {
                c(look::GRAY_200)
            } else if hot {
                c(look::GRAY_300)
            } else {
                c(look::GRAY_400)
            };
            let mut r = div()
                .id(id)
                .on(Action::Click)
                .sense(Sense::DOUBLE)
                .row()
                .items_center()
                .gap(5.0)
                .h(ROW_H)
                .pl(BASE_LEFT + *depth as f32 * INDENT + 16.0)
                .pr(4.0)
                .bg(if on {
                    white(0.10)
                } else {
                    white(0.05 * s.ui.hover(id))
                })
                .role(Role::TreeItem, &entry.name)
                .selected(on)
                .child(file_icon(&entry.name, 16.0))
                .child(
                    div().grow().min_w0().child(
                        text(&entry.name, 13.5, fg)
                            .line_height(16.0)
                            .truncate()
                            .silent(),
                    ),
                )
                .child(
                    div()
                        .id(pop)
                        .on(Action::Click)
                        .p(2.0)
                        .rounded(4.0)
                        .opacity(if hot { 1.0 } else { 0.0 })
                        .bg(white(0.08 * s.ui.hover(pop)))
                        .role(Role::Button, format!("Open {} as its own card", entry.name))
                        .child(ico(
                            "square-arrow-out-up-right",
                            11.0,
                            2.0,
                            c(look::INK).alpha(0.2).mix(c(look::WHITE), s.ui.hover(pop)),
                        )),
                );
            for g in guides(*depth) {
                r = r.child(g);
            }
            r
        }
    }
}

/// `ToolbarBtn`.
fn toolbar_btn(s: &Screen<'_>, act: Act, glyph: &str, label: &str, on: bool, off: bool) -> El {
    let id = s.acts.on("editor-tool", act);
    let mut b = div()
        .id(id)
        .p(4.0)
        .rounded(4.0)
        .shrink0()
        .bg(white(0.10 * if on { 1.0 } else { s.ui.hover(id) }))
        .role(Role::Button, label)
        .disabled(off)
        .opacity(if off { 0.4 } else { 1.0 })
        .child(ico(glyph, 14.0, 2.0, c(look::INK)));
    if !off {
        b = b.on(Action::Click);
    }
    tooltip(s.ui, b, id, label, None)
}

/// The path cut from the left, as `dir="rtl"` truncates it.
fn tail(path: &str, max: usize) -> String {
    let n = path.chars().count();
    if n <= max {
        return path.to_owned();
    }
    let rest: String = path.chars().skip(n - max + 1).collect();
    format!("\u{2026}{rest}")
}

/// `FilePanel`: the path strip, the find bar, the notices and the body.
pub(super) fn file_panel(s: &Screen<'_>, key: &str, cwd: &str, extra: Option<El>) -> El {
    let col = div().col().min_w0().min_h(0.0).bg(pane_bg());
    let Some(b) = s.st.editors.buffers.get(key) else {
        return col;
    };
    let name = base(&b.path);
    let rel = editor::rel_path(cwd, &b.path);
    let dirty = b.dirty();
    let can_find = b.content.is_some() && !b.loading;
    let mut strip = div()
        .row()
        .items_center()
        .gap(4.0)
        .pl(8.0)
        .pr(4.0)
        .py(2.0)
        .shrink0()
        .bg(pane_bg())
        .child(file_icon(name, 12.0))
        .child(
            div().row().justify_end().grow().min_w0().ml(4.0).child(
                text(tail(rel, 48), 11.0, c(look::GRAY_400))
                    .mono()
                    .line_height(16.0)
                    .truncate()
                    .role(Role::Label, &b.path),
            ),
        );
    if dirty {
        strip = strip.child(
            div()
                .mx(4.0)
                .size(6.0, 6.0)
                .rounded(3.0)
                .bg(c(look::AMBER_400))
                .role(Role::Image, "Unsaved changes"),
        );
    }
    strip = strip.child(toolbar_btn(
        s,
        edit(EditAct::ToggleFind(key.to_owned())),
        "search",
        "Find in file",
        b.find.is_some(),
        !can_find,
    ));
    if dirty {
        strip = strip
            .child(toolbar_btn(
                s,
                edit(EditAct::Save(key.to_owned())),
                "save",
                if b.saving { "Saving\u{2026}" } else { "Save" },
                false,
                b.saving,
            ))
            .child(toolbar_btn(
                s,
                edit(EditAct::Discard(key.to_owned())),
                "undo-2",
                "Discard changes",
                false,
                false,
            ));
    }
    let mut col = col.child(strip.child_if(extra));
    if let (Some(f), true) = (&b.find, can_find) {
        col = col.child(find_bar(s, key, b, f));
    }
    if b.conflict {
        col = col.child(conflict(s, key));
    }
    if b.read_only == Some(ReadOnly::Truncated) {
        col = col.child(
            div()
                .px(12.0)
                .py(4.0)
                .border_top(1.0, white(0.06))
                .shrink0()
                .child(
                    text(
                        "Too large to edit here. Showing the first part, read-only.",
                        11.0,
                        c(look::INK).alpha(look::INK_FAINT),
                    )
                    .line_height(16.0)
                    .silent(),
                ),
        );
    }
    let body = if b.loading {
        div()
            .col()
            .center()
            .grow()
            .child(ico("loader-circle", 16.0, 2.0, c(look::GRAY_500)))
    } else if matches!(b.read_only, Some(ReadOnly::Binary | ReadOnly::Unreadable)) {
        let msg = if b.read_only == Some(ReadOnly::Binary) {
            "Binary file \u{2014} preview unavailable"
        } else {
            "This file could not be read"
        };
        div().col().center().grow().child(
            text(msg, 13.0, c(look::GRAY_600))
                .line_height(18.0)
                .silent(),
        )
    } else if b.ready {
        editor_view(key, b)
    } else if b.content.is_some() {
        read_view(key, b)
    } else {
        div().grow()
    };
    col = col.child(body);
    if let Some(e) = &b.error {
        col = col.child(
            div()
                .px(12.0)
                .py(4.0)
                .bg(c(look::DANGER).alpha(0.1))
                .border_top(1.0, white(0.06))
                .shrink0()
                .child(text(e, 11.0, c(look::DANGER)).line_height(16.0).wrapping()),
        );
    }
    col
}

fn find_bar(s: &Screen<'_>, key: &str, b: &Buffer, f: &editor::Find) -> El {
    let n = b.matches().len();
    let count = if n == 0 {
        "0/0".to_owned()
    } else {
        format!("{}/{n}", f.index % n + 1)
    };
    let arrow = |place: &str, d: i32, glyph: &str, label: &str| {
        let id = s.acts.on(place, edit(EditAct::FindStep(key.to_owned(), d)));
        let off = n == 0;
        let mut el = div()
            .id(id)
            .px(4.0)
            .role(Role::Button, label)
            .disabled(off)
            .opacity(if off { 0.4 } else { 1.0 })
            .child(
                text(
                    glyph,
                    11.0,
                    c(look::GRAY_500).mix(c(look::WHITE), s.ui.hover(id)),
                )
                .line_height(16.0)
                .silent(),
            );
        if !off {
            el = el.on(Action::Click);
        }
        tooltip(s.ui, el, id, label, None)
    };
    let close = s
        .acts
        .on("find-close", edit(EditAct::CloseFind(key.to_owned())));
    div()
        .row()
        .items_center()
        .gap(8.0)
        .px(12.0)
        .py(4.0)
        .shrink0()
        .bg(pane_bg())
        .child(ico("search", 12.0, 2.0, c(look::GRAY_500)))
        .child(
            div().grow().min_w0().frames(find_id(key)).child(
                field(
                    find_id(key),
                    "Find in file",
                    "Find in file",
                    TextStyle::mono(13.0),
                    c(look::GRAY_200),
                )
                .w_full(),
            ),
        )
        .child(
            text(count, 11.0, c(look::GRAY_500))
                .line_height(16.0)
                .silent(),
        )
        .child(arrow("find-prev", -1, "\u{2191}", "Previous (Shift+Enter)"))
        .child(arrow("find-next", 1, "\u{2193}", "Next (Enter)"))
        .child(tooltip(
            s.ui,
            div()
                .id(close)
                .on(Action::Click)
                .p(2.0)
                .role(Role::Button, "Close (Esc)")
                .child(ico(
                    "x",
                    12.0,
                    2.0,
                    c(look::GRAY_500).mix(c(look::WHITE), s.ui.hover(close)),
                )),
            close,
            "Close (Esc)",
            None,
        ))
}

/// The moved-file banner's three ways out.
fn conflict(s: &Screen<'_>, key: &str) -> El {
    let link = |place: &str, label: &str, act: Act| {
        let id = s.acts.on(place, act);
        div()
            .id(id)
            .on(Action::Click)
            .role(Role::Button, label)
            .child(
                text(
                    label,
                    11.0,
                    c(look::GRAY_300).mix(c(look::GRAY_100), s.ui.hover(id)),
                )
                .line_height(16.0)
                .silent(),
            )
            .child(div().h(1.0).w_full().bg(c(look::GRAY_300).alpha(0.6)))
    };
    div()
        .row()
        .items_center()
        .gap(12.0)
        .px(12.0)
        .py(6.0)
        .bg(c(look::BRONZO).alpha(0.1))
        .border_top(1.0, white(0.06))
        .shrink0()
        .child(
            div().grow().min_w0().child(
                text(
                    "This file changed on disk while your edit was open.",
                    11.0,
                    c(look::BRONZO),
                )
                .line_height(16.0)
                .wrapping(),
            ),
        )
        .child(link(
            "conflict-save",
            "Save mine anyway",
            edit(EditAct::SaveAnyway(key.to_owned())),
        ))
        .child(link(
            "conflict-disk",
            "Discard mine",
            edit(EditAct::TakeDisk(key.to_owned())),
        ))
        .child(link(
            "conflict-keep",
            "Keep editing",
            edit(EditAct::KeepEditing(key.to_owned())),
        ))
}

/// Right-aligned line numbers in a mono column, one per line.
fn gutter(lines: usize, from_top: f32) -> El {
    let w = lines.max(1).to_string().len();
    let numbers: Vec<String> = (1..=lines.max(1)).map(|n| format!("{n:>w$}")).collect();
    div()
        .pl(8.0)
        .pr(12.0)
        .pt(from_top)
        .shrink0()
        .bg(pane_bg())
        .child(
            text(numbers.join("\n"), 12.0, c(look::GRAY_600))
                .mono()
                .line_height(LINE_HEIGHT)
                .silent(),
        )
}

/// A span list with `range` laid over it in `color`, still sorted and
/// apart (the active find match's text).
fn overlay(
    spans: &[(std::ops::Range<usize>, Rgba)],
    range: std::ops::Range<usize>,
    color: Rgba,
) -> Vec<(std::ops::Range<usize>, Rgba)> {
    let mut out = Vec::with_capacity(spans.len() + 2);
    for (r, col) in spans {
        if r.end <= range.start || r.start >= range.end {
            out.push((r.clone(), *col));
            continue;
        }
        if r.start < range.start {
            out.push((r.start..range.start, *col));
        }
        if r.end > range.end {
            out.push((range.end..r.end, *col));
        }
    }
    out.push((range, color));
    out.sort_by_key(|(r, _)| r.start);
    out
}

/// `CodeEditor`: the gutter beside the coloured, editable text, with the
/// find marks.
fn editor_view(key: &str, b: &Buffer) -> El {
    let text_ = b.text();
    let lines = text_.split('\n').count();
    let mut spans: Vec<(std::ops::Range<usize>, Rgba)> = b
        .coloured
        .as_ref()
        .filter(|(t, _)| t == text_)
        .map(|(_, sp)| {
            sp.iter()
                .map(|(r, (rgb, a))| (r.clone(), Rgba::hexa(*rgb, f32::from(*a) / 255.0)))
                .collect()
        })
        .unwrap_or_default();
    let mut marks = Vec::new();
    if let Some(f) = &b.find {
        let m = b.matches();
        let active = (!m.is_empty()).then(|| f.index % m.len());
        for (i, hit) in m.iter().enumerate() {
            let on = Some(i) == active;
            marks.push((
                hit.range.0..hit.range.1,
                c(look::AMBER_300).alpha(if on { 0.7 } else { 0.25 }),
            ));
            spans = overlay(
                &spans,
                hit.range.0..hit.range.1,
                if on { c(0x000000) } else { c(look::GRAY_100) },
            );
        }
    }
    let style = TextStyle {
        line_height: LINE_HEIGHT,
        ..TextStyle::mono(13.0)
    };
    let field = code(
        editor_id(key),
        &format!("Edit {}", base(&b.path)),
        style,
        c(look::GRAY_300),
    )
    .edit_colors(spans, marks)
    .w_full()
    .h(lines as f32 * LINE_HEIGHT);
    div()
        .id(scroller_id(key))
        .col()
        .grow()
        .min_h(0.0)
        .scroll_y()
        .child(
            div()
                .row()
                .child(gutter(lines, 4.0))
                .child(div().grow().min_w0().pt(4.0).pb(4.0).pr(12.0).child(field)),
        )
}

/// `ReadView`: a file shown but not editable, capped at 2000 lines.
fn read_view(key: &str, b: &Buffer) -> El {
    let all: Vec<&str> = b.text().split('\n').collect();
    let capped = all.len() > editor::MAX_HIGHLIGHT_LINES;
    let shown = &all[..all.len().min(editor::MAX_HIGHLIGHT_LINES)];
    let mut col = div()
        .id(scroller_id(key))
        .col()
        .grow()
        .min_h(0.0)
        .scroll_y()
        .child(
            div().row().child(gutter(shown.len(), 0.0)).child(
                div().grow().min_w0().px(4.0).child(
                    text(shown.join("\n"), 13.0, c(look::GRAY_400))
                        .mono()
                        .line_height(LINE_HEIGHT)
                        .silent(),
                ),
            ),
        );
    if capped {
        col = col.child(
            div().px(12.0).py(8.0).child(
                text(
                    format!(
                        "Showing first {} of {} lines",
                        editor::MAX_HIGHLIGHT_LINES,
                        all.len()
                    ),
                    11.0,
                    c(look::GRAY_600),
                )
                .italic()
                .line_height(16.0)
                .silent(),
            ),
        );
    }
    col
}

/// `TerminalsCard`: the shells a session holds, one in front.
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
            trailing: controls(s, SideKind::Terminals, owner, None),
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
            .child(div().w(8.0).shrink0())
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

/// `PaneColumn`: `owner`'s panes stacked, resizable, one maximized over
/// the rest.
fn column(s: &Screen<'_>, owner: &str, session: &Session) -> Option<El> {
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
            SideKind::Files => files_card(s, owner, session),
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
    let own = column(s, owner, session);
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

/// `EditorCard`: a file popped out of a session, as a cell of its own.
pub(super) fn editor_card(s: &Screen<'_>, card: &str) -> El {
    let Some(cd) = s.st.side.saved.cards.get(card) else {
        return div();
    };
    let Some(owner) = s.store.session(&cd.session_id) else {
        return div();
    };
    let selected = s.st.selected.as_deref() == Some(card);
    let name = base(&cd.file_path);
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
    let ctl = div()
        .row()
        .items_center()
        .gap(2.0)
        .shrink0()
        .child(icon_button(
            s,
            "card-return",
            pane(SideAct::ReturnCard(card.to_owned())),
            "corner-down-left",
            "Put back in its session card",
            false,
        ))
        .child(icon_button(
            s,
            "card-close",
            pane(SideAct::CloseCard(card.to_owned())),
            "x",
            "Close",
            false,
        ));
    let extra = div().row().items_center().gap(4.0).child(label).child(ctl);
    let cwd = crate::git::session_cwd(&owner.info);
    let id = crate::act::Acts::id("editor-card", &pane(SideAct::SelectCard(card.to_owned())));
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
        .role(Role::Group, name)
        .selected(selected)
        .child(file_panel(s, card, &cwd, Some(extra)).grow());
    s.acts
        .click(
            el,
            "editor-card",
            pane(SideAct::SelectCard(card.to_owned())),
        )
        .sense(Sense::HOVER)
}

/// The Files pane's overflow list of its tabs.
pub(super) fn tab_items(s: &Screen<'_>, owner: &str) -> Vec<super::menus::Item> {
    let Some(fp) = s.st.side.saved.files.get(owner) else {
        return Vec::new();
    };
    let mut out: Vec<super::menus::Item> = fp
        .tabs
        .iter()
        .map(|p| {
            let mut it = super::menus::Item::option(
                base(p),
                fp.active.as_deref() == Some(p.as_str()),
                pane(SideAct::Tab(owner.to_owned(), p.clone())),
            )
            .mark(file_icon(base(p), 12.0));
            it.picker = true;
            it
        })
        .collect();
    out.push(
        super::menus::Item::option(
            "Close saved tabs",
            false,
            pane(SideAct::CloseSavedTabs(owner.to_owned())),
        )
        .sep(),
    );
    out
}

/// "Discard unsaved changes?", as `window.confirm` asks it.
pub(super) fn discard(s: &Screen<'_>) -> El {
    alert(
        s,
        "discard",
        "Discard unsaved changes?",
        None,
        ("OK", edit(EditAct::ConfirmDiscard)),
        edit(EditAct::CancelDiscard),
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
        .bg(Rgba::hexa(0, 0.3))
        .layered()
        .center()
        .child(panel)
}

/// The card header's pane buttons: Browse files, and the terminals panel
/// for an agent (or a shell that has one open), as act, icon and label.
pub(super) fn header_buttons(
    s: &Screen<'_>,
    session: &Session,
) -> Vec<(Act, &'static str, &'static str)> {
    let sid = session.info.id.clone();
    let mut out = vec![(
        pane(SideAct::ToggleFiles(sid.clone())),
        "folder-open",
        "Browse files",
    )];
    let has = s.st.side.saved.panels.contains_key(&sid);
    if !session.is_shell() || has {
        out.push((
            pane(SideAct::ToggleTerminals(sid)),
            "square-terminal",
            if has {
                "Close terminals"
            } else {
                "Add a terminal"
            },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_active_match_is_laid_over_the_colours() {
        let a = Rgba::hex(0x111111);
        let b = Rgba::hex(0x222222);
        let m = Rgba::hex(0);
        let out = overlay(&[(0..4, a), (4..8, b)], 2..6, m);
        let ranges: Vec<_> = out.iter().map(|(r, _)| r.clone()).collect();
        assert_eq!(ranges, vec![0..2, 2..6, 6..8]);
        assert!(out.windows(2).all(|w| w[0].0.end <= w[1].0.start));
    }

    #[test]
    fn long_paths_keep_their_ends() {
        assert_eq!(tail("src/app.ts", 48), "src/app.ts");
        let t = tail(
            "a/very/long/path/that/goes/on/and/on/and/on/to/the/file.ts",
            20,
        );
        assert!(t.starts_with('\u{2026}') && t.ends_with("file.ts"));
        assert_eq!(t.chars().count(), 20);
    }

    #[test]
    fn tabs_overflow_by_their_names() {
        assert!(!overflows(&["a.ts"], 400.0));
        assert!(overflows(&["a-long-file-name.ts"; 6], 400.0));
    }
}
