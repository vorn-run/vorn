//! Elements: a tree of boxes laid out by taffy's flexbox, each painting a
//! background, a border and one piece of content, and optionally exposing
//! itself to assistive technology. The tree is rebuilt every frame from the
//! app's state; only text shaping and rasters are cached across frames.

use accesskit::Role;
use taffy::prelude::*;

use crate::gpu::Rgba;
use crate::text::TextStyle;

pub enum Content {
    None,
    Text {
        text: String,
        style: TextStyle,
        color: Rgba,
        wrap: bool,
    },
    /// An SVG icon tinted with `color`, `size` logical pixels square.
    Icon {
        svg: String,
        size: f32,
        color: Rgba,
    },
    /// An RGBA image the app registered with [`crate::Ui::image`].
    Image {
        id: u32,
        color: Rgba,
    },
    /// A box the app paints itself after layout (a terminal pane).
    Custom(u64),
}

pub struct El {
    pub(crate) style: Style,
    pub(crate) bg: Option<Rgba>,
    pub(crate) border: Option<(f32, Rgba)>,
    pub(crate) radius: f32,
    pub(crate) content: Content,
    pub(crate) children: Vec<El>,
    pub(crate) role: Option<Role>,
    pub(crate) label: Option<String>,
    pub(crate) value: Option<String>,
    pub(crate) selected: bool,
}

pub fn div() -> El {
    El {
        style: Style::default(),
        bg: None,
        border: None,
        radius: 0.0,
        content: Content::None,
        children: Vec::new(),
        role: None,
        label: None,
        value: None,
        selected: false,
    }
}

/// A single-line label; screen readers read it as static text.
pub fn text(s: impl Into<String>, size: f32, color: Rgba) -> El {
    let s = s.into();
    let mut e = div();
    e.role = Some(Role::Label);
    e.label = Some(s.clone());
    e.style.flex_shrink = 0.0;
    e.content = Content::Text {
        text: s,
        style: TextStyle::ui(size),
        color,
        wrap: false,
    };
    e
}

pub fn icon(svg: String, size: f32, color: Rgba) -> El {
    div()
        .size(size, size)
        .content(Content::Icon { svg, size, color })
}

pub fn image(id: u32, w: f32, h: f32) -> El {
    div().size(w, h).content(Content::Image {
        id,
        color: Rgba([1.0; 4]),
    })
}

pub fn custom(id: u64) -> El {
    div().content(Content::Custom(id))
}

impl El {
    pub fn content(mut self, c: Content) -> El {
        self.content = c;
        self
    }
    pub fn row(mut self) -> El {
        self.style.display = Display::Flex;
        self.style.flex_direction = FlexDirection::Row;
        self
    }
    pub fn col(mut self) -> El {
        self.style.display = Display::Flex;
        self.style.flex_direction = FlexDirection::Column;
        self
    }
    pub fn wrap_children(mut self) -> El {
        self.style.flex_wrap = FlexWrap::Wrap;
        self
    }
    pub fn gap(mut self, g: f32) -> El {
        self.style.gap = Size {
            width: length(g),
            height: length(g),
        };
        self
    }
    pub fn px(mut self, p: f32) -> El {
        self.style.padding.left = length(p);
        self.style.padding.right = length(p);
        self
    }
    pub fn py(mut self, p: f32) -> El {
        self.style.padding.top = length(p);
        self.style.padding.bottom = length(p);
        self
    }
    pub fn p(self, p: f32) -> El {
        self.px(p).py(p)
    }
    pub fn pl(mut self, p: f32) -> El {
        self.style.padding.left = length(p);
        self
    }
    pub fn pr(mut self, p: f32) -> El {
        self.style.padding.right = length(p);
        self
    }
    pub fn mt(mut self, m: f32) -> El {
        self.style.margin.top = length(m);
        self
    }
    pub fn ml(mut self, m: f32) -> El {
        self.style.margin.left = length(m);
        self
    }
    pub fn w(mut self, w: f32) -> El {
        self.style.size.width = length(w);
        self
    }
    pub fn h(mut self, h: f32) -> El {
        self.style.size.height = length(h);
        self
    }
    pub fn size(self, w: f32, h: f32) -> El {
        self.w(w).h(h).shrink0()
    }
    pub fn w_full(mut self) -> El {
        self.style.size.width = percent(1.0f32);
        self
    }
    pub fn h_full(mut self) -> El {
        self.style.size.height = percent(1.0f32);
        self
    }
    pub fn max_w(mut self, w: f32) -> El {
        self.style.max_size.width = length(w);
        self
    }
    pub fn min_h(mut self, h: f32) -> El {
        self.style.min_size.height = length(h);
        self
    }
    pub fn grow(mut self) -> El {
        self.style.flex_grow = 1.0;
        self.style.flex_basis = length(0.0f32);
        self
    }
    pub fn shrink0(mut self) -> El {
        self.style.flex_shrink = 0.0;
        self
    }
    pub fn items_center(mut self) -> El {
        self.style.align_items = Some(AlignItems::Center);
        self
    }
    pub fn justify_center(mut self) -> El {
        self.style.justify_content = Some(JustifyContent::Center);
        self
    }
    pub fn justify_between(mut self) -> El {
        self.style.justify_content = Some(JustifyContent::SpaceBetween);
        self
    }
    pub fn center(self) -> El {
        self.items_center().justify_center()
    }
    /// Places the box at `(x, y)` in its parent, out of the flow.
    pub fn absolute(mut self, x: f32, y: f32) -> El {
        self.style.position = Position::Absolute;
        self.style.inset.left = length(x);
        self.style.inset.top = length(y);
        self
    }
    pub fn absolute_right(mut self, right: f32, y: f32) -> El {
        self.style.position = Position::Absolute;
        self.style.inset.right = length(right);
        self.style.inset.top = length(y);
        self
    }
    pub fn bg(mut self, c: Rgba) -> El {
        self.bg = Some(c);
        self
    }
    pub fn border(mut self, w: f32, c: Rgba) -> El {
        self.border = Some((w, c));
        self.style.border = Rect {
            left: length(w),
            right: length(w),
            top: length(w),
            bottom: length(w),
        };
        self
    }
    pub fn rounded(mut self, r: f32) -> El {
        self.radius = r;
        self
    }
    /// Opacity of an image or icon, or of a text element's color.
    pub fn alpha(mut self, a: f32) -> El {
        match &mut self.content {
            Content::Image { color, .. }
            | Content::Icon { color, .. }
            | Content::Text { color, .. } => *color = color.alpha(a),
            Content::None | Content::Custom(_) => {}
        }
        self
    }
    /// Text weight, for a text element.
    pub fn weight(mut self, w: u16) -> El {
        if let Content::Text { style, .. } = &mut self.content {
            style.weight = w;
        }
        self
    }
    pub fn line_height(mut self, h: f32) -> El {
        if let Content::Text { style, .. } = &mut self.content {
            style.line_height = h;
        }
        self
    }
    /// Lets a text element wrap to its box instead of overflowing it.
    pub fn wrapping(mut self) -> El {
        if let Content::Text { wrap, .. } = &mut self.content {
            *wrap = true;
        }
        self.style.flex_shrink = 1.0;
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
    /// Exposes the box to assistive technology.
    pub fn role(mut self, role: Role, label: impl Into<String>) -> El {
        self.role = Some(role);
        self.label = Some(label.into());
        self
    }
    /// Keeps a text element out of the accessibility tree (its parent's
    /// label already says it).
    pub fn silent(mut self) -> El {
        self.role = None;
        self.label = None;
        self
    }
    pub fn value(mut self, v: impl Into<String>) -> El {
        self.value = Some(v.into());
        self
    }
    pub fn selected(mut self, s: bool) -> El {
        self.selected = s;
        self
    }
}
