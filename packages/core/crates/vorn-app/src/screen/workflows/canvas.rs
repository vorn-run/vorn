//! `WorkflowCanvas`: the cards, loop frames and add buttons where the
//! layout or the stored positions put them, the edges between their ports,
//! the dot grid, the zoom controls and the minimap, under a pan and zoom.

use std::collections::HashMap;

use serde_json::Value;

use super::act;
use vorn_components::graph as kit;
use vorn_components::template::{self as tpl, Bind, Press};
use vorn_view::{Kind, Row, Source};

use crate::app::canvas::{card_id, mini_id, pane_id, panel_width, port_id};
use crate::canvas::{self, Card, Element, Elements, LOOP_HEADER};
use crate::look;
use crate::runs::{self, s as sv};
use crate::screen::dashed;
use crate::screen::{c, ico, white, Screen};
use crate::ui::{div, text, Action, El, Id, Rgba, Role, Sense};
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
    let side = if s.st.sidebar { s.sidebar_w() } else { 0.0 };
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
    let mut world = Vec::new();
    for e in &els.edges {
        world.extend(edge_line(&els, e));
    }
    for e in &els.edges {
        world.extend(edge_pill(s, &els, e));
    }
    for e in &els.nodes {
        world.push(element(s, d, &els, e, &status));
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
            world.extend(kit::edge(from, *to));
        }
    }
    s.acts.bind(pane_id(), act(WfAct::Settings));
    let m = canvas::minimap(&els, view, (f64::from(w), f64::from(h)));
    kit::surface(
        pane_id(),
        (w, h),
        view,
        world,
        vec![controls(s, h), kit::minimap_view(mini_id(), &m, h)],
    )
    .role(Role::Group, "Workflow canvas")
}

fn edge_line(els: &Elements, e: &canvas::Edge) -> Option<El> {
    let (from, to) = els.ends(e)?;
    kit::edge(from, to)
}

/// The edge's anchor in the library, when the open library points at it.
fn anchored(s: &Screen<'_>, data: &canvas::EdgeData) -> bool {
    matches!(&s.st.wf.panel, Panel::Library(a)
        if a.after.as_deref() == Some(data.after.as_str()) && a.before.as_deref() == Some(data.before.as_str()) && a.replace.is_none())
}

/// The branch pill and the + an edge shows on hover, centred on it.
fn edge_pill(s: &Screen<'_>, els: &Elements, e: &canvas::Edge) -> Option<El> {
    let (from, to) = els.ends(e)?;
    let (_, mid) = kit::edge_curve(from, to);
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
        stack = stack.child(kit::edge_label(l));
    }
    if insertable && (hovered || on) {
        if let Some(a) = plus_at {
            stack = stack.child(plus(s, a, on));
        }
    }
    let (sw, sh) = (
        if e.label.is_some() {
            60.0
        } else {
            kit::PLUS as f32
        },
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
    kit::plus(id, "Add a step", active, s.ui.hover(id))
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

/// An input port, as the kit draws one.
fn handle(x: f32, y: f32) -> El {
    kit::handle(x, y)
}

/// An output port that a connection is drawn from.
fn out_port(s: &Screen<'_>, node: &str, body: bool, x: f32, y: f32) -> El {
    let id = port_id(node, body);
    kit::port(id, s.ui.hover(id), x, y)
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

/// What a step's card says, as the Workflow editor's template draws it: its
/// glyph, label and kind, and the start of what it runs or says.
fn card_content(s: &Screen<'_>, n: &Value) -> El {
    let Some(card) = super::editor_point(s).and_then(|p| match &p.template {
        Ok(Some(t)) => match &t.view.kind {
            Kind::Graph(g) => Some(&g.card),
            _ => None,
        },
        _ => None,
    }) else {
        return div();
    };
    let id = sv(n, "id").unwrap_or("");
    let footer = wf::footer(n);
    let code = matches!(
        sv(n, "type").unwrap_or(""),
        "script" | "condition" | "httpRequest" | "callConnectorAction"
    );
    let mut fields = serde_json::Map::new();
    fields.insert("title".into(), sv(n, "label").unwrap_or("").into());
    fields.insert("subtitle".into(), wf::subtitle(n).into());
    fields.insert("icon".into(), format!("node:{id}").into());
    match footer {
        Some(f) if code => {
            fields.insert("code".into(), f.into());
        }
        Some(f) => {
            fields.insert("note".into(), f.into());
        }
        None => {}
    }
    let row = Row {
        id: id.to_owned(),
        fields,
    };
    let bind = CardBind { s, n };
    tpl::node_view(s.ui, card, Some(&row), &NoRows, &bind, 1)
}

/// The rows a card draws from: only its own, given it directly.
struct NoRows;

impl Source for NoRows {
    fn rows(&self, _collection: &str) -> &[Row] {
        &[]
    }
}

/// A card's bindings: it has no controls; `node:<id>` is the step's own glyph.
struct CardBind<'a, 'b> {
    s: &'a Screen<'b>,
    n: &'a Value,
}

impl Bind for CardBind<'_, '_> {
    fn press(&self, slot: u64, _press: Press) -> Id {
        Id::new("wf-card-press").child(slot)
    }

    fn field(&self, slot: u64, _row: Option<&str>, _call: &str, _value: &str) -> Id {
        Id::new("wf-card-field").child(slot)
    }

    fn confirming(&self, _id: Id) -> Option<(Id, Id)> {
        None
    }

    fn menu(&self, slot: u64, _row: Option<&str>) -> Id {
        Id::new("wf-card-menu").child(slot)
    }

    fn menu_open(&self, _id: Id) -> bool {
        false
    }

    fn icon(&self, name: &str, size: f32) -> Option<El> {
        name.strip_prefix("node:")
            .map(|_| super::node_glyph(self.s, self.n, size.max(18.0)))
    }
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
    let draggable = e.draggable();
    let look = kit::CardLook {
        selected,
        failed: status == Some("error"),
        hover: if s.ui.hovered(cid) { 1.0 } else { 0.0 },
    };
    let mut card = kit::card(cid, label, look, card_content(s, n));
    if draggable {
        card = card.sense(Sense::MOVE);
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
        kit::side_button(s.ui, bid, icon, label, s.ui.hover(bid))
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

/// React Flow's `Controls`: zoom in, zoom out, fit, tidy up, on the kit's strip.
fn controls(s: &Screen<'_>, h: f32) -> El {
    let control = |a: WfAct, icon: &'static str, label: &'static str| {
        let id = s.acts.on("wf-controls", act(a));
        kit::Control {
            id,
            icon,
            label,
            hot: s.ui.hover(id),
        }
    };
    kit::controls(
        s.ui,
        &[
            control(WfAct::ZoomIn, "plus", "Zoom in"),
            control(WfAct::ZoomOut, "minus", "Zoom out"),
            control(WfAct::Fit, "maximize", "Fit the workflow"),
            control(WfAct::Tidy, "align-vertical-space-around", "Tidy up"),
        ],
        h,
    )
}
