//! An extension's element tree ([`vorn_tree`]) drawn with the kit on pixels and cells; the caller binds ids.

use vorn_tree::{
    ButtonStyle, Cards, Entry, Event, EventKind, Kind, Meter, Node, Paint, Props, TextStyle, Token,
    Tone,
};
use vornui::accesskit::Role;
use vornui::native::Sem;
use vornui::theme::{color, radius, text as size};
use vornui::widgets::{self, Tone as ConfirmTone};
use vornui::{div, text, Action, El, Id, Place, Rgba, Sense, Ui};

use crate::board::{Board, Lane};

/// How wide a `number` text's column is, so figures line up down a list.
pub const NUMBER_WIDTH: f32 = 80.0;

/// What a control of the tree does, for the caller to bind.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Ev {
    /// Tells the extension `event`; `ask` when the control asks first.
    Send { event: Event, ask: bool },
    /// A one-line field holding `value`; Return sends it as `submit`.
    Field { node: String, value: String },
    /// Checks rows of a list, or unchecks them.
    Check {
        list: String,
        rows: Vec<String>,
        on: bool,
        /// The list's Select all, apart from a one-row list's own box.
        all: bool,
    },
    /// Opens an entry's more menu, or closes it.
    Menu { node: String },
    /// An entry's row, which shows its actions while hovered.
    Hover { node: String },
    /// A lazy list's scroll view.
    Scroll { list: String },
    /// A field that reports `change` as it is typed, holding `value`.
    Live { node: String, value: String },
    /// A split's divider, dragged to a ratio the client reports as `resize`.
    Split { node: String },
    /// A second control that tells `event`, kept apart from the first by `slot`.
    Item { event: Event, slot: String },
}

/// How the caller binds a tree's controls.
pub trait TreeBind {
    /// The id the control doing `ev` takes.
    fn on(&self, ev: Ev) -> Id;
    /// The confirm's buttons when button `id` asks before it runs and is asking now.
    fn confirming(&self, id: Id) -> Option<(Id, Id)>;
    /// Whether the more menu of button `id` is open.
    fn menu_open(&self, id: Id) -> bool;
    /// Whether row `row` of list `list` is checked.
    fn checked(&self, list: &str, row: &str) -> bool;
    /// An icon the client draws for a name not in the kit's set; `None` draws the kit's.
    fn icon(&self, name: &str, size: f32) -> Option<El> {
        let _ = (name, size);
        None
    }
    /// A board whose cards the client reads itself, drawn with the kit's
    /// [`Board`] over the client's cards; `None` draws its columns empty.
    fn board(&self, cards: Cards, lanes: &[Lane<'_>], empty: Option<&str>) -> Option<El> {
        let _ = (cards, lanes, empty);
        None
    }
    /// The workflow editor's canvas, which the client draws; `None` draws nothing.
    fn graph(&self) -> Option<El> {
        None
    }
    /// Control `id` also tells the extension `event` on a double click.
    fn double(&self, id: Id, event: Event) {
        let _ = (id, event);
    }
    /// Split `node`'s ratio while its divider is being dragged.
    fn dragging(&self, node: &str) -> Option<f32> {
        let _ = node;
        None
    }
    /// The client's own controls at the end of tab strip `node`, as a pane's.
    fn chrome(&self, node: &str) -> Option<El> {
        let _ = node;
        None
    }
    /// A file's text to edit, which the client draws and holds; the button nodes `buttons` end its strip.
    fn code(&self, node: &str, code: &vorn_tree::Code, buttons: &[Node]) -> Option<El> {
        let _ = (node, code, buttons);
        None
    }
    /// A shell's commands over its live rows, which the client cuts from its screen.
    fn log(&self, log: &vorn_tree::Log) -> Option<El> {
        let _ = log;
        None
    }
}

/// A lazy list's height: it scrolls on its own, building the rows in view.
pub const LAZY_HEIGHT: f32 = 480.0;
/// How a meter's segments draw on cells, in order; the track is `░`.
const BLOCKS: [&str; 3] = ["█", "▓", "▒"];
const METER_CELLS: usize = 40;
const ENTRY_METER_CELLS: usize = 8;

/// The colour token `t` stands for, in token set [`vorn_tree::TOKENS`].
pub fn token(t: Token) -> Rgba {
    match t {
        Token::Base => color::SURFACE_BASE,
        Token::Sunken => color::SURFACE_SUNKEN,
        Token::Raised => Rgba::white(0.02),
        Token::Overlay => color::SURFACE_OVERLAY,
        Token::Line => Rgba::white(0.08),
        Token::Ink | Token::Unknown => color::INK,
        Token::InkSecondary => color::INK_SECONDARY,
        Token::InkFaint => color::INK_FAINT,
        Token::Accent => color::STATUS_BLUE,
        Token::Good => color::GREEN_400,
        Token::Warn => color::AMBER_400,
        Token::Bad => color::RED_400,
        Token::Info => color::SKY_400,
        Token::White => color::WHITE,
    }
}

pub fn paint(p: Paint) -> Rgba {
    match p {
        Paint::Token(t) => token(t),
        Paint::Tint { token: t, alpha } => {
            let c = token(t);
            c.alpha(c.0[3] * alpha.clamp(0.0, 1.0))
        }
    }
}

/// The colour a tone draws in.
pub fn tone(t: Tone) -> Rgba {
    match t {
        Tone::Neutral => color::GRAY_300,
        Tone::Good => color::GREEN_400,
        Tone::Warn => color::AMBER_400,
        Tone::Bad => color::RED_400,
        Tone::Info => color::SKY_400,
        Tone::Accent => color::STATUS_BLUE,
        Tone::Busy => color::BRONZO,
        Tone::Muted => color::INK_FAINT,
        Tone::Idle => color::STATUS_SLATE,
    }
}

/// Tree `tree` drawn whole.
pub fn view(ui: &Ui, tree: &Node, bind: &dyn TreeBind) -> El {
    Cx { ui, bind }.node(tree)
}

/// Tree `tree` as a page: its toolbar, for the page's header, and the rest.
pub fn page(ui: &Ui, tree: &Node, bind: &dyn TreeBind) -> (Option<El>, Option<El>) {
    let cx = Cx { ui, bind };
    if matches!(tree.kind, Kind::Toolbar) {
        return (Some(cx.node(tree)), None);
    }
    let at = matches!(tree.kind, Kind::Box)
        .then(|| (tree.children.iter()).position(|k| matches!(k.kind, Kind::Toolbar)))
        .flatten();
    let Some(at) = at else {
        return (None, Some(cx.node(tree)));
    };
    let bar = cx.node(&tree.children[at]);
    let rest = tree.children.iter().enumerate().filter(|(i, _)| *i != at);
    let body = cx.boxed(&tree.props, rest.map(|(_, k)| cx.node(k)));
    (Some(bar), Some(body))
}

struct Cx<'a> {
    ui: &'a Ui,
    bind: &'a dyn TreeBind,
}

/// A list being drawn: its id, whether it checks rows, and those checked.
struct Listing<'a> {
    id: &'a str,
    select: bool,
    checked: Vec<String>,
}

impl Cx<'_> {
    /// `el` tagged for a native backend; other surfaces skip building the tag.
    fn sem(&self, el: El, s: impl FnOnce() -> Sem) -> El {
        if self.ui.is_native() {
            el.sem(s())
        } else {
            el
        }
    }

    fn node(&self, n: &Node) -> El {
        let el = match &n.kind {
            Kind::Box => return self.boxed(&n.props, n.children.iter().map(|k| self.node(k))),
            Kind::Text {
                text: t,
                style,
                tone: tn,
                wrap,
            } => {
                let mut e = styled(t, *style, tn.map(tone));
                e = if *wrap { e.wrapping() } else { e.truncate() };
                let e = self.sem(e, || Sem::Text(text_role(*style, *tn)));
                match style {
                    TextStyle::Number => {
                        div().row().justify_end().w(NUMBER_WIDTH).shrink0().child(e)
                    }
                    TextStyle::Heading => e.role(Role::Heading, t.as_str()),
                    // Growing, it takes the room left and still cuts short in it.
                    _ if n.props.grow => div().min_w0().child(e),
                    _ if *wrap => e,
                    _ => e.shrink0(),
                }
            }
            Kind::Icon { name, tone: tn } => {
                let e = self.icon(name, 14.0, tn.map_or(color::GRAY_400, tone));
                self.sem(e, || Sem::Icon {
                    name: name.clone(),
                    tone: tn.map_or("", sem_tone),
                    svg: None,
                })
            }
            Kind::Shape {
                svg,
                glyph,
                w,
                h,
                ink,
            } => {
                let ink = ink.map_or(color::GRAY_300, paint);
                let name = n.props.label.as_deref().unwrap_or(glyph.as_str());
                let e = vornui::shape(svg.as_str(), *w, *h, ink)
                    .glyph(glyph.as_str())
                    .glyph_tint(ink)
                    .shrink0()
                    .role(Role::Image, name);
                self.sem(e, || Sem::Icon {
                    name: name.to_owned(),
                    tone: "",
                    svg: Some(svg.clone()),
                })
            }
            Kind::Button {
                label,
                icon,
                style,
                confirm,
            } => {
                let id = n.id.as_deref().unwrap_or_default();
                let name = n.props.label.as_deref().unwrap_or(label);
                let ev = send(id, EventKind::Click, confirm.is_some());
                self.button(ev, label, name, icon.as_deref(), *style, confirm.as_deref())
                    .1
            }
            Kind::Field {
                value,
                placeholder,
                secret,
                live,
            } => {
                let node = n.id.clone().unwrap_or_default();
                let name = n.props.label.as_deref().unwrap_or(placeholder);
                let value = value.clone();
                let id = self.bind.on(if *live {
                    Ev::Live { node, value }
                } else {
                    Ev::Field { node, value }
                });
                let input = widgets::text_input(self.ui, id, name, placeholder)
                    .grow()
                    .min_w0();
                let input = if *secret { input.secure() } else { input };
                self.sem(input, || Sem::Field(name.to_owned()))
            }
            Kind::Choice { options } => self.choice(n, options),
            Kind::Badge(b) => self.sem(badge(&b.text, b.tone), || Sem::Badge {
                value: b.text.clone(),
                tone: sem_tone(b.tone),
            }),
            Kind::Dot { tone: t } => {
                let ink = tone(*t);
                let d = if self.ui.is_cells() {
                    text("●", 12.0, ink).line_height(16.0).silent()
                } else {
                    div().size(6.0, 6.0).rounded(3.0).bg(ink)
                };
                let name = n.props.label.as_deref().unwrap_or(tone_name(*t));
                self.sem(d.shrink0().role(Role::Status, name), || Sem::Status {
                    value: name.to_owned(),
                    tone: sem_tone(*t),
                })
            }
            Kind::Toggle { label, on } => {
                let node = n.id.clone().unwrap_or_default();
                let event = Event {
                    id: node,
                    kind: EventKind::Toggle,
                    value: (!on).to_string(),
                    selected: Vec::new(),
                    scope: None,
                    width: None,
                };
                let id = self.bind.on(Ev::Send { event, ask: false });
                let name = n.props.label.as_deref().unwrap_or(label);
                let switch = widgets::toggle(self.ui, id, name, *on, false).shrink0();
                if label.is_empty() {
                    switch
                } else {
                    let words = styled(label, TextStyle::Body, None).truncate();
                    div()
                        .row()
                        .items_center()
                        .gap(8.0)
                        .min_w0()
                        .child(div().grow().min_w0().child(self.sem(words, || Sem::Hidden)))
                        .child(switch)
                }
            }
            Kind::Notice { text: t, tone: tn } => {
                self.sem(notice(t, *tn), || Sem::Notice(tn.map_or("", sem_tone)))
            }
            Kind::Stat {
                value,
                label,
                icon,
                tone: tn,
                end,
            } => self.stat(value, label.as_deref(), icon.as_deref(), *tn, *end),
            Kind::Meter(m) => self.meter(m, false).unwrap_or_else(div),
            Kind::Section { title } => div()
                .sem(Sem::Section(title.clone()))
                .col()
                .gap(12.0)
                .min_w0()
                .child(
                    text(title.to_uppercase(), size::XS.0, color::GRAY_500)
                        .line_height(size::XS.1)
                        .weight(size::MEDIUM)
                        .tracking(0.05)
                        .truncate()
                        .role(Role::Heading, title.as_str())
                        .sem(Sem::Hidden),
                )
                .children(n.children.iter().map(|k| self.node(k))),
            Kind::Toolbar => div()
                .sem(Sem::Toolbar)
                .row()
                .items_center()
                .justify_end()
                .gap(8.0)
                .shrink0()
                .role(Role::Toolbar, n.props.label.as_deref().unwrap_or("Actions"))
                .children(n.children.iter().map(|k| self.node(k))),
            Kind::List {
                empty,
                loading,
                select,
                bulk,
                lazy,
            } => self.list(
                n,
                empty.as_deref(),
                loading.as_deref(),
                *select,
                bulk,
                *lazy,
            ),
            Kind::Group { .. } => {
                let l = Listing {
                    id: "",
                    select: false,
                    checked: Vec::new(),
                };
                self.group(n, &l)
            }
            Kind::Entry(e) => self.entry(n, e, None),
            Kind::Board {
                cards: Some(cards),
                empty,
            } => {
                let lanes: Vec<Lane<'_>> = n
                    .children
                    .iter()
                    .filter_map(|c| match &c.kind {
                        Kind::Column { title, tone, icon } => Some(Lane {
                            value: c.id.as_deref().unwrap_or_default(),
                            title,
                            icon: icon.as_deref(),
                            tone: *tone,
                        }),
                        _ => None,
                    })
                    .collect();
                match self.bind.board(*cards, &lanes, empty.as_deref()) {
                    Some(drawn) => drawn,
                    None => Board {
                        ui: self.ui,
                        name: "tree",
                        lanes: &lanes,
                        empty: empty.as_deref(),
                        cards: &Unread,
                        bind: &Unbound,
                    }
                    .view(),
                }
            }
            Kind::Board { cards: None, .. } => div()
                .row()
                .items_start()
                .gap(12.0)
                .min_w0()
                .children(n.children.iter().map(|c| self.column(c))),
            Kind::Graph => self.bind.graph().unwrap_or_else(|| div().sem(Sem::Hidden)),
            Kind::Column { .. } => self.column(n),
            Kind::Divider => div()
                .w_full()
                .h(0.0)
                .border_top(1.0, Rgba::white(0.08))
                .sem(Sem::Divider),
            Kind::Tabs(t) => self.tabs(key(n), t),
            Kind::Outline(o) => self.outline(key(n), o),
            Kind::Split {
                ratio,
                label,
                narrow,
            } => self.split(n, *ratio, label, *narrow),
            Kind::Code(c) => self
                .bind
                .code(key(n), c, &n.children)
                .unwrap_or_else(|| parts::plain_code(c)),
            Kind::Log(l) => self.bind.log(l).unwrap_or_else(|| div().sem(Sem::Hidden)),
        };
        sized(el, &n.props)
    }

    /// A plain box laid out by its props.
    fn boxed(&self, p: &Props, kids: impl Iterator<Item = El>) -> El {
        let mut b = if p.row {
            div().row().min_w0()
        } else {
            div().col().min_w0()
        };
        if p.center {
            b = b.items_center();
        }
        if p.between {
            b = b.justify_between();
        }
        if p.wrap {
            b = b.wrap_children();
        }
        if let Some(bg) = p.bg {
            b = b.bg(paint(bg));
        }
        if let Some(edge) = p.border {
            b = b.border(1.0, paint(edge));
        }
        if p.rounded {
            b = b.rounded(radius::SM);
        }
        if let Some(l) = &p.label {
            b = b.role(Role::Group, l.as_str());
        }
        b = self.sem(b, || if p.row { Sem::Row } else { Sem::Column });
        sized(b.gap(p.gap.unwrap_or(0.0)).children(kids), p)
    }

    fn icon(&self, name: &str, px: f32, ink: Rgba) -> El {
        let e = match self.bind.icon(name, px) {
            Some(drawn) => drawn.shrink0(),
            None => crate::lucide(name, px, 2.0, ink).shrink0(),
        };
        self.sem(e, || Sem::icon(name))
    }

    /// A button and its id; with `confirm`, it asks inline before it counts.
    fn button(
        &self,
        ev: Ev,
        label: &str,
        name: &str,
        icon: Option<&str>,
        style: ButtonStyle,
        confirm: Option<&str>,
    ) -> (Id, El) {
        let id = self.bind.on(ev);
        let hot = self.ui.hover(id);
        let (rest, lit) = match style {
            ButtonStyle::Danger => (color::RED_400, color::RED_300),
            ButtonStyle::Primary => (color::WHITE, color::WHITE),
            ButtonStyle::Plain => (color::GRAY_300, color::WHITE),
        };
        let ink = rest.mix(lit, hot);
        let fill = match style {
            ButtonStyle::Primary => Rgba::white(0.10).mix(Rgba::white(0.15), hot),
            _ => Rgba::TRANSPARENT.mix(Rgba::white(0.06), hot),
        };
        let b = div()
            .id(id)
            .on(Action::Click)
            .row()
            .items_center()
            .gap(4.0)
            .px(if label.is_empty() { 4.0 } else { 8.0 })
            .py(4.0)
            .shrink0()
            .rounded(radius::SM)
            .bg(fill)
            .ring(radius::SM)
            .role(Role::Button, name);
        let mut b = self.sem(b, || Sem::Button {
            style: sem_style(style),
            confirm: confirm.map(str::to_owned),
            icon: icon.map(str::to_owned),
        });
        if let Some(i) = icon {
            b = b.child(self.icon(i, 12.0, ink));
        }
        if !label.is_empty() {
            b = b.child(
                text(label, size::XS.0, ink)
                    .line_height(size::XS.1)
                    .weight(size::MEDIUM)
                    .silent(),
            );
        }
        let b = match (confirm, self.bind.confirming(id)) {
            (Some(message), Some((ok, cancel))) => {
                let shown = if label.is_empty() { name } else { label };
                widgets::confirm(self.ui, b, message, (ok, shown), cancel, ask_tone(style))
            }
            _ => b,
        };
        (id, b)
    }

    /// Each option as a chip, the chosen one filled; one press per option.
    fn choice(&self, n: &Node, options: &[vorn_tree::Choose]) -> El {
        let node = n.id.as_deref().unwrap_or_default();
        let name = n.props.label.as_deref().unwrap_or(node);
        let line = div()
            .row()
            .wrap_children()
            .gap(4.0)
            .min_w0()
            .role(Role::RadioGroup, name);
        let line = self.sem(line, || Sem::Choice);
        line.children(options.iter().map(|o| {
            let event = Event::new(node, EventKind::Select, o.id.clone());
            let id = self.bind.on(Ev::Send { event, ask: false });
            let hot = self.ui.hover(id);
            let on = o.selected;
            let ink = if on {
                color::WHITE
            } else {
                color::GRAY_400.mix(color::WHITE, hot)
            };
            div()
                .id(id)
                .on(Action::Click)
                .px(8.0)
                .py(3.0)
                .rounded(radius::SM)
                .border(1.0, Rgba::white(if on { 0.25 } else { 0.10 }))
                .bg(if on {
                    Rgba::white(0.14)
                } else {
                    Rgba::TRANSPARENT.mix(Rgba::white(0.06), hot)
                })
                .ring(radius::SM)
                .role(Role::RadioButton, o.label.as_str())
                .toggled(on)
                .child(
                    text(&o.label, size::XS.0, ink)
                        .line_height(size::XS.1)
                        .silent(),
                )
        }))
    }

    /// A figure under its quiet label, as a page's total; `end` at the right, smaller.
    fn stat(
        &self,
        value: &str,
        label: Option<&str>,
        icon: Option<&str>,
        t: Option<Tone>,
        end: bool,
    ) -> El {
        let label = label.filter(|l| !l.is_empty());
        if self.ui.is_native() {
            return div()
                .sem(Sem::Entry {
                    title: label.unwrap_or(value).to_owned(),
                    detail: String::new(),
                })
                .child_if(icon.map(|i| self.icon(i, 12.0, color::GRAY_500)))
                .child_if(label.map(|_| {
                    let e = text(value, size::XS.0, color::GRAY_100);
                    e.sem(Sem::Text(text_role(TextStyle::Number, t)))
                }));
        }
        let said = label.map_or_else(|| value.to_owned(), |l| format!("{l}: {value}"));
        let label = label.map(|l| {
            div()
                .row()
                .items_center()
                .gap(8.0)
                .min_w0()
                .child_if(icon.map(|i| self.icon(i, 12.0, color::GRAY_500)))
                .child(
                    text(l.to_uppercase(), 11.0, color::GRAY_500)
                        .line_height(16.0)
                        .weight(size::MEDIUM)
                        .tracking(0.05)
                        .truncate()
                        .silent(),
                )
        });
        let ink = t.map_or(color::GRAY_100, tone);
        let big = if end { size::XL } else { size::XL2 };
        let e = div()
            .col()
            .gap(4.0)
            .min_w0()
            .role(Role::Label, said.as_str())
            .child_if(label)
            .child(
                text(value, big.0, ink)
                    .line_height(big.1)
                    .weight(size::SEMIBOLD)
                    .truncate()
                    .silent(),
            );
        match (end, self.ui.is_cells()) {
            // A terminal's narrow page cuts the long label, not the page's total.
            (true, true) => e.items_end(),
            (true, false) => e.items_end().shrink0(),
            (false, true) => e.shrink0(),
            (false, false) => e.grow(),
        }
    }

    /// A meter's bar and its legend when labelled; in an entry a short bar, or nothing when empty.
    fn meter(&self, m: &Meter, short: bool) -> Option<El> {
        let max = if m.max.is_finite() && m.max > 0.0 {
            m.max
        } else {
            0.0
        };
        let mut seen: Vec<Rgba> = Vec::new();
        let parts: Vec<(f32, Rgba, &str)> = m
            .segments
            .iter()
            .map(|s| {
                let share = if max > 0.0 && s.value.is_finite() {
                    (s.value / max).clamp(0.0, 1.0) as f32
                } else {
                    0.0
                };
                let base = tone(s.tone);
                // Segments of one tone step down in strength, so each reads apart.
                let n = seen.iter().filter(|c| **c == base).count();
                seen.push(base);
                (
                    share,
                    base.alpha([0.85, 0.4, 0.2][n.min(2)]),
                    s.label.as_str(),
                )
            })
            .collect();
        if short && parts.iter().all(|(s, ..)| *s <= 0.0) {
            return None;
        }
        let said: Vec<String> = parts
            .iter()
            .map(|(s, _, l)| {
                let pct = format!("{:.0}%", s * 100.0);
                if l.is_empty() {
                    pct
                } else {
                    format!("{l} {pct}")
                }
            })
            .collect();
        let aside = m.aside.as_deref().filter(|a| !a.is_empty());
        if self.ui.is_native() {
            // A platform shows the shares as a line; an entry's short bar repeats its figure.
            let line = said
                .iter()
                .map(String::as_str)
                .chain(aside)
                .collect::<Vec<_>>();
            return (!short).then(|| {
                text(line.join(" · "), size::XS.0, color::GRAY_500).sem(Sem::Text("secondary"))
            });
        }
        let cells = self.ui.is_cells();
        let bar = if cells {
            let width = if short {
                ENTRY_METER_CELLS
            } else {
                METER_CELLS
            };
            let mut used = 0;
            let mut runs = div().row().shrink0();
            for (i, (share, ink, _)) in parts.iter().enumerate() {
                let n = ((*share * width as f32).round() as usize).min(width - used);
                used += n;
                if n > 0 {
                    runs = runs.child(text(BLOCKS[i.min(2)].repeat(n), size::XS.0, *ink).silent());
                }
            }
            runs.child_if(
                (used < width)
                    .then(|| text("░".repeat(width - used), size::XS.0, color::GRAY_700).silent()),
            )
        } else {
            let mut track = div()
                .row()
                .w_full()
                .h(if short { 4.0 } else { 6.0 })
                .rounded(2.0)
                .bg(Rgba::white(0.06))
                .clip();
            for (share, ink, _) in &parts {
                track = track.child(div().h_full().w_pct(*share).bg(*ink));
            }
            track
        }
        .role(Role::Meter, said.join(", ").as_str());
        if short {
            return Some(bar);
        }
        let labelled: Vec<_> = (parts.iter().enumerate())
            .filter(|(_, (_, _, l))| !l.is_empty())
            .collect();
        if labelled.is_empty() && aside.is_none() {
            return Some(bar);
        }
        let legend = div()
            .row()
            .items_center()
            .gap(16.0)
            .min_w0()
            .children(labelled.into_iter().map(|(i, (_, ink, l))| {
                let key = if cells {
                    text(BLOCKS[i.min(2)], size::XS.0, *ink).silent()
                } else {
                    div().size(8.0, 8.0).rounded(2.0).bg(*ink)
                };
                div()
                    .row()
                    .items_center()
                    .gap(8.0)
                    .shrink0()
                    .child(key.shrink0())
                    .child(
                        text(*l, size::XS.0, color::GRAY_400)
                            .line_height(size::XS.1)
                            .silent(),
                    )
            }))
            .child(div().grow())
            .child_if(aside.map(|a| {
                text(a, size::XS.0, color::GRAY_500)
                    .line_height(size::XS.1)
                    .truncate()
                    .silent()
            }));
        Some(div().col().gap(8.0).min_w0().child(bar).child(legend))
    }

    /// A list: its check head, then its entries in one box or its groups under their headings.
    fn list(
        &self,
        n: &Node,
        empty: Option<&str>,
        loading: Option<&str>,
        select: bool,
        bulk: &[Node],
        lazy: bool,
    ) -> El {
        let id = n.id.as_deref().unwrap_or_default();
        if let Some(l) = loading {
            return state(l);
        }
        let entries: Vec<(&Node, &Entry)> = rows(&n.children).collect();
        if entries.is_empty() {
            return empty.map_or_else(div, state);
        }
        let checked: Vec<String> = entries
            .iter()
            .filter(|(r, e)| select && e.selectable && self.bind.checked(id, key(r)))
            .map(|(r, _)| key(r).to_owned())
            .collect();
        let can: Vec<String> = entries
            .iter()
            .filter(|(_, e)| e.selectable)
            .map(|(r, _)| key(r).to_owned())
            .collect();
        let l = Listing {
            id,
            select,
            checked,
        };
        let head = select.then(|| self.list_head(&l, &can, bulk));
        let body = if lazy && !self.ui.is_native() {
            self.lazy(n, &l)
        } else {
            let mut col = div().col().gap(24.0).min_w0();
            let mut run: Vec<&Node> = Vec::new();
            for c in &n.children {
                if matches!(c.kind, Kind::Entry(_)) {
                    run.push(c);
                    continue;
                }
                if !run.is_empty() {
                    col = col.child(self.items(std::mem::take(&mut run), &l));
                }
                col = col.child(self.group(c, &l));
            }
            if !run.is_empty() {
                col = col.child(self.items(run, &l));
            }
            col
        };
        let name = n.props.label.as_deref().unwrap_or(id);
        let list = div()
            .col()
            .gap(8.0)
            .min_w0()
            .role(Role::List, name)
            .child_if(head)
            .child(body);
        self.sem(list, || Sem::List)
    }

    /// A list's head: a box that checks every row, the count checked and the bulk buttons.
    fn list_head(&self, l: &Listing<'_>, can: &[String], bulk: &[Node]) -> El {
        let all = !can.is_empty() && l.checked.len() == can.len();
        let some = !l.checked.is_empty();
        let check = self.check(
            Ev::Check {
                list: l.id.to_owned(),
                rows: can.to_vec(),
                on: !all,
                all: true,
            },
            all,
            "Select all",
        );
        let said = if some {
            format!("{} selected", l.checked.len())
        } else {
            "Select all".to_owned()
        };
        let mut head = div()
            .row()
            .items_center()
            .gap(8.0)
            .min_h(28.0)
            .px(12.0)
            .min_w0()
            .role(Role::Toolbar, said.as_str())
            .child(check)
            .child(
                div().grow().min_w0().child(
                    text(&said, size::XS.0, color::GRAY_400)
                        .line_height(size::XS.1)
                        .truncate()
                        .silent(),
                ),
            );
        head = self.sem(head, || Sem::Row);
        if some {
            head = head.children(bulk.iter().filter_map(|b| {
                let Kind::Button {
                    label,
                    icon,
                    style,
                    confirm,
                } = &b.kind
                else {
                    return None;
                };
                let event = Event {
                    id: b.id.clone().unwrap_or_default(),
                    kind: EventKind::Click,
                    value: String::new(),
                    selected: l.checked.clone(),
                    scope: None,
                    width: None,
                };
                let ev = Ev::Send {
                    event,
                    ask: confirm.is_some(),
                };
                let name = b.props.label.as_deref().unwrap_or(label);
                Some(
                    self.button(ev, label, name, icon.as_deref(), *style, confirm.as_deref())
                        .1,
                )
            }));
        }
        head
    }

    /// A check box: a ticked square, `[x]` on cells.
    fn check(&self, ev: Ev, on: bool, name: &str) -> El {
        let id = self.bind.on(ev);
        let hot = self.ui.hover(id);
        let mark = if self.ui.is_cells() {
            text(if on { "[x]" } else { "[ ]" }, size::XS.0, color::GRAY_300).silent()
        } else {
            let edge = if on {
                Rgba::TRANSPARENT
            } else {
                color::GRAY_600.mix(color::GRAY_400, hot)
            };
            let b = div()
                .size(12.0, 12.0)
                .rounded(3.0)
                .center()
                .border(1.0, edge)
                .bg(if on {
                    color::GRAY_200
                } else {
                    Rgba::white(0.05)
                });
            if on {
                b.child(crate::lucide("check", 9.0, 3.0, color::SURFACE_BASE))
            } else {
                b
            }
        };
        div()
            .id(id)
            .on(Action::Click)
            .size(20.0, 20.0)
            .shrink0()
            .center()
            .ring(radius::SM)
            .role(Role::CheckBox, name)
            .toggled(on)
            .sem(Sem::Select)
            .child(mark)
    }

    /// Entries in one box, a hairline between each.
    fn items(&self, entries: Vec<&Node>, l: &Listing<'_>) -> El {
        let items = div()
            .col()
            .min_w0()
            .rounded(radius::SM)
            .border(1.0, Rgba::white(0.06))
            .bg(Rgba::white(0.02))
            .clip()
            .children(entries.into_iter().enumerate().map(|(i, n)| {
                let e = match &n.kind {
                    Kind::Entry(e) => self.entry(n, e, Some(l)),
                    _ => self.node(n),
                };
                if i == 0 {
                    e
                } else {
                    e.border_top(1.0, Rgba::white(0.06))
                }
            }));
        self.sem(items, || Sem::List)
    }

    /// A group: its heading and its entries.
    fn group(&self, n: &Node, l: &Listing<'_>) -> El {
        let heading = match &n.kind {
            Kind::Group { heading, .. } => heading.as_str(),
            _ => "",
        };
        let kids: Vec<&Node> = n.children.iter().collect();
        let group = div().col().gap(8.0).min_w0().role(Role::Group, heading);
        let group = self.sem(group, || Sem::Section(heading.to_owned()));
        if self.ui.is_native() {
            // A platform's section lists the group's entries itself, under its heading.
            return group
                .child(self.group_head(n))
                .children(kids.into_iter().map(|k| match &k.kind {
                    Kind::Entry(e) => self.entry(k, e, Some(l)),
                    _ => self.node(k),
                }));
        }
        group
            .child(self.group_head(n))
            .child_if((!kids.is_empty()).then(|| self.items(kids, l)))
    }

    /// A group's heading with its icon and note, and its aside and buttons at the right.
    fn group_head(&self, n: &Node) -> El {
        let Kind::Group {
            heading,
            note,
            aside,
            icon,
            actions,
        } = &n.kind
        else {
            return div();
        };
        let note = note.as_deref().filter(|s| !s.is_empty());
        let cells = self.ui.is_cells();
        let title = text(heading, 13.0, color::GRAY_100)
            .line_height(18.0)
            .weight(size::MEDIUM)
            .truncate()
            .role(Role::Heading, heading.as_str());
        if self.ui.is_native() {
            let quiet = |s: &str| text(s, size::XS.0, color::GRAY_500).sem(Sem::Text("secondary"));
            return div()
                .child_if(icon.as_deref().map(|i| self.icon(i, 14.0, color::GRAY_400)))
                .child_if(note.map(quiet))
                .child_if(aside.as_deref().filter(|a| !a.is_empty()).map(quiet))
                .children(actions.iter().map(|b| self.node(b)));
        }
        let mut head = div().row().items_center().gap(8.0).min_w0();
        match icon {
            Some(i) => {
                if !cells {
                    head = head.child(
                        div()
                            .size(28.0, 28.0)
                            .shrink0()
                            .center()
                            .rounded(radius::SM)
                            .bg(Rgba::white(0.04))
                            .child(self.icon(i, 14.0, color::GRAY_400)),
                    );
                }
                head = head.child(div().col().grow().min_w0().child(title).child_if(note.map(
                    |t| {
                        text(t, 11.0, color::GRAY_500)
                            .line_height(16.0)
                            .truncate()
                            .silent()
                    },
                )));
            }
            None => {
                head = head
                    .child(title.shrink0())
                    .child(div().grow().min_w0().child_if(note.map(|t| {
                        text(t, 11.0, color::GRAY_600)
                            .mono()
                            .line_height(16.0)
                            .truncate()
                            .silent()
                    })));
            }
        }
        head.child_if(aside.as_deref().filter(|a| !a.is_empty()).map(|a| {
            text(a, size::XS.0, color::GRAY_500)
                .line_height(size::XS.1)
                .shrink0()
                .silent()
        }))
        .children(actions.iter().map(|b| self.node(b)))
    }

    /// A long list in one scroll view, building only the headings and entries in view.
    fn lazy(&self, n: &Node, l: &Listing<'_>) -> El {
        enum Item<'a> {
            Head(&'a Node),
            Row(&'a Node, &'a Entry),
        }
        let mut items = Vec::new();
        for c in &n.children {
            match &c.kind {
                Kind::Entry(e) => items.push(Item::Row(c, e)),
                Kind::Group { .. } => {
                    items.push(Item::Head(c));
                    items.extend(rows(&c.children).map(|(r, e)| Item::Row(r, e)));
                }
                _ => {}
            }
        }
        let scroll = self.bind.on(Ev::Scroll {
            list: l.id.to_owned(),
        });
        let keys: Vec<Id> = items
            .iter()
            .enumerate()
            .map(|(i, it)| match it {
                Item::Head(_) => scroll.child("head").child(i as u64),
                Item::Row(r, _) => self.bind.on(Ev::Hover {
                    node: key(r).to_owned(),
                }),
            })
            .collect();
        let view = div()
            .id(scroll)
            .col()
            .h(LAZY_HEIGHT)
            .min_w0()
            .rounded(radius::SM)
            .border(1.0, Rgba::white(0.06))
            .bg(Rgba::white(0.02))
            .scroll_y();
        self.ui
            .windowed(scroll, &keys, 52.0, &[])
            .fill(view, |i| match &items[i] {
                Item::Head(g) => self.group_head(g).id(keys[i]).px(12.0).pt(16.0).pb(8.0),
                Item::Row(r, e) => self.entry(r, e, Some(l)).border_top(1.0, Rgba::white(0.06)),
            })
    }

    /// Button node `b` as an entry's action: its id, its box and its style.
    fn action(&self, b: &Node) -> Option<(Id, El, ButtonStyle)> {
        let Kind::Button {
            label,
            icon,
            style,
            confirm,
        } = &b.kind
        else {
            return None;
        };
        let bid = b.id.as_deref().unwrap_or_default();
        let name = b.props.label.as_deref().unwrap_or(label);
        let ev = send(bid, EventKind::Click, confirm.is_some());
        let (id, el) = self.button(ev, label, name, icon.as_deref(), *style, confirm.as_deref());
        Some((id, el, *style))
    }

    /// One entry as a platform's list row: its check, marks, figure, primary actions and the rest in its menu.
    fn native_entry(&self, n: &Node, e: &Entry, l: Option<&Listing<'_>>, id: Id) -> El {
        let node = key(n);
        let detail: Vec<&str> = [e.code.as_deref(), e.detail.as_deref()]
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .collect();
        let check = l.filter(|l| l.select && e.selectable).map(|l| {
            let on = l.checked.iter().any(|c| c == node);
            let ev = Ev::Check {
                list: l.id.to_owned(),
                rows: vec![node.to_owned()],
                on: !on,
                all: false,
            };
            self.check(ev, on, &format!("Select {}", e.title))
        });
        let primary = |b: &&Node| {
            matches!(
                b.kind,
                Kind::Button {
                    style: ButtonStyle::Primary,
                    ..
                }
            )
        };
        let menu: Vec<&Node> = (e.actions.iter().filter(|b| !primary(b)))
            .chain(&e.more)
            .collect();
        let quiet = |s: &str| text(s, size::XS.0, color::GRAY_500).sem(Sem::Text("secondary"));
        div()
            .id(id)
            .sem(Sem::Entry {
                title: e.title.clone(),
                detail: detail.join(" · "),
            })
            .child_if(check)
            .child_if(
                e.icon
                    .as_deref()
                    .map(|i| self.icon(i, 14.0, color::GRAY_400)),
            )
            .child_if(e.status.map(|t| {
                div().sem(Sem::Status {
                    value: tone_name(t).to_owned(),
                    tone: sem_tone(t),
                })
            }))
            .child_if(e.badge.as_ref().map(|b| {
                badge(&b.text, b.tone).sem(Sem::Badge {
                    value: b.text.clone(),
                    tone: sem_tone(b.tone),
                })
            }))
            .children(e.chips.iter().map(|c| quiet(c)))
            .children(n.children.iter().map(|k| self.node(k)))
            .child_if(
                e.aside
                    .as_deref()
                    .map(|a| text(a, size::XS.0, color::GRAY_300).sem(Sem::Text("number"))),
            )
            .children(
                e.actions
                    .iter()
                    .filter(primary)
                    .filter_map(|b| self.action(b))
                    .map(|(_, el, _)| el),
            )
            .child_if(self.more(node, &menu))
    }

    /// One entry; its actions show on hover, and on cells join its more menu.
    fn entry(&self, n: &Node, e: &Entry, l: Option<&Listing<'_>>) -> El {
        let node = key(n);
        let id = self.bind.on(Ev::Hover {
            node: node.to_owned(),
        });
        if self.ui.is_native() {
            return self.native_entry(n, e, l, id);
        }
        let hot = self.ui.hover(id);
        let cells = self.ui.is_cells();
        let check = l.filter(|l| l.select).map(|l| {
            if e.selectable {
                let on = l.checked.iter().any(|c| c == node);
                self.check(
                    Ev::Check {
                        list: l.id.to_owned(),
                        rows: vec![node.to_owned()],
                        on: !on,
                        all: false,
                    },
                    on,
                    &format!("Select {}", e.title),
                )
            } else {
                // Keeps the column for a row that cannot be checked.
                div().size(20.0, 20.0).shrink0()
            }
        });
        let lead = e.icon.as_deref().filter(|_| !cells).map(|i| {
            div()
                .size(28.0, 28.0)
                .shrink0()
                .center()
                .rounded(radius::SM)
                .bg(Rgba::white(0.04))
                .child(self.icon(i, 14.0, color::GRAY_400))
        });
        let dot = e.status.map(|t| {
            let ink = tone(t);
            let d = if cells {
                text("●", 12.0, ink).line_height(16.0).silent()
            } else {
                div().size(6.0, 6.0).rounded(3.0).bg(ink)
            };
            d.shrink0().role(Role::Status, tone_name(t))
        });
        let line = div()
            .row()
            .items_center()
            .gap(8.0)
            .min_w0()
            .child_if(dot)
            .child(
                text(&e.title, 13.0, color::GRAY_100)
                    .line_height(18.0)
                    .weight(size::MEDIUM)
                    .truncate()
                    .silent(),
            )
            .child_if(e.badge.as_ref().map(|b| badge(&b.text, b.tone)));
        let code = e.code.as_deref().filter(|c| !c.is_empty());
        let detail = e.detail.as_deref().filter(|d| !d.is_empty());
        let two = code.is_some() || detail.is_some();
        let second = two.then(|| {
            let sep = code.is_some() && detail.is_some();
            div()
                .row()
                .items_center()
                .min_w0()
                .child_if(code.map(|c| {
                    div().min_w0().child(
                        text(c, 11.0, color::GRAY_500)
                            .mono()
                            .line_height(size::XS.1)
                            .truncate()
                            .silent(),
                    )
                }))
                .child_if(detail.map(|d| {
                    let d = if sep {
                        format!(" · {d}")
                    } else {
                        d.to_owned()
                    };
                    div().grow().min_w0().child(
                        text(d, size::XS.0, color::GRAY_500)
                            .line_height(size::XS.1)
                            .truncate()
                            .silent(),
                    )
                }))
        });
        let chips = (!e.chips.is_empty()).then(|| {
            div()
                .row()
                .wrap_children()
                .gap(4.0)
                .pt(4.0)
                .min_w0()
                .children(e.chips.iter().map(|c| {
                    div()
                        .px(6.0)
                        .py(2.0)
                        .min_w0()
                        .rounded(radius::SM)
                        .bg(Rgba::white(0.04))
                        .child(
                            text(c, 11.0, color::GRAY_400)
                                .line_height(16.0)
                                .truncate()
                                .silent(),
                        )
                }))
        });
        let middle = div()
            .col()
            .grow()
            .min_w0()
            .child(line)
            .child_if(second)
            .child_if(chips)
            .children(n.children.iter().map(|k| self.node(k)));
        let mut acts = Vec::new();
        let mut menu: Vec<&Node> = e.more.iter().collect();
        for b in &e.actions {
            let Some((bid, el, style)) = self.action(b) else {
                continue;
            };
            let pinned = style == ButtonStyle::Primary
                || self.ui.key_focused(bid)
                || self.bind.confirming(bid).is_some();
            if pinned {
                acts.push(el);
            } else if cells {
                menu.insert(0, b);
            } else {
                acts.push(el.opacity(hot.max(self.ui.hover(bid))));
            }
        }
        let figure = e.aside.as_deref().map(|a| {
            text(a, size::XS.0, color::GRAY_300)
                .line_height(size::XS.1)
                .truncate()
                .silent()
        });
        let bar = e.meter.as_ref().and_then(|m| self.meter(m, true));
        let column = (figure.is_some() || bar.is_some()).then(|| {
            div()
                .col()
                .gap(4.0)
                .w(NUMBER_WIDTH)
                .shrink0()
                .items_end()
                .child_if(figure)
                .child_if(bar)
        });
        let mut row = div()
            .id(id)
            .sense(Sense::HOVER)
            .row()
            .items_center()
            .gap(12.0)
            .px(12.0)
            .min_w0()
            .bg(Rgba::TRANSPARENT.mix(Rgba::white(0.03), hot))
            .role(Role::ListItem, e.title.as_str())
            .child_if(check)
            .child_if(lead)
            .child(middle);
        if !cells {
            row = row.min_h(if two { 52.0 } else { 40.0 }).py(8.0);
        }
        // Hover actions sit before the figure so figures stay in one column.
        if !acts.is_empty() {
            row = row.child(div().row().items_center().gap(4.0).shrink0().children(acts));
        }
        row.child_if(column).child_if(self.more(node, &menu))
    }

    /// A "more" button and its open menu; a confirm asks under the button.
    fn more(&self, owner: &str, items: &[&Node]) -> Option<El> {
        if items.is_empty() {
            return None;
        }
        let menu = self.bind.on(Ev::Menu {
            node: owner.to_owned(),
        });
        let mut entries = Vec::new();
        let mut asking = None;
        for b in items {
            let Kind::Button {
                label,
                icon,
                style,
                confirm,
            } = &b.kind
            else {
                continue;
            };
            let bid = b.id.as_deref().unwrap_or_default();
            let name = b.props.label.as_deref().unwrap_or(label);
            let shown = if label.is_empty() { name } else { label };
            let id = self.bind.on(send(bid, EventKind::Click, confirm.is_some()));
            if let (Some(q), Some(ids)) = (confirm, self.bind.confirming(id)) {
                asking = Some((q.as_str(), shown, *style, ids));
            }
            let item = self.menu_item(id, shown, *style, icon.as_deref());
            entries.push(self.sem(item, || Sem::Button {
                style: sem_style(*style),
                confirm: confirm.clone(),
                icon: icon.clone(),
            }));
        }
        if self.ui.is_native() {
            return Some(div().id(menu).sem(Sem::More).children(entries));
        }
        let hot = self.ui.hover(menu);
        let open = self.bind.menu_open(menu);
        let mut b = div()
            .id(menu)
            .on(Action::Click)
            .px(4.0)
            .py(4.0)
            .shrink0()
            .rounded(radius::SM)
            .bg(Rgba::TRANSPARENT.mix(Rgba::white(0.06), hot))
            .ring(radius::SM)
            .role(Role::Button, "More")
            .child(crate::lucide(
                "ellipsis",
                12.0,
                2.0,
                color::GRAY_400.mix(color::WHITE, hot),
            ));
        if open {
            let panel = div()
                .col()
                .min_w(160.0)
                .py(4.0)
                .bg(color::SURFACE_OVERLAY)
                .border(1.0, Rgba::white(0.10))
                .rounded(radius::SM)
                .role(Role::Menu, "More")
                .children(entries);
            b = b.overlay(panel, Place::BelowEnd, 4.0);
        }
        Some(match asking {
            Some((q, label, style, (ok, cancel))) => {
                widgets::confirm(self.ui, b, q, (ok, label), cancel, ask_tone(style))
            }
            None => b,
        })
    }

    fn menu_item(&self, id: Id, label: &str, style: ButtonStyle, icon: Option<&str>) -> El {
        let hot = self.ui.hover(id);
        let ink = match style {
            ButtonStyle::Danger => color::RED_400.mix(color::RED_300, hot),
            _ => color::GRAY_300.mix(color::WHITE, hot),
        };
        div()
            .id(id)
            .on(Action::Click)
            .row()
            .items_center()
            .gap(8.0)
            .mx(4.0)
            .px(8.0)
            .py(5.0)
            .rounded(radius::SM)
            .bg(Rgba::TRANSPARENT.mix(Rgba::white(0.06), hot))
            .role(Role::MenuItem, label)
            .child_if(icon.map(|i| self.icon(i, 12.0, ink)))
            .child(
                text(label, size::XS.0, ink)
                    .line_height(size::XS.1)
                    .truncate()
                    .silent(),
            )
    }

    /// A board's column: its title, a count, and each child on a card.
    fn column(&self, n: &Node) -> El {
        let Kind::Column { title, tone: t, .. } = &n.kind else {
            return self.node(n);
        };
        let ink = t.map_or(color::GRAY_500, tone);
        let head = div()
            .row()
            .items_center()
            .gap(8.0)
            .px(4.0)
            .min_w0()
            .child(if self.ui.is_cells() {
                text("●", 12.0, ink).line_height(16.0).silent()
            } else {
                div().size(6.0, 6.0).rounded(3.0).bg(ink).shrink0()
            })
            .child(
                text(title, size::XS.0, color::GRAY_300)
                    .line_height(size::XS.1)
                    .weight(size::MEDIUM)
                    .truncate()
                    .role(Role::Heading, title.as_str()),
            )
            .child(
                text(n.children.len().to_string(), size::XS.0, color::GRAY_500)
                    .line_height(size::XS.1)
                    .silent(),
            );
        div()
            .col()
            .gap(8.0)
            .flex(1.0)
            .min_w0()
            .role(Role::Group, title.as_str())
            .child(head)
            .children(n.children.iter().map(|c| {
                div()
                    .col()
                    .gap(4.0)
                    .p(12.0)
                    .min_w0()
                    .rounded(radius::SM)
                    .border(1.0, Rgba::white(0.08))
                    .bg(color::SURFACE_PANEL)
                    .child(self.node(c))
            }))
    }
}

fn send(id: &str, kind: EventKind, ask: bool) -> Ev {
    Ev::Send {
        event: Event {
            id: id.to_owned(),
            kind,
            value: String::new(),
            selected: Vec::new(),
            scope: None,
            width: None,
        },
        ask,
    }
}

fn key(n: &Node) -> &str {
    n.id.as_deref().unwrap_or_default()
}

/// A list's entries, those in its groups too.
fn rows(kids: &[Node]) -> impl Iterator<Item = (&Node, &Entry)> {
    kids.iter().flat_map(|c| match &c.kind {
        Kind::Entry(e) => vec![(c, &**e)],
        Kind::Group { .. } => c
            .children
            .iter()
            .filter_map(|r| match &r.kind {
                Kind::Entry(e) => Some((r, &**e)),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    })
}

fn ask_tone(style: ButtonStyle) -> ConfirmTone {
    if style == ButtonStyle::Danger {
        ConfirmTone::Danger
    } else {
        ConfirmTone::Neutral
    }
}

/// A board whose client reads no cards: its columns, empty.
struct Unread;

impl crate::board::Cards for Unread {
    fn placed(&self) -> &[(String, String)] {
        &[]
    }
    fn card(&self, _: usize) -> Option<crate::board::Card> {
        None
    }
}

/// The ids of a board whose client binds nothing.
struct Unbound;

impl crate::board::BoardBind for Unbound {
    fn card(&self, card: &str) -> Id {
        crate::board::card_id("tree", card)
    }
    fn dragging(&self) -> Option<(String, Option<String>)> {
        None
    }
    fn add(&self, lane: &str, foot: bool) -> Id {
        Id::new("tree-board-add").child(lane).child(foot)
    }
    fn menu(&self, card: &str) -> Id {
        Id::new("tree-board-menu").child(card)
    }
    fn menu_open(&self, _: Id) -> bool {
        false
    }
    fn action(&self, card: &str, a: &crate::board::CardAction) -> Id {
        Id::new("tree-board-action").child(card).child(a.id)
    }
    fn confirming(&self, _: Id) -> Option<(Id, Id)> {
        None
    }
}

/// Tone `t` as a native backend names it.
pub fn native_tone(t: Tone) -> &'static str {
    sem_tone(t)
}

fn sem_tone(t: Tone) -> &'static str {
    match t {
        Tone::Neutral => "muted",
        Tone::Good => "good",
        Tone::Warn => "warn",
        Tone::Bad => "bad",
        Tone::Info => "busy",
        Tone::Accent => "lit",
        Tone::Busy => "busy",
        Tone::Muted => "muted",
        Tone::Idle => "idle",
    }
}

fn sem_style(s: ButtonStyle) -> &'static str {
    match s {
        ButtonStyle::Plain => "plain",
        ButtonStyle::Primary => "primary",
        ButtonStyle::Danger => "danger",
    }
}

/// The text role a native backend draws `style` in; bad text reads as an error.
fn text_role(style: TextStyle, t: Option<Tone>) -> &'static str {
    match (style, t) {
        (_, Some(Tone::Bad)) => "error",
        (TextStyle::Title | TextStyle::Heading, _) => "title",
        (TextStyle::Body, _) => "body",
        (TextStyle::Secondary, _) => "secondary",
        (TextStyle::Mono, _) => "mono",
        (TextStyle::Number, _) => "number",
    }
}

fn tone_name(t: Tone) -> &'static str {
    match t {
        Tone::Neutral => "Neutral",
        Tone::Good => "Good",
        Tone::Warn => "Warning",
        Tone::Bad => "Problem",
        Tone::Info => "Info",
        Tone::Accent => "Active",
        Tone::Busy => "Busy",
        Tone::Muted => "Quiet",
        Tone::Idle => "Waiting",
    }
}

/// Props that size any node: its width, height, padding and growth.
fn sized(mut e: El, p: &Props) -> El {
    if let Some(v) = p.w {
        e = e.w(v);
    }
    if let Some(v) = p.h {
        e = e.h(v);
    }
    if let Some(v) = p.p {
        e = e.p(v);
    }
    if let Some(v) = p.px {
        e = e.px(v);
    }
    if let Some(v) = p.py {
        e = e.py(v);
    }
    if p.grow {
        e = e.grow();
    }
    e
}

fn styled(s: &str, style: TextStyle, ink: Option<Rgba>) -> El {
    let (px, lh, rest) = match style {
        TextStyle::Title => (13.0, 18.0, color::GRAY_200),
        TextStyle::Body | TextStyle::Number => (size::XS.0, size::XS.1, color::GRAY_300),
        TextStyle::Secondary => (11.0, 16.0, color::GRAY_500),
        TextStyle::Mono => (11.0, 16.0, color::GRAY_400),
        TextStyle::Heading => (size::XS.0, size::XS.1, color::GRAY_500),
    };
    let ink = ink.unwrap_or(rest);
    let shown = if style == TextStyle::Heading {
        s.to_uppercase()
    } else {
        s.to_owned()
    };
    let t = text(shown, px, ink).line_height(lh).silent();
    match style {
        TextStyle::Title => t.weight(size::SEMIBOLD),
        TextStyle::Mono => t.mono(),
        TextStyle::Heading => t.weight(size::MEDIUM).tracking(0.05),
        _ => t,
    }
}

/// A short label in a tone.
fn badge(s: &str, t: Tone) -> El {
    let ink = tone(t);
    let bg = if t == Tone::Neutral {
        Rgba::white(0.06)
    } else {
        ink.alpha(0.12)
    };
    widgets::badge(s, ink, bg).role(Role::Label, s)
}

/// A line that stands out, as an error says what happened.
fn notice(s: &str, t: Option<Tone>) -> El {
    let (icon, ink) = match t {
        Some(Tone::Bad) => ("circle-alert", color::RED_400),
        Some(Tone::Warn) => ("triangle-alert", color::AMBER_400),
        Some(other) => ("info", tone(other)),
        None => ("info", color::GRAY_500),
    };
    div()
        .row()
        .items_start()
        .gap(8.0)
        .px(12.0)
        .py(8.0)
        .min_w0()
        .rounded(radius::SM)
        .border(1.0, Rgba::white(0.08))
        .bg(color::SURFACE_SUNKEN)
        .role(Role::Status, s)
        .child(crate::lucide(icon, 14.0, 2.0, ink).mt(1.0).shrink0())
        .child(
            div().grow().min_w0().child(
                text(s, size::XS.0, color::GRAY_300)
                    .line_height(size::XS.1)
                    .wrapping()
                    .silent(),
            ),
        )
}

/// What a list says in place of its rows: empty, or still loading.
fn state(copy: &str) -> El {
    div()
        .px(12.0)
        .py(16.0)
        .min_w0()
        .rounded(radius::SM)
        .border(1.0, Rgba::white(0.06))
        .role(Role::Status, copy)
        .child(
            text(copy, size::XS.0, color::GRAY_500)
                .line_height(size::XS.1)
                .wrapping()
                .silent(),
        )
}

mod parts;
#[cfg(test)]
mod tests;

/// The box drawn in place of a tree that cannot be drawn: which place, and each reason.
pub fn invalid(place: &str, errors: &[String]) -> El {
    let mut b = div()
        .col()
        .gap(4.0)
        .px(8.0)
        .py(6.0)
        .rounded(radius::SM)
        .border(1.0, color::DANGER)
        .role(Role::Alert, place)
        .child(
            text(
                format!("{place} cannot be drawn"),
                size::XS.0,
                color::RED_300,
            )
            .line_height(size::XS.1)
            .weight(600)
            .wrapping(),
        );
    for e in errors {
        b = b.child(
            text(e, 11.0, color::RED_300)
                .mono()
                .line_height(16.0)
                .wrapping(),
        );
    }
    b
}
