//! Model C's guest side: vornui's builder names (`div`, `text`, `widgets::button`, theme tokens), producing a tree the client lays out and draws.

wit_bindgen::generate!({ world: "sdk", path: "../wit" });

pub use tree::{
    Confirm, EventKind, Kind, Node, Paint, Prop, Radius, Role, Surface, TextSize, Token, Tone,
};
pub use vorn::ext_ui_poc::tree;
pub use wit_bindgen;

/// One box and the boxes inside it.
pub struct El {
    kind: Kind,
    id: Option<String>,
    props: Vec<Prop>,
    children: Vec<El>,
}

pub fn div() -> El {
    El {
        kind: Kind::Box,
        id: None,
        props: Vec::new(),
        children: Vec::new(),
    }
}

pub fn text(s: impl Into<String>, size: TextSize, ink: Paint) -> El {
    El {
        kind: Kind::Text((s.into(), size, ink)),
        ..div()
    }
}

/// `svg` stretched over a `w`×`h` box and painted `ink`.
pub fn shape(svg: impl Into<String>, w: f32, h: f32, ink: Paint) -> El {
    El {
        kind: Kind::Shape((svg.into(), ink)),
        ..div()
    }
    .w(w)
    .h(h)
}

macro_rules! props {
    ($($name:ident => $prop:ident),* $(,)?) => {
        $(pub fn $name(self) -> El { self.prop(Prop::$prop) })*
    };
    ($($name:ident($t:ty) => $prop:ident),* $(,)?) => {
        $(pub fn $name(self, v: $t) -> El { self.prop(Prop::$prop(v)) })*
    };
}

impl El {
    fn prop(mut self, p: Prop) -> El {
        self.props.push(p);
        self
    }
    pub fn id(mut self, id: impl Into<String>) -> El {
        self.id = Some(id.into());
        self
    }
    pub fn child(mut self, c: El) -> El {
        self.children.push(c);
        self
    }
    pub fn children(mut self, cs: impl IntoIterator<Item = El>) -> El {
        self.children.extend(cs);
        self
    }
    props! {
        row => Row, col => Col, w_full => WFull, h_full => HFull, grow => Grow,
        items_center => ItemsCenter, justify_between => JustifyBetween,
        mono => Mono, truncate => Truncate, on_click => Click, on_drag => Drag,
    }
    props! {
        gap(f32) => Gap, p(f32) => Pad, px(f32) => PadX, py(f32) => PadY, w(f32) => W, h(f32) => H,
        min_h(f32) => MinH, bg(Paint) => Bg, hover_bg(Paint) => HoverBg, rounded(Radius) => Rounded,
        opacity(f32) => Opacity, weight(u16) => Weight,
    }
    pub fn border(self, width: f32, ink: Paint) -> El {
        self.prop(Prop::Border((width, ink)))
    }
    pub fn disabled(self, on: bool) -> El {
        if on {
            self.prop(Prop::Disabled)
        } else {
            self
        }
    }
    pub fn glyph(self, g: impl Into<String>) -> El {
        self.prop(Prop::Glyph(g.into()))
    }
    pub fn access(self, role: Role, name: impl Into<String>) -> El {
        self.prop(Prop::Access((role, name.into())))
    }

    /// The tree depth first, as the client reads it.
    pub fn flatten(self) -> Vec<Node> {
        let mut out = Vec::new();
        self.push(&mut out);
        out
    }

    fn push(self, out: &mut Vec<Node>) {
        out.push(Node {
            kind: self.kind,
            id: self.id,
            children: self.children.len() as u32,
            props: self.props,
        });
        for c in self.children {
            c.push(out);
        }
    }
}

/// The app's controls, drawn by the client with its own look.
pub mod widgets {
    use super::*;

    pub fn button(id: impl Into<String>, label: impl Into<String>) -> El {
        El {
            kind: Kind::Button(label.into()),
            ..div()
        }
        .id(id)
    }

    pub fn text_input(id: impl Into<String>, name: &str, placeholder: &str) -> El {
        El {
            kind: Kind::Field((name.to_owned(), placeholder.to_owned())),
            ..div()
        }
        .id(id)
    }

    pub fn list(id: impl Into<String>, name: &str, rows: impl IntoIterator<Item = El>) -> El {
        El {
            kind: Kind::List(name.to_owned()),
            ..div()
        }
        .id(id)
        .children(rows)
    }

    pub fn badge(text: impl Into<String>, tone: Tone) -> El {
        El {
            kind: Kind::Badge((text.into(), tone)),
            ..div()
        }
    }

    /// `trigger`'s confirm popover: `<id>.ok` and `<id>.cancel` report clicks.
    pub fn confirm(trigger: El, message: impl Into<String>, ok: &str, danger: bool) -> El {
        trigger.prop(Prop::Confirm(Confirm {
            message: message.into(),
            ok: ok.to_owned(),
            danger,
        }))
    }
}

/// Theme tokens: colours by meaning, sizes and radii by name.
pub mod theme {
    use super::{Paint, Token};

    pub const BASE: Paint = Paint::Token(Token::Base);
    pub const INK: Paint = Paint::Token(Token::Ink);
    pub const INK_SECONDARY: Paint = Paint::Token(Token::InkSecondary);
    pub const INK_FAINT: Paint = Paint::Token(Token::InkFaint);
    pub const MUTED: Paint = Paint::Token(Token::Muted);
    pub const ACCENT: Paint = Paint::Token(Token::Accent);
    pub const CLEAR: Paint = Paint::Clear;

    pub const fn white(alpha: f32) -> Paint {
        Paint::Tint((Token::White, alpha))
    }
    pub const fn tint(t: Token, alpha: f32) -> Paint {
        Paint::Tint((t, alpha))
    }
}
