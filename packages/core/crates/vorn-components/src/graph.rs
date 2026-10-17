//! A graph drawn by the kit: cards on a canvas joined by edges, under a pan
//! and zoom, over a dot grid, with zoom controls and a minimap. The workflow
//! editor draws on these pieces, with its own layout and its own controls
//! beside the cards.
//!
//! What a click, a drag or a wheel does is the client's: the kit gives each
//! control an [`Id`] and draws what the client's state says.

use vornui::theme::{color, rgb};
use vornui::widgets::{tooltip, tooltip_at};
use vornui::{div, shape, text, Action, Cursor, El, Id, Place, Rgba, Sense};

use vornui::accesskit::Role;

/// The farthest a canvas zooms out.
pub const MIN_ZOOM: f64 = 0.2;
/// The farthest a canvas zooms in.
pub const MAX_ZOOM: f64 = 1.75;
/// The round + button's size.
pub const PLUS: f64 = 22.0;
/// A card's width.
pub const CARD_WIDTH: f64 = 280.0;
/// The minimap's size.
pub const MINI: (f64, f64) = (96.0, 64.0);
/// An edge this close to vertical is drawn straight.
const STRAIGHT: f64 = 16.0;

fn white(a: f32) -> Rgba {
    Rgba::white(a)
}

fn f(v: f64) -> f32 {
    v as f32
}

/// A pan and zoom: flow point `p` draws at `p * zoom + (x, y)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}

/// A zoom step by `factor` about `center`, held between the canvas's limits.
pub fn zoom_by(v: Viewport, factor: f64, center: (f64, f64)) -> Viewport {
    let zoom = (v.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
    let k = zoom / v.zoom;
    Viewport {
        x: center.0 - (center.0 - v.x) * k,
        y: center.1 - (center.1 - v.y) * k,
        zoom,
    }
}

/// The view that shows `bounds` (`x, y, w, h` in flow units) in a pane of
/// `size`, with `pad` around it, no closer than 100%.
pub fn fit((bx, by, bw, bh): (f64, f64, f64, f64), size: (f64, f64), pad: f64) -> Viewport {
    let zoom = ((size.0 - 2.0 * pad) / bw.max(1.0))
        .min((size.1 - 2.0 * pad) / bh.max(1.0))
        .clamp(MIN_ZOOM, 1.0);
    Viewport {
        x: (size.0 - bw * zoom) / 2.0 - bx * zoom,
        y: (size.1 - bh * zoom) / 2.0 - by * zoom,
        zoom,
    }
}

/// A bezier's control offset at curvature 0.25: half the distance ahead, a
/// root-scaled reach behind.
fn control_offset(distance: f64) -> f64 {
    if distance >= 0.0 {
        0.5 * distance
    } else {
        0.25 * 25.0 * (-distance).sqrt()
    }
}

/// An edge from `from` to `to`: straight down a column, a bezier only for a
/// real fork. Its four control points (equal to the ends when straight) and
/// where its label sits.
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

/// What the minimap draws, in its own pixels.
#[derive(Debug, Clone)]
pub struct Minimap {
    /// Flow units per minimap pixel.
    pub scale: f64,
    pub nodes: Vec<(f64, f64, f64, f64)>,
    pub view: (f64, f64, f64, f64),
}

/// Every card (`x, y, w, h`) and the visible area of a pane of `size`,
/// fitted with a margin into [`MINI`].
pub fn minimap(cards: &[(f64, f64, f64, f64)], v: Viewport, (w, h): (f64, f64)) -> Minimap {
    let vr = (-v.x / v.zoom, -v.y / v.zoom, w / v.zoom, h / v.zoom);
    let mut b = vr;
    for c in cards {
        let (x0, y0) = (b.0.min(c.0), b.1.min(c.1));
        let (x1, y1) = ((b.0 + b.2).max(c.0 + c.2), (b.1 + b.3).max(c.1 + c.3));
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
        nodes: cards.iter().copied().map(map).collect(),
        view: map(vr),
    }
}

/// The dot grid: 20 apart at 100%, one pixel, faint white.
pub fn dots(v: Viewport, (w, h): (f32, f32)) -> El {
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

/// An edge's line from `from` to `to`, in flow units.
pub fn edge(from: (f64, f64), to: (f64, f64)) -> Option<El> {
    let (pts, _) = edge_curve(from, to);
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

/// An edge's label: a small pill, centred on the edge by the caller.
pub fn edge_label(label: &str) -> El {
    div()
        .px(10.0)
        .py(2.0)
        .rounded(10.0)
        .border(1.0, white(0.08))
        .bg(color::SURFACE_BASE)
        .child(
            text(label, 10.0, color::INK_SECONDARY)
                .weight(600)
                .line_height(15.0)
                .silent(),
        )
}

/// The round + that adds something where it sits; `active` while what it
/// opened is open, `hot` while hovered.
pub fn plus(id: Id, label: &str, active: bool, hot: f32) -> El {
    let (fg, bg, border) = if active {
        (color::WHITE, white(0.08), white(0.40))
    } else {
        (
            color::GRAY_500.mix(color::WHITE, hot),
            color::SURFACE_SUNKEN,
            white(0.10).mix(white(0.20), hot),
        )
    };
    div()
        .id(id)
        .on(Action::Click)
        .size(f(PLUS), f(PLUS))
        .rounded(11.0)
        .center()
        .bg(bg)
        .border(1.0, border)
        .role(Role::Button, label)
        .child(crate::lucide("plus", 13.0, 2.5, fg))
}

/// An input port: 7 px, ringed, at `(x, y)` of its card.
pub fn handle(x: f32, y: f32) -> El {
    div()
        .absolute(x - 3.5, y - 3.5)
        .size(7.0, 7.0)
        .rounded(3.5)
        .bg(color::SURFACE_BASE)
        .border(1.0, white(0.35))
}

/// An output port a connection is drawn from, at `(x, y)` of its card.
pub fn port(id: Id, hot: f32, x: f32, y: f32) -> El {
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
                .bg(color::SURFACE_BASE)
                .border(1.0, white(0.35).mix(white(0.7), hot)),
        )
}

/// How a card looks: chosen, failed, under the pointer.
#[derive(Debug, Clone, Copy, Default)]
pub struct CardLook {
    pub selected: bool,
    pub failed: bool,
    pub hover: f32,
}

/// A card's frame, `CARD_WIDTH` wide, holding `content`.
pub fn card(id: Id, label: &str, look: CardLook, content: El) -> El {
    let border = if look.selected {
        white(0.40)
    } else if look.failed {
        Rgba::hexa(rgb::DANGER, 0.6)
    } else {
        white(0.08)
    };
    div()
        .id(id)
        .on(Action::Click)
        .col()
        .w(f(CARD_WIDTH))
        .px(12.0)
        .py(10.0)
        .rounded(6.0)
        .border(1.0, border)
        .bg(color::SURFACE_SUNKEN.mix(white(0.02), look.hover))
        .role(Role::Button, label)
        .selected(look.selected)
        .child(content)
}

/// One control of the canvas's control strip.
#[derive(Debug, Clone)]
pub struct Control<'a> {
    pub id: Id,
    pub icon: &'a str,
    pub label: &'a str,
    pub hot: f32,
}

/// The control strip, bottom left of a pane `h` tall: zoom in and out, fit,
/// and whatever else the client adds.
pub fn controls(ui: &vornui::Ui, controls: &[Control<'_>], h: f32) -> El {
    let n = controls.len();
    let mut strip = div()
        .absolute(15.0, h - 15.0 - (n as f32 * 26.0 + (n as f32 - 1.0) + 2.0))
        .col()
        .rounded(6.0)
        .border(1.0, white(0.12))
        .bg(color::SURFACE_OVERLAY)
        .clip()
        .role(Role::Toolbar, "Canvas controls");
    for (i, c) in controls.iter().enumerate() {
        let mut b = div()
            .id(c.id)
            .on(Action::Click)
            .size(26.0, 26.0)
            .center()
            .bg(white(0.0).mix(white(0.06), c.hot))
            .role(Role::Button, c.label)
            .child(if matches!(c.icon, "plus" | "minus") {
                crate::lucide(c.icon, 14.0, 2.75, color::GRAY_200.mix(color::WHITE, c.hot))
            } else {
                crate::lucide(c.icon, 12.0, 2.0, color::GRAY_400.mix(color::WHITE, c.hot))
            });
        if i + 1 < n {
            b = b.border_bottom(1.0, white(0.08));
        }
        strip = strip.child(tooltip(ui, b, c.id, c.label, None));
    }
    strip
}

/// A button beside a card that shows on hover, its tooltip to the right.
pub fn side_button(ui: &vornui::Ui, id: Id, icon: &str, label: &str, hot: f32) -> El {
    let b = div()
        .id(id)
        .on(Action::Click)
        .p(6.0)
        .rounded(6.0)
        .bg(color::SURFACE_OVERLAY)
        .border(1.0, white(0.12).mix(white(0.20), hot))
        .role(Role::Button, label)
        .child(crate::lucide(
            icon,
            12.0,
            2.0,
            color::GRAY_400.mix(color::WHITE, hot),
        ));
    tooltip_at(ui, b, id, label, None, Place::Right)
}

/// The minimap, 96×64, top right of a pane `h` tall: its cards and the visible area.
pub fn minimap_view(id: Id, m: &Minimap, h: f32) -> El {
    let (mw, mh) = (f(MINI.0), f(MINI.1));
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
        .id(id)
        .sense(Sense::MOVE)
        .absolute_right(15.0, h - mh - 15.0 - 2.0)
        .size(mw + 2.0, mh + 2.0)
        .p(1.0)
        .rounded(4.0)
        .border(1.0, white(0.12))
        .bg(color::SURFACE_PANEL)
        .clip()
        .role(Role::Image, "Mini map")
        .child(shape(nodes, mw, mh, white(0.25)).absolute(1.0, 1.0))
        .child(shape(mask, mw, mh, Rgba::hexa(0x000000, 0.55)).absolute(1.0, 1.0))
}

/// The canvas pane `size` big at `view`: the dot grid under `world` (cards
/// and edges in flow units), with what floats over it.
/// The part of the world a surface of `size` shows at `view`, as `(x0, y0, x1, y1)`,
/// widened by `margin` screen points, so what lies outside it need not be drawn.
pub fn visible(view: Viewport, size: (f32, f32), margin: f64) -> (f64, f64, f64, f64) {
    // Scaled about the box's centre and shifted as `surface` does: a world point lands at `p * zoom + view`.
    let (w, h) = (f64::from(size.0), f64::from(size.1));
    let z = view.zoom.max(f64::EPSILON);
    (
        (-margin - view.x) / z,
        (-margin - view.y) / z,
        (w + margin - view.x) / z,
        (h + margin - view.y) / z,
    )
}

pub fn surface(id: Id, size: (f32, f32), view: Viewport, world: Vec<El>, over: Vec<El>) -> El {
    let (w, h) = size;
    let z = f(view.zoom);
    let center = (w / 2.0, h / 2.0);
    let offset = (
        f(view.x) - center.0 * (1.0 - z),
        f(view.y) - center.1 * (1.0 - z),
    );
    let mut flow = div().absolute(0.0, 0.0).size(w, h).transform(z, offset);
    for el in world {
        flow = flow.child(el);
    }
    let mut pane = div()
        .id(id)
        .on(Action::Click)
        .sense(Sense::MOVE | Sense::WHEEL)
        .grow()
        .min_w0()
        .min_h(0.0)
        .h_full()
        .clip()
        .bg(color::SURFACE_BASE)
        .role(Role::Group, "Canvas")
        .child(dots(view, size))
        .child(flow);
    for el in over {
        pane = pane.child(el);
    }
    pane
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_visible_world_is_the_pane_undone_by_the_view() {
        let view = Viewport {
            x: 100.0,
            y: -50.0,
            zoom: 2.0,
        };
        assert_eq!(
            visible(view, (400.0, 300.0), 0.0),
            (-50.0, 25.0, 150.0, 175.0)
        );
        assert_eq!(
            visible(Viewport::default(), (400.0, 300.0), 10.0),
            (-10.0, -10.0, 410.0, 310.0)
        );
    }

    #[test]
    fn a_fit_shows_everything_no_closer_than_full_size() {
        let v = fit((0.0, 0.0, 280.0, 100.0), (800.0, 600.0), 40.0);
        assert_eq!(v.zoom, 1.0);
        let small = fit((0.0, 0.0, 2800.0, 4000.0), (800.0, 600.0), 40.0);
        assert!(small.zoom < 1.0 && small.zoom >= MIN_ZOOM);
    }

    #[test]
    fn a_straight_edge_stays_straight() {
        let (pts, mid) = edge_curve((0.0, 0.0), (5.0, 100.0));
        assert_eq!(pts[1], pts[0]);
        assert_eq!(mid, (2.5, 50.0));
    }
}
