//! Where extensions' trees draw: each place asks for its points and lays
//! them out with the kit's one tree renderer.

use vorn_components::tree::{self, Ev, TreeBind};

use super::Screen;
use crate::act::{Act, Acts};
use crate::points::{kind, Point, PointAct, Points};
use crate::ui::theme::color as k;
use crate::ui::{div, text, Action, El, Id, Rgba, Role, Ui};

/// `connector:<id>`: a connector's mark, a plug for one without.
fn connector_icon(name: &str, size: f32) -> Option<El> {
    let id = name.strip_prefix("connector:")?;
    let name = match id {
        "http" => "globe",
        "mcp" | "sdk" => "mcp",
        other if crate::look::icon(other, 2.0).is_some() => other,
        _ => "plug",
    };
    Some(super::ico(name, size, 2.0, k::GRAY_400))
}

/// Binds a tree page's controls to the app's acts.
pub(crate) struct TreeBinder<'a> {
    pub(crate) acts: &'a Acts,
    pub(crate) points: &'a Points,
    pub(crate) key: &'a str,
}

impl TreeBinder<'_> {
    fn base(&self) -> Id {
        Id::new(self.key).child("tree")
    }

    fn act(&self, a: PointAct) -> Act {
        Act::Point(a)
    }
}

impl TreeBind for TreeBinder<'_> {
    fn on(&self, ev: Ev) -> Id {
        let key = self.key.to_owned();
        match ev {
            Ev::Send { event, ask } => {
                let id = self.base().child(&event.id).child(&event.value);
                let act = self.act(PointAct::Tree { key, event });
                if !ask {
                    self.acts.bind(id, act);
                } else if self.points.asking == Some(id.0) {
                    self.acts.bind(id, Act::Point(PointAct::Ask(None)));
                    self.acts
                        .bind(id.child("no"), Act::Point(PointAct::Ask(None)));
                    self.acts.bind(id.child("ok"), act);
                } else {
                    self.acts.bind(id, Act::Point(PointAct::Ask(Some(id.0))));
                }
                id
            }
            Ev::Field { node, value } => {
                let id = self.base().child("field").child(&node);
                self.acts
                    .text(id, self.act(PointAct::TreeField { key, node }), value);
                id
            }
            Ev::Check {
                list,
                rows,
                on,
                all,
            } => {
                let id = self.base().child("check").child(&list);
                let id = match rows.first() {
                    Some(row) if !all => id.child(row),
                    _ => id.child("all"),
                };
                let act = self.act(PointAct::TreeCheck {
                    key,
                    list,
                    rows,
                    on,
                });
                self.acts.bind(id, act);
                id
            }
            Ev::Menu { node } => {
                let id = self.base().child("more").child(&node);
                let open = self.points.more == Some(id.0);
                self.acts
                    .bind(id, Act::Point(PointAct::More((!open).then_some(id.0))));
                id
            }
            Ev::Hover { node } => self.base().child("entry").child(&node),
            Ev::Scroll { list } => self.base().child("scroll").child(&list),
        }
    }

    fn confirming(&self, id: Id) -> Option<(Id, Id)> {
        (self.points.asking == Some(id.0)).then(|| (id.child("ok"), id.child("no")))
    }

    fn menu_open(&self, id: Id) -> bool {
        self.points.more == Some(id.0)
    }

    fn checked(&self, list: &str, row: &str) -> bool {
        self.points.is_checked(self.key, list, row)
    }

    fn icon(&self, name: &str, size: f32) -> Option<El> {
        connector_icon(name, size)
    }
}

/// Tree point `p`'s tree whole, or its error box; nothing until it renders,
/// when it is an empty box, or for a point that draws no tree.
pub fn draw_tree(ui: &Ui, acts: &Acts, points: &Points, p: &Point) -> Option<El> {
    if !p.tree {
        return None;
    }
    let key = p.key();
    match points.trees.get(&key)? {
        Ok(t) if vorn_tree::is_blank(t) => None,
        Ok(t) => {
            let bind = TreeBinder {
                acts,
                points,
                key: &key,
            };
            Some(tree::view(ui, t, &bind))
        }
        Err(errors) => Some(tree::invalid(&p.place(), errors)),
    }
}

/// What per-session tree point `p` draws for `session`: its child for that session.
fn session_tree(ui: &Ui, acts: &Acts, points: &Points, p: &Point, session: &str) -> Option<El> {
    let key = p.key();
    let part = points.session_tree(&key, session)?;
    if vorn_tree::is_blank(part) {
        return None;
    }
    let bind = TreeBinder {
        acts,
        points,
        key: &key,
    };
    Some(tree::view(ui, part, &bind))
}

/// Point `p`'s tree as a page: its toolbar apart, for the page's header, and its body.
fn draw_page(s: &Screen<'_>, p: &Point) -> (Option<El>, Option<El>) {
    if !p.tree {
        return (None, None);
    }
    let key = p.key();
    match s.st.points.trees.get(&key) {
        Some(Ok(t)) => {
            let bind = TreeBinder {
                acts: s.acts,
                points: &s.st.points,
                key: &key,
            };
            tree::page(s.ui, t, &bind)
        }
        Some(Err(errors)) => (None, Some(tree::invalid(&p.place(), errors))),
        None => (None, None),
    }
}

fn drawn(s: &Screen<'_>, p: &Point) -> Option<El> {
    draw_tree(s.ui, s.acts, &s.st.points, p)
}

/// Every point at a whole-screen place, each drawn.
fn every(s: &Screen<'_>, at: &str) -> Vec<El> {
    s.st.points.at(at).filter_map(|p| drawn(s, p)).collect()
}

/// The points at a per-session place, each its tree's child for that session;
/// a tree that cannot be drawn shows its box only once, on the selected card.
fn for_session(s: &Screen<'_>, at: &str, session: &str) -> Vec<El> {
    s.st.points
        .at(at)
        .filter(|p| p.tree)
        .filter_map(|p| match s.st.points.trees.get(&p.key())? {
            Ok(_) => session_tree(s.ui, s.acts, &s.st.points, p, session),
            Err(e) if s.st.selected.as_deref() == Some(session) => {
                Some(tree::invalid(&p.place(), e))
            }
            Err(_) => None,
        })
        .collect()
}

/// The sidebar's extension sections, under the projects.
pub(super) fn sidebar(s: &Screen<'_>) -> Option<El> {
    let mut col = div().col().gap(8.0).pt(8.0);
    let mut any = false;
    for p in s.st.points.at(kind::SIDEBAR) {
        let Some(body) = drawn(s, p) else {
            continue;
        };
        any = true;
        col = col.child(
            div()
                .col()
                .gap(4.0)
                .role(Role::Group, p.title.as_str())
                .child(heading(&p.title))
                .child(body),
        );
    }
    any.then_some(col)
}

fn heading(title: &str) -> El {
    text(title, 10.0, k::GRAY_500)
        .weight(500)
        .tracking(0.05)
        .line_height(15.0)
        .truncate()
        .role(Role::Heading, title)
}

/// A session row's decorations, in a line under it.
pub(super) fn decorations(s: &Screen<'_>, session: &str) -> Option<El> {
    row_of(for_session(s, kind::DECORATIONS, session))
}

/// A card's badges, beside its title.
pub(super) fn badges(s: &Screen<'_>, session: &str) -> Option<El> {
    row_of(for_session(s, kind::BADGES, session))
}

/// A card's footers, under its terminal.
pub(super) fn footers(s: &Screen<'_>, session: &str) -> Option<El> {
    let all = for_session(s, kind::CARD_FOOTERS, session);
    (!all.is_empty()).then(|| {
        div()
            .col()
            .gap(4.0)
            .px(8.0)
            .py(4.0)
            .border_top(1.0, Rgba::white(0.06))
            .children(all)
    })
}

fn row_of(all: Vec<El>) -> Option<El> {
    (!all.is_empty()).then(|| div().row().items_center().gap(4.0).min_w0().children(all))
}

/// The composer's extension strip, over its prompt.
pub(super) fn composer(s: &Screen<'_>) -> Option<El> {
    let all = every(s, kind::COMPOSER);
    (!all.is_empty()).then(|| div().col().gap(8.0).children(all))
}

/// The items extensions add to menu `menu`, as quick actions: act, icon, label, stroke.
pub(super) fn menu_items<'a>(
    s: &'a Screen<'_>,
    menu: &str,
    args: &str,
) -> Vec<(Act, &'a str, &'a str, f32)> {
    s.st.points
        .at(kind::MENUS)
        .filter(|p| p.menu.as_deref() == Some(menu) && p.rows.is_none())
        .map(|p| {
            let act = Act::Point(PointAct::Choose {
                key: p.key(),
                args: args.to_owned(),
            });
            (
                act,
                p.icon.as_deref().unwrap_or("puzzle"),
                p.title.as_str(),
                2.0,
            )
        })
        .collect()
}

/// The new-session launcher's extension entries, under its prompt.
pub(super) fn launchers(s: &Screen<'_>) -> Option<El> {
    let project = s.st.composer.project.clone().unwrap_or_default();
    let args = serde_json::json!({ "project": project }).to_string();
    let all: Vec<El> =
        s.st.points
            .at(kind::LAUNCHERS)
            .map(|p| {
                let id = s.acts.on(
                    "ext-launcher",
                    Act::Point(PointAct::Choose {
                        key: p.key(),
                        args: args.clone(),
                    }),
                );
                let h = s.ui.hover(id);
                let ink = k::GRAY_400.mix(k::WHITE, h);
                div()
                    .id(id)
                    .on(Action::Click)
                    .row()
                    .items_center()
                    .gap(8.0)
                    .px(8.0)
                    .py(4.0)
                    .rounded(4.0)
                    .bg(Rgba::TRANSPARENT.mix(Rgba::white(0.06), h))
                    .ring(4.0)
                    .role(Role::Button, p.title.as_str())
                    .child(vorn_components::lucide(
                        p.icon.as_deref().unwrap_or("puzzle"),
                        14.0,
                        1.5,
                        ink,
                    ))
                    .child(text(&p.title, 12.0, ink).line_height(16.0).silent())
            })
            .collect();
    (!all.is_empty()).then(|| div().row().wrap_children().gap(8.0).pt(16.0).children(all))
}

/// The settings page of point `key`, under its title.
pub(super) fn settings_page(s: &Screen<'_>, key: &str) -> El {
    let Some(p) = s.st.points.get(key) else {
        return text("This page is gone", 13.0, k::GRAY_500).line_height(20.0);
    };
    let desc = p.description.clone().unwrap_or_default();
    let (bar, body) = draw_page(s, p);
    div()
        .col()
        .child(super::settings::header(&p.title, &desc, bar))
        .child_if(body)
}

/// An extension pane's body.
pub(super) fn pane(s: &Screen<'_>, key: &str) -> El {
    match s.st.points.get(key) {
        Some(p) => div().col().p(12.0).gap(8.0).child_if(drawn(s, p)),
        None => text("This pane's extension is off", 12.0, k::GRAY_500).line_height(16.0),
    }
}

/// The dialog an extension opened, centred over a scrim.
pub(super) fn dialog(s: &Screen<'_>) -> Option<El> {
    let key = s.st.points.dialog.as_deref()?;
    let p = s.st.points.get(key)?;
    let body = drawn(s, p)?;
    let panel = div()
        .col()
        .w(400.0_f32.min(s.size.0 - 32.0))
        .p(16.0)
        .gap(12.0)
        .rounded(4.0)
        .border(1.0, Rgba::white(0.1))
        .bg(k::SURFACE_OVERLAY)
        .role(Role::Dialog, p.title.as_str())
        .child(body);
    let panel = s.acts.click(panel, "ext-dialog-panel", Act::Nothing);
    let bg = s.acts.on("ext-dialog", Act::Point(PointAct::CloseDialog));
    Some(
        div()
            .id(bg)
            .on(Action::Click)
            .absolute(0.0, 0.0)
            .size(s.size.0, s.size.1)
            .bg(Rgba::hexa(0, 0.3))
            .layered()
            .center()
            .child(panel),
    )
}

/// A small close button in a window point's title bar.
pub fn close_button(ui: &Ui, id: Id, label: &str) -> El {
    let ink = k::INK_FAINT.mix(k::INK, ui.hover(id));
    div()
        .id(id)
        .on(Action::Click)
        .p(4.0)
        .rounded(4.0)
        .bg(Rgba::TRANSPARENT.mix(Rgba::white(0.06), ui.hover(id)))
        .ring(4.0)
        .role(Role::Button, label)
        .child(vorn_components::lucide("x", 12.0, 2.0, ink))
}

/// Window points drawn in the app, where they cannot have windows of their own.
pub(super) fn floating(s: &Screen<'_>) -> Option<El> {
    if !s.st.points.floating {
        return None;
    }
    let mut col = div().col().gap(8.0);
    let mut any = false;
    for key in &s.st.points.windows {
        let Some(p) = s.st.points.get(key) else {
            continue;
        };
        let Some(body) = drawn(s, p) else {
            continue;
        };
        any = true;
        let w = p.size.map_or(280.0, |(w, _)| w);
        let mut h = p.size.map_or(400.0, |(_, h)| h);
        if s.ui.is_cells() {
            // A frame, a title and a rule take four rows; the body keeps at least one.
            h = h.max(5.0 * crate::ui::cells::units::ROW);
        }
        let close = s.acts.on(
            "ext-window",
            Act::Point(PointAct::Window {
                key: key.clone(),
                open: false,
            }),
        );
        col = col.child(
            div()
                .col()
                .w(w)
                .max_h(h)
                .rounded(4.0)
                .bg(k::SURFACE_OVERLAY)
                .border(1.0, Rgba::white(0.08))
                .role(Role::Dialog, p.title.as_str())
                .child(
                    div()
                        .row()
                        .items_center()
                        .px(12.0)
                        .py(8.0)
                        .border_bottom(1.0, Rgba::white(0.06))
                        .child(div().grow().min_w0().child(heading(&p.title)))
                        .child(close_button(s.ui, close, "Close")),
                )
                .child(
                    div()
                        .id(Id::new("ext-window-body").child(key.as_str()))
                        .col()
                        .px(12.0)
                        .py(8.0)
                        .scroll_y()
                        .child(body),
                ),
        );
    }
    any.then(|| div().absolute_right(16.0, 16.0).layered().child(col))
}
