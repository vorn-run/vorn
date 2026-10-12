//! `WorkflowCanvas`: the cards, loop frames and add buttons where the
//! layout or the stored positions put them, the edges between their ports,
//! the dot grid, the zoom controls and the minimap, under a pan and zoom.

use std::collections::HashMap;

use serde_json::Value;

use super::act;
use crate::app::canvas::{card_id, mini_id, pane_id, panel_width, port_id};
use crate::canvas::{self, Card, Element, Elements, Viewport, LOOP_HEADER, PLUS};
use crate::look;
use crate::runs::{self, s as sv};
use crate::screen::{c, ico, white, Screen};
use crate::ui::{div, shape, text, tooltip, Action, Cursor, El, Id, Rgba, Role, Sense};
use crate::workflows::{self as wf, Anchor, Draft, Gesture, Panel, WfAct};

fn ink(a: f32) -> Rgba {
    Rgba::hexa(look::INK, a)
}

fn secondary() -> Rgba {
    ink(look::INK_SECONDARY)
}

fn f(v: f64) -> f32 {
    v as f32
}

/// `liveNodeStatus`: the worst state each node is in across the runs
/// going now; none when nothing runs.
fn live_status(s: &Screen<'_>, d: &Draft) -> HashMap<String, String> {
    let rank = |st: &str| match st {
        "waiting" => 4,
        "running" => 3,
        "error" => 2,
        "success" => 1,
        _ => 0,
    };
    let mut out: HashMap<String, String> = HashMap::new();
    for r in s.store.runs.values() {
        if sv(r, "status") != Some("running") || sv(r, "workflowId") != Some(d.id()) {
            continue;
        }
        for n in runs::states(r) {
            let (Some(id), Some(st)) = (sv(n, "nodeId"), sv(n, "status")) else {
                continue;
            };
            let r2 = rank(st);
            if r2 > 0 && r2 > out.get(id).map_or(0, |o| rank(o)) {
                out.insert(id.to_owned(), st.to_owned());
            }
        }
    }
    out
}

/// The canvas pane's size, as the app computes it.
fn pane_size(s: &Screen<'_>) -> (f32, f32) {
    let side = if s.st.sidebar { look::SIDEBAR_W } else { 0.0 };
    (
        (s.size.0 - side - panel_width(&s.st.wf.panel)).max(0.0),
        (s.size.1 - look::TOP_BAR_H).max(0.0),
    )
}

/// The canvas with its controls.
pub fn canvas(s: &Screen<'_>, d: &Draft) -> El {
    let (w, h) = pane_size(s);
    let mut els = canvas::elements(d.nodes(), d.edges());
    let view =
        s.st.wf
            .canvas
            .views
            .get(d.id())
            .copied()
            .unwrap_or_else(|| canvas::opening(&els, f64::from(w)));
    // A dragged card, and the steps inside a dragged loop, go with the pointer.
    for (id, p) in &s.st.wf.canvas.dragged {
        let Some(e) = els.nodes.iter().find(|e| &e.id == id).cloned() else {
            continue;
        };
        let (dx, dy) = (p.0 - e.x, p.1 - e.y);
        for el in els.nodes.iter_mut() {
            if &el.id == id || el.parent.as_deref() == Some(id.as_str()) {
                el.x += dx;
                el.y += dy;
            }
        }
    }
    let status = live_status(s, d);
    let z = f(view.zoom);
    let center = (w / 2.0, h / 2.0);
    let offset = (
        f(view.x) - center.0 * (1.0 - z),
        f(view.y) - center.1 * (1.0 - z),
    );
    let mut world = div().absolute(0.0, 0.0).size(w, h).transform(z, offset);
    for e in &els.edges {
        world = world.child_if(edge_line(&els, e));
    }
    for e in &els.edges {
        world = world.child_if(edge_pill(s, &els, e));
    }
    for e in &els.nodes {
        world = world.child(element(s, d, &els, e, &status));
    }
    if let Some(Gesture::Connect {
        source,
        body_port,
        to,
    }) = &s.st.wf.canvas.gesture
    {
        if let Some(src) = els.get(source) {
            let from = if *body_port {
                (src.x + src.w / 2.0, src.y + LOOP_HEADER + 7.0)
            } else {
                (src.x + src.w / 2.0, src.y + src.h)
            };
            world = world.child_if(line(from, *to, None));
        }
    }
    s.acts.bind(pane_id(), act(WfAct::Settings));
    div()
        .id(pane_id())
        .on(Action::Click)
        .sense(Sense::MOVE | Sense::WHEEL)
        .grow()
        .min_w0()
        .min_h(0.0)
        .h_full()
        .clip()
        .bg(c(look::SURFACE_BASE))
        .role(Role::Group, "Workflow canvas")
        .child(dots(view, (w, h)))
        .child(world)
        .child(controls(s, h))
        .child(minimap(s, &els, view, (w, h)))
}

/// `Background` with dots: 20 apart at 100%, one pixel, white at 5%.
fn dots(v: Viewport, (w, h): (f32, f32)) -> El {
    let gap = 20.0 * v.zoom;
    let mut layer = div().absolute(0.0, 0.0).size(w, h);
    if gap < 8.0 {
        return layer;
    }
    let size = f((1.0 * v.zoom).max(1.0));
    let x0 = v.x.rem_euclid(gap);
    let y0 = v.y.rem_euclid(gap);
    let mut y = y0;
    while y < f64::from(h) {
        let mut x = x0;
        while x < f64::from(w) {
            layer = layer.child(
                div()
                    .absolute(f(x) - size / 2.0, f(y) - size / 2.0)
                    .size(size, size)
                    .rounded(size / 2.0)
                    .bg(white(0.037)),
            );
            x += gap;
        }
        y += gap;
    }
    layer
}

/// An edge's line from `from` to `to`, as `stepEdgePath` draws it.
fn line(from: (f64, f64), to: (f64, f64), _label: Option<()>) -> Option<El> {
    let (pts, _) = canvas::edge_curve(from, to);
    let pad = 2.0;
    let x0 = pts.iter().map(|p| p.0).fold(f64::INFINITY, f64::min) - pad;
    let y0 = pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min) - pad;
    let x1 = pts.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max) + pad;
    let y1 = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max) + pad;
    let (bw, bh) = (x1 - x0, y1 - y0);
    if bw <= 0.0 || bh <= 0.0 {
        return None;
    }
    let p = |i: usize| format!("{:.2} {:.2}", pts[i].0 - x0, pts[i].1 - y0);
    let d = if pts[1] == pts[0] && pts[2] == pts[3] {
        format!("M {} L {}", p(0), p(3))
    } else {
        format!("M {} C {}, {}, {}", p(0), p(1), p(2), p(3))
    };
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{bw:.2}\" height=\"{bh:.2}\" viewBox=\"0 0 {bw:.2} {bh:.2}\"><path d=\"{d}\" fill=\"none\" stroke=\"#ffffff\" stroke-width=\"1.5\"/></svg>"
    );
    Some(shape(svg, f(bw), f(bh), white(0.16)).absolute(f(x0), f(y0)))
}

fn edge_line(els: &Elements, e: &canvas::Edge) -> Option<El> {
    let (from, to) = els.ends(e)?;
    line(from, to, None)
}

/// The edge's anchor in the library, when the open library points at it.
fn anchored(s: &Screen<'_>, data: &canvas::EdgeData) -> bool {
    matches!(&s.st.wf.panel, Panel::Library(a)
        if a.after.as_deref() == Some(data.after.as_str()) && a.before.as_deref() == Some(data.before.as_str()) && a.replace.is_none())
}

/// The branch pill and the + an edge shows on hover, centred on it.
fn edge_pill(s: &Screen<'_>, els: &Elements, e: &canvas::Edge) -> Option<El> {
    let (from, to) = els.ends(e)?;
    let (_, mid) = canvas::edge_curve(from, to);
    let hit = Id::new("wf-edge").child(&e.id);
    let insertable = e.data.is_some();
    let plus_at = e.data.as_ref().map(|data| Anchor {
        after: Some(data.after.clone()),
        before: Some(data.before.clone()),
        inside_branch: data.inside_branch,
        body_only: data.body_of.is_some(),
        ..Anchor::default()
    });
    let plus_id = plus_at
        .as_ref()
        .map(|a| crate::act::Acts::id("wf-plus", &act(WfAct::Library(a.clone()))));
    let on = e.data.as_ref().is_some_and(|d| anchored(s, d));
    let hovered = s.ui.hovered(hit) || plus_id.is_some_and(|p| s.ui.hovered(p));
    let mut stack = div().col().items_center().gap(4.0);
    if let Some(l) = e.label {
        stack = stack.child(
            div()
                .px(10.0)
                .py(2.0)
                .rounded(10.0)
                .border(1.0, white(0.08))
                .bg(c(look::SURFACE_BASE))
                .child(
                    text(l, 10.0, secondary())
                        .weight(600)
                        .line_height(15.0)
                        .silent(),
                ),
        );
    }
    if insertable && (hovered || on) {
        if let Some(a) = plus_at {
            stack = stack.child(plus(s, a, on));
        }
    }
    let (sw, sh) = (
        if e.label.is_some() { 60.0 } else { PLUS as f32 },
        f32::from(u8::from(e.label.is_some())) * 23.0
            + if insertable && (hovered || on) {
                26.0
            } else {
                0.0
            },
    );
    // A 40-wide band along the edge, which a hover turns into the +.
    let (x0, x1) = (from.0.min(to.0), from.0.max(to.0));
    let band = div()
        .id(hit)
        .sense(Sense::HOVER)
        .absolute(f(x0) - 20.0, f(from.1))
        .size(f(x1 - x0) + 40.0, f((to.1 - from.1).max(0.0)));
    let group = div()
        .absolute(f(mid.0) - sw / 2.0, f(mid.1) - sh / 2.0)
        .w(sw)
        .child(stack.w_full());
    Some(
        div()
            .absolute(0.0, 0.0)
            .child(if insertable { band } else { div() })
            .child(group),
    )
}

/// `ConnectorButton`: the round + that opens the library at `at`.
fn plus(s: &Screen<'_>, at: Anchor, active: bool) -> El {
    let id = s.acts.on("wf-plus", act(WfAct::Library(at)));
    let hot = s.ui.hover(id);
    let (fg, bg, border) = if active {
        (c(look::WHITE), white(0.08), white(0.40))
    } else {
        (
            c(look::GRAY_500).mix(c(look::WHITE), hot),
            c(look::SURFACE_SUNKEN),
            white(0.10).mix(white(0.20), hot),
        )
    };
    div()
        .id(id)
        .on(Action::Click)
        .size(PLUS as f32, PLUS as f32)
        .rounded(11.0)
        .center()
        .bg(bg)
        .border(1.0, border)
        .role(Role::Button, "Add a step")
        .child(ico("plus", 13.0, 2.5, fg))
}

fn element(
    s: &Screen<'_>,
    d: &Draft,
    els: &Elements,
    e: &Element,
    status: &HashMap<String, String>,
) -> El {
    let at = div().absolute(f(e.x), f(e.y));
    match &e.card {
        Card::AddStep {
            after,
            inside_branch,
        } => {
            let a = Anchor {
                after: Some(after.clone()),
                inside_branch: *inside_branch,
                ..Anchor::default()
            };
            let on = matches!(&s.st.wf.panel, Panel::Library(l)
                if l.after.as_deref() == Some(after.as_str()) && l.before.is_none() && l.replace.is_none() && !l.trigger);
            at.child(plus(s, a, on))
        }
        Card::AddTrigger => at.child(add_trigger(s)),
        Card::Step => match d.node(&e.id) {
            Some(n) => at.child(step(s, d, els, e, n, status.get(&e.id).map(String::as_str))),
            None => at,
        },
        Card::Loop => match d.node(&e.id) {
            Some(n) => at.child(loop_frame(
                s,
                d,
                els,
                e,
                n,
                status.get(&e.id).map(String::as_str),
            )),
            None => at,
        },
    }
}

/// `AddTriggerNode`: the dashed spot where a trigger goes.
fn add_trigger(s: &Screen<'_>) -> El {
    let at = Anchor {
        trigger: true,
        ..Anchor::default()
    };
    let active = matches!(&s.st.wf.panel, Panel::Library(a) if a.trigger);
    let id = s.acts.on("wf-canvas", act(WfAct::Library(at)));
    let hot = s.ui.hover(id);
    let (fg, border) = if active {
        (c(look::WHITE), white(0.40))
    } else {
        (
            c(look::GRAY_500).mix(c(look::GRAY_300), hot),
            white(0.15).mix(white(0.30), hot),
        )
    };
    div()
        .id(id)
        .on(Action::Click)
        .row()
        .center()
        .gap(8.0)
        .size(280.0, 58.0)
        .role(Role::Button, "Add a trigger")
        .child(dashed(280.0, 58.0, 8.0, border))
        .child(ico("zap", 14.0, 2.0, fg))
        .child(text("Add a trigger", 13.0, fg).line_height(19.5).silent())
}

/// A dashed outline over a `w`×`h` box.
fn dashed(w: f32, h: f32, r: f32, color: Rgba) -> El {
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\"><rect x=\"0.5\" y=\"0.5\" width=\"{}\" height=\"{}\" rx=\"{r}\" fill=\"none\" stroke=\"#ffffff\" stroke-width=\"1\" stroke-dasharray=\"3 3\"/></svg>",
        w - 1.0,
        h - 1.0
    );
    shape(svg, w, h, color).absolute(0.0, 0.0)
}

/// `HANDLE_CLASS`: a port, 7 px, ringed.
fn handle(x: f32, y: f32) -> El {
    div()
        .absolute(x - 3.5, y - 3.5)
        .size(7.0, 7.0)
        .rounded(3.5)
        .bg(c(look::SURFACE_BASE))
        .border(1.0, white(0.35))
}

/// An output port that a connection is drawn from.
fn out_port(s: &Screen<'_>, node: &str, body: bool, x: f32, y: f32) -> El {
    let id = port_id(node, body);
    let hot = s.ui.hover(id);
    div()
        .id(id)
        .sense(Sense::MOVE | Sense::HOVER)
        .cursor(Cursor::Pointer)
        .absolute(x - 7.5, y - 7.5)
        .size(15.0, 15.0)
        .hit_slop(4.0, 4.0)
        .center()
        .role(Role::Button, "Connect")
        .child(
            div()
                .size(7.0, 7.0)
                .rounded(3.5)
                .bg(c(look::SURFACE_BASE))
                .border(1.0, white(0.35).mix(white(0.7), hot)),
        )
}

fn status_dot_color(st: &str) -> Option<Rgba> {
    matches!(
        st,
        "waiting" | "running" | "error" | "success" | "pending" | "skipped"
    )
    .then(|| {
        let (rgb, a) = runs::dot(st);
        Rgba::hexa(rgb, a)
    })
}

/// `StepNode`: the card (`NodeShell`) with its ports and hover toolbar.
fn step(
    s: &Screen<'_>,
    d: &Draft,
    els: &Elements,
    e: &Element,
    n: &Value,
    status: Option<&str>,
) -> El {
    let id = sv(n, "id").unwrap_or("");
    let label = sv(n, "label").unwrap_or("");
    let kind = sv(n, "type").unwrap_or("");
    let selected = s.st.wf.node.as_deref() == Some(id) && s.st.wf.panel == Panel::Node;
    let cid = card_id(id);
    s.acts.bind(cid, act(WfAct::SelectNode(id.to_owned())));
    let tools = [
        Id::new("wf-card-run").child(id),
        Id::new("wf-card-replace").child(id),
        Id::new("wf-card-delete").child(id),
    ];
    let hovering = s.ui.hovered(cid) || tools.iter().any(|t| s.ui.hovered(*t));
    let border = if selected {
        white(0.40)
    } else if status == Some("error") {
        Rgba::hexa(look::DANGER, 0.6)
    } else {
        white(0.08)
    };
    let draggable = e.draggable();
    let mut card = div()
        .id(cid)
        .on(Action::Click)
        .col()
        .w(280.0)
        .px(12.0)
        .py(10.0)
        .rounded(6.0)
        .border(1.0, border)
        .bg(c(look::SURFACE_SUNKEN).mix(white(0.02), if s.ui.hovered(cid) { 1.0 } else { 0.0 }))
        .role(Role::Button, label)
        .selected(selected)
        .child(
            div()
                .row()
                .items_center()
                .gap(12.0)
                .child(super::node_glyph(s, n, 18.0))
                .child(
                    div()
                        .col()
                        .grow()
                        .min_w0()
                        .child(
                            text(label, 13.0, c(look::WHITE))
                                .weight(500)
                                .line_height(19.5)
                                .truncate()
                                .silent(),
                        )
                        .child(
                            text(wf::subtitle(n), 11.0, c(look::GRAY_500))
                                .line_height(16.5)
                                .truncate()
                                .silent(),
                        ),
                ),
        );
    if draggable {
        card = card.sense(Sense::MOVE);
    }
    if let Some(foot) = wf::footer(n) {
        let mono = matches!(
            kind,
            "script" | "condition" | "httpRequest" | "callConnectorAction"
        );
        let mut t = text(foot, 11.0, c(look::GRAY_600))
            .line_height(16.5)
            .truncate();
        if mono {
            t = t.mono();
        }
        card = card.child(div().mt(8.0).pt(8.0).border_top(1.0, white(0.06)).child(t));
    }
    if status == Some("running") {
        card = card.child(
            div()
                .absolute(-1.0, -1.0)
                .size(280.0, f(e.h))
                .rounded(6.0)
                .border(1.0, white(0.35)),
        );
    }
    if let Some(col) = status.and_then(status_dot_color) {
        card = card.child(
            div()
                .absolute_right(8.0, 8.0)
                .size(6.0, 6.0)
                .rounded(3.0)
                .bg(col),
        );
    }
    let mut wrap = div().w(280.0).child(card);
    if kind != "trigger" {
        wrap = wrap.child(handle(140.0, 0.0));
    }
    let h = f(crate::canvas::drawn_height(n));
    wrap = wrap.child(out_port(s, id, false, 140.0, h));
    if hovering {
        wrap = wrap.child(hover_tools(s, d, els, id, kind, &tools, h));
    }
    wrap
}

/// `NodeHoverToolbar`: run to here, replace and delete, beside the card.
fn hover_tools(
    s: &Screen<'_>,
    d: &Draft,
    els: &Elements,
    id: &str,
    kind: &str,
    ids: &[Id; 3],
    h: f32,
) -> El {
    let trigger = kind == "trigger";
    let body = crate::graph::loop_body_owners(d.nodes()).contains_key(id);
    let _ = els;
    let replaceable = crate::graph::replaceable(kind);
    let button = |bid: Id, a: WfAct, icon: &str, label: &str| {
        s.acts.bind(bid, act(a));
        let hv = s.ui.hover(bid);
        let b = div()
            .id(bid)
            .on(Action::Click)
            .p(6.0)
            .rounded(6.0)
            .bg(c(look::SURFACE_OVERLAY))
            .border(1.0, white(0.12).mix(white(0.20), hv))
            .role(Role::Button, label)
            .child(ico(
                icon,
                12.0,
                2.0,
                c(look::GRAY_400).mix(c(look::WHITE), hv),
            ));
        tooltip(s.ui, b, bid, label, None)
    };
    let mut col = div().col().gap(4.0);
    let mut count = 0.0;
    if !trigger && !body {
        col = col.child(button(
            ids[0],
            WfAct::RunTo(id.to_owned()),
            "step-forward",
            "Run to this step",
        ));
        count += 1.0;
    }
    if replaceable || trigger {
        let at = if trigger {
            Anchor {
                trigger: true,
                ..Anchor::default()
            }
        } else {
            Anchor {
                after: Some(id.to_owned()),
                replace: Some(id.to_owned()),
                body_only: body,
                ..Anchor::default()
            }
        };
        col = col.child(button(
            ids[1],
            WfAct::Library(at),
            "replace",
            "Replace step",
        ));
        count += 1.0;
    }
    col = col.child(button(
        ids[2],
        WfAct::DeleteNode(id.to_owned()),
        "trash-2",
        "Delete step",
    ));
    count += 1.0;
    let tall = count * 26.0 + (count - 1.0) * 4.0;
    col.absolute(286.0, h / 2.0 - tall / 2.0)
}

/// `LoopNode`: the frame, its header, the body drawn over it, its + and
/// footer.
fn loop_frame(
    s: &Screen<'_>,
    d: &Draft,
    els: &Elements,
    e: &Element,
    n: &Value,
    status: Option<&str>,
) -> El {
    let id = sv(n, "id").unwrap_or("");
    let label = sv(n, "label").unwrap_or("");
    let cfg = n.get("config").cloned().unwrap_or_default();
    let for_each = sv(&cfg, "mode") == Some("forEach");
    let selected = s.st.wf.node.as_deref() == Some(id) && s.st.wf.panel == Panel::Node;
    let cid = card_id(id);
    s.acts.bind(cid, act(WfAct::SelectNode(id.to_owned())));
    let until = cfg
        .pointer("/until/variable")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .map(|v| {
            format!(
                " \u{b7} until {v} {} {}",
                cfg.pointer("/until/operator")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                wf::text_of(cfg.pointer("/until/value"))
            )
        });
    let footer = if for_each {
        let items = sv(&cfg, "items")
            .map(str::trim)
            .filter(|i| !i.is_empty())
            .unwrap_or("a list not chosen yet");
        format!("for each in {items}{}", until.clone().unwrap_or_default())
    } else {
        match &until {
            Some(u) => u.chars().skip(3).collect(),
            None => "runs every pass".into(),
        }
    };
    let body: Vec<&str> = cfg
        .get("bodyNodeIds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|b| d.node(b).is_some())
        .collect();
    let layout = canvas::layout_loop_body(n, d.nodes(), d.edges());
    let border = if selected {
        white(0.40)
    } else if status == Some("error") {
        Rgba::hexa(look::DANGER, 0.6)
    } else {
        white(0.08)
    };
    let w = f(e.w);
    let chip = if for_each {
        "each".to_owned()
    } else {
        format!(
            "max {}",
            cfg.get("maxIterations")
                .filter(|m| !m.is_null())
                .map_or("1".to_owned(), |m| wf::text_of(Some(m)))
        )
    };
    let mut head = div()
        .id(cid)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(8.0)
        .px(16.0)
        .py(10.0)
        .border_bottom(1.0, white(0.06))
        .bg(white(0.0).mix(white(0.02), s.ui.hover(cid)))
        .role(Role::Button, label)
        .selected(selected)
        .child(ico("repeat", 13.0, 2.0, secondary()))
        .child(
            div().grow().min_w0().child(
                text(label, 12.5, c(look::WHITE))
                    .weight(600)
                    .line_height(18.75)
                    .truncate()
                    .silent(),
            ),
        )
        .child(
            div().px(6.0).py(2.0).rounded(4.0).bg(white(0.06)).child(
                text(chip, 10.0, c(look::GRAY_400))
                    .mono()
                    .line_height(15.0)
                    .silent(),
            ),
        );
    if e.draggable() {
        head = head.sense(Sense::MOVE);
    }
    if let Some(col) = status.and_then(status_dot_color) {
        head = head.child(div().size(6.0, 6.0).rounded(3.0).bg(col));
    }
    let last = body.last().copied().unwrap_or(id).to_owned();
    let anchor = Anchor {
        after: Some(last.clone()),
        before: Some(crate::graph::LOOP_BODY.to_owned()),
        body_only: true,
        ..Anchor::default()
    };
    let on = matches!(&s.st.wf.panel, Panel::Library(l) if l.after.as_deref() == Some(last.as_str()) && l.before.as_deref() == Some(crate::graph::LOOP_BODY));
    let mut inner = div().col().items_center().px(16.0).pt(16.0);
    inner = if body.is_empty() {
        inner.child(
            div()
                .w_full()
                .h(52.0)
                .center()
                .child(dashed(w - 32.0, 52.0, 6.0, white(0.12)))
                .child(
                    text(
                        "No steps yet \u{2014} add one below",
                        11.0,
                        c(look::GRAY_500),
                    )
                    .line_height(16.5)
                    .silent(),
                ),
        )
    } else {
        inner.child(div().h(f(layout.height)))
    };
    inner = inner
        .child(div().w(1.0).h(18.0).bg(white(0.08)))
        .child(plus(s, anchor, on));
    let rail = div()
        .col()
        .w(w)
        .rounded(8.0)
        .border(1.0, border)
        .bg(c(look::SURFACE_SUNKEN))
        .child(head)
        .child(inner)
        .child(
            div()
                .px(16.0)
                .pt(10.0)
                .pb(12.0)
                .row()
                .justify_center()
                .child(
                    text(format!("\u{21bb} {footer}"), 10.0, c(look::GRAY_500))
                        .mono()
                        .line_height(15.0)
                        .truncate()
                        .silent(),
                ),
        );
    let tools = [
        Id::new("wf-card-run").child(id),
        Id::new("wf-card-replace").child(id),
        Id::new("wf-card-delete").child(id),
    ];
    let hovering = s.ui.hovered(cid) || tools.iter().any(|t| s.ui.hovered(*t));
    let mut wrap = div()
        .w(w)
        .child(rail)
        .child(handle(w / 2.0, 0.0))
        .child(out_port(s, id, true, w / 2.0, f(LOOP_HEADER) + 7.0))
        .child(out_port(s, id, false, w / 2.0, f(e.h)));
    if hovering {
        let tools_el = hover_tools(s, d, els, id, "loop", &tools, f(e.h));
        wrap = wrap.child(tools_el.absolute(w + 6.0, f(e.h) / 2.0 - 41.0));
    }
    wrap
}

/// React Flow's `Controls`: zoom in, zoom out, fit, tidy up.
fn controls(s: &Screen<'_>, h: f32) -> El {
    let button = |a: WfAct, icon: &str, label: &str, last: bool| {
        let id = s.acts.on("wf-controls", act(a));
        let hot = s.ui.hover(id);
        let mut b = div()
            .id(id)
            .on(Action::Click)
            .size(26.0, 26.0)
            .center()
            .bg(white(0.0).mix(white(0.06), hot))
            .role(Role::Button, label)
            .child(if matches!(icon, "plus" | "minus") {
                // React Flow's own zoom marks: larger and near white.
                ico(icon, 14.0, 2.75, c(look::GRAY_200).mix(c(look::WHITE), hot))
            } else {
                ico(icon, 12.0, 2.0, c(look::GRAY_400).mix(c(look::WHITE), hot))
            });
        if !last {
            b = b.border_bottom(1.0, white(0.08));
        }
        tooltip(s.ui, b, id, label, None)
    };
    div()
        .absolute(15.0, h - 15.0 - (4.0 * 26.0 + 3.0 + 2.0))
        .col()
        .rounded(6.0)
        .border(1.0, white(0.12))
        .bg(c(look::SURFACE_OVERLAY))
        .clip()
        .role(Role::Toolbar, "Canvas controls")
        .child(button(WfAct::ZoomIn, "plus", "Zoom in", false))
        .child(button(WfAct::ZoomOut, "minus", "Zoom out", false))
        .child(button(WfAct::Fit, "maximize", "Fit the workflow", false))
        .child(button(
            WfAct::Tidy,
            "align-vertical-space-around",
            "Tidy up",
            true,
        ))
}

/// `MiniMap`: every element and the visible area, 96×64, draggable.
fn minimap(_s: &Screen<'_>, els: &Elements, v: Viewport, (w, h): (f32, f32)) -> El {
    let m = canvas::minimap(els, v, (f64::from(w), f64::from(h)));
    let (mw, mh) = (f(canvas::MINI.0), f(canvas::MINI.1));
    let rects: String = m
        .nodes
        .iter()
        .map(|r| {
            format!(
                "<rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" rx=\"1\"/>",
                r.0, r.1, r.2, r.3
            )
        })
        .collect();
    let nodes = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{mw}\" height=\"{mh}\" viewBox=\"0 0 {mw} {mh}\" fill=\"#ffffff\">{rects}</svg>"
    );
    let vr = m.view;
    let mask = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{mw}\" height=\"{mh}\" viewBox=\"0 0 {mw} {mh}\"><path fill-rule=\"evenodd\" fill=\"#ffffff\" d=\"M0 0 H{mw} V{mh} H0 Z M{:.2} {:.2} h{:.2} v{:.2} h{:.2} Z\"/></svg>",
        vr.0, vr.1, vr.2, vr.3, -vr.2
    );
    div()
        .id(mini_id())
        .sense(Sense::MOVE)
        .absolute_right(15.0, h - mh - 15.0 - 2.0)
        .size(mw + 2.0, mh + 2.0)
        .p(1.0)
        .rounded(4.0)
        .border(1.0, white(0.12))
        .bg(c(look::SURFACE_PANEL))
        .clip()
        .role(Role::Image, "Mini map")
        .child(shape(nodes, mw, mh, white(0.25)).absolute(1.0, 1.0))
        .child(shape(mask, mw, mh, Rgba::hexa(0x000000, 0.55)).absolute(1.0, 1.0))
}
