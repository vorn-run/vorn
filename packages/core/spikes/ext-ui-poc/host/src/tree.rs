//! Model C's client: draws the element tree a UI component returns with the client's own vornui, and turns input into events by id.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use vornui::accesskit::Role as AkRole;
use vornui::backend::{Backend as _, Cells};
use vornui::element::Content;
use vornui::theme::{color, radius, rgb, text as size};
use vornui::widgets::{self, Tone as WTone};
use vornui::{div, text, Action, El, Event, Id, Laid, RenderMode, Rgba, Sense, Ui};

use crate::wasm::tree::{
    EventKind, Kind, Node, Paint, Prop, Radius, Role, Surface, TextSize, Token, Tone,
};
use crate::wasm::UiC;

fn token(t: Token) -> Rgba {
    match t {
        Token::Base => color::SURFACE_BASE,
        Token::Overlay => color::SURFACE_OVERLAY,
        Token::Ink => color::INK,
        Token::InkSecondary => color::INK_SECONDARY,
        Token::InkFaint => color::INK_FAINT,
        Token::Muted => color::GRAY_300,
        Token::Accent => color::BRONZO,
        Token::Good => color::GREEN_400,
        Token::Warn => color::YELLOW_500,
        Token::Bad => color::RED_400,
        Token::Info => color::STATUS_BLUE,
        Token::Neutral => color::GRAY_400,
        Token::White => color::WHITE,
    }
}

fn paint(p: Paint) -> Rgba {
    match p {
        Paint::Token(t) => token(t),
        Paint::Tint((t, a)) => token(t).alpha(a),
        Paint::Clear => Rgba::TRANSPARENT,
    }
}

fn tone(t: Tone) -> (Rgba, Rgba) {
    match t {
        Tone::Good => (color::GREEN_400, Rgba::hexa(rgb::GREEN_500, 0.15)),
        Tone::Warn => (color::YELLOW_500, Rgba::hexa(rgb::YELLOW_500, 0.15)),
        Tone::Bad => (color::RED_400, Rgba::hexa(rgb::RED_500, 0.15)),
        Tone::Info => (color::STATUS_BLUE, Rgba::white(0.08)),
        Tone::Neutral => (color::GRAY_400, Rgba::white(0.06)),
    }
}

/// Builds vornui elements from a tree, remembering which ids the component named.
#[derive(Default)]
pub struct Draw {
    names: HashMap<Id, String>,
    drags: Vec<Id>,
}

impl Draw {
    fn name(&mut self, s: &str) -> Id {
        let id = Id::new(s);
        self.names.insert(id, s.to_owned());
        id
    }

    pub fn tree(&mut self, ui: &Ui, nodes: &[Node]) -> El {
        self.names.clear();
        self.drags.clear();
        let mut i = 0;
        if nodes.is_empty() {
            return div();
        }
        self.node(ui, nodes, &mut i)
    }

    fn node(&mut self, ui: &Ui, nodes: &[Node], i: &mut usize) -> El {
        let n = &nodes[*i];
        *i += 1;
        let id = n.id.as_deref().map(|s| self.name(s));
        let kids: Vec<El> = (0..n.children).map(|_| self.node(ui, nodes, i)).collect();
        let anon = Id(0);
        let mut el = match &n.kind {
            Kind::Box => div().children(kids),
            Kind::Text((s, sz, ink)) => {
                let px = match sz {
                    TextSize::Xs => size::XS.0,
                    TextSize::Sm => size::SM.0,
                    TextSize::Base => size::BASE.0,
                    TextSize::Lg => size::LG.0,
                };
                text(s.clone(), px, paint(*ink))
            }
            Kind::Shape((svg, ink)) => div().content(Content::Svg {
                svg: svg.clone(),
                color: paint(*ink),
            })
            .shrink0(),
            Kind::Button(label) => widgets::button(ui, id.unwrap_or(anon), label),
            Kind::Field((name, ph)) => widgets::text_input(ui, id.unwrap_or(anon), name, ph),
            Kind::Badge((v, t)) => {
                let (fg, bg) = tone(*t);
                widgets::badge(v, fg, bg)
            }
            Kind::List(name) => widgets::list(id.unwrap_or(anon), name, kids),
        };
        if let (Some(id), Kind::Box | Kind::Text(_) | Kind::Shape(_)) = (id, &n.kind) {
            el = el.id(id);
        }
        let (mut bg, mut hover) = (None, None);
        for p in &n.props {
            el = match p {
                Prop::Row => el.row(),
                Prop::Col => el.col(),
                Prop::Gap(v) => el.gap(*v),
                Prop::Pad(v) => el.p(*v),
                Prop::PadX(v) => el.px(*v),
                Prop::PadY(v) => el.py(*v),
                Prop::W(v) => el.w(*v),
                Prop::H(v) => el.h(*v),
                Prop::WFull => el.w_full(),
                Prop::HFull => el.h_full(),
                Prop::Grow => el.grow(),
                Prop::MinH(v) => el.min_h(*v),
                Prop::ItemsCenter => el.items_center(),
                Prop::JustifyBetween => el.justify_between(),
                Prop::Bg(c) => {
                    bg = Some(paint(*c));
                    el
                }
                Prop::HoverBg(c) => {
                    hover = Some(paint(*c));
                    el
                }
                Prop::Border((w, c)) => el.border(*w, paint(*c)),
                Prop::Rounded(r) => el.rounded(match r {
                    Radius::Sm => radius::SM,
                    Radius::Md => radius::MD,
                    Radius::Lg => radius::LG,
                    Radius::Full => radius::FULL,
                }),
                Prop::Opacity(a) => el.opacity(*a),
                Prop::Weight(w) => el.weight(*w),
                Prop::Mono => el.mono(),
                Prop::Truncate => el.truncate(),
                Prop::Click => el.on(Action::Click),
                Prop::Drag => {
                    self.drags.extend(id);
                    el.sense(Sense::MOVE)
                }
                Prop::Disabled => el.disabled(true),
                Prop::Glyph(g) => el.glyph(g.as_str()),
                Prop::Access((r, name)) => el.role(
                    match r {
                        Role::Group => AkRole::Group,
                        Role::Image => AkRole::Image,
                        Role::Status => AkRole::Status,
                    },
                    name.as_str(),
                ),
                Prop::Confirm(c) => {
                    let base = n.id.as_deref().unwrap_or("confirm");
                    let ok = self.name(&format!("{base}.ok"));
                    let cancel = self.name(&format!("{base}.cancel"));
                    let t = if c.danger {
                        WTone::Danger
                    } else {
                        WTone::Neutral
                    };
                    widgets::confirm(ui, el, &c.message, (ok, &c.ok), cancel, t)
                }
            };
        }
        match (bg, hover, id) {
            (Some(b), Some(h), Some(id)) => el.bg(b.mix(h, ui.hover(id))),
            (Some(b), _, _) => el.bg(b),
            _ => el,
        }
    }

    /// The component's events for what vornui reported.
    pub fn events(&self, ui: &Ui, evs: Vec<Event>) -> Vec<(String, EventKind, String)> {
        let name = |id: &Id| self.names.get(id).cloned();
        let mut out = Vec::new();
        for e in evs {
            let ev = match e {
                Event::Click(id) => name(&id).map(|n| (n, EventKind::Click, String::new())),
                Event::Changed(id) => {
                    name(&id).map(|n| (n, EventKind::Change, ui.edit_text(id).to_owned()))
                }
                Event::Submit(id) => name(&id).map(|n| (n, EventKind::Submit, String::new())),
                Event::Move { id, x, y, .. } => {
                    let over = self.drags.iter().find(|d| {
                        ui.rect_of(**d).is_some_and(|r| {
                            y >= r.y && y < r.y + r.h && x >= r.x && x < r.x + r.w
                        })
                    });
                    match (name(&id), over.and_then(name)) {
                        (Some(n), Some(o)) => Some((n, EventKind::DragOver, o)),
                        _ => None,
                    }
                }
                _ => None,
            };
            out.extend(ev);
        }
        out
    }
}

/// The tree's size in a compact encoding: what crossing a process or network boundary would carry.
pub fn tree_bytes(nodes: &[Node]) -> usize {
    let p = |p: &Paint| if matches!(p, Paint::Tint(_)) { 6 } else { 2 };
    nodes
        .iter()
        .map(|n| {
            let kind = match &n.kind {
                Kind::Box => 1,
                Kind::Text((s, _, c)) => 2 + s.len() + p(c),
                Kind::Shape((s, c)) => 1 + s.len() + p(c),
                Kind::Button(s) | Kind::List(s) => 1 + s.len(),
                Kind::Field((a, b)) => 1 + a.len() + b.len(),
                Kind::Badge((s, _)) => 2 + s.len(),
            };
            let props: usize = n
                .props
                .iter()
                .map(|q| match q {
                    Prop::Gap(_)
                    | Prop::Pad(_)
                    | Prop::PadX(_)
                    | Prop::PadY(_)
                    | Prop::W(_)
                    | Prop::H(_)
                    | Prop::MinH(_)
                    | Prop::Opacity(_) => 5,
                    Prop::Bg(c) | Prop::HoverBg(c) => 1 + p(c),
                    Prop::Border((_, c)) => 5 + p(c),
                    Prop::Weight(_) => 3,
                    Prop::Rounded(_) => 2,
                    Prop::Glyph(s) => 2 + s.len(),
                    Prop::Access((_, s)) => 3 + s.len(),
                    Prop::Confirm(c) => 4 + c.message.len() + c.ok.len(),
                    _ => 1,
                })
                .sum();
            kind + 3 + n.id.as_ref().map_or(0, |s| 1 + s.len()) + props
        })
        .sum()
}

/// A client drawing model C: its Ui, the component, and the last tree it returned.
pub struct CClient {
    pub ui: Ui,
    pub comp: UiC,
    pub nodes: Vec<Node>,
    draw: Draw,
    dirty: bool,
    pub target: Option<vornui::Offscreen>,
    pub cells: Option<Cells>,
    t0: Instant,
    pub renders: Vec<f64>,
    pub laid: Option<Laid>,
}

impl CClient {
    pub fn new(mode: Option<RenderMode>, comp: UiC, cfg: &vornui::UiConfig) -> CClient {
        let (ui, cells) = match mode {
            Some(m) => (Ui::headless(m, cfg), None),
            None => (
                Ui::cells(cfg),
                Some(Cells::offscreen(crate::GRID.0, crate::GRID.1)),
            ),
        };
        CClient {
            ui,
            comp,
            nodes: Vec::new(),
            draw: Draw::default(),
            dirty: true,
            target: None,
            cells,
            t0: Instant::now(),
            renders: Vec::new(),
            laid: None,
        }
    }

    pub fn size(&self) -> (f32, f32) {
        if self.cells.is_some() {
            vornui::cells::units::px(crate::GRID.0, crate::GRID.1)
        } else {
            crate::LOGICAL
        }
    }

    pub fn data(&mut self, rows: &str) {
        self.comp.data(rows);
        self.dirty = true;
    }

    pub fn patch(&mut self, op: &str) {
        self.comp.patch(op);
        self.dirty = true;
    }

    /// Input goes to the client's Ui; what it means goes to the component by id.
    pub fn apply(&mut self, s: &crate::Step) {
        use crate::Step;
        match s {
            Step::Move(x, y) => self.ui.pointer_move(*x, *y),
            Step::Down => self.ui.pointer_down(),
            Step::Up => self.ui.pointer_up(),
            Step::Wheel(dy) => self.ui.wheel(*dy),
            Step::Key(k) => {
                self.ui.key(k);
            }
        }
        let evs = self.ui.take_events();
        for (id, kind, data) in self.draw.events(&self.ui, evs) {
            self.dirty |= self.comp.event(&id, kind, &data);
        }
    }

    pub fn frame(&mut self) {
        let size = self.size();
        if self.dirty {
            let t = Instant::now();
            self.nodes = self.comp.render(Surface {
                cells: self.cells.is_some(),
                width: size.0,
                height: size.1,
            });
            self.renders.push(t.elapsed().as_secs_f64() * 1e3);
            self.dirty = false;
        }
        self.ui
            .begin_at(color::SURFACE_BASE, self.t0 + Duration::from_secs(3600));
        let root = self.draw.tree(&self.ui, &self.nodes);
        self.laid = Some(self.ui.layout(root, size));
        if let Some(c) = &mut self.cells {
            c.present(&mut self.ui).expect("cells");
        } else {
            let px = (
                (crate::LOGICAL.0 * crate::SCALE) as u32,
                (crate::LOGICAL.1 * crate::SCALE) as u32,
            );
            if self.target.is_none() {
                self.target = Some(self.ui.offscreen(px));
            }
            self.ui.render_offscreen(self.target.as_ref().unwrap());
        }
    }

    pub fn shoot(&mut self, path: &str) {
        if let Some(c) = &self.cells {
            std::fs::write(path, c.buffer.to_html("model C on Cells")).expect("html");
        } else {
            let t = self.target.as_ref().unwrap();
            let px = self.ui.read(t).expect("read");
            vornui::write_png(path, t.size, &px).expect("png");
        }
    }
}
