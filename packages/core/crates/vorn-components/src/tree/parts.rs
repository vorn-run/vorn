//! The parts a pane draws whose hover, typing and dragging stay in the
//! client: a strip of tabs, an outline of folders and files, and a split.

use vorn_tree::{Code, Event, EventKind, Line, LineKind, Outline, Tabs, Tool};
use vornui::accesskit::Role;
use vornui::native::Sem;
use vornui::theme::color;
use vornui::widgets;
use vornui::{div, text, Action, El, Id, Place, Rgba, Sense, TextStyle};

use super::{Cx, Ev};

const ROW_H: f32 = 22.0;
const INDENT: f32 = 16.0;
const BASE_LEFT: f32 = 8.0;

fn white(a: f32) -> Rgba {
    Rgba::white(a)
}

/// `s` cut to `max` characters with an ellipsis.
fn cut(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((at, _)) => format!("{}\u{2026}", s[..at].trim_end()),
        None => s.to_owned(),
    }
}

/// Whether tabs named `names` would overflow a strip `w` wide.
pub fn overflows(names: &[&str], w: f32) -> bool {
    let need: f32 = names
        .iter()
        .map(|n| (n.chars().count().min(22) as f32 * 6.2 + 44.0).min(170.0) + 2.0)
        .sum();
    need > (w - 120.0).max(0.0)
}

/// What a code part shows where the client draws none: its text, read only.
pub fn plain_code(c: &Code) -> El {
    let t = c.draft.as_deref().unwrap_or(&c.text);
    div().col().grow().min_h(0.0).scroll_y().child(
        text(t, 13.0, color::GRAY_400)
            .mono()
            .line_height(21.0)
            .silent(),
    )
}

fn event(node: &str, kind: EventKind, value: &str) -> Event {
    Event::new(node, kind, value)
}

impl Cx<'_> {
    fn send(&self, node: &str, kind: EventKind, value: &str) -> Id {
        self.bind.on(Ev::Send {
            event: event(node, kind, value),
            ask: false,
        })
    }

    /// A strip's tool: an icon with its label as the tooltip.
    fn tool(&self, t: &Tool) -> El {
        let id = self.bind.on(Ev::Send {
            event: event(&t.id, EventKind::Click, ""),
            ask: false,
        });
        let b = div()
            .id(id)
            .on(Action::Click)
            .p(4.0)
            .rounded(4.0)
            .shrink0()
            .bg(white(
                0.10 * self.ui.hover(id).max(if t.on { 1.0 } else { 0.0 }),
            ))
            .role(Role::Button, t.label.as_str())
            .child(self.icon(&t.icon, 14.0, color::INK));
        let b = self.sem(b, || Sem::Button {
            style: "plain",
            confirm: None,
            icon: Some(t.icon.clone()),
        });
        widgets::tooltip(self.ui, b, id, &t.label, None)
    }

    /// A file's icon for `name`, as the client draws file types.
    fn file_icon(&self, name: &str, px: f32) -> El {
        match self.bind.icon(&format!("file:{name}"), px) {
            Some(i) => i.shrink0(),
            None => div().size(px, px),
        }
    }

    pub(super) fn tabs(&self, node: &str, t: &Tabs) -> El {
        let strip_id = Id::new("tree-tabs").child(node);
        let mut list = div()
            .row()
            .gap(2.0)
            .grow()
            .min_w0()
            .clip()
            .role(Role::TabList, node);
        if t.tabs.is_empty() {
            if let Some(e) = &t.empty {
                list = list.child(
                    div().pl(4.0).row().items_center().child(
                        text(e.as_str(), 12.0, color::INK)
                            .weight(500)
                            .line_height(16.0)
                            .silent(),
                    ),
                );
            }
        }
        for tab in &t.tabs {
            let id = self.send(node, EventKind::Select, &tab.id);
            self.bind.double(id, event(node, EventKind::Open, &tab.id));
            let close_id = self.send(node, EventKind::Close, &tab.id);
            let hot = self.ui.hovered(id) || self.ui.hovered(close_id);
            let fg = if tab.active {
                color::GRAY_200
            } else {
                color::GRAY_500.mix(color::GRAY_300, self.ui.hover(id))
            };
            let mut name = text(cut(&tab.label, 22), 11.0, fg)
                .line_height(16.0)
                .silent();
            if tab.preview {
                name = name.italic();
            }
            let mut el = div()
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
                .bg(if tab.active {
                    white(0.06)
                } else {
                    white(0.03 * self.ui.hover(id))
                })
                .role(Role::Tab, tab.label.as_str())
                .selected(tab.active);
            if tab.file {
                el = el.child(self.file_icon(&tab.label, 12.0));
            }
            el = el.child(name);
            if let Some(n) = &tab.note {
                el = el.child(
                    text(n.as_str(), 11.0, color::INK.alpha(0.35))
                        .line_height(16.0)
                        .silent(),
                );
            }
            let slot = if tab.dirty && !hot {
                div()
                    .size(14.0, 14.0)
                    .center()
                    .role(Role::Image, "Unsaved changes")
                    .child(div().size(6.0, 6.0).rounded(3.0).bg(color::AMBER_400))
            } else {
                div()
                    .id(close_id)
                    .on(Action::Click)
                    .p(2.0)
                    .rounded(4.0)
                    .opacity(if hot { 1.0 } else { 0.0 })
                    .role(Role::Button, format!("Close {}", tab.label))
                    .child(crate::lucide(
                        "x",
                        10.0,
                        2.5,
                        color::GRAY_600.mix(color::WHITE, self.ui.hover(close_id)),
                    ))
            };
            list = list.child(el.child(slot));
        }
        let mut strip = div()
            .id(strip_id)
            .sense(Sense::HOVER)
            .row()
            .items_center()
            .gap(4.0)
            .pl(6.0)
            .pr(4.0)
            .pt(4.0)
            .shrink0()
            .children(t.lead.iter().map(|l| self.tool(l)))
            .child(list);
        let names: Vec<&str> = t.tabs.iter().map(|x| x.label.as_str()).collect();
        let width = self.ui.rect_of(strip_id).map_or(400.0, |r| r.w);
        if !t.tabs.is_empty() && overflows(&names, width) {
            strip = strip.child(self.overflow(node, t));
        }
        let tools = t.tools.iter().map(|x| self.tool(x)).collect();
        strip.child(self.bind.chrome(node, tools))
    }

    /// The chevron that lists every tab, and the list while it is open.
    fn overflow(&self, node: &str, t: &Tabs) -> El {
        let id = self.bind.on(Ev::Menu {
            node: format!("{node}:all"),
        });
        let b = div()
            .id(id)
            .on(Action::Click)
            .p(4.0)
            .rounded(4.0)
            .shrink0()
            .bg(white(0.10 * self.ui.hover(id)))
            .role(Role::Button, "All open tabs")
            .child(crate::lucide("chevron-down", 14.0, 2.0, color::INK));
        let b = widgets::tooltip(self.ui, b, id, "All open tabs", None);
        if !self.bind.menu_open(id) {
            return b;
        }
        let row = |id: Id, label: &str, file: bool, on: bool| {
            let mut r = div()
                .id(id)
                .on(Action::Click)
                .row()
                .items_center()
                .gap(8.0)
                .px(10.0)
                .py(5.0)
                .rounded(4.0)
                .bg(white(0.06 * self.ui.hover(id)))
                .role(Role::MenuItem, label);
            if file {
                r = r.child(self.file_icon(label, 12.0));
            }
            r.child(
                text(label, 12.0, if on { color::WHITE } else { color::GRAY_300 })
                    .line_height(16.0)
                    .truncate()
                    .silent(),
            )
        };
        let mut menu = div()
            .col()
            .w(240.0)
            .p(4.0)
            .rounded(6.0)
            .border(1.0, white(0.1))
            .bg(color::SURFACE_OVERLAY)
            .role(Role::Menu, "All open tabs");
        for tab in &t.tabs {
            let id = self.bind.on(Ev::Item {
                event: event(node, EventKind::Select, &tab.id),
                slot: "menu".to_owned(),
            });
            menu = menu.child(row(id, &tab.label, tab.file, tab.active));
        }
        if !t.more.is_empty() {
            menu = menu.child(div().h(1.0).mt(4.0).mb(4.0).bg(white(0.06)));
        }
        for m in &t.more {
            let id = self.bind.on(Ev::Send {
                event: event(&m.id, EventKind::Click, ""),
                ask: false,
            });
            menu = menu.child(row(id, &m.label, false, false));
        }
        b.dropdown(menu, Place::BelowEnd, 4.0)
    }

    pub(super) fn outline(&self, node: &str, o: &Outline) -> El {
        let col = div().col().min_w0().min_h(0.0);
        if o.loading {
            return col
                .center()
                .child(crate::lucide("loader-circle", 20.0, 2.0, color::GRAY_500));
        }
        if let Some(e) = &o.empty {
            return col.center().child(
                text(e.as_str(), 14.0, color::GRAY_500)
                    .line_height(20.0)
                    .silent(),
            );
        }
        let mut col = col;
        if let Some(f) = &o.filter {
            col = col.child(self.filter(node, f));
        }
        let scroll = self.bind.on(Ev::Scroll {
            list: node.to_owned(),
        });
        let keys: Vec<Id> = o
            .rows
            .iter()
            .enumerate()
            .map(|(i, l)| match l.kind {
                LineKind::Note => scroll.child("note").child(i as u64),
                _ => self.send(node, EventKind::Select, &l.id),
            })
            .collect();
        let list = div().id(scroll).col().grow().min_h(0.0).py(2.0).scroll_y();
        let list = if self.ui.is_native() {
            list.children(o.rows.iter().map(|l| self.line(node, o, l)))
        } else {
            self.ui
                .windowed(scroll, &keys, ROW_H, &[])
                .fill(list, |i| self.line(node, o, &o.rows[i]))
        };
        let list = match &o.note {
            Some(n) => list.child(
                div().px(12.0).py(8.0).child(
                    text(n.as_str(), 11.0, color::GRAY_600)
                        .italic()
                        .line_height(16.0)
                        .silent(),
                ),
            ),
            None => list,
        };
        col.child(list)
    }

    /// The filter over an outline: its field, and ✕ while it holds something.
    fn filter(&self, node: &str, f: &vorn_tree::Filter) -> El {
        let fid = self.bind.on(Ev::Live {
            node: node.to_owned(),
            value: f.value.clone(),
        });
        let focus = self.ui.focused(fid);
        let name = f.placeholder.trim_end_matches('\u{2026}');
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
            .child(crate::lucide("search", 13.0, 2.0, color::GRAY_600))
            .child(
                div().grow().min_w0().child(
                    widgets::field(
                        fid,
                        name,
                        &f.placeholder,
                        TextStyle::ui(13.0),
                        color::GRAY_200,
                    )
                    .w_full(),
                ),
            );
        if !f.value.is_empty() {
            let id = self.send(node, EventKind::Change, "");
            search = search.child(
                div()
                    .id(id)
                    .on(Action::Click)
                    .p(2.0)
                    .role(Role::Button, "Clear filter")
                    .child(crate::lucide(
                        "x",
                        11.0,
                        2.0,
                        color::GRAY_600.mix(color::WHITE, self.ui.hover(id)),
                    )),
            );
        }
        div().row().px(6.0).py(6.0).shrink0().child(search)
    }

    fn line(&self, node: &str, o: &Outline, l: &Line) -> El {
        let depth = f32::from(l.depth);
        let guides = (0..l.depth).map(|i| {
            div()
                .absolute(BASE_LEFT + 7.0 + f32::from(i) * INDENT, 0.0)
                .w(1.0)
                .h(ROW_H)
                .bg(white(0.06))
        });
        match l.kind {
            LineKind::Note => div()
                .row()
                .items_center()
                .h(ROW_H)
                .pl(BASE_LEFT + depth * INDENT + 16.0)
                .child(
                    text(l.label.as_str(), 11.0, color::GRAY_600)
                        .italic()
                        .line_height(14.0)
                        .silent(),
                )
                .children(guides),
            LineKind::Folder { open, busy } => {
                let id = self.send(node, EventKind::Select, &l.id);
                let glyph = if busy {
                    crate::lucide("loader-circle", 14.0, 2.0, color::GRAY_600)
                } else {
                    crate::lucide(
                        if open {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        },
                        14.0,
                        2.0,
                        color::GRAY_500,
                    )
                };
                div()
                    .id(id)
                    .on(Action::Click)
                    .row()
                    .items_center()
                    .gap(5.0)
                    .h(ROW_H)
                    .pl(BASE_LEFT + depth * INDENT)
                    .pr(8.0)
                    .bg(white(0.05 * self.ui.hover(id)))
                    .role(Role::TreeItem, l.label.as_str())
                    .expanded(open)
                    .child(glyph)
                    .child(
                        text(l.label.as_str(), 13.5, color::GRAY_300)
                            .line_height(16.0)
                            .truncate()
                            .silent(),
                    )
                    .children(guides)
            }
            LineKind::Leaf => {
                let id = self.send(node, EventKind::Select, &l.id);
                self.bind.double(id, event(node, EventKind::Open, &l.id));
                let on = o.selected.as_deref() == Some(l.id.as_str());
                let pop = o
                    .action
                    .as_ref()
                    .map(|a| (a, self.send(&a.id, EventKind::Click, &l.id)));
                let hot = self.ui.hovered(id) || pop.is_some_and(|(_, p)| self.ui.hovered(p));
                let fg = if on {
                    color::GRAY_200
                } else if hot {
                    color::GRAY_300
                } else {
                    color::GRAY_400
                };
                let mut r = div()
                    .id(id)
                    .on(Action::Click)
                    .sense(Sense::DOUBLE)
                    .row()
                    .items_center()
                    .gap(5.0)
                    .h(ROW_H)
                    .pl(BASE_LEFT + depth * INDENT + 16.0)
                    .pr(4.0)
                    .bg(if on {
                        white(0.10)
                    } else {
                        white(0.05 * self.ui.hover(id))
                    })
                    .role(Role::TreeItem, l.label.as_str())
                    .selected(on);
                if o.files {
                    r = r.child(self.file_icon(&l.label, 16.0));
                }
                r = r.child(
                    div().grow().min_w0().child(
                        text(l.label.as_str(), 13.5, fg)
                            .line_height(16.0)
                            .truncate()
                            .silent(),
                    ),
                );
                if let Some((a, pid)) = pop {
                    r = r.child(
                        div()
                            .id(pid)
                            .on(Action::Click)
                            .p(2.0)
                            .rounded(4.0)
                            .opacity(if hot { 1.0 } else { 0.0 })
                            .bg(white(0.08 * self.ui.hover(pid)))
                            .role(Role::Button, a.label.replace("{}", &l.label))
                            .child(crate::lucide(
                                &a.icon,
                                11.0,
                                2.0,
                                color::INK.alpha(0.2).mix(color::WHITE, self.ui.hover(pid)),
                            )),
                    );
                }
                r.children(guides)
            }
        }
    }

    pub(super) fn split(
        &self,
        n: &vorn_tree::Node,
        ratio: f32,
        label: &str,
        narrow: Option<f32>,
    ) -> El {
        let node = super::key(n);
        let id = Id::new("tree-split").child(node);
        let mut body = div()
            .id(id)
            .sense(Sense::HOVER)
            .row()
            .grow()
            .min_h(0.0)
            .pt(2.0);
        let width = self.ui.rect_of(id).map(|r| r.w);
        let tight = matches!((width, narrow), (Some(w), Some(n)) if w > 0.0 && w < n);
        let kids: Vec<&vorn_tree::Node> =
            n.children.iter().take(if tight { 1 } else { 2 }).collect();
        if kids.len() < 2 {
            for k in kids {
                body = body.child(self.node(k).flex(1.0));
            }
            return body;
        }
        let r = self
            .bind
            .dragging(node)
            .unwrap_or(ratio)
            .clamp(vorn_tree::MIN_SPLIT, vorn_tree::MAX_SPLIT);
        let did = self.bind.on(Ev::Split {
            node: node.to_owned(),
        });
        let hot = if self.ui.pressed(did) {
            1.0
        } else {
            self.ui.hover(did)
        };
        let divider = div()
            .id(did)
            .shrink0()
            .sense(Sense::DRAG)
            .bg(white(0.06).mix(Rgba::hexa(0x2b7fff, 0.4), hot))
            .role(Role::Splitter, label)
            .vertical(true)
            .w(1.0)
            .h_full()
            .hit_slop(3.0, 0.0);
        body.child(self.node(kids[0]).flex(r))
            .child(divider)
            .child(self.node(kids[1]).flex(1.0 - r))
    }
}
