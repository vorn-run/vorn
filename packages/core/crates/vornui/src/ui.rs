//! [`Ui`]: one window's worth of UI. Each frame the app builds an element
//! tree, [`Ui::layout`] lays it out with taffy, paints it into the scene,
//! records where interactive boxes landed and builds the accessibility
//! tree; [`Ui::render_offscreen`] or the window then draws the scene.

use std::collections::HashMap;
use std::time::Instant;

use accesskit::{HasPopup, Node, NodeId, Orientation, Role, Toggled, TreeId, TreeInfo, TreeUpdate};
use taffy::prelude::{AvailableSpace, Dimension, Size, TaffyTree};
use taffy::{NodeId as LayoutNode, Overflow, Point};

use crate::atlas::Atlases;
use crate::edit::{Clipboard, MemoryClipboard, TextEdit};
use crate::element::{Action, Content, EditLook, El, Id, Overlay, Place, Sense};
use crate::gpu::Gpu;
use crate::interact::{EditGeom, Hit, State};
use crate::render::{read_texture, Offscreen, RenderMode, Renderer, OFFSCREEN_FORMAT};
use crate::scene::{Rect, Rgba, Scene};
use crate::text::{TextStyle, TextSystem};
use crate::{a11y, theme};

/// What a laid-out frame leaves for the app: where its custom boxes landed
/// (physical pixels) and the accessibility tree.
pub struct Laid {
    pub customs: Vec<(u64, Rect)>,
    pub tree: TreeUpdate,
}

/// The node of the window itself in the accessibility tree.
pub const ROOT_NODE: NodeId = NodeId(0);

/// Accessibility content of a custom box: role, label, value.
pub type CustomA11y = (Role, String, String);

/// Fonts and sizes a [`Ui`] starts with.
#[derive(Clone, Copy, Debug)]
pub struct UiConfig {
    pub scale: f32,
    pub ui_font: &'static str,
    pub mono_font: &'static str,
    pub term_size: f32,
}

impl UiConfig {
    /// The faces the app's CSS stacks resolve to on this platform
    /// (`-apple-system, "Segoe UI", Roboto, sans-serif` and the terminal's
    /// monospace).
    pub fn system(scale: f32, term_size: f32) -> UiConfig {
        let (ui_font, mono_font) = if cfg!(target_os = "macos") {
            ("Helvetica Neue", "Menlo")
        } else if cfg!(windows) {
            ("Segoe UI", "Consolas")
        } else {
            ("DejaVu Sans", "DejaVu Sans Mono")
        };
        UiConfig {
            scale,
            ui_font,
            mono_font,
            term_size,
        }
    }
}

struct Image {
    w: u32,
    h: u32,
    rgba: Vec<u8>,
    uv: Option<[f32; 4]>,
}

pub struct Ui {
    /// `None` when there is no adapter at all: frames are CPU-only and can
    /// only be read back, not shown.
    pub gpu: Option<Gpu>,
    pub atlases: Atlases,
    pub text: TextSystem,
    pub scene: Scene,
    /// Custom boxes' accessibility content, set by the app before layout.
    pub custom_a11y: HashMap<u64, CustomA11y>,
    pub(crate) renderer: Renderer,
    pub(crate) st: State,
    pub(crate) clipboard: Box<dyn Clipboard>,
    clear: Rgba,
    icons: HashMap<(u64, u32), Option<[f32; 4]>>,
    images: Vec<Image>,
    anon: u64,
}

enum Measure<'a> {
    Text {
        text: &'a str,
        style: TextStyle,
        wrap: bool,
    },
    Edit {
        line: f32,
    },
}

/// Per-layout scratch: the taffy tree, overlays waiting to be placed, and
/// the clip hits are cut to.
struct Cx<'a> {
    taffy: TaffyTree<Measure<'a>>,
    overlays: Vec<(&'a Overlay, Rect)>,
    customs: Vec<(u64, Rect)>,
    nodes: Vec<(NodeId, Node)>,
    clip: Rect,
    layer: u32,
    focus_seen: bool,
}

impl Ui {
    /// A UI drawing to targets of `format` with `gpu` (or the CPU alone).
    pub fn new(
        gpu: Option<Gpu>,
        mode: RenderMode,
        format: wgpu::TextureFormat,
        cfg: &UiConfig,
    ) -> Ui {
        let (renderer, atlases) = Renderer::new(mode, gpu.as_ref(), format);
        let now = Instant::now();
        Ui {
            gpu,
            atlases,
            text: TextSystem::new(cfg.scale, cfg.ui_font, cfg.mono_font, cfg.term_size),
            scene: Scene::default(),
            custom_a11y: HashMap::new(),
            renderer,
            st: State::new(now),
            clipboard: Box::new(MemoryClipboard::default()),
            clear: Rgba::TRANSPARENT,
            icons: HashMap::new(),
            images: Vec::new(),
            anon: 0,
        }
    }

    /// A UI for offscreen frames: a headless adapter unless `mode` is CPU
    /// or there is none.
    pub fn headless(mode: RenderMode, cfg: &UiConfig) -> Ui {
        let gpu = match mode {
            RenderMode::Cpu => None,
            RenderMode::Auto | RenderMode::Gpu => Gpu::headless().ok(),
        };
        Ui::new(gpu, mode, OFFSCREEN_FORMAT, cfg)
    }

    pub fn scale(&self) -> f32 {
        self.text.scale
    }

    /// Moves to another display density.
    pub fn set_scale(&mut self, scale: f32) {
        self.text.set_scale(scale);
        self.reset_atlases();
    }

    pub fn is_cpu(&self) -> bool {
        self.renderer.is_cpu()
    }

    /// What draws the frames, for logs and reports.
    pub fn renderer_name(&self) -> String {
        let adapter = self
            .gpu
            .as_ref()
            .map_or_else(|| "no adapter".to_owned(), Gpu::adapter_name);
        if self.is_cpu() {
            format!("cpu raster (present: {adapter})")
        } else {
            adapter
        }
    }

    pub fn set_clipboard(&mut self, c: Box<dyn Clipboard>) {
        self.clipboard = c;
    }

    /// Registers an RGBA image (straight alpha) for [`crate::image`] elements.
    pub fn image(&mut self, w: u32, h: u32, rgba: &[u8]) -> Option<u32> {
        if rgba.len() < (w * h * 4) as usize {
            return None;
        }
        let uv = self.atlases.insert_color(w, h, rgba);
        uv?;
        self.images.push(Image {
            w,
            h,
            rgba: rgba.to_vec(),
            uv,
        });
        Some(self.images.len() as u32 - 1)
    }

    /// Starts a frame on an empty scene.
    pub fn begin(&mut self, clear: Rgba) {
        self.begin_at(clear, Instant::now());
    }

    /// Starts a frame at `now`: tests drive animations and timers with it.
    pub fn begin_at(&mut self, clear: Rgba, now: Instant) {
        self.st.now = now;
        self.scene.clear();
        self.clear = clear;
    }

    /// Lays `root` out in a `size` (logical pixels) window and paints it.
    /// Overlays (menus, tooltips) go in layers above; afterwards the scene
    /// records into the window's layer again, so what the app paints into
    /// its custom boxes stays under them.
    pub fn layout(&mut self, root: El, size: (f32, f32)) -> Laid {
        let mut cx = Cx {
            taffy: TaffyTree::new(),
            overlays: Vec::new(),
            customs: Vec::new(),
            nodes: Vec::new(),
            clip: Rect::EVERYTHING,
            layer: 0,
            focus_seen: false,
        };
        self.st.hits.clear();
        self.st.focus_order.clear();
        self.st.groups.clear();
        self.st.ime = None;
        self.anon = 0;
        let main_layer = self.scene.current_layer();
        let node = self.lay(&mut cx, &root, None, size, AvailableSpace::Definite);
        let mut root_children = Vec::new();
        self.paint(
            &mut cx,
            node,
            &root,
            (0.0, 0.0),
            Rect::new(0.0, 0.0, size.0, size.1),
            &mut root_children,
        );
        let mut i = 0;
        while let Some(&(ov, host)) = cx.overlays.get(i) {
            i += 1;
            self.place_overlay(&mut cx, ov, host, size, &mut root_children);
        }
        self.scene.set_layer(main_layer);
        self.after_layout(&cx);
        let k = self.scale();
        let mut win = Node::new(Role::Window);
        win.set_label("Vorn");
        win.set_children(root_children);
        win.set_bounds(accesskit::Rect::new(
            0.0,
            0.0,
            f64::from(size.0 * k),
            f64::from(size.1 * k),
        ));
        let focus = match self.st.focus {
            Some(f) if cx.focus_seen => NodeId(f.0),
            _ => ROOT_NODE,
        };
        let mut nodes = cx.nodes;
        nodes.insert(0, (ROOT_NODE, win));
        Laid {
            customs: cx.customs,
            tree: TreeUpdate {
                nodes,
                tree: Some(TreeInfo::new(ROOT_NODE)),
                tree_id: TreeId::ROOT,
                focus,
            },
        }
    }

    /// Builds and computes the layout of `el` as a root.
    fn lay<'a>(
        &mut self,
        cx: &mut Cx<'a>,
        el: &'a El,
        min_w: Option<f32>,
        size: (f32, f32),
        space: fn(f32) -> AvailableSpace,
    ) -> LayoutNode {
        let mut scrolls = Vec::new();
        let node = build(&mut cx.taffy, el, &mut scrolls);
        if let Some(w) = min_w {
            if let Ok(s) = cx.taffy.style(node) {
                let mut s = s.clone();
                let own = if s.min_size.width.tag() == taffy::CompactLength::LENGTH_TAG {
                    s.min_size.width.value()
                } else {
                    0.0
                };
                s.min_size.width = Dimension::length(own.max(w));
                let _ = cx.taffy.set_style(node, s);
            }
        }
        let avail = Size {
            width: space(size.0),
            height: space(size.1),
        };
        self.compute(&mut cx.taffy, node, avail);
        // `overflow-y: auto` takes its scrollbar's width only when it
        // overflows, which needs a second pass once that is known.
        let mut again = false;
        for n in scrolls {
            let Ok(l) = cx.taffy.layout(n) else { continue };
            if l.content_size.height > l.size.height + 0.5 {
                if let Ok(s) = cx.taffy.style(n) {
                    let mut s = s.clone();
                    s.scrollbar_width = theme::scrollbar::WIDTH;
                    let _ = cx.taffy.set_style(n, s);
                    again = true;
                }
            }
        }
        if again {
            self.compute(&mut cx.taffy, node, avail);
        }
        node
    }

    fn compute(
        &mut self,
        t: &mut TaffyTree<Measure<'_>>,
        node: LayoutNode,
        avail: Size<AvailableSpace>,
    ) {
        let text = &mut self.text;
        // Layout only fails on unknown node ids, which `build` never makes.
        let _ = t.compute_layout_with_measure(node, avail, |known, avail, _, ctx, _| {
            let Some(m) = ctx else {
                return Size::ZERO;
            };
            match m {
                Measure::Text {
                    text: s,
                    style,
                    wrap,
                } => {
                    let max_w = wrap.then(|| {
                        known.width.unwrap_or(match avail.width {
                            AvailableSpace::Definite(w) => w,
                            _ => f32::MAX,
                        })
                    });
                    let sh = text.shape(s, *style, max_w);
                    let k = text.scale;
                    Size {
                        width: known.width.unwrap_or(sh.w / k),
                        height: known.height.unwrap_or(sh.h / k),
                    }
                }
                Measure::Edit { line } => Size {
                    width: known.width.unwrap_or(0.0),
                    height: known.height.unwrap_or(*line),
                },
            }
        });
    }

    fn place_overlay<'a>(
        &mut self,
        cx: &mut Cx<'a>,
        ov: &'a Overlay,
        host: Rect,
        win: (f32, f32),
        a11y_parent: &mut Vec<NodeId>,
    ) {
        let min_w = ov.owner_width.then_some(host.w);
        let node = self.lay(cx, &ov.el, min_w, win, |_| AvailableSpace::MaxContent);
        let Ok(l) = cx.taffy.layout(node) else {
            return;
        };
        let (w, h) = (l.size.width, l.size.height);
        let g = ov.gap;
        let (mut x, mut y) = match ov.place {
            Place::Above => (host.x + (host.w - w) / 2.0, host.y - g - h),
            Place::Below => (host.x + (host.w - w) / 2.0, host.bottom() + g),
            Place::BelowStart => (host.x, host.bottom() + g),
            Place::AboveStart => (host.x, host.y - g - h),
            Place::Right => (host.right() + g, host.y + (host.h - h) / 2.0),
        };
        // Flip to the other side when there is no room, then keep it on
        // screen.
        const MARGIN: f32 = 4.0;
        if y + h > win.1 - MARGIN && host.y - g - h >= MARGIN {
            y = host.y - g - h;
        } else if y < MARGIN && host.bottom() + g + h <= win.1 - MARGIN {
            y = host.bottom() + g;
        }
        x = x.min(win.0 - MARGIN - w).max(MARGIN);
        y = y.min(win.1 - MARGIN - h).max(MARGIN);
        self.scene.layer();
        cx.layer += 1;
        let saved = std::mem::replace(&mut cx.clip, Rect::EVERYTHING);
        let origin = (x - l.location.x, y - l.location.y);
        self.paint(
            cx,
            node,
            &ov.el,
            origin,
            Rect::new(0.0, 0.0, win.0, win.1),
            a11y_parent,
        );
        cx.clip = saved;
    }

    #[allow(clippy::too_many_lines)]
    fn paint<'a>(
        &mut self,
        cx: &mut Cx<'a>,
        node: LayoutNode,
        el: &'a El,
        origin: (f32, f32),
        parent: Rect,
        a11y_parent: &mut Vec<NodeId>,
    ) {
        let Ok(l) = cx.taffy.layout(node).copied() else {
            return;
        };
        let k = self.scale();
        let (x, y) = (origin.0 + l.location.x, origin.1 + l.location.y);
        let rl = Rect::new(x, y, l.size.width, l.size.height);
        let r = rl.scaled(k);
        let focused = el.id.is_some() && self.st.focus == el.id;
        let ring = focused && self.st.focus_visible && el.ring.is_some();
        if el.opacity < 1.0 {
            self.scene.push_opacity(el.opacity);
        }
        if let Some((s, (dx, dy))) = el.transform {
            self.scene
                .push_transform(s, (r.x + r.w / 2.0, r.y + r.h / 2.0), (dx * k, dy * k));
        }
        let radius = if ring {
            theme::focus::RADIUS
        } else {
            el.radius
        };
        if el.bg.is_some() || el.border.is_some() {
            let fill = el.bg.unwrap_or_default();
            let border = el.border.map(|(w, c)| (w * k, c));
            self.scene.quad(r, fill, radius * k, border);
        }
        if let Some((w, c)) = el.top_rule {
            self.scene
                .quad(Rect::new(x, y, rl.w, w).scaled(k), c, 0.0, None);
        }
        if let Some(id) = el.id {
            if el.sense != Sense::NONE || el.action.is_some() || el.tip {
                let (sx, sy) = el.slop;
                let target = Rect::new(x - sx, y - sy, rl.w + 2.0 * sx, rl.h + 2.0 * sy);
                self.st.hits.push(Hit {
                    id,
                    rect: target.intersect(&cx.clip),
                    sense: el.sense,
                    action: el.action,
                    cursor: el.cursor,
                    layer: cx.layer,
                    tip: el.tip,
                    parent,
                    vertical: el.a11y.vertical.unwrap_or(false),
                });
            }
            if el.sense.has(Sense::FOCUS) {
                self.st.focus_order.push(id);
            }
            if let Some(Action::Select { group, index }) = el.action {
                let items = self.st.groups.entry(group).or_default();
                if items.len() <= index {
                    items.resize(index + 1, (id, false));
                }
                items[index] = (id, el.a11y.selected == Some(true));
            }
            if focused {
                cx.focus_seen = true;
            }
        }
        let inner = Rect::new(
            x + l.border.left + l.padding.left,
            y + l.border.top + l.padding.top,
            (l.size.width
                - l.border.left
                - l.border.right
                - l.padding.left
                - l.padding.right
                - l.scrollbar_size.width)
                .max(0.0),
            (l.size.height - l.border.top - l.border.bottom - l.padding.top - l.padding.bottom)
                .max(0.0),
        );
        let mut edit_runs = None;
        match &el.content {
            Content::None => {}
            Content::Text {
                text,
                style,
                color,
                wrap,
                truncate,
            } => {
                let sh = if *truncate {
                    self.text.shape_truncated(text, *style, inner.w)
                } else {
                    self.text.shape(text, *style, wrap.then_some(inner.w))
                };
                self.text.draw(
                    &mut self.scene,
                    &mut self.atlases,
                    &sh,
                    (inner.x * k, inner.y * k),
                    *color,
                );
            }
            Content::Icon { svg, size, color } => {
                let px = (size * k).round() as u32;
                if let Some(uv) = self.icon_raster(svg, px) {
                    let ir = Rect::new((inner.x * k).round(), (inner.y * k).round(), uv[2], uv[3]);
                    self.scene.sprite(ir, uv, *color, false);
                }
            }
            Content::Image { id, color } => {
                if let Some(uv) = self.images.get(*id as usize).and_then(|i| i.uv) {
                    self.scene.sprite(r, uv, *color, true);
                }
            }
            Content::Edit(look) => {
                if let Some(id) = el.id {
                    edit_runs = Some(self.paint_edit(id, look, inner, focused));
                }
            }
            Content::Custom(id) => cx.customs.push((*id, r)),
        }
        let clip_box = Rect::new(
            x + l.border.left,
            y + l.border.top,
            rl.w - l.border.left - l.border.right,
            rl.h - l.border.top - l.border.bottom,
        );
        let saved_clip = cx.clip;
        if el.clip {
            self.scene.push_clip(clip_box.scaled(k));
            cx.clip = cx.clip.intersect(&clip_box);
        }
        let mut scrolled = 0.0;
        let mut max = 0.0;
        if el.scroll {
            if let Some(id) = el.id {
                max = (l.content_size.height - l.size.height + l.border.top + l.border.bottom)
                    .max(0.0);
                let s = self.st.scroll.entry(id).or_default();
                s.max = max;
                s.offset = s.offset.clamp(0.0, max);
                scrolled = s.offset;
            }
        }
        let custom = match el.content {
            Content::Custom(id) => self.custom_a11y.get(&id).cloned(),
            _ => None,
        };
        let edit_role = match &el.content {
            Content::Edit(look) if look.multiline => Some(Role::MultilineTextInput),
            Content::Edit(_) => Some(Role::TextInput),
            _ => None,
        };
        let role = el.a11y.role.or(custom.as_ref().map(|a| a.0)).or(edit_role);
        let mut own_children = Vec::new();
        for (i, ce) in el.children.iter().enumerate() {
            let Ok(c) = cx.taffy.child_at_index(node, i) else {
                break;
            };
            let target = if role.is_some() {
                &mut own_children
            } else {
                &mut *a11y_parent
            };
            self.paint(cx, c, ce, (x, y - scrolled), rl, target);
        }
        if max > 0.0 {
            let view = clip_box.h;
            let content = view + max;
            let th = (view * view / content).max(20.0).min(view);
            let ty = clip_box.y + (view - th) * scrolled / max;
            let track = Rect::new(
                clip_box.right() - theme::scrollbar::WIDTH,
                ty,
                theme::scrollbar::WIDTH,
                th,
            );
            let over = self.st.pointer.is_some_and(|p| track.contains(p.0, p.1));
            let c = if over {
                theme::scrollbar::THUMB_HOVER
            } else {
                theme::scrollbar::THUMB
            };
            self.scene
                .quad(track.scaled(k), c, theme::scrollbar::RADIUS * k, None);
        }
        if el.clip {
            self.scene.pop_clip();
            cx.clip = saved_clip;
        }
        if ring {
            let d = theme::focus::OFFSET + theme::focus::WIDTH;
            self.scene.quad(
                rl.outset(d).scaled(k),
                Rgba::TRANSPARENT,
                (theme::focus::RADIUS + d) * k,
                Some((theme::focus::WIDTH * k, theme::focus::RING)),
            );
        }
        for ov in &el.overlays {
            cx.overlays.push((ov, rl));
        }
        if el.transform.is_some() {
            self.scene.pop_transform();
        }
        if el.opacity < 1.0 {
            self.scene.pop_opacity();
        }
        let Some(role) = role else {
            return;
        };
        let id = match el.id {
            Some(i) => NodeId(i.0),
            None => {
                self.anon += 1;
                NodeId((1 << 63) | (self.anon << 1))
            }
        };
        let mut n = Node::new(role);
        let a = &el.a11y;
        if let Some(v) = &a.label {
            n.set_label(v.as_str());
        }
        if let Some(v) = &a.value {
            n.set_value(v.as_str());
        }
        if let Some(v) = &a.description {
            n.set_description(v.as_str());
        }
        if let Some(v) = &a.placeholder {
            n.set_placeholder(v.as_str());
        }
        if let Some(v) = &a.shortcut {
            n.set_keyboard_shortcut(v.as_str());
        }
        if let Some(v) = a.selected {
            n.set_selected(v);
        }
        if let Some(v) = a.toggled {
            n.set_toggled(if v { Toggled::True } else { Toggled::False });
        }
        if let Some(v) = a.expanded {
            n.set_expanded(v);
        }
        if let Some(v) = a.vertical {
            n.set_orientation(if v {
                Orientation::Vertical
            } else {
                Orientation::Horizontal
            });
        }
        if a.disabled {
            n.set_disabled();
        }
        if a.has_popup {
            n.set_has_popup(HasPopup::Menu);
        }
        if let Some((_, label, value)) = custom {
            n.set_label(label);
            n.set_value(value);
        }
        if el.action.is_some() {
            n.add_action(accesskit::Action::Click);
        }
        if el.sense.has(Sense::FOCUS) {
            n.add_action(accesskit::Action::Focus);
        }
        if let Some(Action::ToggleMenu(m)) = el.action {
            n.add_action(if self.menu_open(m) {
                accesskit::Action::Collapse
            } else {
                accesskit::Action::Expand
            });
        }
        if el.scroll && max > 0.0 {
            n.set_scroll_y(f64::from(scrolled));
            n.set_scroll_y_min(0.0);
            n.set_scroll_y_max(f64::from(max));
            n.add_action(accesskit::Action::ScrollDown);
            n.add_action(accesskit::Action::ScrollUp);
        }
        if let (Content::Edit(look), Some(eid)) = (&el.content, el.id) {
            let text = self.edit_text(eid).to_owned();
            n.set_value(text);
            if a.placeholder.is_none() && !look.placeholder.is_empty() {
                n.set_placeholder(look.placeholder.as_str());
            }
            n.add_action(accesskit::Action::SetValue);
            n.add_action(accesskit::Action::SetTextSelection);
            if let Some((runs, sel)) = edit_runs.take() {
                own_children.extend(runs.iter().map(|r| r.0));
                cx.nodes.extend(runs);
                n.set_text_selection(sel);
            }
        }
        n.set_bounds(accesskit::Rect::new(
            f64::from(r.x),
            f64::from(r.y),
            f64::from(r.x + r.w),
            f64::from(r.y + r.h),
        ));
        if !own_children.is_empty() {
            n.set_children(own_children);
        }
        cx.nodes.push((id, n));
        a11y_parent.push(id);
    }

    /// Draws edit field `id` into `inner` (logical) and answers its text
    /// runs for the accessibility tree.
    fn paint_edit(
        &mut self,
        id: Id,
        look: &EditLook,
        inner: Rect,
        focused: bool,
    ) -> (Vec<(NodeId, Node)>, accesskit::TextSelection) {
        let k = self.scale();
        let lit = focused && self.caret_lit();
        let e = self
            .st
            .edits
            .entry(id)
            .or_insert_with(|| TextEdit::new("", look.multiline));
        e.set_multiline(look.multiline);
        let (shown, pre) = e.display();
        let caret_at = if e.preedit().is_empty() {
            e.caret()
        } else {
            pre.end
        };
        let sel = e.selection();
        let sh = self
            .text
            .shape(&shown, look.style, look.multiline.then_some(inner.w));
        // Keep the caret in view.
        let (cx_, line) = sh.caret(caret_at);
        let e = self.st.edits.get_mut(&id).expect("inserted above");
        let line_h = look.style.line_height;
        if look.multiline {
            let top = sh.lines.get(line).map_or(0.0, |l| l.top) / k;
            if top < e.scroll_y {
                e.scroll_y = top;
            } else if top + line_h > e.scroll_y + inner.h {
                e.scroll_y = top + line_h - inner.h;
            }
            e.scroll_y = e.scroll_y.clamp(0.0, (sh.h / k - inner.h).max(0.0));
        } else {
            let cxl = cx_ / k;
            let sx = &mut e.scroll_y; // a single line scrolls sideways
            if cxl < *sx {
                *sx = cxl;
            } else if cxl > *sx + inner.w - 1.0 {
                *sx = cxl - inner.w + 1.0;
            }
            *sx = sx.clamp(0.0, (sh.w / k - inner.w + 1.0).max(0.0));
        }
        let (ox, oy) = if look.multiline {
            (inner.x * k, (inner.y - e.scroll_y) * k)
        } else {
            ((inner.x - e.scroll_y) * k, inner.y * k)
        };
        let anchor_caret = (e.anchor(), e.caret());
        self.scene.push_clip(inner.scaled(k));
        if !sel.is_empty() {
            for (i, l) in sh.lines.iter().enumerate() {
                let start = sel.start.max(l.start);
                let end = sel.end.min(a11y::run_end(&sh, i, shown.len()));
                if start >= end {
                    continue;
                }
                let x0 = sh.caret(start).0;
                let x1 = if end > l.end {
                    l.w + 4.0 * k
                } else {
                    sh.caret(end).0
                };
                let rr = Rect::new(ox + x0, oy + l.top, (x1 - x0).max(1.0), l.height);
                self.scene.quad(rr, theme::color::SELECTION, 0.0, None);
            }
        }
        if shown.is_empty() && !look.placeholder.is_empty() {
            let ph = self.text.shape(
                &look.placeholder,
                look.style,
                look.multiline.then_some(inner.w),
            );
            self.text.draw(
                &mut self.scene,
                &mut self.atlases,
                &ph,
                (inner.x * k, inner.y * k),
                look.placeholder_color,
            );
        }
        self.text.draw(
            &mut self.scene,
            &mut self.atlases,
            &sh,
            (ox, oy),
            look.color,
        );
        if !pre.is_empty() {
            for (i, l) in sh.lines.iter().enumerate() {
                let start = pre.start.max(l.start);
                let end = pre.end.min(a11y::run_end(&sh, i, shown.len()));
                if start >= end {
                    continue;
                }
                let x0 = sh.caret(start).0;
                let x1 = sh.caret(end).0.max(x0 + 1.0);
                let ul = Rect::new(
                    ox + x0,
                    oy + l.top + l.height - k.round().max(1.0),
                    x1 - x0,
                    k.round().max(1.0),
                );
                self.scene.quad(ul, look.color, 0.0, None);
            }
        }
        let caret_line = sh.lines.get(line).copied();
        let (top, h) = caret_line.map_or((0.0, line_h * k), |l| (l.top, l.height));
        let caret = Rect::new((ox + cx_).round(), oy + top, k.round().max(1.0), h);
        if focused {
            self.st.ime = Some(Rect::new(
                caret.x / k,
                caret.y / k,
                caret.w / k,
                caret.h / k,
            ));
        }
        if lit && sel.is_empty() {
            self.scene.quad(caret, look.color, 0.0, None);
        }
        self.scene.pop_clip();
        let runs = a11y::text_runs(id, &shown, &sh, (ox, oy), anchor_caret);
        self.st.geoms.insert(
            id,
            EditGeom {
                shaped: sh,
                origin: (ox, oy),
            },
        );
        runs
    }

    /// Fixes up state the new layout invalidated.
    fn after_layout(&mut self, cx: &Cx<'_>) {
        if !cx.focus_seen && self.st.focus.is_some_and(|f| self.st.hit(f).is_none()) {
            self.st.focus = None;
        }
        if let Some(m) = self.st.menu {
            let trigger_alive = self.st.hit(m.trigger).is_some();
            if !trigger_alive {
                self.st.menu = None;
            } else if m.focus_in {
                if let Some(items) = self.st.groups.get(&m.id) {
                    let pick = items.iter().find(|i| i.1).or(items.first()).map(|i| i.0);
                    if let Some(p) = pick {
                        self.st.focus = Some(p);
                        self.st.focus_visible = true;
                    }
                    if let Some(menu) = &mut self.st.menu {
                        menu.focus_in = false;
                    }
                }
            }
        }
        if self.st.drag.is_some_and(|d| self.st.hit(d).is_none()) {
            self.st.drag = None;
        }
        self.st
            .geoms
            .retain(|id, _| self.st.hits.iter().any(|h| h.id == *id));
        self.st.rehover();
        self.st.gc();
    }

    fn icon_raster(&mut self, svg: &str, px: u32) -> Option<[f32; 4]> {
        let key = (Id::new(svg).0, px);
        if let Some(hit) = self.icons.get(&key) {
            return *hit;
        }
        let uv = rasterize_svg(svg, px).and_then(|mask| self.atlases.insert_mask(px, px, &mask));
        if uv.is_some() || !self.atlases.full {
            self.icons.insert(key, uv);
        }
        uv
    }

    /// Empties the atlases and everything that pointed into them; images
    /// are uploaded again.
    fn reset_atlases(&mut self) {
        self.atlases.reset();
        self.text.clear_rasters();
        self.icons.clear();
        for img in &mut self.images {
            img.uv = self.atlases.insert_color(img.w, img.h, &img.rgba);
        }
        self.renderer.invalidate();
    }

    /// A target to draw frames into without a window.
    pub fn offscreen(&self, size: (u32, u32)) -> Offscreen {
        Offscreen::new(self.gpu.as_ref(), size)
    }

    /// Draws the scene into `target`. Returns `false` when the frame was
    /// not drawn because the atlases filled up and were emptied: build and
    /// lay the frame out again.
    pub fn render_offscreen(&mut self, target: &Offscreen) -> bool {
        self.render_target(target.target(), target.size, true)
    }

    pub(crate) fn render_target(
        &mut self,
        target: Option<(&wgpu::Texture, &wgpu::TextureView)>,
        size: (u32, u32),
        keeps: bool,
    ) -> bool {
        if self.atlases.full {
            self.reset_atlases();
            return false;
        }
        self.renderer.draw(
            self.gpu.as_ref(),
            &self.scene,
            &self.atlases,
            self.clear,
            target,
            size,
            keeps,
        );
        self.text.trim();
        true
    }

    /// The pixels of the last frame drawn into `target`, as RGBA rows.
    pub fn read(&mut self, target: &Offscreen) -> Result<Vec<u8>, String> {
        if let Some(r) = self.renderer.cpu_frame() {
            if r.size() == target.size {
                let mut px = r.bytes().to_vec();
                if r.is_bgra() {
                    px.chunks_exact_mut(4).for_each(|p| p.swap(0, 2));
                }
                return Ok(px);
            }
        }
        match (self.gpu.as_ref(), target.target()) {
            (Some(g), Some((t, _))) => read_texture(g, t, target.size),
            _ => Err("nothing drawn".into()),
        }
    }

    /// Writes the last frame drawn into `target` to a PNG at `path`.
    pub fn save_png(&mut self, target: &Offscreen, path: &str) -> Result<(), String> {
        let px = self.read(target)?;
        write_png(path, target.size, &px)
    }
}

fn build<'a>(
    t: &mut TaffyTree<Measure<'a>>,
    el: &'a El,
    scrolls: &mut Vec<LayoutNode>,
) -> LayoutNode {
    let mut style = el.style.clone();
    if el.clip {
        style.overflow = Point {
            x: Overflow::Hidden,
            y: Overflow::Hidden,
        };
    }
    if el.scroll {
        style.overflow.y = Overflow::Scroll;
        style.scrollbar_width = 0.0;
    }
    let ctx = match &el.content {
        Content::Text {
            text,
            style: ts,
            wrap,
            ..
        } => Some(Measure::Text {
            text,
            style: *ts,
            wrap: *wrap,
        }),
        Content::Edit(look) => Some(Measure::Edit {
            line: look.style.line_height,
        }),
        _ => None,
    };
    // Taffy only fails on unknown node ids, which this never passes.
    let node = match ctx {
        Some(m) => t.new_leaf_with_context(style, m),
        None => {
            let kids: Vec<LayoutNode> = el.children.iter().map(|c| build(t, c, scrolls)).collect();
            t.new_with_children(style, &kids)
        }
    }
    .unwrap_or_else(|_| unreachable!("taffy fails only on unknown node ids"));
    if el.scroll {
        scrolls.push(node);
    }
    node
}

/// An SVG rendered to a `px`×`px` coverage mask.
pub fn rasterize_svg(svg: &str, px: u32) -> Option<Vec<u8>> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).ok()?;
    let mut pm = tiny_skia::Pixmap::new(px, px)?;
    let s = tree.size();
    let tf = tiny_skia::Transform::from_scale(px as f32 / s.width(), px as f32 / s.height());
    resvg::render(&tree, tf, &mut pm.as_mut());
    Some(pm.data().chunks(4).map(|p| p[3]).collect())
}

/// Decodes a PNG to straight-alpha RGBA.
pub fn decode_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut d = png::Decoder::new(std::io::Cursor::new(bytes));
    d.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = d.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; r.output_buffer_size().ok_or("png too large")?];
    let info = r.next_frame(&mut buf).map_err(|e| e.to_string())?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .chunks(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("indexed png not expanded".into()),
    };
    Ok((info.width, info.height, rgba))
}

pub fn write_png(path: &str, size: (u32, u32), rgba: &[u8]) -> Result<(), String> {
    if let Some(dir) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let f = std::fs::File::create(path).map_err(|e| format!("{path}: {e}"))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), size.0, size.1);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(rgba).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_masks_cover_their_strokes() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M2 12h20" stroke="#fff" stroke-width="4"/></svg>"##;
        let m = rasterize_svg(svg, 24).unwrap();
        assert_eq!(m.len(), 24 * 24);
        assert_eq!(m[12 * 24 + 12], 255);
        assert_eq!(m[0], 0);
    }

    #[test]
    fn png_round_trips() {
        let dir = std::env::temp_dir().join(format!("vornui-png-{}", std::process::id()));
        let path = dir.join("a.png");
        let px = [1u8, 2, 3, 4, 5, 6, 7, 8];
        write_png(path.to_str().unwrap(), (2, 1), &px).unwrap();
        let (w, h, back) = decode_png(&std::fs::read(&path).unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!((w, h, back.as_slice()), (2, 1, &px[..]));
    }
}
