//! [`Ui`]: one window's worth of UI. Each frame the app builds an element
//! tree, [`Ui::layout`] lays it out with taffy, paints it into the scene,
//! records where interactive boxes landed and builds the accessibility
//! tree; [`Ui::render_offscreen`] or the window then draws the scene.

use std::cell::Cell;
use std::collections::HashMap;
use web_time::Instant;

use accesskit::{HasPopup, Node, NodeId, Orientation, Role, Toggled, TreeId, TreeInfo, TreeUpdate};
use taffy::prelude::{AvailableSpace, Dimension, Size, TaffyTree};
use taffy::{NodeId as LayoutNode, Overflow, Point};

use crate::atlas::Atlases;
use crate::cells::{self, PaintList};
use crate::edit::{Clipboard, MemoryClipboard, TextEdit};
use crate::element::{Action, Content, EditLook, El, Id, Overlay, Place, Sense};
use crate::gpu::{adapter_name, instance_desc, Gpu};
use crate::interact::{EditGeom, Hit, Scroll, State};
use crate::render::{read_texture, Offscreen, RenderMode, Renderer, OFFSCREEN_FORMAT};
use crate::scene::{Rect, Rgba, Scene};
use crate::text::{TextStyle, TextSystem};
use crate::{a11y, icons, theme};

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

/// A CPU frame's bytes, its size and the rows that changed.
pub type CpuPixels<'a> = (&'a [u8], (u32, u32), &'a [(u32, u32)]);

/// Fonts and sizes a [`Ui`] starts with.
#[derive(Clone, Copy, Debug)]
pub struct UiConfig {
    pub scale: f32,
    pub ui_font: &'static str,
    pub mono_font: &'static str,
    pub term_size: f32,
}

impl UiConfig {
    /// The faces the theme's font stacks resolve to on this platform
    /// ([`crate::theme::font`]).
    pub fn system(scale: f32, term_size: f32) -> UiConfig {
        let (ui_font, mono_font) = (crate::theme::font::UI, crate::theme::font::MONO);
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

/// A picture [`Ui::picture`] keeps in the colour atlas while frames draw it.
struct Picture {
    uv: Option<[f32; 4]>,
    used: bool,
}

pub struct Ui {
    /// `None` when the CPU draws: frames stay in memory, to be read back
    /// or presented by a window without a device.
    pub gpu: Option<Gpu>,
    /// The adapter found, for reports, even when no device was opened on it.
    adapter: Option<String>,
    pub atlases: Atlases,
    pub text: TextSystem,
    pub scene: Scene,
    /// Custom boxes' accessibility content, set by the app before layout.
    pub custom_a11y: HashMap<u64, CustomA11y>,
    pub(crate) renderer: Renderer,
    pub(crate) st: State,
    pub(crate) clipboard: Box<dyn Clipboard>,
    clear: Rgba,
    /// See [`Ui::viewport`].
    pub(crate) viewport: (f32, f32),
    icons: HashMap<(u64, u32), Option<[f32; 4]>>,
    images: Vec<Image>,
    pictures: HashMap<u64, Picture>,
    /// A picture filled the atlas and the frame is being drawn again on an empty one: one that
    /// still does not fit is left out rather than emptying it again and again.
    pictures_redo: bool,
    anon: u64,
    /// The earliest frame a view asked for while building this one, for
    /// motion that is not a hover fade (a spinner, a pulse). A `Cell` so a
    /// view holding `&Ui` can ask.
    pub(crate) wake: Cell<Option<Instant>>,
    /// Set on a cells surface: layout is in cells and frames are recorded here.
    pub(crate) cells: Option<PaintList>,
    /// Set on a native surface: each layout records its semantic tree here.
    pub(crate) native: Option<crate::native::Node>,
    /// The scene layers the last layout's overlays (menus, popovers,
    /// tooltips) drew into; empty when none was open.
    pub(crate) overlays: std::ops::Range<usize>,
    /// Boxes this frame wants the system's glass under: device pixels and radius.
    pub(crate) glass: Vec<(Rect, f32)>,
    /// Icons the system draws in that glass: device-pixel box, SVG, colour.
    pub(crate) glass_icons: Vec<(Rect, std::rc::Rc<str>, Rgba)>,
    /// Labels the system draws in that glass: device-pixel box, text, size, colour.
    pub(crate) glass_labels: Vec<(Rect, std::rc::Rc<str>, f32, Rgba)>,
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
    /// Each overlay with its owner's box and the clip the owner had.
    overlays: Vec<(&'a Overlay, Rect, Rect)>,
    customs: Vec<(u64, Rect)>,
    nodes: Vec<(NodeId, Node)>,
    clip: Rect,
    layer: u32,
    /// Painting above the window's flow (an overlay or layered box): on
    /// cells its frames draw whole rather than sharing edges.
    float: bool,
    /// Attributes a selected box gives the text inside it on cells.
    emph: u8,
    focus_seen: bool,
    /// The transform painting is under, in logical pixels, so what is hit
    /// is where it is drawn.
    xf: Xf,
}

/// How close an overlay comes to the window's edges.
const MARGIN: f32 = 4.0;

/// Where a [`Place::Beside`] overlay `w` wide starts: right of `host`,
/// else left of it when only that side fits, else the roomier side.
fn beside_x(host: Rect, w: f32, gap: f32, win_w: f32) -> f32 {
    let right = host.right() + gap;
    let left = host.x - gap - w;
    let room_right = win_w - MARGIN - right;
    let room_left = host.x - gap - MARGIN;
    if room_right >= w || room_right >= room_left {
        right
    } else {
        left
    }
}

/// A border `w` logical pixels wide at scale `k`, in whole physical pixels
/// and never under one, so a 1px border is one crisp pixel at 1.5x too.
fn border_px(w: f32, k: f32) -> f32 {
    if w <= 0.0 {
        0.0
    } else {
        (w * k + 0.01).floor().max(1.0)
    }
}

/// Scale about `origin`, then shift: an element's `transform`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Xf {
    scale: f32,
    origin: (f32, f32),
    offset: (f32, f32),
}

impl Xf {
    const IDENTITY: Xf = Xf {
        scale: 1.0,
        origin: (0.0, 0.0),
        offset: (0.0, 0.0),
    };

    fn apply(&self, r: Rect) -> Rect {
        if *self == Xf::IDENTITY {
            return r;
        }
        let (ox, oy) = self.origin;
        Rect::new(
            ox + (r.x - ox) * self.scale + self.offset.0,
            oy + (r.y - oy) * self.scale + self.offset.1,
            r.w * self.scale,
            r.h * self.scale,
        )
    }
}

impl Ui {
    /// A UI drawing to targets of `format` with `gpu` (or the CPU alone).
    pub fn new(
        gpu: Option<Gpu>,
        mode: RenderMode,
        format: wgpu::TextureFormat,
        cfg: &UiConfig,
    ) -> Ui {
        let adapter = gpu.as_ref().map(Gpu::adapter_name);
        Ui::build(gpu, adapter, mode, format, cfg)
    }

    pub(crate) fn build(
        gpu: Option<Gpu>,
        adapter: Option<String>,
        mode: RenderMode,
        format: wgpu::TextureFormat,
        cfg: &UiConfig,
    ) -> Ui {
        let (renderer, atlases) = Renderer::new(mode, gpu.as_ref(), format);
        let gpu = gpu.filter(|_| !renderer.is_cpu());
        let now = Instant::now();
        Ui {
            gpu,
            adapter,
            atlases,
            text: TextSystem::new(cfg.scale, cfg.ui_font, cfg.mono_font, cfg.term_size),
            scene: Scene::default(),
            custom_a11y: HashMap::new(),
            renderer,
            st: State::new(now),
            clipboard: Box::new(MemoryClipboard::default()),
            clear: Rgba::TRANSPARENT,
            viewport: (0.0, 0.0),
            icons: HashMap::new(),
            images: Vec::new(),
            pictures: HashMap::new(),
            pictures_redo: false,
            anon: 0,
            wake: Cell::new(None),
            cells: None,
            native: None,
            overlays: 0..0,
            glass: Vec::new(),
            glass_icons: Vec::new(),
            glass_labels: Vec::new(),
        }
    }

    /// A UI on the Cells surface: lengths are laid out in terminal cells
    /// (one column by one row) and frames are recorded as a [`PaintList`]
    /// for a cells backend rather than shaped into pixels.
    pub fn cells(cfg: &UiConfig) -> Ui {
        let cfg = UiConfig { scale: 1.0, ..*cfg };
        let mut ui = Ui::build(
            None,
            None,
            RenderMode::Cpu,
            wgpu::TextureFormat::Bgra8Unorm,
            &cfg,
        );
        ui.text.cells = true;
        ui.cells = Some(PaintList::default());
        ui
    }

    /// Whether this UI lays out and draws in terminal cells.
    pub fn is_cells(&self) -> bool {
        self.cells.is_some()
    }

    /// Moves this UI to the Cells surface (`cells`) or to pixels at
    /// `scale`, keeping its state and images: a terminal changing how it
    /// draws keeps every image id the app holds.
    pub fn set_surface(&mut self, cells: bool, scale: f32) {
        self.text.cells = cells;
        self.cells = cells.then(PaintList::default);
        self.set_scale(if cells { 1.0 } else { scale });
    }

    /// The last frame's paint list, on a cells surface.
    pub fn paint_list(&self) -> Option<&PaintList> {
        self.cells.as_ref()
    }

    /// The paint list, for the app to add what it paints into its custom boxes.
    pub fn paint_list_mut(&mut self) -> Option<&mut PaintList> {
        self.cells.as_mut()
    }

    /// A UI for offscreen frames: a headless device unless `mode` is CPU,
    /// the adapter is a software one (with `Auto`), or there is none. CPU
    /// frames are BGRA, as a window's buffer takes them.
    pub fn headless(mode: RenderMode, cfg: &UiConfig) -> Ui {
        let cpu = |name| Ui::build(None, name, mode, wgpu::TextureFormat::Bgra8Unorm, cfg);
        if mode == RenderMode::Cpu {
            return cpu(None);
        }
        let instance = wgpu::Instance::new(instance_desc());
        let Ok(adapter) = Gpu::adapter(&instance, None) else {
            return cpu(None);
        };
        let name = Some(adapter_name(&adapter));
        if !mode.wants_device(&adapter) {
            return cpu(name);
        }
        match Gpu::open(instance, adapter) {
            Ok(gpu) => Ui::build(Some(gpu), name, mode, OFFSCREEN_FORMAT, cfg),
            Err(_) => cpu(name),
        }
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
        let adapter = self.adapter.as_deref().unwrap_or("no adapter");
        if self.is_cpu() {
            format!("cpu raster (adapter not used: {adapter})")
        } else {
            adapter.to_owned()
        }
    }

    pub fn set_clipboard(&mut self, c: Box<dyn Clipboard>) {
        self.clipboard = c;
    }

    /// The clipboard edits cut to and paste from, for the app's own paste.
    pub fn clipboard(&mut self) -> &mut dyn Clipboard {
        self.clipboard.as_mut()
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

    /// Draws `src` (x, y, width, height in picture pixels) of a straight-alpha RGBA picture
    /// `size` into `dst`, uploading it the first frame `key` is drawn: a key names one picture's
    /// pixels for as long as frames draw it. Senders keep pictures small (vornd sends at most
    /// 1024 px a side); one the atlas cannot take is not drawn.
    pub fn picture(&mut self, key: u64, size: (u32, u32), rgba: &[u8], src: [f32; 4], dst: Rect) {
        self.picture_rounded(key, size, rgba, src, dst, 0.0);
    }

    /// [`Ui::picture`] with its corners rounded to `radius`, as a screen in
    /// a rounded frame; cut as it is drawn, so no pixel is copied for it.
    pub fn picture_rounded(
        &mut self,
        key: u64,
        size: (u32, u32),
        rgba: &[u8],
        src: [f32; 4],
        dst: Rect,
        radius: f32,
    ) {
        let (w, h) = size;
        if w == 0 || h == 0 || rgba.len() < w as usize * h as usize * 4 {
            return;
        }
        let pic = match self.pictures.get_mut(&key) {
            Some(p) => p,
            None => {
                let uv = self.atlases.color.insert(w, h, rgba);
                if uv.is_none() && !self.pictures_redo {
                    self.atlases.full = true;
                    self.pictures_redo = true;
                    return;
                }
                self.pictures
                    .entry(key)
                    .or_insert(Picture { uv, used: false })
            }
        };
        pic.used = true;
        let Some([u, v, _, _]) = pic.uv else {
            return;
        };
        let uv = [u + src[0], v + src[1], src[2], src[3]];
        self.scene
            .rounded_image(dst, uv, crate::theme::bright(1.0), radius);
    }

    /// Starts a frame on an empty scene.
    pub fn begin(&mut self, clear: impl Into<Rgba>) {
        self.begin_at(clear, Instant::now());
    }

    /// Starts a frame at `now`: tests drive animations and timers with it.
    pub fn begin_at(&mut self, clear: impl Into<Rgba>, now: Instant) {
        let clear = clear.into();
        self.st.now = now;
        self.wake.set(None);
        self.scene.clear();
        self.overlays = 0..0;
        self.glass.clear();
        self.glass_icons.clear();
        self.glass_labels.clear();
        self.clear = clear;
        // Pictures the last frame did not draw give their entry back.
        self.pictures.retain(|_, p| std::mem::take(&mut p.used));
        if let Some(list) = &mut self.cells {
            list.begin(clear);
        }
    }

    /// The logical size the frame is for: the window's while a window
    /// builds it, else the last [`Ui::layout`]'s. Views that lay out by
    /// the window's size (how many cards fit) read it.
    pub fn viewport(&self) -> (f32, f32) {
        self.viewport
    }

    /// Lays `root` out in a `size` (logical pixels) window and paints it.
    /// Overlays (menus, tooltips) go in layers above; afterwards the scene
    /// records into the window's layer again, so what the app paints into
    /// its custom boxes stays under them.
    pub fn layout(&mut self, root: El, size: (f32, f32)) -> Laid {
        self.viewport = size;
        self.record_native(&root);
        let size = if self.is_cells() {
            use cells::units::{COL, ROW};
            ((size.0 / COL).round(), (size.1 / ROW).round())
        } else {
            size
        };
        let mut cx = Cx {
            taffy: TaffyTree::new(),
            overlays: Vec::new(),
            customs: Vec::new(),
            nodes: Vec::new(),
            clip: Rect::EVERYTHING,
            layer: 0,
            float: false,
            emph: 0,
            focus_seen: false,
            xf: Xf::IDENTITY,
        };
        self.st.hits.clear();
        self.st.focus_order.clear();
        self.st.groups.clear();
        self.st.ime = None;
        self.anon = 0;
        let main_layer = self.scene.current_layer();
        let node = self.lay(&mut cx, &root, None, size, AvailableSpace::Definite, false);
        let mut root_children = Vec::new();
        self.paint(
            &mut cx,
            node,
            &root,
            (0.0, 0.0),
            Rect::new(0.0, 0.0, size.0, size.1),
            &mut root_children,
        );
        let first = self.scene.layers.len();
        let mut i = 0;
        while let Some(&(ov, host, clip)) = cx.overlays.get(i) {
            i += 1;
            self.place_overlay(&mut cx, ov, (host, clip), size, &mut root_children);
        }
        self.overlays = first..self.scene.layers.len();
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
        float: bool,
    ) -> LayoutNode {
        let mut scrolls = Vec::new();
        let cells = self.is_cells();
        let bar = if cells { 1.0 } else { theme::scrollbar::WIDTH };
        let node = build(
            &mut cx.taffy,
            el,
            (&mut scrolls, &self.st.scroll, bar),
            cells,
            (false, float),
        );
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
        // overflows, which needs another pass once that is known. A view
        // that overflowed last frame starts with its bar and keeps it while
        // it overflows by more than the bar could have caused; nearer the
        // edge it goes back to the rule.
        for _ in 0..2 {
            let mut again = false;
            for (n, has) in &mut scrolls {
                let Ok(l) = cx.taffy.layout(*n) else { continue };
                let over = l.content_size.height + l.border.bottom - l.size.height;
                let set = match *has {
                    Bar::Guessed if over <= 0.5 || over < l.size.height / 4.0 => {
                        *has = Bar::Off;
                        0.0
                    }
                    Bar::Off if over > 0.5 => {
                        *has = Bar::Set;
                        bar
                    }
                    _ => continue,
                };
                if let Ok(s) = cx.taffy.style(*n) {
                    let mut s = s.clone();
                    s.scrollbar_width = set;
                    let _ = cx.taffy.set_style(*n, s);
                    again = true;
                }
            }
            if !again {
                break;
            }
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
        let cells = text.cells;
        // Layout only fails on unknown node ids, which `build` never makes.
        let _ = t.compute_layout_with_measure(node, avail, |known, avail, _, ctx, _| {
            let Some(m) = ctx else {
                return Size::ZERO;
            };
            match m {
                Measure::Text { text: s, wrap, .. } if cells => {
                    let s = &cells::keys(s);
                    let w = cells::width(s) as f32;
                    let max = known.width.unwrap_or(match avail.width {
                        AvailableSpace::Definite(w) => w,
                        AvailableSpace::MinContent if *wrap => s
                            .split_whitespace()
                            .map(|w| cells::width(w) as f32)
                            .fold(0.0, f32::max),
                        // A line that does not wrap truncates rather than holding its
                        // row open; a short label ("Open", "Alt+1") keeps its width.
                        AvailableSpace::MinContent if w > 8.0 => 1.0,
                        _ => f32::MAX,
                    });
                    let lines = if *wrap && w > max {
                        cells::wrap(s, max as usize).len()
                    } else {
                        s.lines()
                            .count()
                            .max(1)
                            .min(if *wrap { usize::MAX } else { 1 })
                    };
                    Size {
                        width: known.width.unwrap_or(w.min(max.max(1.0))),
                        height: known.height.unwrap_or(lines as f32),
                    }
                }
                Measure::Edit { .. } if cells => Size {
                    width: known.width.unwrap_or(0.0),
                    height: known.height.unwrap_or(1.0),
                },
                Measure::Text {
                    text: s,
                    style,
                    wrap,
                } => {
                    let k = text.scale;
                    let max_w = wrap.then(|| {
                        known.width.unwrap_or(match avail.width {
                            AvailableSpace::Definite(w) => w,
                            // As CSS sizes it: the longest word.
                            AvailableSpace::MinContent => s
                                .split_whitespace()
                                .map(|w| text.shape(w, *style, None).w / k)
                                .fold(0.0, f32::max)
                                .ceil(),
                            AvailableSpace::MaxContent => f32::MAX,
                        })
                    });
                    let sh = text.shape(s, *style, max_w);
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
        (host, host_clip): (Rect, Rect),
        win: (f32, f32),
        a11y_parent: &mut Vec<NodeId>,
    ) {
        let min_w = ov.owner_width.then_some(host.w);
        let node = self.lay(cx, &ov.el, min_w, win, |_| AvailableSpace::MaxContent, true);
        let Ok(l) = cx.taffy.layout(node) else {
            return;
        };
        let (w, h) = (l.size.width, l.size.height);
        let g = ov.gap;
        let (mut x, mut y) = match ov.place {
            Place::Above => (host.x + (host.w - w) / 2.0, host.y - g - h),
            Place::Below => (host.x + (host.w - w) / 2.0, host.bottom() + g),
            Place::BelowStart => (host.x, host.bottom() + g),
            Place::BelowEnd => (host.right() - w, host.bottom() + g),
            Place::AboveStart => (host.x, host.y - g - h),
            Place::Right => (host.right() + g, host.y + (host.h - h) / 2.0),
            Place::Beside => (beside_x(host, w, g, win.0), host.y - ov.lift),
            Place::InsideEnd => (host.right() - g - w, host.bottom() - g - h),
        };
        let inside = ov.place == Place::InsideEnd;
        if !inside {
            // Flip to the other side when there is no room, then keep it on
            // screen.
            let margin = if self.is_cells() { 0.0 } else { MARGIN };
            if ov.place == Place::Beside {
                // Slides up to fit rather than flipping above its entry.
            } else if y + h > win.1 - margin && host.y - g - h >= margin {
                y = host.y - g - h;
            } else if y < margin && host.bottom() + g + h <= win.1 - margin {
                y = host.bottom() + g;
            }
            x = x.min(win.0 - margin - w).max(margin);
            y = y.min(win.1 - margin - h).max(margin);
        }
        self.scene.layer();
        cx.layer += 1;
        cx.float = true;
        // Inside its owner, it is cut where the owner is.
        let clip = if inside { host_clip } else { Rect::EVERYTHING };
        let clipped = clip != Rect::EVERYTHING;
        if clipped {
            self.scene.push_clip(clip.scaled(self.scale()));
        }
        let saved = std::mem::replace(&mut cx.clip, clip);
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
        if clipped {
            self.scene.pop_clip();
        }
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
        let (was_float, was_emph) = (cx.float, cx.emph);
        if self.cells.is_some() && el.a11y.selected == Some(true) {
            cx.emph |= cells::attr::BOLD;
            if el.a11y.role == Some(Role::Tab) {
                cx.emph |= cells::attr::UNDERLINE;
            }
        }
        if el.layered {
            self.scene.layer();
            cx.layer += 1;
            cx.float = true;
        }
        let focused = el.id.is_some() && self.st.focus == el.id;
        let ring = focused && self.st.focus_visible && el.ring.is_some();
        if el.opacity < 1.0 {
            self.scene.push_opacity(el.opacity);
            if let Some(list) = &mut self.cells {
                list.push_alpha(el.opacity);
            }
        }
        let cells = self.cells.is_some();
        let saved_xf = cx.xf;
        // Motion is dropped on cells: a box draws where it lands.
        if let Some((s, (dx, dy))) = el.transform.filter(|_| !cells) {
            self.scene
                .push_transform(s, (r.x + r.w / 2.0, r.y + r.h / 2.0), (dx * k, dy * k));
            cx.xf = Xf {
                scale: s,
                origin: (rl.x + rl.w / 2.0, rl.y + rl.h / 2.0),
                offset: (dx, dy),
            };
        }
        let radius = if ring {
            theme::focus::RADIUS
        } else {
            el.radius
        };
        // Unscaled boxes snap to whole pixels; a scaling transform is mid
        // animation, where snapping would make it judder.
        let snap = cx.xf.scale == 1.0;
        let bx = if snap { r.snapped() } else { r };
        let stroke = |w: f32| if snap { border_px(w, k) } else { w * k };
        if cells {
            self.record_box(cx, el, rl, ring);
        }
        // A surface drawn over glass hides it: the glass views sit above
        // everything the app draws.
        let covers = el.backdrop_blur.is_some() || el.bg.is_some_and(|c| c.0[3] >= 0.5);
        if covers && !el.native_glass && !cells && !self.glass.is_empty() {
            // Only the part left showing covers: a card scrolled under the
            // top bar is clipped away there. A badge or dot on a button is
            // smaller than its glass and leaves it.
            let seen = bx.intersect(&cx.clip.scaled(k));
            let hit = |r: &Rect| {
                seen.w >= r.w
                    && seen.h >= r.h
                    && r.x < seen.x + seen.w
                    && seen.x < r.x + r.w
                    && r.y < seen.y + seen.h
                    && seen.y < r.y + r.h
            };
            self.glass.retain(|(r, _)| !hit(r));
            self.glass_icons.retain(|(r, ..)| !hit(r));
            self.glass_labels.retain(|(r, ..)| !hit(r));
        }
        if el.native_glass && !cells {
            self.glass.push((bx, radius * k));
        }
        if let Some((svg, size, color)) = el.native_icon.as_ref().filter(|_| !cells) {
            let s = size * k;
            let at = Rect {
                x: bx.x + (bx.w - s) / 2.0,
                y: bx.y + (bx.h - s) / 2.0,
                w: s,
                h: s,
            };
            self.glass_icons.push((at, svg.clone(), *color));
        }
        if let Some((t, size, color)) = el.native_label.as_ref().filter(|_| !cells) {
            self.glass_labels.push((bx, t.clone(), size * k, *color));
        }
        if let Some(sigma) = el.backdrop_blur.filter(|_| !cells) {
            self.scene.backdrop_blur(bx, sigma * k, radius * k);
        }
        for s in el.shadows.iter().filter(|_| !cells) {
            self.scene.shadow(bx, radius * k, &s.scaled(k));
        }
        if !cells && (el.bg.is_some() || el.border.is_some()) {
            let fill = el.bg.unwrap_or_default();
            let border = el
                .border
                .filter(|_| !el.dashed)
                .map(|(w, c)| (stroke(w), c));
            self.scene.quad(bx, fill, radius * k, border);
        }
        if let Some((w, c)) = el.border.filter(|_| el.dashed && !cells) {
            let svg = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\"><rect x=\"{h}\" y=\"{h}\" width=\"{}\" height=\"{}\" rx=\"{radius}\" fill=\"none\" stroke=\"#fff\" stroke-width=\"{w}\" stroke-dasharray=\"{d} {d}\"/></svg>",
                rl.w,
                rl.h,
                rl.w - w,
                rl.h - w,
                h = w / 2.0,
                d = 3.0 * w,
            );
            let z = cx.xf.scale;
            let px = ((rl.w * k * z).round() as u32, (rl.h * k * z).round() as u32);
            if let Some(uv) = self.svg_raster(&svg, px) {
                self.scene.sprite(r, uv, c, false);
            }
        }
        for (side, rule) in el.rules.iter().enumerate().filter(|_| !cells) {
            let Some((w, c)) = *rule else { continue };
            let w = stroke(w);
            let line = match side {
                0 => Rect::new(bx.x, bx.y, bx.w, w),
                1 => Rect::new(bx.right() - w, bx.y, w, bx.h),
                2 => Rect::new(bx.x, bx.bottom() - w, bx.w, w),
                _ => Rect::new(bx.x, bx.y, w, bx.h),
            };
            self.scene.quad(line, c, 0.0, None);
        }
        if let Some(id) = el.id {
            if el.sense != Sense::NONE || el.action.is_some() || el.tip {
                let (sx, sy) = el.slop;
                let target = Rect::new(x - sx, y - sy, rl.w + 2.0 * sx, rl.h + 2.0 * sy);
                self.st.hits.push(Hit {
                    id,
                    rect: cx.xf.apply(target).intersect(&cx.clip),
                    sense: el.sense,
                    action: el.action,
                    cursor: el.cursor,
                    layer: cx.layer,
                    tip: el.tip,
                    parent,
                    vertical: el.a11y.vertical.unwrap_or(false),
                    frames: el.frames,
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
        let glyph = cell_glyph(el).filter(|_| cells);
        match &el.content {
            _ if cells && !matches!(el.content, Content::Custom(_)) => {
                self.record_content(cx, el, glyph, inner, focused);
            }
            Content::None => {}
            Content::Text {
                text,
                style,
                color,
                wrap,
                truncate,
            } => {
                // Rounded layout can leave a box up to a pixel short of what its text measured.
                let room = inner.w + 1.0;
                // Under a zoom, text is shaped at the size it shows, so it stays sharp.
                let z = cx.xf.scale;
                let (style, room, at) = if z == 1.0 {
                    (*style, room, inner)
                } else {
                    let scaled = TextStyle {
                        size: style.size * z,
                        line_height: style.line_height * z,
                        ..*style
                    };
                    (scaled, room * z, cx.xf.apply(inner))
                };
                let sh = if *truncate {
                    self.text.shape_truncated(text, style, room)
                } else {
                    self.text.shape(text, style, wrap.then_some(room))
                };
                if z != 1.0 {
                    self.scene.push_transform(1.0, (0.0, 0.0), (0.0, 0.0));
                }
                self.text.draw(
                    &mut self.scene,
                    &mut self.atlases,
                    &sh,
                    (at.x * k, at.y * k),
                    *color,
                );
                if z != 1.0 {
                    self.scene.pop_transform();
                }
            }
            Content::Icon { svg, size, color } => {
                let z = cx.xf.scale;
                let at = if z == 1.0 { inner } else { cx.xf.apply(inner) };
                let px = (size * k * z).round() as u32;
                if let Some(uv) = self.icon_raster(svg, px) {
                    let ir = Rect::new((at.x * k).round(), (at.y * k).round(), uv[2], uv[3]);
                    if z != 1.0 {
                        self.scene.push_transform(1.0, (0.0, 0.0), (0.0, 0.0));
                    }
                    self.scene.sprite(ir, uv, *color, false);
                    if z != 1.0 {
                        self.scene.pop_transform();
                    }
                }
            }
            Content::Svg { svg, color } => {
                let z = cx.xf.scale;
                let px = (
                    (inner.w * k * z).round() as u32,
                    (inner.h * k * z).round() as u32,
                );
                if let Some(uv) = self.svg_raster(svg, px) {
                    self.scene.sprite(inner.scaled(k), uv, *color, false);
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
        // On cells a frame is a clip too: content never draws over the box drawing.
        let clips = el.clip || (cells && el.border.is_some() && l.border.left > 0.0);
        if clips {
            self.scene.push_clip(clip_box.scaled(k));
            cx.clip = cx.clip.intersect(&cx.xf.apply(clip_box));
        }
        let children = if glyph.is_some() {
            &[][..]
        } else {
            &el.children[..]
        };
        let mut scrolled = 0.0;
        let mut max = 0.0;
        if el.scroll {
            if let Some(id) = el.id {
                // Taffy measures the content from the border box's top edge.
                max = l.content_size.height + l.border.bottom - l.size.height;
                // The rounded box against unrounded content is not an overflow.
                if max < 1.0 {
                    max = 0.0;
                }
                // The children with ids: where each starts in the content, and how tall.
                let items: Vec<(Id, f32, f32)> = if el.windowed || self.st.reveal.is_some() {
                    children
                        .iter()
                        .enumerate()
                        .filter_map(|(i, ce)| {
                            let c = cx.taffy.child_at_index(node, i).ok()?;
                            let cl = cx.taffy.layout(c).ok()?;
                            Some((ce.id?, cl.location.y - l.border.top, cl.size.height))
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                let find = |want: Id| items.iter().find(|i| i.0 == want).copied();
                let s = self.st.scroll.entry(id).or_default();
                s.max = max;
                let mut offset = if s.follow && s.at_end { max } else { s.offset };
                // What changed above the first item in view (a height measured
                // in place of its guess, an item added) leaves it where it was.
                if el.windowed && offset > 0.0 {
                    let anchor = self.st.virt.get(&id).and_then(|v| v.anchor);
                    if let Some((now, was)) = anchor.and_then(|(a, was)| Some((find(a)?.1, was))) {
                        offset += now - was;
                    }
                }
                let view = clip_box.h;
                if let Some((_, top, h)) = self.st.reveal.and_then(find) {
                    if top < offset || h > view {
                        offset = top;
                    } else if top + h > offset + view {
                        offset = top + h - view;
                    }
                    self.st.reveal = None;
                }
                let s = self.st.scroll.entry(id).or_default();
                s.offset = offset.clamp(0.0, max);
                scrolled = s.offset;
                if el.windowed {
                    let gap = cx.taffy.style(node).map_or(0.0, |st| {
                        let g = st.gap.height.into_raw();
                        if g.tag() == taffy::CompactLength::LENGTH_TAG {
                            g.value()
                        } else {
                            0.0
                        }
                    });
                    let v = self.st.virt.entry(id).or_default();
                    v.gap = gap;
                    v.top = l.padding.top;
                    v.view = view;
                    for &(i, _, h) in &items {
                        v.measured(i, h);
                    }
                    v.anchor = items
                        .iter()
                        .find(|i| i.1 + i.2 > scrolled)
                        .map(|i| (i.0, i.1));
                }
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
        for (i, ce) in children.iter().enumerate() {
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
            if cells {
                let rows = view.round().max(1.0);
                let len = (rows * rows / (rows + max)).round().clamp(1.0, rows);
                let at = clip_box.y + ((rows - len) * scrolled / max).round();
                let thumb = (at as i32, len as i32);
                let track = Rect::new(clip_box.right() - 1.0, clip_box.y, 1.0, rows);
                let color = theme::scrollbar::THUMB_HOVER;
                self.record(
                    cx,
                    cells::Op::Scroll {
                        thumb,
                        color: color.rgba(),
                    },
                    track,
                );
            }
            self.scene.quad(
                track.scaled(k),
                c.rgba(),
                theme::scrollbar::RADIUS * k,
                None,
            );
        }
        if clips {
            self.scene.pop_clip();
            cx.clip = saved_clip;
        }
        if ring && !cells {
            let d = theme::focus::OFFSET + theme::focus::WIDTH;
            let w = stroke(theme::focus::WIDTH);
            let outer = rl.outset(d).scaled(k);
            self.scene.quad(
                if snap { outer.snapped() } else { outer },
                Rgba::TRANSPARENT,
                (theme::focus::RADIUS + d) * k,
                Some((w, theme::focus::RING.rgba())),
            );
        }
        for ov in &el.overlays {
            cx.overlays.push((ov, cx.xf.apply(rl), cx.clip));
        }
        if el.transform.is_some() {
            self.scene.pop_transform();
        }
        cx.xf = saved_xf;
        (cx.float, cx.emph) = (was_float, was_emph);
        if el.opacity < 1.0 {
            self.scene.pop_opacity();
            if let Some(list) = &mut self.cells {
                list.pop_alpha();
            }
        }
        let Some(role) = role else {
            return;
        };
        let id = match (el.id, &el.content) {
            (Some(i), _) => NodeId(i.0),
            (None, Content::Custom(c)) => custom_node(*c),
            (None, _) => {
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
            if look.secure {
                n.set_value(
                    text.chars()
                        .map(|_| crate::interact::MASK)
                        .collect::<String>(),
                );
            } else {
                n.set_value(text);
            }
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

    /// Records `op` over `rect` in the cells paint list, under the clip and layer painting is in.
    fn record(&mut self, cx: &Cx<'_>, op: cells::Op, rect: Rect) {
        if let Some(list) = &mut self.cells {
            list.clip = Some(cx.clip);
            list.layer = cx.layer;
            list.push(op, rect);
        }
    }

    /// A box on cells: its fill, its border as box drawing, its rules, and
    /// keyboard focus as one step brighter.
    fn record_box(&mut self, cx: &Cx<'_>, el: &El, rl: Rect, ring: bool) {
        // A button's states: a faint fill at rest, brighter under the pointer
        // or keyboard focus, brightest while pressed.
        if let Some(id) = el.id.filter(|_| is_button(el)) {
            let fill = if self.pressed(id) {
                0.24
            } else if ring || self.hover(id) > 0.5 {
                0.14
            } else if (icon_only(el) || (rl.w <= 16.0 && rl.h <= 1.0))
                && el.bg.is_none_or(|b| b.0[3] < 0.05)
                && el.opacity > 0.5
            {
                // Rows highlight on hover alone; a compact control shows at rest.
                0.07
            } else {
                0.0
            };
            if fill > 0.0 {
                let op = cells::Op::Fill {
                    color: crate::theme::bright(fill),
                };
                self.record(cx, op, rl);
            }
        }
        let shape = Shape::of(el);
        // A chip a pixel surface tints faintly still reads as one in cells.
        let chip = |c: Rgba| {
            let small = rl.h <= 1.0 && rl.w <= 24.0 && el.radius > 0.0;
            if small && c.0[3] > 0.01 && c.0[3] < 0.10 {
                // literal-colour: the chip's own colour, only its alpha raised.
                Rgba([c.0[0], c.0[1], c.0[2], 0.10])
            } else {
                c
            }
        };
        if let Some(bg) = el.bg {
            let op = match shape {
                Shape::Box => cells::Op::Fill { color: chip(bg) },
                Shape::HLine => cells::Op::Rule { side: 0, color: bg },
                Shape::VLine if !cells::is_tint(bg) => cells::Op::Bar { color: bg },
                Shape::VLine => cells::Op::Rule { side: 3, color: bg },
                Shape::Dot => cells::Op::Glyph {
                    glyph: "●".into(),
                    color: bg,
                    icon: 0,
                },
            };
            self.record(cx, op, rl);
        }
        if ring {
            let color = crate::theme::bright(0.06);
            self.record(cx, cells::Op::Fill { color }, rl);
        }
        if let Some((_, color)) = el.border.filter(|(w, c)| *w > 0.0 && c.0[3] > 0.0) {
            let op = if framed(el) {
                let round = el.radius > 0.0;
                let open = !cx.float;
                cells::Op::Frame { color, round, open }
            } else {
                cells::Op::Fill { color: chip(color) }
            };
            self.record(cx, op, rl);
        }
        for (side, rule) in el.rules.iter().enumerate() {
            if let Some((w, color)) = rule.filter(|(w, c)| *w > 0.0 && c.0[3] > 0.0) {
                let op = if side == 3 && w >= 2.0 && !cells::is_tint(color) {
                    cells::Op::Bar { color }
                } else {
                    let side = side as u8;
                    cells::Op::Rule { side, color }
                };
                self.record(cx, op, rl);
            }
        }
    }

    /// A box's content on cells: text by role, a glyph for an icon, an edit
    /// field's text with the terminal cursor at its end.
    fn record_content(
        &mut self,
        cx: &Cx<'_>,
        el: &El,
        glyph: Option<&str>,
        inner: Rect,
        focused: bool,
    ) {
        if let Some(g) = glyph {
            let color = el.glyph_tint.unwrap_or(match &el.content {
                Content::Icon { color, .. }
                | Content::Svg { color, .. }
                | Content::Text { color, .. } => *color,
                _ => theme::color::INK.rgba(),
            });
            let glyph = g.to_owned();
            let icon = match &el.content {
                Content::Icon { svg, .. } => cells::icon_key(svg),
                _ => 0,
            };
            self.record(cx, cells::Op::Glyph { glyph, color, icon }, inner);
            return;
        }
        match &el.content {
            Content::Text {
                text,
                style,
                color,
                wrap,
                ..
            } => {
                let mut attr = cx.emph;
                if style.weight >= 600 {
                    attr |= cells::attr::BOLD;
                }
                if style.italic {
                    attr |= cells::attr::ITALIC;
                }
                if color.0[3] < 0.6 {
                    attr |= cells::attr::DIM;
                }
                let op = cells::Op::Text {
                    text: cells::keys(text).into_owned(),
                    color: *color,
                    attr,
                    wrap: *wrap,
                };
                self.record(cx, op, inner);
            }
            Content::Edit(look) => {
                let Some(id) = el.id else { return };
                // Registered as paint_edit does, so the field takes keys before it has text.
                let e = self
                    .st
                    .edits
                    .entry(id)
                    .or_insert_with(|| TextEdit::new("", look.multiline));
                e.set_multiline(look.multiline);
                e.set_textarea(look.textarea);
                e.set_secure(look.secure);
                let (shown, pre) = e.display();
                let caret = if pre.is_empty() { e.caret() } else { pre.end };
                let sel = e.selection();
                if shown.is_empty() {
                    let op = cells::Op::Text {
                        text: look.placeholder.clone(),
                        color: look.placeholder_color,
                        attr: cells::attr::DIM,
                        wrap: look.multiline && !look.nowrap,
                    };
                    self.record(cx, op, inner);
                }
                let mask = crate::interact::MASK;
                let (shown, caret, sel, pre) = if look.secure {
                    let n = |i: usize| shown[..i].chars().count() * mask.len_utf8();
                    let masked: String = shown.chars().map(|_| mask).collect();
                    (
                        masked,
                        n(caret),
                        n(sel.start)..n(sel.end),
                        n(pre.start)..n(pre.end),
                    )
                } else {
                    (shown, caret, sel, pre)
                };
                let size = (inner.w.max(1.0) as usize, inner.h.max(1.0) as usize);
                let marks = [
                    (sel.clone(), cells::attr::INVERSE),
                    (pre, cells::attr::UNDERLINE),
                ];
                let laid =
                    cells::edit_cells(&shown, caret, &marks, size, look.multiline && !look.nowrap);
                for run in laid.runs {
                    let w = cells::width(&run.text) as f32;
                    let at = Rect::new(
                        inner.x + f32::from(run.col),
                        inner.y + f32::from(run.row),
                        w,
                        1.0,
                    );
                    let op = cells::Op::Text {
                        text: run.text,
                        color: look.color,
                        attr: run.attr,
                        wrap: false,
                    };
                    self.record(cx, op, at);
                }
                if let (true, true, Some((col, row))) = (focused, sel.is_empty(), laid.caret) {
                    if let Some(list) = &mut self.cells {
                        list.cursor = Some(cells::CursorAt {
                            col: (inner.x + f32::from(col)).max(0.0) as u16,
                            row: (inner.y + f32::from(row)).max(0.0) as u16,
                            shape: cells::CursorShape::Bar,
                        });
                    }
                }
            }
            _ => {}
        }
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
        e.set_textarea(look.textarea);
        e.set_secure(look.secure);
        let (real, pre) = e.display();
        let caret_at = if e.preedit().is_empty() {
            e.caret()
        } else {
            pre.end
        };
        let sel = e.selection();
        let anchor_caret = (e.anchor(), e.caret());
        // A secure field shapes a bullet per character; every offset moves with it.
        let mask = crate::interact::MASK;
        let map = |i: usize| {
            if look.secure {
                real[..i.min(real.len())].chars().count() * mask.len_utf8()
            } else {
                i
            }
        };
        let (caret_at, sel, pre, anchor_caret) = (
            map(caret_at),
            map(sel.start)..map(sel.end),
            map(pre.start)..map(pre.end),
            (map(anchor_caret.0), map(anchor_caret.1)),
        );
        let shown = if look.secure {
            real.chars().map(|_| mask).collect()
        } else {
            real
        };
        let wrap = look.multiline && !look.nowrap;
        let sh = self.text.shape(&shown, look.style, wrap.then_some(inner.w));
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
        }
        if !wrap {
            let cxl = cx_ / k;
            // A single line keeps its sideways scroll in `scroll_y`.
            let sx = if look.multiline {
                &mut e.scroll_x
            } else {
                &mut e.scroll_y
            };
            if cxl < *sx {
                *sx = cxl;
            } else if cxl > *sx + inner.w - 1.0 {
                *sx = cxl - inner.w + 1.0;
            }
            *sx = sx.clamp(0.0, (sh.w / k - inner.w + 1.0).max(0.0));
        }
        // A centred line sits mid-field while it fits; longer, it scrolls as usual.
        let centre = |w: f32| {
            if look.centered && !look.multiline {
                ((inner.w * k - w) / 2.0).max(0.0)
            } else {
                0.0
            }
        };
        let (ox, oy) = match (look.multiline, wrap) {
            (true, true) => (inner.x * k, (inner.y - e.scroll_y) * k),
            (true, false) => ((inner.x - e.scroll_x) * k, (inner.y - e.scroll_y) * k),
            (false, _) => ((inner.x - e.scroll_y) * k + centre(sh.w), inner.y * k),
        };
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
                self.scene
                    .quad(rr, theme::color::SELECTION.rgba(), 0.0, None);
            }
        }
        for (range, bg) in &look.marks {
            for (i, l) in sh.lines.iter().enumerate() {
                let start = range.start.max(l.start);
                let end = range.end.min(a11y::run_end(&sh, i, shown.len()));
                if start >= end {
                    continue;
                }
                let x0 = sh.caret(start).0;
                let x1 = sh.caret(end).0.max(x0 + 1.0);
                let rr = Rect::new(ox + x0, oy + l.top, x1 - x0, l.height);
                self.scene.quad(rr, *bg, k, None);
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
                (inner.x * k + centre(ph.w), inner.y * k),
                look.placeholder_color,
            );
        }
        if look.spans.is_empty() || !pre.is_empty() {
            self.text.draw(
                &mut self.scene,
                &mut self.atlases,
                &sh,
                (ox, oy),
                look.color,
            );
        } else {
            self.text.draw_spans(
                &mut self.scene,
                &mut self.atlases,
                &sh,
                (ox, oy),
                look.color,
                &look.spans,
            );
        }
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
                masked: look.secure,
            },
        );
        runs
    }

    /// Fixes up state the new layout invalidated.
    fn after_layout(&mut self, cx: &Cx<'_>) {
        self.st.reveal = None;
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

    /// An SVG's coverage at `w`×`h` physical pixels, kept with the icons.
    fn svg_raster(&mut self, svg: &str, (w, h): (u32, u32)) -> Option<[f32; 4]> {
        if w == 0 || h == 0 {
            return None;
        }
        let key = (Id::new(svg).child((w, h)).0, w);
        if let Some(hit) = self.icons.get(&key) {
            return *hit;
        }
        let uv = rasterize_svg_to(svg, w, h).and_then(|mask| self.atlases.insert_mask(w, h, &mask));
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
        self.pictures.clear();
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
        self.render_target(target.view(), target.size)
    }

    pub(crate) fn render_target(
        &mut self,
        target: Option<&wgpu::TextureView>,
        size: (u32, u32),
    ) -> bool {
        if self.atlases.full {
            self.reset_atlases();
            return false;
        }
        self.pictures_redo = false;
        self.renderer.draw(
            self.gpu.as_ref(),
            &self.scene,
            &self.atlases,
            self.clear,
            target,
            size,
        );
        self.text.trim();
        true
    }

    /// Draws the scene into each raster of `jobs` without its range of
    /// layers, for a host that shows the overlays apart from what is under
    /// them. Same `false` as [`Ui::render_offscreen`].
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn render_rasters(
        &mut self,
        jobs: &mut [(&mut crate::cpu::CpuRaster, std::ops::Range<usize>)],
        size: (u32, u32),
    ) -> bool {
        if self.atlases.full {
            self.reset_atlases();
            return false;
        }
        self.pictures_redo = false;
        for (raster, skip) in jobs.iter_mut() {
            raster.render_without(&self.scene, &self.atlases, self.clear, size, skip.clone());
        }
        self.text.trim();
        true
    }

    /// The scene layers the last layout's overlays drew into.
    pub fn overlay_layers(&self) -> std::ops::Range<usize> {
        self.overlays.clone()
    }

    /// Copies the CPU renderer's last frame into `dst`, a window's buffer
    /// of the same size with one `0x00RRGGBB` pixel each: only the rows
    /// that changed, unless `whole` (the buffer does not hold the frame
    /// before). Answers the rows that changed, or `None` when the GPU draws
    /// or the sizes differ.
    pub fn copy_cpu_frame(&self, dst: &mut [u32], whole: bool) -> Option<&[(u32, u32)]> {
        self.renderer.cpu_frame()?.copy_xrgb(dst, whole)
    }

    /// The CPU renderer's last frame (4 bytes a pixel, in the order its
    /// format names), its size and the rows that changed, for a host that
    /// presents the bytes itself: a browser's 2D canvas.
    pub fn cpu_pixels(&self) -> Option<CpuPixels<'_>> {
        let r = self.renderer.cpu_frame()?;
        Some((r.bytes(), r.size(), r.damage()))
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
        match (self.gpu.as_ref(), target.texture.as_ref()) {
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

/// Whether a scroll view has its scrollbar's width, and why.
#[derive(Clone, Copy)]
enum Bar {
    /// It overflowed last frame.
    Guessed,
    Off,
    /// It overflowed without.
    Set,
}

fn build<'a>(
    t: &mut TaffyTree<Measure<'a>>,
    el: &'a El,
    (scrolls, last, bar): (&mut Vec<(LayoutNode, Bar)>, &HashMap<Id, Scroll>, f32),
    cells: bool,
    (in_row, float): (bool, bool),
) -> LayoutNode {
    let mut style = el.style.clone();
    let float = float || el.layered;
    if cells {
        to_cells(&mut style, el, in_row, float);
    }
    let row = matches!(
        style.flex_direction,
        taffy::FlexDirection::Row | taffy::FlexDirection::RowReverse
    );
    if el.clip {
        style.overflow = Point {
            x: Overflow::Hidden,
            y: Overflow::Hidden,
        };
    }
    let guess = el.scroll
        && el
            .id
            .and_then(|id| last.get(&id))
            .is_some_and(|s| s.max > 0.0);
    if el.scroll {
        style.overflow.y = Overflow::Scroll;
        style.scrollbar_width = if guess { bar } else { 0.0 };
    }
    let ctx = match &el.content {
        _ if cells && cell_glyph(el).is_some() => None,
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
            let kids: &[El] = if cells && cell_glyph(el).is_some() {
                &[]
            } else {
                &el.children
            };
            let kids: Vec<LayoutNode> = kids
                .iter()
                .map(|c| build(t, c, (&mut *scrolls, last, bar), cells, (row, float)))
                .collect();
            t.new_with_children(style, &kids)
        }
    }
    .unwrap_or_else(|_| unreachable!("taffy fails only on unknown node ids"));
    if el.scroll {
        scrolls.push((node, if guess { Bar::Guessed } else { Bar::Off }));
    }
    node
}

/// Rewrites a style's pixel lengths in cells: widths in columns, heights in
/// rows, borders one cell where they draw and none where they only reserve.
/// What a box shows on cells in place of its content: its own glyph, or
/// a Lucide icon's stand-in.
fn cell_glyph(el: &El) -> Option<&str> {
    match (&el.glyph, &el.content) {
        (Some(g), _) => Some(g),
        (None, Content::Icon { svg, .. }) => icons::glyph_of(svg),
        _ => None,
    }
}

/// How a box with a fill draws on cells, read from its pixel size.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Shape {
    Box,
    /// A hairline divider: a line of box-drawing characters.
    HLine,
    VLine,
    /// A status dot or badge: one `●`.
    Dot,
}

impl Shape {
    fn of(el: &El) -> Shape {
        let (w, h) = (fixed(el.style.size.width), fixed(el.style.size.height));
        if el.bg.is_none() || !el.children.is_empty() {
            return Shape::Box;
        }
        match (w, h) {
            (_, Some(h)) if h <= 2.0 => Shape::HLine,
            (Some(w), _) if w <= 2.0 => Shape::VLine,
            (Some(w), Some(h)) if w <= 10.0 && h <= 10.0 => Shape::Dot,
            _ => Shape::Box,
        }
    }
}

/// A length in pixels, when the style fixes one.
fn fixed(d: taffy::Dimension) -> Option<f32> {
    let raw = d.into_raw();
    (raw.tag() == taffy::CompactLength::LENGTH_TAG).then(|| raw.value())
}

/// Whether a bordered box keeps its frame on cells: a column taller than
/// 40 px; rows (chips, pills, buttons) and shorter fields tint instead.
/// A control a press activates: on a terminal it gets room and a fill.
fn is_button(el: &El) -> bool {
    el.a11y.role == Some(Role::Button) && el.action == Some(Action::Click) && !el.a11y.disabled
}

/// A button that shows only an icon.
fn icon_only(el: &El) -> bool {
    let icon = |e: &El| {
        matches!(e.content, Content::Icon { .. } | Content::Svg { .. }) || e.glyph.is_some()
    };
    icon(el) || (el.children.len() == 1 && icon(&el.children[0]))
}

fn framed(el: &El) -> bool {
    let column = matches!(
        el.style.flex_direction,
        taffy::FlexDirection::Column | taffy::FlexDirection::ColumnReverse
    ) || el.style.display != taffy::Display::Flex;
    if !column {
        return false;
    }
    let h = fixed(el.style.size.height).or(fixed(el.style.min_size.height));
    match h.filter(|h| *h > 0.0) {
        Some(h) => h >= 40.0,
        None => el.children.len() > 1 || el.children.iter().any(|c| !c.children.is_empty()),
    }
}

/// Whether `el` draws a frame on cells.
fn shows_frame(el: &El) -> bool {
    el.border.is_some_and(|(w, c)| w > 0.0 && c.0[3] > 0.0) && framed(el)
}

fn to_cells(s: &mut taffy::Style, el: &El, in_row: bool, float: bool) {
    use cells::units;
    use taffy::{CompactLength, Dimension, LengthPercentage as Lp, LengthPercentageAuto as Lpa};
    fn px(raw: CompactLength) -> Option<f32> {
        (raw.tag() == CompactLength::LENGTH_TAG).then(|| raw.value())
    }
    let dim = |d: Dimension, f: fn(f32) -> f32| match px(d.into_raw()) {
        Some(v) if v > 0.0 => Dimension::length(f(v)),
        _ => d,
    };
    let lp = |d: Lp, f: fn(f32) -> f32| match px(d.into_raw()) {
        Some(v) => Lp::length(f(v)),
        None => d,
    };
    let lpa = |d: Lpa, f: fn(f32) -> f32| match px(d.into_raw()) {
        Some(v) => Lpa::length(f(v)),
        None => d,
    };
    let (w, h, sx, sy) = (
        units::width as fn(f32) -> f32,
        units::height as fn(f32) -> f32,
        units::space_x as fn(f32) -> f32,
        units::space_y as fn(f32) -> f32,
    );
    // An empty box with nothing to draw is spacing, not a control.
    let spacer = el.children.is_empty()
        && el.bg.is_none()
        && el.border.is_none()
        && el.rules.iter().all(Option::is_none)
        && matches!(el.content, Content::None);
    let (w, h) = if spacer { (sx, sy) } else { (w, h) };
    let glyph = cell_glyph(el);
    let icon = matches!(
        el.content,
        Content::Icon { .. } | Content::Svg { .. } | Content::Image { .. }
    );
    match Shape::of(el) {
        Shape::Box => {}
        Shape::HLine => s.size.height = Dimension::length(1.0),
        Shape::VLine => s.size.width = Dimension::length(1.0),
        Shape::Dot => {
            s.size = taffy::Size {
                width: Dimension::length(1.0),
                height: Dimension::length(1.0),
            };
        }
    }
    if glyph.is_some() || icon {
        let gw = glyph.map_or(1, |g| cells::width(g).max(1)) as f32;
        if icon || px(s.size.width.into_raw()).is_some_and(|v| v <= 24.0) {
            s.size.width = Dimension::length(gw);
            s.min_size.width = Dimension::length(gw);
        }
        if icon || px(s.size.height.into_raw()).is_some_and(|v| v <= 28.0) {
            s.size.height = Dimension::length(1.0);
            s.min_size.height = Dimension::length(1.0);
        }
    }
    s.size.width = dim(s.size.width, w);
    s.size.height = dim(s.size.height, h);
    s.min_size.width = dim(s.min_size.width, w);
    s.min_size.height = dim(s.min_size.height, h);
    s.max_size.width = dim(s.max_size.width, w);
    s.max_size.height = dim(s.max_size.height, h);
    s.flex_basis = dim(s.flex_basis, if in_row { w } else { h });
    s.padding.left = lp(s.padding.left, sx);
    s.padding.right = lp(s.padding.right, sx);
    s.padding.top = lp(s.padding.top, sy);
    s.padding.bottom = lp(s.padding.bottom, sy);
    s.margin.left = lpa(s.margin.left, sx);
    s.margin.right = lpa(s.margin.right, sx);
    s.margin.top = lpa(s.margin.top, sy);
    s.margin.bottom = lpa(s.margin.bottom, sy);
    s.inset.left = lpa(s.inset.left, sx);
    s.inset.right = lpa(s.inset.right, sx);
    s.inset.top = lpa(s.inset.top, sy);
    s.inset.bottom = lpa(s.inset.bottom, sy);
    s.gap.width = lp(s.gap.width, sx);
    s.gap.height = lp(s.gap.height, sy);
    // A button reads as one on a terminal: a cell of room either side, and a
    // cell between buttons that sit side by side, so their fills do not run together.
    let at_least = |l: Lp, n: f32| match px(l.into_raw()) {
        Some(v) if v >= n => l,
        _ => Lp::length(n),
    };
    // Buttons hidden until hover keep their size, so the row they share keeps its width.
    if is_button(el) && el.opacity >= 0.99 {
        s.padding.left = at_least(s.padding.left, 1.0);
        s.padding.right = at_least(s.padding.right, 1.0);
    }
    let row = matches!(
        s.flex_direction,
        taffy::FlexDirection::Row | taffy::FlexDirection::RowReverse
    );
    let shown = |c: &&El| is_button(c) && c.opacity >= 0.99;
    if row && el.children.iter().filter(shown).count() >= 2 {
        s.gap.width = at_least(s.gap.width, 1.0);
    }
    // Frames in the window's flow share edges: each draws its top and left
    // and touches its neighbours, so a run of cards costs one column each.
    if !float && el.children.iter().any(shows_frame) {
        let row = matches!(
            s.flex_direction,
            taffy::FlexDirection::Row | taffy::FlexDirection::RowReverse
        );
        if row {
            s.gap.width = Lp::length(0.0);
        } else {
            s.gap.height = Lp::length(0.0);
        }
    }
    let shows = |c: Option<(f32, Rgba)>| c.is_some_and(|(w, c)| w > 0.0 && c.0[3] > 0.0);
    let frame = shows_frame(el);
    let edge = |i: usize| {
        let open = !float && (i == 1 || i == 2);
        if (frame && !open) || shows(el.rules[i]) {
            1.0
        } else {
            0.0
        }
    };
    s.border = taffy::Rect {
        top: Lp::length(edge(0)),
        right: Lp::length(edge(1)),
        bottom: Lp::length(edge(2)),
        left: Lp::length(edge(3)),
    };
}

/// The accessibility node of a custom element with no id: from its custom
/// id, so a terminal keeps its node when nodes before it come and go
/// (unnamed nodes are numbered in tree order, with the low bit clear).
fn custom_node(custom: u64) -> NodeId {
    NodeId((1 << 63) | (custom << 1) | 1)
}

/// Options for an SVG someone else wrote: an `<image>` may be inline `data:`,
/// never a file or address it names.
fn svg_options() -> resvg::usvg::Options<'static> {
    use resvg::usvg::{ImageHrefResolver, Options};
    Options {
        image_href_resolver: ImageHrefResolver {
            resolve_data: ImageHrefResolver::default_data_resolver(),
            resolve_string: Box::new(|_, _| None),
        },
        ..Options::default()
    }
}

/// An SVG rendered to a `px`×`px` coverage mask.
pub fn rasterize_svg(svg: &str, px: u32) -> Option<Vec<u8>> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_str(svg, &svg_options()).ok()?;
    let mut pm = tiny_skia::Pixmap::new(px, px)?;
    let s = tree.size();
    let tf = tiny_skia::Transform::from_scale(px as f32 / s.width(), px as f32 / s.height());
    resvg::render(&tree, tf, &mut pm.as_mut());
    Some(pm.data().chunks(4).map(|p| p[3]).collect())
}

/// An SVG stretched over `w`×`h` pixels, as a coverage mask.
pub fn rasterize_svg_to(svg: &str, w: u32, h: u32) -> Option<Vec<u8>> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_str(svg, &svg_options()).ok()?;
    let mut pm = tiny_skia::Pixmap::new(w, h)?;
    let s = tree.size();
    let tf = tiny_skia::Transform::from_scale(w as f32 / s.width(), h as f32 / s.height());
    resvg::render(&tree, tf, &mut pm.as_mut());
    Some(pm.data().chunks(4).map(|p| p[3]).collect())
}

/// An SVG in its own colours at `w`×`h` pixels, as straight-alpha RGBA.
pub fn rasterize_svg_rgba(svg: &str, w: u32, h: u32) -> Option<Vec<u8>> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_str(svg, &svg_options()).ok()?;
    let mut pm = tiny_skia::Pixmap::new(w, h)?;
    let s = tree.size();
    let tf = tiny_skia::Transform::from_scale(w as f32 / s.width(), h as f32 / s.height());
    resvg::render(&tree, tf, &mut pm.as_mut());
    Some(
        pm.pixels()
            .iter()
            .flat_map(|p| {
                let c = p.demultiply();
                [c.red(), c.green(), c.blue(), c.alpha()]
            })
            .collect(),
    )
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
    fn an_svg_draws_no_file_it_names() {
        let path = std::env::temp_dir().join(format!("vornui-href-{}.png", std::process::id()));
        let red = [255u8, 0, 0, 255].repeat(16);
        write_png(path.to_str().unwrap(), (4, 4), &red).unwrap();
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><image href="{}" width="4" height="4"/></svg>"#,
            path.display()
        );
        let drawn = rasterize_svg_rgba(&svg, 4, 4).unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(drawn.chunks(4).all(|p| p[3] == 0), "drew the file it named");
        let shape = r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4" fill="red"/></svg>"#;
        assert!(rasterize_svg_rgba(shape, 4, 4)
            .unwrap()
            .chunks(4)
            .all(|p| p[3] == 255));
    }

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
