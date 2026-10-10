//! Elements: a tree of boxes laid out by taffy's flexbox, each painting a
//! background, a border and one piece of content, optionally reacting to
//! the pointer and keyboard and exposing itself to assistive technology.
//!
//! The tree is rebuilt every frame from the app's state (widgets read hover,
//! focus and open menus from the [`Ui`](crate::Ui) as they build); only text
//! shaping, rasters and the interaction state survive between frames.

use std::hash::{Hash, Hasher};

use accesskit::Role;
use taffy::prelude::*;

use crate::scene::Rgba;
use crate::text::TextStyle;

/// A stable name for an element across frames: what hover, focus, scroll
/// offsets, text being edited and accessibility node ids hang off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id(pub u64);

impl Id {
    /// The id named `name`; the same name gives the same id in every run.
    pub fn new(name: &str) -> Id {
        Id(fnv(FNV_OFFSET, name.as_bytes()) | 1)
    }

    /// An id under this one, for the parts of a widget (`menu.child(3)`).
    pub fn child(self, part: impl Hash) -> Id {
        let mut h = Fnv(self.0);
        part.hash(&mut h);
        Id(h.0 | 1)
    }
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// FNV-1a: stable across runs and Rust versions, unlike `DefaultHasher`.
struct Fnv(u64);

impl Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        self.0 = fnv(self.0, bytes);
    }
}

/// What an element reacts to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sense(u8);

impl Sense {
    pub const NONE: Sense = Sense(0);
    /// Hover is tracked (tooltips, hover colours).
    pub const HOVER: Sense = Sense(1);
    /// Pointer presses activate it.
    pub const CLICK: Sense = Sense(2 | 1);
    /// It takes keyboard focus (Tab stops at it).
    pub const FOCUS: Sense = Sense(4);
    /// Pointer drags report deltas (split dividers).
    pub const DRAG: Sense = Sense(8 | 1);
    /// The wheel scrolls it.
    pub const SCROLL: Sense = Sense(16);
    /// It edits text: keys and IME go to it while focused.
    pub const TEXT: Sense = Sense(32 | 4 | 1);

    pub fn has(self, s: Sense) -> bool {
        self.0 & s.0 == s.0
    }
}

impl std::ops::BitOr for Sense {
    type Output = Sense;
    fn bitor(self, o: Sense) -> Sense {
        Sense(self.0 | o.0)
    }
}

/// What activating an element (click, Enter, Space, a screen reader's
/// default action) does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Reports [`Event::Click`](crate::Event::Click) with the element's id.
    Click,
    /// Reports [`Event::Select`](crate::Event::Select) of `index` in the
    /// group `group` and closes the group's menu if it is open. Arrow keys
    /// move between the elements of one group.
    Select { group: Id, index: usize },
    /// Opens the menu `menu`, or closes it if open.
    ToggleMenu(Id),
}

/// The pointer's shape over an element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cursor {
    #[default]
    Default,
    Pointer,
    Text,
    ColResize,
    RowResize,
}

/// Where an overlay sits against the element that owns it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Centred above (a tooltip's default side).
    Above,
    /// Centred below.
    Below,
    /// Below, left edges aligned (a dropdown menu).
    BelowStart,
    /// Above, left edges aligned (a dropdown that has no room below).
    AboveStart,
    /// To the right, vertically centred.
    Right,
}

/// A box drawn over everything, placed against its owner: menus, tooltips.
pub struct Overlay {
    pub el: El,
    pub place: Place,
    /// Logical pixels between the owner and the overlay.
    pub gap: f32,
    /// At least as wide as the owner (a dropdown under its trigger).
    pub owner_width: bool,
}

pub enum Content {
    None,
    Text {
        text: String,
        style: TextStyle,
        color: Rgba,
        wrap: bool,
        truncate: bool,
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
    /// The text the [`Ui`](crate::Ui) is editing under the element's id.
    Edit(Box<EditLook>),
    /// A box the app paints itself after layout (a terminal pane).
    Custom(u64),
}

/// How an edit field draws its text.
#[derive(Debug, Clone, PartialEq)]
pub struct EditLook {
    pub style: TextStyle,
    pub color: Rgba,
    pub placeholder: String,
    pub placeholder_color: Rgba,
    pub multiline: bool,
}

/// Accessibility facts beyond role and label.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct A11y {
    pub role: Option<Role>,
    pub label: Option<String>,
    pub value: Option<String>,
    pub description: Option<String>,
    pub placeholder: Option<String>,
    pub shortcut: Option<String>,
    pub selected: Option<bool>,
    pub toggled: Option<bool>,
    pub expanded: Option<bool>,
    pub disabled: bool,
    pub has_popup: bool,
    pub vertical: Option<bool>,
}

pub struct El {
    pub(crate) style: Style,
    pub(crate) bg: Option<Rgba>,
    pub(crate) border: Option<(f32, Rgba)>,
    pub(crate) radius: f32,
    pub(crate) content: Content,
    pub(crate) children: Vec<El>,
    pub(crate) a11y: A11y,
    pub(crate) id: Option<Id>,
    pub(crate) sense: Sense,
    pub(crate) action: Option<Action>,
    pub(crate) cursor: Option<Cursor>,
    pub(crate) clip: bool,
    pub(crate) scroll: bool,
    pub(crate) opacity: f32,
    /// Scale about the box's centre and a shift, for enter animations.
    pub(crate) transform: Option<(f32, (f32, f32))>,
    /// Draw the focus-visible ring (with this radius) when focused by keyboard.
    pub(crate) ring: Option<f32>,
    pub(crate) overlays: Vec<Overlay>,
    /// Hovering it long enough shows its tooltip overlay.
    pub(crate) tip: bool,
    pub(crate) top_rule: Option<(f32, Rgba)>,
    pub(crate) slop: (f32, f32),
}

pub fn div() -> El {
    El {
        style: Style::default(),
        bg: None,
        border: None,
        radius: 0.0,
        content: Content::None,
        children: Vec::new(),
        a11y: A11y::default(),
        id: None,
        sense: Sense::NONE,
        action: None,
        cursor: None,
        clip: false,
        scroll: false,
        opacity: 1.0,
        transform: None,
        ring: None,
        overlays: Vec::new(),
        tip: false,
        top_rule: None,
        slop: (0.0, 0.0),
    }
}

/// A single-line label; screen readers read it as static text.
pub fn text(s: impl Into<String>, size: f32, color: Rgba) -> El {
    let s = s.into();
    let mut e = div();
    e.a11y.role = Some(Role::Label);
    e.a11y.label = Some(s.clone());
    e.style.flex_shrink = 0.0;
    e.content = Content::Text {
        text: s,
        style: TextStyle::ui(size),
        color,
        wrap: false,
        truncate: false,
    };
    e
}

pub fn icon(svg: impl Into<String>, size: f32, color: Rgba) -> El {
    div().size(size, size).content(Content::Icon {
        svg: svg.into(),
        size,
        color,
    })
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
    pub fn pt(mut self, p: f32) -> El {
        self.style.padding.top = length(p);
        self
    }
    pub fn pb(mut self, p: f32) -> El {
        self.style.padding.bottom = length(p);
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
    /// Horizontal margins; negative pulls a hover background past the
    /// text it surrounds (`-mx-1.5`).
    pub fn mx(mut self, m: f32) -> El {
        self.style.margin.left = length(m);
        self.style.margin.right = length(m);
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
    pub fn w_pct(mut self, f: f32) -> El {
        self.style.size.width = percent(f);
        self
    }
    pub fn h_pct(mut self, f: f32) -> El {
        self.style.size.height = percent(f);
        self
    }
    pub fn max_w(mut self, w: f32) -> El {
        self.style.max_size.width = length(w);
        self
    }
    pub fn max_h(mut self, h: f32) -> El {
        self.style.max_size.height = length(h);
        self
    }
    pub fn min_w(mut self, w: f32) -> El {
        self.style.min_size.width = length(w);
        self
    }
    /// `min-w-0`: lets a flex child shrink below its content (for truncation).
    pub fn min_w0(self) -> El {
        self.min_w(0.0)
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
    /// Takes share `f` of the free space, from a zero basis (`flex: f 1 0`).
    pub fn flex(mut self, f: f32) -> El {
        self.style.flex_grow = f;
        self.style.flex_shrink = 1.0;
        self.style.flex_basis = length(0.0_f32);
        self.style.min_size = Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        };
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
    pub fn items_stretch(mut self) -> El {
        self.style.align_items = Some(AlignItems::Stretch);
        self
    }
    pub fn self_center(mut self) -> El {
        self.style.align_self = Some(AlignSelf::Center);
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
    /// Out of the flow, pinned to the parent's bottom edge, full width.
    pub fn absolute_bottom(mut self) -> El {
        self.style.position = Position::Absolute;
        self.style.inset.left = length(0.0_f32);
        self.style.inset.right = length(0.0_f32);
        self.style.inset.bottom = length(0.0_f32);
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
    /// A top border only (`border-t`).
    pub fn border_top(mut self, w: f32, c: Rgba) -> El {
        self.style.border.top = length(w);
        self.top_rule = Some((w, c));
        self
    }
    /// Widens the pointer target past the box by `dx` each side horizontally
    /// and `dy` vertically (a 1px divider grabbed 3px either side).
    pub fn hit_slop(mut self, dx: f32, dy: f32) -> El {
        self.slop = (dx, dy);
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
            Content::None | Content::Custom(_) | Content::Edit(_) => {}
        }
        self
    }
    /// Opacity of the box and everything in it (`opacity-40`).
    pub fn opacity(mut self, a: f32) -> El {
        self.opacity *= a;
        self
    }
    /// Draws the box scaled by `scale` about its centre and shifted by
    /// `offset` logical pixels; layout and hit-testing ignore it.
    pub fn transform(mut self, scale: f32, offset: (f32, f32)) -> El {
        self.transform = Some((scale, offset));
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
    /// Monospace text, for a text element.
    pub fn mono(mut self) -> El {
        if let Content::Text { style, .. } = &mut self.content {
            style.mono = true;
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
    /// Cuts a text element with an ellipsis when its box is too narrow.
    pub fn truncate(mut self) -> El {
        if let Content::Text { truncate, .. } = &mut self.content {
            *truncate = true;
        }
        self.style.flex_shrink = 1.0;
        self.style.min_size.width = length(0.0_f32);
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
    pub fn child_if(self, c: Option<El>) -> El {
        match c {
            Some(c) => self.child(c),
            None => self,
        }
    }
    /// Names the element so it keeps its interaction state across frames.
    pub fn id(mut self, id: Id) -> El {
        self.id = Some(id);
        self
    }
    pub fn sense(mut self, s: Sense) -> El {
        self.sense = self.sense | s;
        self
    }
    /// What activating it does; implies click and focus.
    pub fn on(mut self, a: Action) -> El {
        self.action = Some(a);
        self.sense = self.sense | Sense::CLICK | Sense::FOCUS;
        self
    }
    pub fn cursor(mut self, c: Cursor) -> El {
        self.cursor = Some(c);
        self
    }
    /// Clips children to the box (`overflow-hidden`).
    pub fn clip(mut self) -> El {
        self.clip = true;
        self
    }
    /// Scrolls children vertically (`overflow-y-auto`); needs an id.
    pub fn scroll_y(mut self) -> El {
        self.scroll = true;
        self.clip = true;
        self.sense = self.sense | Sense::SCROLL;
        self
    }
    /// Draws the focus-visible ring with corner radius `r` when focused by
    /// keyboard.
    pub fn ring(mut self, r: f32) -> El {
        self.ring = Some(r);
        self
    }
    pub fn overlay(mut self, el: El, place: Place, gap: f32) -> El {
        self.overlays.push(Overlay {
            el,
            place,
            gap,
            owner_width: false,
        });
        self
    }
    /// An overlay at least as wide as this element.
    pub fn dropdown(mut self, el: El, place: Place, gap: f32) -> El {
        self.overlays.push(Overlay {
            el,
            place,
            gap,
            owner_width: true,
        });
        self
    }
    /// Marks the element as a tooltip's owner: [`crate::Ui::tooltip_shown`]
    /// turns true after the pointer rests on it. Needs an id.
    pub fn tooltip_owner(mut self) -> El {
        self.tip = true;
        self.sense = self.sense | Sense::HOVER;
        self
    }
    /// Exposes the box to assistive technology.
    pub fn role(mut self, role: Role, label: impl Into<String>) -> El {
        self.a11y.role = Some(role);
        self.a11y.label = Some(label.into());
        self
    }
    /// A role with no label of its own (its children say it).
    pub fn role_only(mut self, role: Role) -> El {
        self.a11y.role = Some(role);
        self
    }
    /// Keeps a text element out of the accessibility tree (its parent's
    /// label already says it).
    pub fn silent(mut self) -> El {
        self.a11y.role = None;
        self.a11y.label = None;
        self
    }
    pub fn value(mut self, v: impl Into<String>) -> El {
        self.a11y.value = Some(v.into());
        self
    }
    pub fn description(mut self, d: impl Into<String>) -> El {
        self.a11y.description = Some(d.into());
        self
    }
    pub fn shortcut(mut self, s: impl Into<String>) -> El {
        self.a11y.shortcut = Some(s.into());
        self
    }
    pub fn selected(mut self, s: bool) -> El {
        self.a11y.selected = Some(s);
        self
    }
    pub fn toggled(mut self, t: bool) -> El {
        self.a11y.toggled = Some(t);
        self
    }
    pub fn expanded(mut self, e: bool) -> El {
        self.a11y.expanded = Some(e);
        self
    }
    pub fn has_popup(mut self) -> El {
        self.a11y.has_popup = true;
        self
    }
    pub fn vertical(mut self, v: bool) -> El {
        self.a11y.vertical = Some(v);
        self
    }
    /// Greys the element out of interaction: no clicks, focus or hover.
    pub fn disabled(mut self, d: bool) -> El {
        self.a11y.disabled = d;
        if d {
            self.sense = Sense::NONE;
            self.action = None;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_and_distinct() {
        assert_eq!(Id::new("composer"), Id::new("composer"));
        assert_ne!(Id::new("composer"), Id::new("composer2"));
        let m = Id::new("menu");
        assert_eq!(m.child(3), m.child(3));
        assert_ne!(m.child(3), m.child(4));
        assert_ne!(m.child(3).0, 0, "never the window's node id");
    }

    #[test]
    fn sense_composes() {
        let s = Sense::CLICK | Sense::FOCUS;
        assert!(s.has(Sense::HOVER));
        assert!(s.has(Sense::FOCUS));
        assert!(!s.has(Sense::TEXT));
        let d = div().on(Action::Click).disabled(true);
        assert_eq!(d.sense, Sense::NONE);
        assert!(d.action.is_none());
    }
}
