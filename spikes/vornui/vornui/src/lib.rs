//! vornui: the thin UI layer the spike weighs against GPUI. It owns only
//! the glue: winit gives windows and input, wgpu draws, cosmic-text and
//! swash shape and rasterize text, taffy lays out, resvg draws icons and
//! AccessKit speaks to screen readers. What is here is the part no crate
//! provides: the scene and its two shaders, the atlases, the element tree
//! and the terminal cell painter.

pub mod atlas;
pub mod element;
pub mod gpu;
pub mod input;
pub mod text;
pub mod window;

use std::collections::HashMap;

use accesskit::{Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate};
use taffy::prelude::{AvailableSpace, Size, TaffyTree};

pub use {accesskit, wgpu, winit};

pub use element::{custom, div, icon, image, text, Content, El};
pub use gpu::{Gpu, Offscreen, Rect, Renderer, Rgba, Scene};
pub use input::Input;
pub use text::{TextStyle, TextSystem};

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

pub struct Ui {
    pub gpu: Gpu,
    pub renderer: Renderer,
    pub text: TextSystem,
    pub scene: Scene,
    icons: HashMap<(String, u32), Option<[f32; 4]>>,
    images: Vec<[f32; 4]>,
    /// Custom boxes' accessibility content, set by the app before layout.
    pub custom_a11y: HashMap<u64, CustomA11y>,
}

/// Fonts and sizes a [`Ui`] starts with.
#[derive(Clone, Copy)]
pub struct UiConfig {
    pub scale: f32,
    pub ui_font: &'static str,
    pub mono_font: &'static str,
    pub term_size: f32,
}

struct Measure {
    text: String,
    style: TextStyle,
    wrap: bool,
}

type LayoutNode = taffy::NodeId;

impl Ui {
    pub fn new(gpu: Gpu, format: wgpu::TextureFormat, cfg: &UiConfig) -> Ui {
        let renderer = Renderer::new(&gpu, format);
        Ui {
            gpu,
            renderer,
            text: TextSystem::new(cfg.scale, cfg.ui_font, cfg.mono_font, cfg.term_size),
            scene: Scene::default(),
            icons: HashMap::new(),
            images: Vec::new(),
            custom_a11y: HashMap::new(),
        }
    }

    pub fn scale(&self) -> f32 {
        self.text.scale
    }

    /// Registers an RGBA image (straight alpha) for [`image`] elements.
    pub fn image(&mut self, w: u32, h: u32, rgba: &[u8]) -> Option<u32> {
        let uv = self.renderer.color.insert(&self.gpu.queue, w, h, rgba)?;
        self.images.push(uv);
        Some(self.images.len() as u32 - 1)
    }

    /// Starts a frame on an empty scene.
    pub fn begin(&mut self, clear: Rgba) {
        self.scene.clear();
        self.renderer.clear = clear;
    }

    /// Lays `root` out in a `size` (logical pixels) window and paints it
    /// into the scene.
    pub fn layout(&mut self, root: El, size: (f32, f32)) -> Laid {
        let mut taffy: TaffyTree<Measure> = TaffyTree::new();
        let node = build(&mut taffy, &root);
        let text = &mut self.text;
        // Layout only fails on unknown node ids, which `build` never makes.
        let _ = taffy.compute_layout_with_measure(
            node,
            Size {
                width: AvailableSpace::Definite(size.0),
                height: AvailableSpace::Definite(size.1),
            },
            |known, avail, _, ctx, _| {
                let Some(m) = ctx else {
                    return Size::ZERO;
                };
                let max_w = m.wrap.then(|| {
                    known.width.unwrap_or(match avail.width {
                        AvailableSpace::Definite(w) => w,
                        _ => f32::MAX,
                    })
                });
                let s = text.shape(&m.text, m.style, max_w);
                let k = text.scale;
                Size {
                    width: known.width.unwrap_or(s.w / k),
                    height: known.height.unwrap_or(s.h / k),
                }
            },
        );
        let mut laid = Laid {
            customs: Vec::new(),
            tree: TreeUpdate {
                nodes: Vec::new(),
                tree: Some(TreeInfo::new(ROOT_NODE)),
                tree_id: TreeId::ROOT,
                focus: ROOT_NODE,
            },
        };
        let mut root_children = Vec::new();
        let mut next_id = 1u64;
        let mut cx = PaintCx {
            taffy: &taffy,
            laid: &mut laid,
            next_id: &mut next_id,
        };
        self.paint(&mut cx, node, &root, (0.0, 0.0), &mut root_children);
        let mut win = Node::new(Role::Window);
        win.set_label("Vorn");
        win.set_children(root_children);
        let k = self.scale();
        win.set_bounds(accesskit::Rect::new(
            0.0,
            0.0,
            f64::from(size.0 * k),
            f64::from(size.1 * k),
        ));
        laid.tree.nodes.insert(0, (ROOT_NODE, win));
        laid
    }

    fn paint(
        &mut self,
        cx: &mut PaintCx<'_>,
        node: LayoutNode,
        el: &El,
        origin: (f32, f32),
        a11y_parent: &mut Vec<NodeId>,
    ) {
        let Ok(l) = cx.taffy.layout(node) else {
            return;
        };
        let k = self.scale();
        let (x, y) = (origin.0 + l.location.x, origin.1 + l.location.y);
        let r = Rect::new(x * k, y * k, l.size.width * k, l.size.height * k);
        if el.bg.is_some() || el.border.is_some() {
            let fill = el.bg.unwrap_or_default();
            let border = el.border.map(|(w, c)| (w * k, c));
            self.scene.quad(r, fill, el.radius * k, border);
        }
        let ix = (x + l.padding.left + l.border.left) * k;
        let iy = (y + l.padding.top + l.border.top) * k;
        match &el.content {
            Content::None => {}
            Content::Text {
                text,
                style,
                color,
                wrap,
            } => {
                let inner = l.size.width - l.padding.left - l.padding.right;
                let sh = self.text.shape(text, *style, wrap.then_some(inner));
                self.text.draw(
                    &mut self.scene,
                    &mut self.renderer,
                    &self.gpu.queue,
                    &sh,
                    (ix, iy),
                    *color,
                );
            }
            Content::Icon { svg, size, color } => {
                let px = (size * k).round() as u32;
                if let Some(uv) = self.icon_raster(svg, px) {
                    let ir = Rect::new(ix.round(), iy.round(), uv[2], uv[3]);
                    self.scene.sprite(ir, uv, *color, false);
                }
            }
            Content::Image { id, color } => {
                if let Some(uv) = self.images.get(*id as usize).copied() {
                    self.scene.sprite(r, uv, *color, true);
                }
            }
            Content::Custom(id) => cx.laid.customs.push((*id, r)),
        }
        let custom = match el.content {
            Content::Custom(id) => self.custom_a11y.get(&id).cloned(),
            _ => None,
        };
        let role = el.role.or(custom.as_ref().map(|a| a.0));
        let mut own_children = Vec::new();
        let kids = cx.taffy.children(node).unwrap_or_default();
        for (c, ce) in kids.iter().zip(&el.children) {
            let target = if role.is_some() {
                &mut own_children
            } else {
                &mut *a11y_parent
            };
            self.paint(cx, *c, ce, (x, y), target);
        }
        let Some(role) = role else {
            return;
        };
        let id = NodeId(*cx.next_id);
        *cx.next_id += 1;
        let mut n = Node::new(role);
        if let Some(label) = &el.label {
            n.set_label(label.clone());
        }
        if let Some(v) = &el.value {
            n.set_value(v.clone());
        }
        if let Some((_, label, value)) = custom {
            n.set_label(label);
            n.set_value(value);
        }
        if el.selected {
            n.set_selected(true);
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
        cx.laid.tree.nodes.push((id, n));
        a11y_parent.push(id);
    }

    fn icon_raster(&mut self, svg: &str, px: u32) -> Option<[f32; 4]> {
        let key = (svg.to_owned(), px);
        if let Some(hit) = self.icons.get(&key) {
            return *hit;
        }
        let uv = rasterize_svg(svg, px)
            .and_then(|mask| self.renderer.mask.insert(&self.gpu.queue, px, px, &mask));
        self.icons.insert(key, uv);
        uv
    }

    /// Submits the scene to `view` (`size` physical pixels).
    pub fn render(&mut self, view: &wgpu::TextureView, size: (u32, u32)) {
        self.renderer.render(&self.gpu, &self.scene, view, size);
        self.text.trim();
    }
}

struct PaintCx<'a> {
    taffy: &'a TaffyTree<Measure>,
    laid: &'a mut Laid,
    next_id: &'a mut u64,
}

fn build(t: &mut TaffyTree<Measure>, el: &El) -> LayoutNode {
    if let Content::Text {
        text, style, wrap, ..
    } = &el.content
    {
        let m = Measure {
            text: text.clone(),
            style: *style,
            wrap: *wrap,
        };
        return t
            .new_leaf_with_context(el.style.clone(), m)
            .expect("taffy only fails on unknown node ids");
    }
    let kids: Vec<LayoutNode> = el.children.iter().map(|c| build(t, c)).collect();
    t.new_with_children(el.style.clone(), &kids)
        .expect("taffy only fails on unknown node ids")
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
