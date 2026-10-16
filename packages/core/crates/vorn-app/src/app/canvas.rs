//! What the workflow canvas does with the pointer and the keys: pan, zoom, fit
//! and tidy, drag cards, draw a connection or drop it on empty canvas, and undo
//! the definition.

use std::collections::HashMap;

use super::App;
use crate::canvas::{self, Card, Elements, Viewport};
use crate::look;
use crate::ui::{Event as UiEvent, Id, Ui};
use crate::workflows::{self as wf, Anchor, Gesture, Panel};

/// The canvas pane's id.
pub fn pane_id() -> Id {
    Id::new("wf-canvas")
}

/// A card's id, which it is dragged by.
pub fn card_id(node: &str) -> Id {
    App::control(
        "wf-card",
        &crate::act::Act::Workflow(wf::WfAct::SelectNode(node.into())),
    )
}

/// A step's output port, which a connection is drawn from.
pub fn port_id(node: &str, body: bool) -> Id {
    Id::new(if body { "wf-body-port" } else { "wf-port" }).child(node)
}

/// The minimap.
pub fn mini_id() -> Id {
    Id::new("wf-minimap")
}

/// The zoom step.
const ZOOM_STEP: f64 = 1.2;
/// How near a drag must end to a handle to connect, in screen pixels.
const CONNECT_RADIUS: f64 = 60.0;

/// The right panel's width (`w-[…]` of each panel).
pub fn panel_width(panel: &Panel) -> f32 {
    match panel {
        Panel::None => 0.0,
        Panel::Library(_) | Panel::StartFrom => 280.0,
        Panel::Node => 420.0,
        Panel::Properties | Panel::History => 340.0,
    }
}

impl App {
    /// The canvas pane's size now: the window less the sidebar and panel.
    pub(super) fn canvas_size(&self) -> (f64, f64) {
        let side = if self.st.sidebar {
            self.st.sidebar_width.unwrap_or(look::SIDEBAR_W)
        } else {
            0.0
        };
        let needs = if self.st.wf.still_needed().is_empty() {
            0.0
        } else {
            280.0
        };
        let w = self.size.0 - side - panel_width(&self.st.wf.panel) - needs;
        let h = self.size.1 - look::TOP_BAR_H;
        (f64::from(w.max(0.0)), f64::from(h.max(0.0)))
    }

    /// The open workflow's view, set the first time it opens here.
    pub(super) fn open_view(&mut self) {
        let Some(d) = &self.st.wf.editor else { return };
        if self.st.wf.canvas.views.contains_key(d.id()) {
            return;
        }
        let els = canvas::elements(d.nodes(), d.edges());
        let v = canvas::opening(&els, self.canvas_size().0);
        let id = d.id().to_owned();
        self.st.wf.canvas.views.insert(id, v);
    }

    fn view_mut(&mut self) -> Option<&mut Viewport> {
        let id = self.st.wf.editor.as_ref()?.id().to_owned();
        self.st.wf.canvas.views.get_mut(&id)
    }

    fn current_view(&self) -> Option<Viewport> {
        let id = self.st.wf.editor.as_ref()?.id();
        self.st.wf.canvas.views.get(id).copied()
    }

    fn elements(&self) -> Option<Elements> {
        let d = self.st.wf.editor.as_ref()?;
        Some(canvas::elements(d.nodes(), d.edges()))
    }

    /// An edit to the definition, recorded for undo when it changed the
    /// graph.
    pub(super) fn edit_graph(&mut self, f: impl FnOnce(&mut wf::Draft)) {
        let Some(d) = self.st.wf.editor.as_mut() else {
            return;
        };
        let before = (d.wf["nodes"].clone(), d.wf["edges"].clone());
        f(d);
        if before.0 != d.wf["nodes"] || before.1 != d.wf["edges"] {
            self.st.wf.undo.record(before, crate::state::now_ms());
        }
    }

    pub(super) fn undo(&mut self, redo: bool) {
        let Some(d) = self.st.wf.editor.as_mut() else {
            return;
        };
        let u = &mut self.st.wf.undo;
        let snap = if redo { u.future.pop() } else { u.past.pop() };
        let Some((nodes, edges)) = snap else { return };
        let now = (d.wf["nodes"].clone(), d.wf["edges"].clone());
        if redo {
            u.past.push(now);
        } else {
            u.future.push(now);
        }
        u.last = 0.0;
        d.wf["nodes"] = nodes;
        d.wf["edges"] = edges;
        let gone = self
            .st
            .wf
            .node
            .as_deref()
            .is_some_and(|n| d.node(n).is_none());
        if gone {
            self.st.wf.node = None;
            self.st.wf.panel = Panel::Properties;
        }
    }

    pub(super) fn zoom(&mut self, factor: f64) {
        let (w, h) = self.canvas_size();
        if let Some(v) = self.view_mut() {
            *v = canvas::zoom_by(*v, factor, (w / 2.0, h / 2.0));
        }
    }

    pub(super) fn zoom_to(&mut self, zoom: f64) {
        let Some(v) = self.current_view() else { return };
        self.zoom(zoom / v.zoom);
    }

    /// Fits the workflow to the canvas, top-aligned.
    pub(super) fn fit(&mut self) {
        let (w, h) = self.canvas_size();
        let Some(next) = self.elements().and_then(|els| canvas::fit(&els, w, h)) else {
            return;
        };
        if let Some(v) = self.view_mut() {
            *v = next;
        }
    }

    /// Where a window point falls on the canvas, in flow coordinates.
    fn to_flow(&self, ui: &Ui, x: f32, y: f32) -> Option<(f64, f64)> {
        let r = ui.rect_of(pane_id())?;
        let v = self.current_view()?;
        Some((
            (f64::from(x - r.x) - v.x) / v.zoom,
            (f64::from(y - r.y) - v.y) / v.zoom,
        ))
    }

    /// After `after`, into its loop when it is in one.
    pub(super) fn library_after(
        &mut self,
        ui: &mut Ui,
        after: &str,
        at: Option<(f64, f64)>,
        body: bool,
    ) {
        let Some(d) = &self.st.wf.editor else { return };
        let members = crate::graph::loop_body_owners(d.nodes());
        let anchor = if body || members.contains_key(after) {
            Anchor {
                after: Some(after.to_owned()),
                before: Some(crate::graph::LOOP_BODY.to_owned()),
                body_only: true,
                ..Anchor::default()
            }
        } else {
            let els = canvas::elements(d.nodes(), d.edges());
            Anchor {
                after: Some(after.to_owned()),
                inside_branch: els.branch_members.contains(after),
                position: at.map(|(x, y)| (x.round() as i64, y.round() as i64)),
                ..Anchor::default()
            }
        };
        self.run_wf(ui, wf::WfAct::Library(anchor), super::Src::default());
    }

    /// The canvas's pointer events; true when one was the canvas's.
    pub(super) fn canvas_event(&mut self, ui: &mut Ui, event: &UiEvent) -> bool {
        if self.st.wf.editor.is_none() {
            return false;
        }
        match *event {
            UiEvent::Wheel { id, dy, .. } if id == pane_id() => {
                if let Some(v) = self.view_mut() {
                    v.y -= f64::from(dy);
                }
                true
            }
            UiEvent::Move { id, dx, dy, x, y } => self.canvas_move(ui, id, (dx, dy), (x, y)),
            UiEvent::Drop { id, x, y, .. } => self.canvas_drop(ui, id, (x, y)),
            _ => false,
        }
    }

    fn canvas_move(
        &mut self,
        ui: &mut Ui,
        id: Id,
        (dx, dy): (f32, f32),
        (x, y): (f32, f32),
    ) -> bool {
        let Some(v) = self.current_view() else {
            return false;
        };
        let (dx, dy) = (f64::from(dx), f64::from(dy));
        if self.st.wf.canvas.gesture.is_none() {
            let Some(d) = &self.st.wf.editor else {
                return false;
            };
            let gesture = if id == pane_id() {
                Gesture::Pan(v)
            } else if id == mini_id() {
                Gesture::Mini(v)
            } else if let Some(n) = d.nodes().iter().find_map(|n| {
                let nid = crate::runs::s(n, "id")?;
                (card_id(nid) == id).then(|| nid.to_owned())
            }) {
                let els = canvas::elements(d.nodes(), d.edges());
                if !els.get(&n).is_some_and(canvas::Element::draggable) {
                    return true;
                }
                let starts = els
                    .nodes
                    .iter()
                    .filter(|e| e.draggable())
                    .map(|e| (e.id.clone(), (e.x, e.y)))
                    .collect();
                Gesture::Node { id: n, starts }
            } else if let Some((n, body)) = d.nodes().iter().find_map(|n| {
                let nid = crate::runs::s(n, "id")?;
                if port_id(nid, false) == id {
                    Some((nid.to_owned(), false))
                } else if port_id(nid, true) == id {
                    Some((nid.to_owned(), true))
                } else {
                    None
                }
            }) {
                Gesture::Connect {
                    source: n,
                    body_port: body,
                    to: (0.0, 0.0),
                }
            } else {
                return false;
            };
            self.st.wf.canvas.gesture = Some(gesture);
        }
        let at = self.to_flow(ui, x, y);
        let Some(g) = self.st.wf.canvas.gesture.clone() else {
            return false;
        };
        match g {
            Gesture::Pan(from) => {
                if let Some(cur) = self.view_mut() {
                    cur.x = from.x + dx;
                    cur.y = from.y + dy;
                }
            }
            Gesture::Mini(from) => {
                let scale = self.mini_scale();
                let k = scale * from.zoom.max(1.0);
                if let Some(cur) = self.view_mut() {
                    cur.x = from.x - dx * k;
                    cur.y = from.y - dy * k;
                }
            }
            Gesture::Node { id: moving, starts } => {
                let start = starts.iter().find(|(i, _)| *i == moving).map(|(_, p)| *p);
                if let Some((sx, sy)) = start {
                    let p = (
                        crate::graph::snap(sx + dx / v.zoom),
                        crate::graph::snap(sy + dy / v.zoom),
                    );
                    self.st.wf.canvas.dragged = starts.iter().cloned().collect::<HashMap<_, _>>();
                    self.st.wf.canvas.dragged.insert(moving, p);
                }
            }
            Gesture::Connect {
                source, body_port, ..
            } => {
                if let Some(to) = at {
                    self.st.wf.canvas.gesture = Some(Gesture::Connect {
                        source,
                        body_port,
                        to,
                    });
                }
            }
        }
        true
    }

    fn canvas_drop(&mut self, ui: &mut Ui, id: Id, (x, y): (f32, f32)) -> bool {
        let Some(g) = self.st.wf.canvas.gesture.take() else {
            return id == pane_id() || id == mini_id();
        };
        match g {
            Gesture::Pan(_) | Gesture::Mini(_) => {}
            Gesture::Node { .. } => {
                let positions = std::mem::take(&mut self.st.wf.canvas.dragged);
                if !positions.is_empty() {
                    self.edit_graph(|d| wf::commit_positions(d, &positions));
                }
            }
            Gesture::Connect {
                source, body_port, ..
            } => {
                let (Some(at), Some(v)) = (self.to_flow(ui, x, y), self.current_view()) else {
                    return true;
                };
                let Some(d) = &self.st.wf.editor else {
                    return true;
                };
                let els = canvas::elements(d.nodes(), d.edges());
                let radius = CONNECT_RADIUS / v.zoom;
                let target = els
                    .nodes
                    .iter()
                    .filter(|e| matches!(e.card, Card::Step | Card::Loop) && e.id != source)
                    .map(|e| {
                        let (hx, hy) = (e.x + e.w / 2.0, e.y);
                        let inside =
                            at.0 >= e.x && at.0 <= e.x + e.w && at.1 >= e.y && at.1 <= e.y + e.h;
                        let dist = if inside {
                            0.0
                        } else {
                            (hx - at.0).hypot(hy - at.1)
                        };
                        (e.id.clone(), dist)
                    })
                    .filter(|(_, dist)| *dist <= radius)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(i, _)| i);
                match target {
                    Some(t) => {
                        if canvas::can_connect(d.nodes(), d.edges(), &source, &t, body_port) {
                            self.edit_graph(|d| wf::connect(d, &source, &t));
                        }
                    }
                    None => self.library_after(ui, &source, Some(at), body_port),
                }
            }
        }
        true
    }

    /// Flow units per minimap pixel, as the minimap draws them.
    pub(super) fn mini_scale(&self) -> f64 {
        let (w, h) = self.canvas_size();
        match (self.elements(), self.current_view()) {
            (Some(els), Some(v)) => crate::canvas::minimap(&els, v, (w, h)).scale,
            _ => 1.0,
        }
    }

    /// The canvas's keys: Tab adds after the leaf, + - 0
    /// and 1 zoom, Delete removes the selected step; and undo.
    pub(super) fn canvas_key(
        &mut self,
        ui: &mut Ui,
        code: &str,
        typed: Option<&str>,
        held: u16,
    ) -> bool {
        use crate::ui::mods;
        let primary = held & (mods::CTRL | mods::SUPER) != 0;
        if primary && code == "KeyZ" {
            self.undo(held & mods::SHIFT != 0);
            return true;
        }
        if held & !mods::SHIFT != 0 {
            return false;
        }
        match (code, typed) {
            ("Tab", _) => {
                let Some(d) = &self.st.wf.editor else {
                    return false;
                };
                let outs: std::collections::HashSet<&str> = d
                    .edges()
                    .iter()
                    .filter_map(|e| crate::runs::s(e, "source"))
                    .collect();
                let body = crate::graph::loop_body_owners(d.nodes());
                let leaf = d
                    .nodes()
                    .iter()
                    .rev()
                    .filter_map(|n| crate::runs::s(n, "id"))
                    .find(|i| !outs.contains(i) && !body.contains_key(*i))
                    .map(str::to_owned);
                match leaf {
                    Some(l) => {
                        self.library_after(ui, &l, None, false);
                        true
                    }
                    None => false,
                }
            }
            (_, Some("+" | "=")) => {
                self.zoom(ZOOM_STEP);
                true
            }
            (_, Some("-")) => {
                self.zoom(1.0 / ZOOM_STEP);
                true
            }
            (_, Some("0")) => {
                self.zoom_to(1.0);
                true
            }
            (_, Some("1")) => {
                self.fit();
                true
            }
            ("Delete" | "Backspace", _) => {
                let Some(n) = self
                    .st
                    .wf
                    .node
                    .clone()
                    .filter(|_| self.st.wf.panel == Panel::Node)
                else {
                    return false;
                };
                self.run_wf(ui, wf::WfAct::DeleteNode(n), super::Src::default());
                true
            }
            _ => false,
        }
    }

    /// Whether the node exists and is not a loop's body step: a run target.
    pub fn run_target(&self, node: &str) -> bool {
        self.st.wf.editor.as_ref().is_some_and(|d| {
            d.node(node)
                .is_some_and(|n| crate::runs::s(n, "type") != Some("trigger"))
                && !crate::graph::loop_body_owners(d.nodes()).contains_key(node)
        })
    }
}
