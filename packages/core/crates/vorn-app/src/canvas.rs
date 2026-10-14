//! The workflow canvas's geometry (`workflow-canvas-layout.ts`): where each
//! card, loop frame and add button sits, the edges between their ports,
//! which hand-drawn connections are allowed, and the viewport it opens on.
//! The definition stays the source of truth; this only projects it.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::graph::{
    self, collapse_loop_bodies, compute_flow_layout, loop_body_graph, loop_body_owners, snap,
    FlowRow, CARD_WIDTH, LOOP_WIDTH,
};
use crate::runs::s;

/// Horizontal gap between fork branches.
const BRANCH_GAP: f64 = 56.0;
/// Vertical gap between steps.
const ROW_GAP: f64 = 56.0;
/// A loop frame: header, padding above the body, and line, + and footer.
pub const LOOP_HEADER: f64 = 41.0;
const LOOP_BODY_PAD: f64 = 16.0;
const LOOP_FOOTER: f64 = 18.0 + 22.0 + 40.0;
/// The dashed box an empty body shows.
const EMPTY_BODY: f64 = 52.0;
/// `STRAIGHT_EDGE_TOLERANCE`.
const STRAIGHT: f64 = 16.0;
/// `OPENING_TOP`.
const OPENING_TOP: f64 = 48.0;
/// `CANVAS_MIN_ZOOM`.
pub const MIN_ZOOM: f64 = 0.2;
/// The canvas's `maxZoom`.
pub const MAX_ZOOM: f64 = 1.75;
/// The add-trigger spot's size.
pub const TRIGGER_SPOT: (f64, f64) = (CARD_WIDTH, 58.0);
/// The + button.
pub const PLUS: f64 = 22.0;

fn id(v: &Value) -> &str {
    s(v, "id").unwrap_or("")
}

fn kind(v: &Value) -> &str {
    s(v, "type").unwrap_or("")
}

fn config_str<'a>(n: &'a Value, key: &str) -> Option<&'a str> {
    n.pointer(&format!("/config/{key}"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

/// `connectorArgsPreview`.
pub fn args_preview(args: Option<&Value>) -> Option<String> {
    let map = args?.as_object()?;
    (!map.is_empty()).then(|| {
        map.iter()
            .map(|(k, v)| {
                let v = match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                format!("{k}: {v}")
            })
            .collect::<Vec<_>>()
            .join("  ")
    })
}

/// `stepPreview`: the configured body that sizes a card.
pub fn step_preview(n: &Value) -> Option<String> {
    match kind(n) {
        "script" => config_str(n, "scriptContent").map(str::to_owned),
        "launchAgent" => config_str(n, "prompt").map(str::to_owned),
        "approval" => config_str(n, "message").map(str::to_owned),
        "httpRequest" => config_str(n, "body").map(str::to_owned),
        "callConnectorAction" => args_preview(n.pointer("/config/args")),
        _ => None,
    }
}

/// A loop's body laid out inside its frame.
#[derive(Debug, Clone, Default)]
pub struct LoopBody {
    /// Each body step, relative to the frame.
    pub positions: Vec<(String, (f64, f64))>,
    pub branch_members: HashSet<String>,
    pub width: f64,
    pub height: f64,
}

/// `layoutLoopBody`.
pub fn layout_loop_body(lp: &Value, nodes: &[Value], edges: &[Value]) -> LoopBody {
    let (members, inner) = loop_body_graph(nodes, edges, lp);
    if members.is_empty() {
        return LoopBody {
            width: LOOP_WIDTH,
            height: EMPTY_BODY,
            ..LoopBody::default()
        };
    }
    let owned: Vec<Value> = members.iter().map(|m| (*m).clone()).collect();
    let (positions, branch_members) = layout_positions(&owned, &inner);
    let (mut min_x, mut max_x, mut bottom) = (f64::INFINITY, f64::NEG_INFINITY, 0.0_f64);
    for m in &owned {
        let Some(at) = positions.get(id(m)) else {
            continue;
        };
        min_x = min_x.min(at.0);
        max_x = max_x.max(at.0 + CARD_WIDTH);
        bottom = bottom.max(at.1 + estimate_height(m, &owned, &[]));
    }
    let span = max_x - min_x;
    let width = LOOP_WIDTH.max(span + 2.0 * LOOP_BODY_PAD);
    let shift = (width - span) / 2.0 - min_x;
    let mut rel: Vec<(String, (f64, f64))> = owned
        .iter()
        .filter_map(|m| {
            let at = positions.get(id(m))?;
            Some((
                id(m).to_owned(),
                (snap(at.0 + shift), snap(at.1 + LOOP_HEADER + LOOP_BODY_PAD)),
            ))
        })
        .collect();
    rel.sort_by(|a, b| a.0.cmp(&b.0));
    LoopBody {
        positions: rel,
        branch_members,
        width,
        height: bottom,
    }
}

/// `estimateNodeHeight`: only feeds the layout, so drift just widens a gap.
pub fn estimate_height(n: &Value, nodes: &[Value], edges: &[Value]) -> f64 {
    match kind(n) {
        "loop" => {
            LOOP_HEADER + LOOP_BODY_PAD + layout_loop_body(n, nodes, edges).height + LOOP_FOOTER
        }
        "condition" => {
            if config_str(n, "variable").is_some() {
                90.0
            } else {
                58.0
            }
        }
        _ => {
            if step_preview(n).is_some() {
                90.0
            } else {
                58.0
            }
        }
    }
}

/// `layoutPositions`: a vertical trunk, branches side by side, loops as one
/// block; and which steps sit inside a fork's branch.
pub fn layout_positions(
    nodes: &[Value],
    edges: &[Value],
) -> (HashMap<String, (f64, f64)>, HashSet<String>) {
    let rows = compute_flow_layout(nodes, &collapse_loop_bodies(nodes, edges));
    let body: HashSet<String> = loop_body_owners(nodes).into_keys().collect();
    let by_id: HashMap<&str, &Value> = nodes.iter().map(|n| (id(n), n)).collect();
    let mut p = Placer {
        nodes,
        edges,
        by_id,
        body,
        positions: HashMap::new(),
        branch_members: HashSet::new(),
    };
    p.place(&rows, 0.0, 0.0, false);
    (p.positions, p.branch_members)
}

struct Placer<'a> {
    nodes: &'a [Value],
    edges: &'a [Value],
    by_id: HashMap<&'a str, &'a Value>,
    body: HashSet<String>,
    positions: HashMap<String, (f64, f64)>,
    branch_members: HashSet<String>,
}

impl Placer<'_> {
    fn row_width(&self, row: &FlowRow) -> f64 {
        match row {
            FlowRow::Loop { id, .. } => self.by_id.get(id.as_str()).map_or(LOOP_WIDTH, |l| {
                layout_loop_body(l, self.nodes, self.edges).width
            }),
            FlowRow::Node(_) => CARD_WIDTH,
            FlowRow::Fork { branches, .. } => {
                let w: Vec<f64> = branches.iter().map(|b| self.branch_width(b)).collect();
                w.iter().sum::<f64>() + (w.len().saturating_sub(1)) as f64 * BRANCH_GAP
            }
        }
    }

    fn branch_width(&self, b: &[FlowRow]) -> f64 {
        if b.is_empty() {
            CARD_WIDTH
        } else {
            b.iter()
                .map(|r| self.row_width(r))
                .fold(f64::NEG_INFINITY, f64::max)
        }
    }

    fn place(&mut self, rows: &[FlowRow], center: f64, y: f64, inside: bool) -> f64 {
        let mut cursor = y;
        for row in rows {
            match row {
                FlowRow::Node(nid) => {
                    if self.body.contains(nid) {
                        continue;
                    }
                    self.positions
                        .insert(nid.clone(), (snap(center - CARD_WIDTH / 2.0), snap(cursor)));
                    if inside {
                        self.branch_members.insert(nid.clone());
                    }
                    let h = self
                        .by_id
                        .get(nid.as_str())
                        .map_or(58.0, |n| estimate_height(n, self.nodes, &[]));
                    cursor += h + ROW_GAP;
                }
                FlowRow::Loop { id: lid, .. } => {
                    let Some(lp) = self.by_id.get(lid.as_str()).copied() else {
                        continue;
                    };
                    let w = layout_loop_body(lp, self.nodes, self.edges).width;
                    self.positions
                        .insert(lid.clone(), (snap(center - w / 2.0), snap(cursor)));
                    if inside {
                        self.branch_members.insert(lid.clone());
                    }
                    cursor += estimate_height(lp, self.nodes, self.edges) + ROW_GAP;
                }
                FlowRow::Fork { branches, .. } => {
                    let widths: Vec<f64> = branches.iter().map(|b| self.branch_width(b)).collect();
                    let total = widths.iter().sum::<f64>()
                        + (widths.len().saturating_sub(1)) as f64 * BRANCH_GAP;
                    let mut left = center - total / 2.0;
                    let mut deepest = cursor;
                    for (i, b) in branches.iter().enumerate() {
                        let c = left + widths[i] / 2.0;
                        let bottom = self.place(b, c, cursor, true);
                        deepest = deepest.max(bottom);
                        left += widths[i] + BRANCH_GAP;
                    }
                    cursor = deepest;
                }
            }
        }
        cursor
    }
}

/// `latticePosition`.
pub fn lattice(n: &Value) -> (f64, f64) {
    let (x, y) = graph::position(n);
    (snap(x), snap(y))
}

/// `alignedNodes`: positions healed onto the lattice for the next save.
pub fn aligned(nodes: &[Value]) -> Vec<Value> {
    let aligned = nodes.iter().all(|n| lattice(n) == graph::position(n));
    if graph::positions_are_seed(nodes) || aligned {
        return nodes.to_vec();
    }
    nodes
        .iter()
        .map(|n| {
            let mut n = n.clone();
            let (x, y) = lattice(&n);
            n["position"] = serde_json::json!({ "x": graph::num(x), "y": graph::num(y) });
            n
        })
        .collect()
}

/// A step card's height as drawn (`NodeShell`): the header, and the
/// footer line when it has one.
pub fn drawn_height(n: &Value) -> f64 {
    if crate::workflows::footer(n).is_some() {
        58.0 + 33.5
    } else {
        58.0
    }
}

/// What a canvas element is.
#[derive(Debug, Clone, PartialEq)]
pub enum Card {
    Step,
    Loop,
    /// The + under a leaf.
    AddStep {
        after: String,
        inside_branch: bool,
    },
    /// Where a trigger goes when there is none.
    AddTrigger,
}

/// One element on the canvas, in flow coordinates (absolute).
#[derive(Debug, Clone)]
pub struct Element {
    pub id: String,
    pub card: Card,
    /// The loop a body step is drawn inside; such a step is never dragged.
    pub parent: Option<String>,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Element {
    pub fn draggable(&self) -> bool {
        matches!(self.card, Card::Step | Card::Loop) && self.parent.is_none()
    }
}

/// `CanvasEdgeData`: where a step dropped on this edge goes.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeData {
    pub after: String,
    pub before: String,
    pub branch: Option<String>,
    pub inside_branch: bool,
    pub body_of: Option<String>,
}

/// One drawn edge.
#[derive(Debug, Clone)]
pub struct Edge {
    pub id: String,
    pub source: String,
    pub target: String,
    /// From a loop's body port, inside its frame.
    pub body_port: bool,
    pub label: Option<&'static str>,
    pub data: Option<EdgeData>,
}

/// What the canvas draws.
#[derive(Debug, Clone, Default)]
pub struct Elements {
    pub nodes: Vec<Element>,
    pub edges: Vec<Edge>,
    pub branch_members: HashSet<String>,
}

impl Elements {
    pub fn get(&self, id: &str) -> Option<&Element> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Where an edge leaves `source` and enters `target`.
    pub fn ends(&self, e: &Edge) -> Option<((f64, f64), (f64, f64))> {
        let a = self.get(&e.source)?;
        let b = self.get(&e.target)?;
        let from = if e.body_port {
            (a.x + a.w / 2.0, a.y + LOOP_HEADER + 7.0)
        } else {
            (a.x + a.w / 2.0, a.y + a.h)
        };
        Some((from, (b.x + b.w / 2.0, b.y)))
    }
}

/// `toCanvasElements`: stored positions once someone arranged the
/// workflow, the layout otherwise; then the edges, the + under each leaf
/// and the spot where a missing trigger goes.
pub fn elements(nodes: &[Value], edges: &[Value]) -> Elements {
    let owners = loop_body_owners(nodes);
    let (computed, mut branch_members) = layout_positions(nodes, edges);
    let seed = graph::positions_are_seed(nodes);
    let mut out: Vec<Element> = Vec::new();
    for n in nodes {
        if owners.contains_key(id(n)) {
            continue;
        }
        let (x, y) = if seed {
            computed.get(id(n)).copied().unwrap_or((0.0, 0.0))
        } else {
            lattice(n)
        };
        let body = (kind(n) == "loop").then(|| layout_loop_body(n, nodes, edges));
        let w = body.as_ref().map_or(CARD_WIDTH, |b| b.width);
        out.push(Element {
            id: id(n).to_owned(),
            card: if body.is_some() {
                Card::Loop
            } else {
                Card::Step
            },
            parent: None,
            x,
            y,
            w,
            h: estimate_height(n, nodes, edges),
        });
        let Some(body) = body else {
            continue;
        };
        branch_members.extend(body.branch_members.iter().cloned());
        let order = n
            .pointer("/config/bodyNodeIds")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for b in order.iter().filter_map(Value::as_str) {
            let Some((_, at)) = body.positions.iter().find(|(i, _)| i == b) else {
                continue;
            };
            let Some(member) = nodes.iter().find(|m| id(m) == b) else {
                continue;
            };
            out.push(Element {
                id: b.to_owned(),
                card: Card::Step,
                parent: Some(id(n).to_owned()),
                x: x + at.0,
                y: y + at.1,
                w: CARD_WIDTH,
                h: estimate_height(member, nodes, edges),
            });
        }
    }

    let label = |e: &Value| match s(e, "conditionBranch") {
        Some("true") => Some("True"),
        Some("false") => Some("False"),
        _ => None,
    };
    let mut drawn: Vec<Edge> = Vec::new();
    let mut seen = HashSet::new();
    let mut exits: HashMap<String, Vec<String>> = HashMap::new();
    for e in edges {
        let (Some(a), Some(b)) = (s(e, "source"), s(e, "target")) else {
            continue;
        };
        if let (Some(o), None) = (owners.get(a), owners.get(b)) {
            exits
                .entry(format!("{o}->{b}"))
                .or_default()
                .push(id(e).to_owned());
        }
    }
    for e in edges {
        if !seen.insert(id(e).to_owned()) {
            continue;
        }
        let (Some(a), Some(b)) = (s(e, "source"), s(e, "target")) else {
            continue;
        };
        let (so, to) = (owners.get(a), owners.get(b));
        let branch = s(e, "conditionBranch").map(str::to_owned);
        if let Some(t_owner) = to {
            let entry = a == t_owner;
            if !entry && so != Some(t_owner) {
                continue;
            }
            drawn.push(Edge {
                id: id(e).to_owned(),
                source: a.to_owned(),
                target: b.to_owned(),
                body_port: entry,
                label: label(e),
                data: Some(EdgeData {
                    after: a.to_owned(),
                    before: b.to_owned(),
                    inside_branch: branch.is_some() || branch_members.contains(b),
                    branch,
                    body_of: Some(t_owner.clone()),
                }),
            });
            continue;
        }
        if let Some(s_owner) = so {
            let key = format!("{s_owner}->{b}");
            let list = exits.get(&key).cloned().unwrap_or_default();
            if list.first().map(String::as_str) != Some(id(e)) {
                continue;
            }
            drawn.push(Edge {
                id: id(e).to_owned(),
                source: s_owner.clone(),
                target: b.to_owned(),
                body_port: false,
                label: None,
                data: (list.len() == 1).then(|| EdgeData {
                    after: a.to_owned(),
                    before: b.to_owned(),
                    branch: None,
                    inside_branch: branch_members.contains(s_owner) || branch_members.contains(b),
                    body_of: None,
                }),
            });
            continue;
        }
        drawn.push(Edge {
            id: id(e).to_owned(),
            source: a.to_owned(),
            target: b.to_owned(),
            body_port: false,
            label: label(e),
            data: Some(EdgeData {
                after: a.to_owned(),
                before: b.to_owned(),
                inside_branch: branch_members.contains(a)
                    || branch_members.contains(b)
                    || branch.is_some(),
                branch,
                body_of: None,
            }),
        });
    }

    let leads: HashSet<String> = drawn
        .iter()
        .filter(|e| e.data.as_ref().is_none_or(|d| d.body_of.is_none()))
        .map(|e| e.source.clone())
        .collect();
    for n in nodes {
        if owners.contains_key(id(n)) || leads.contains(id(n)) {
            continue;
        }
        let Some(anchor) = out.iter().find(|e| e.id == id(n)).cloned() else {
            continue;
        };
        let add = format!("add:{}", id(n));
        out.push(Element {
            id: add.clone(),
            card: Card::AddStep {
                after: id(n).to_owned(),
                inside_branch: branch_members.contains(id(n)),
            },
            parent: None,
            x: anchor.x + anchor.w / 2.0 - PLUS / 2.0,
            y: anchor.y + estimate_height(n, nodes, edges) + 18.0,
            w: PLUS,
            h: PLUS,
        });
        drawn.push(Edge {
            id: format!("add-edge:{}", id(n)),
            source: id(n).to_owned(),
            target: add,
            body_port: false,
            label: None,
            data: None,
        });
    }

    if !nodes.iter().any(|n| kind(n) == "trigger") {
        let top = out
            .iter()
            .filter(|e| matches!(e.card, Card::Step | Card::Loop) && e.parent.is_none())
            .fold(None::<&Element>, |a, b| match a {
                Some(a) if a.y <= b.y => Some(a),
                _ => Some(b),
            });
        let (x, y) = top.map_or((0.0, 0.0), |t| {
            (
                t.x + t.w / 2.0 - CARD_WIDTH / 2.0,
                t.y - TRIGGER_SPOT.1 - ROW_GAP,
            )
        });
        out.push(Element {
            id: "add-trigger".into(),
            card: Card::AddTrigger,
            parent: None,
            x,
            y,
            w: TRIGGER_SPOT.0,
            h: TRIGGER_SPOT.1,
        });
    }
    Elements {
        nodes: out,
        edges: drawn,
        branch_members,
    }
}

/// `calculateControlOffset` at React Flow's default curvature.
fn control_offset(distance: f64) -> f64 {
    if distance >= 0.0 {
        0.5 * distance
    } else {
        0.25 * 25.0 * (-distance).sqrt()
    }
}

/// `stepEdgePath`: straight down a column, a bezier only for a real fork.
/// The control points (equal to the ends when straight) and the label's
/// point.
pub fn edge_curve(from: (f64, f64), to: (f64, f64)) -> ([(f64, f64); 4], (f64, f64)) {
    if (from.0 - to.0).abs() < STRAIGHT {
        return (
            [from, from, to, to],
            ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0),
        );
    }
    let off = control_offset(to.1 - from.1);
    let c1 = (from.0, from.1 + off);
    let c2 = (to.0, to.1 - off);
    let label = (
        from.0 * 0.125 + c1.0 * 0.375 + c2.0 * 0.375 + to.0 * 0.125,
        from.1 * 0.125 + c1.1 * 0.375 + c2.1 * 0.375 + to.1 * 0.125,
    );
    ([from, c1, c2, to], label)
}

/// `canConnect`: a hand-drawn edge keeps the graph a DAG the engine runs.
pub fn can_connect(
    nodes: &[Value],
    edges: &[Value],
    source: &str,
    target: &str,
    body_port: bool,
) -> bool {
    if source == target {
        return false;
    }
    let (Some(a), Some(b)) = (
        nodes.iter().find(|n| id(n) == source),
        nodes.iter().find(|n| id(n) == target),
    ) else {
        return false;
    };
    if kind(b) == "trigger" || kind(a) == "condition" {
        return false;
    }
    let owners = loop_body_owners(nodes);
    let t_owner = owners.get(target);
    if body_port {
        if t_owner.map(String::as_str) != Some(source) {
            return false;
        }
    } else if t_owner.is_some() && owners.get(source) != t_owner {
        return false;
    }
    if edges
        .iter()
        .any(|e| s(e, "source") == Some(source) && s(e, "target") == Some(target))
    {
        return false;
    }
    !graph::reachable_from(target, edges).contains(source)
}

/// A pan and zoom: flow point `p` draws at `p * zoom + (x, y)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

/// The bounds of the drawn steps (the add buttons left out unless they
/// are all there is).
pub fn bounds(els: &Elements) -> Option<(f64, f64, f64, f64)> {
    let steps: Vec<&Element> = els
        .nodes
        .iter()
        .filter(|e| matches!(e.card, Card::Step | Card::Loop))
        .collect();
    let drawn: Vec<&Element> = if steps.is_empty() {
        els.nodes.iter().collect()
    } else {
        steps
    };
    if drawn.is_empty() {
        return None;
    }
    let min_x = drawn.iter().map(|e| e.x).fold(f64::INFINITY, f64::min);
    let min_y = drawn.iter().map(|e| e.y).fold(f64::INFINITY, f64::min);
    let max_x = drawn
        .iter()
        .map(|e| e.x + e.w)
        .fold(f64::NEG_INFINITY, f64::max);
    let max_y = drawn
        .iter()
        .map(|e| e.y + e.h)
        .fold(f64::NEG_INFINITY, f64::max);
    Some((min_x, min_y, max_x - min_x, max_y - min_y))
}

fn place_at_top((bx, by, bw, _): (f64, f64, f64, f64), width: f64, zoom: f64) -> Viewport {
    Viewport {
        x: width / 2.0 - (bx + bw / 2.0) * zoom,
        y: OPENING_TOP - by * zoom,
        zoom,
    }
}

/// `openingViewport`: 100%, the steps centred, the first near the top.
pub fn opening(els: &Elements, width: f64) -> Viewport {
    match bounds(els) {
        Some((x, y, w, _)) => place_at_top((x, y, w, 0.0), width, 1.0),
        None => Viewport {
            x: width / 2.0,
            y: OPENING_TOP,
            zoom: 1.0,
        },
    }
}

/// `topAlignedFit` over every element: the whole workflow on screen, never
/// past 100%.
pub fn fit(els: &Elements, width: f64, height: f64) -> Option<Viewport> {
    let all: Vec<&Element> = els.nodes.iter().collect();
    if all.is_empty() {
        return None;
    }
    let min_x = all.iter().map(|e| e.x).fold(f64::INFINITY, f64::min);
    let min_y = all.iter().map(|e| e.y).fold(f64::INFINITY, f64::min);
    let max_x = all
        .iter()
        .map(|e| e.x + e.w)
        .fold(f64::NEG_INFINITY, f64::max);
    let max_y = all
        .iter()
        .map(|e| e.y + e.h)
        .fold(f64::NEG_INFINITY, f64::max);
    let (bw, bh) = (max_x - min_x, max_y - min_y);
    let fits = ((width * 0.9) / bw).min((height - 2.0 * OPENING_TOP) / bh);
    let zoom = fits.clamp(MIN_ZOOM, 1.0).min(1.0);
    Some(place_at_top((min_x, min_y, bw, bh), width, zoom))
}

/// React Flow's `zoomIn`/`zoomOut` step about the pane's centre.
pub fn zoom_by(v: Viewport, factor: f64, center: (f64, f64)) -> Viewport {
    let zoom = (v.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
    let k = zoom / v.zoom;
    Viewport {
        x: center.0 - (center.0 - v.x) * k,
        y: center.1 - (center.1 - v.y) * k,
        zoom,
    }
}

/// The minimap's size.
pub const MINI: (f64, f64) = (96.0, 64.0);

/// What the minimap draws, in its own pixels.
#[derive(Debug, Clone)]
pub struct Minimap {
    /// Flow units per minimap pixel.
    pub scale: f64,
    pub nodes: Vec<(f64, f64, f64, f64)>,
    pub view: (f64, f64, f64, f64),
}

/// React Flow's `MiniMap`: every node and the visible area, fitted with a
/// margin into 96×64.
pub fn minimap(els: &Elements, v: Viewport, (w, h): (f64, f64)) -> Minimap {
    let vr = (-v.x / v.zoom, -v.y / v.zoom, w / v.zoom, h / v.zoom);
    let mut b = vr;
    for e in &els.nodes {
        let (x0, y0) = (b.0.min(e.x), b.1.min(e.y));
        let (x1, y1) = ((b.0 + b.2).max(e.x + e.w), (b.1 + b.3).max(e.y + e.h));
        b = (x0, y0, x1 - x0, y1 - y0);
    }
    let scale = (b.2 / MINI.0).max(b.3 / MINI.1);
    let (vw, vh) = (scale * MINI.0, scale * MINI.1);
    let offset = 5.0 * scale;
    let x = b.0 - (vw - b.2) / 2.0 - offset;
    let y = b.1 - (vh - b.3) / 2.0 - offset;
    let full = (vw + 2.0 * offset, vh + 2.0 * offset);
    let k = (MINI.0 / full.0).min(MINI.1 / full.1);
    let map = |r: (f64, f64, f64, f64)| ((r.0 - x) * k, (r.1 - y) * k, r.2 * k, r.3 * k);
    Minimap {
        scale: 1.0 / k,
        nodes: els
            .nodes
            .iter()
            .map(|e| map((e.x, e.y, e.w, e.h)))
            .collect(),
        view: map(vr),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn n(i: &str, k: &str) -> Value {
        json!({ "id": i, "type": k, "label": i, "config": {}, "position": { "x": 0, "y": 0 } })
    }

    fn e(a: &str, b: &str) -> Value {
        json!({ "id": format!("{a}-{b}"), "source": a, "target": b })
    }

    #[test]
    fn a_chain_is_one_centred_column_with_a_plus_under_its_leaf() {
        let mut sc = n("s", "script");
        sc["config"]["scriptContent"] = json!("echo hi");
        let els = elements(&[n("t", "trigger"), sc], &[e("t", "s")]);
        let t = els.get("t").unwrap();
        let s2 = els.get("s").unwrap();
        assert_eq!((t.x, t.y, t.h), (-136.0, 0.0, 58.0));
        assert_eq!((s2.x, s2.y, s2.h), (-136.0, 112.0, 90.0));
        let plus = els.get("add:s").unwrap();
        assert_eq!((plus.x, plus.y), (-7.0, 112.0 + 90.0 + 18.0));
        assert_eq!(els.edges.len(), 2);
        assert!(els.get("add-trigger").is_none());
    }

    #[test]
    fn a_condition_forks_into_tagged_lanes() {
        let nodes = [
            n("t", "trigger"),
            n("c", "condition"),
            n("a", "script"),
            n("b", "script"),
        ];
        let mut ta = e("c", "a");
        ta["conditionBranch"] = json!("true");
        let mut fb = e("c", "b");
        fb["conditionBranch"] = json!("false");
        let els = elements(&nodes, &[e("t", "c"), ta, fb]);
        let (a, b) = (els.get("a").unwrap(), els.get("b").unwrap());
        assert_eq!(a.y, b.y);
        assert!(a.x < b.x && b.x - a.x >= CARD_WIDTH + BRANCH_GAP - 8.0);
        let labels: Vec<_> = els.edges.iter().filter_map(|e| e.label).collect();
        assert_eq!(labels, ["True", "False"]);
        assert!(els.branch_members.contains("a"));
    }

    #[test]
    fn a_loop_frames_its_body_and_feeds_it_from_the_body_port() {
        let mut lp = n("l", "loop");
        lp["config"]["bodyNodeIds"] = json!(["x"]);
        let nodes = [n("t", "trigger"), lp, n("x", "script")];
        let els = elements(&nodes, &[e("t", "l"), e("l", "x")]);
        let l = els.get("l").unwrap();
        let x = els.get("x").unwrap();
        assert_eq!(l.card, Card::Loop);
        assert_eq!(l.w, LOOP_WIDTH);
        assert_eq!(x.parent.as_deref(), Some("l"));
        assert!(x.y > l.y + LOOP_HEADER && x.x > l.x);
        let feed = els.edges.iter().find(|e| e.target == "x").unwrap();
        assert!(feed.body_port);
        assert_eq!(feed.data.as_ref().unwrap().body_of.as_deref(), Some("l"));
    }

    #[test]
    fn no_trigger_leaves_its_spot_above_the_top_card() {
        let els = elements(&[n("s", "script")], &[]);
        let spot = els.get("add-trigger").unwrap();
        assert_eq!((spot.x, spot.y), (-136.0, -58.0 - ROW_GAP));
    }

    #[test]
    fn connections_stay_acyclic_and_off_conditions_and_triggers() {
        let nodes = [
            n("t", "trigger"),
            n("a", "script"),
            n("b", "script"),
            n("c", "condition"),
        ];
        let edges = [e("t", "a"), e("a", "b")];
        assert!(!can_connect(&nodes, &edges, "b", "a", false), "a cycle");
        assert!(
            !can_connect(&nodes, &edges, "a", "t", false),
            "into the trigger"
        );
        assert!(
            !can_connect(&nodes, &edges, "c", "b", false),
            "out of a condition"
        );
        assert!(!can_connect(&nodes, &edges, "a", "b", false), "a duplicate");
        assert!(can_connect(&nodes, &edges, "t", "b", false));
    }

    #[test]
    fn curves_only_for_real_forks() {
        let (pts, mid) = edge_curve((0.0, 0.0), (8.0, 100.0));
        assert_eq!(pts[1], (0.0, 0.0));
        assert_eq!(mid, (4.0, 50.0));
        let (pts, mid) = edge_curve((0.0, 0.0), (200.0, 100.0));
        assert_eq!(pts[1], (0.0, 50.0));
        assert_eq!(mid, (100.0, 50.0));
    }

    #[test]
    fn the_view_opens_at_full_size_and_fits_without_growing() {
        let els = elements(&[n("t", "trigger")], &[]);
        let v = opening(&els, 1000.0);
        assert_eq!((v.x, v.y, v.zoom), (500.0 - 4.0, 48.0, 1.0));
        let fitted = fit(&els, 1000.0, 800.0).unwrap();
        assert_eq!(fitted.zoom, 1.0);
        let z = zoom_by(v, 1.2, (500.0, 400.0));
        assert!((z.zoom - 1.2).abs() < 1e-9);
        assert!((zoom_by(v, 0.01, (0.0, 0.0)).zoom - MIN_ZOOM).abs() < 1e-9);
    }
}
