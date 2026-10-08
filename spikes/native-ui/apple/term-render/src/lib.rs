//! Option B of the Apple client spike: a pane drawn by Rust on the GPU.
//!
//! The host app hands each pane's view (an NSView or a UIView) to a
//! [`Surface`]; the surface adds its own Metal layer to it and draws the
//! pane from the shared grid client on a thread of its own. That thread
//! sleeps until the pane changes, builds one instanced quad per cell
//! background, glyph, decoration and the cursor, and presents in FIFO mode
//! with a one-frame queue, so the drawable pool paces it to the display.
//! The host owns input and accessibility; the renderer draws the IME
//! preedit it is given at the cursor.

pub mod glyphs;
mod layer;

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use glyphs::{Atlas, Font, ATLAS};
use layer::Layer;
use spike_core::view::flags;
use spike_core::PaneView;

/// Padding around the cells inside a pane, in points.
pub const PAD_X: f64 = 8.0;
pub const PAD_Y: f64 = 4.0;

/// What the renderer tells its host after each present.
pub struct Presented {
    pub at: Instant,
    /// CPU time spent building and encoding the frame, without the wait
    /// for a drawable.
    pub work_ms: f64,
    pub probe_hit: bool,
    pub had_view: bool,
}

/// The pane's source and the frame observer.
pub trait Host: Send + Sync + 'static {
    fn view(&self) -> Option<PaneView>;
    fn presented(&self, p: Presented);
}

struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// One glyph cache per (font size, scale), shared by every surface.
    caches: Mutex<HashMap<(u64, u64), Arc<Mutex<Glyphs>>>>,
}

/// A font, its atlas's packer and the atlas texture.
struct Glyphs {
    font: Font,
    atlas: Atlas,
    tex: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Gpu {
    fn glyphs(&self, pt: f64, scale: f64) -> Arc<Mutex<Glyphs>> {
        let mut c = self.caches.lock().unwrap();
        let g = c.entry((pt.to_bits(), scale.to_bits())).or_insert_with(|| {
            let tex = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("atlas"),
                size: wgpu::Extent3d {
                    width: ATLAS,
                    height: ATLAS,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = tex.create_view(&Default::default());
            Arc::new(Mutex::new(Glyphs {
                font: Font::new(pt, scale),
                atlas: Atlas::new(),
                tex,
                view,
            }))
        });
        Arc::clone(g)
    }
}

/// One per app: the device and pipeline every surface shares.
pub struct Renderer {
    gpu: Arc<Gpu>,
}

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    use std::task::{Context, Poll, Waker};
    let mut f = std::pin::pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

impl Renderer {
    pub fn new() -> Result<Renderer, String> {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::METAL;
        let instance = wgpu::Instance::new(desc);
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("term-render"),
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty,
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(
                    1,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(2, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let attrs = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Uint32];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Inst>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attrs,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: FORMAT,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
        Ok(Renderer {
            gpu: Arc::new(Gpu {
                instance,
                adapter,
                device,
                queue,
                pipeline,
                layout,
                sampler,
                caches: Mutex::new(HashMap::new()),
            }),
        })
    }
}

/// One quad: `rect` and `uv` in pixels, premultiplied later.
#[repr(C)]
#[derive(Clone, Copy)]
struct Inst {
    rect: [f32; 4],
    uv: [f32; 4],
    color: [f32; 4],
    kind: u32,
    _pad: [u32; 3],
}

#[derive(Clone)]
struct Params {
    w: f64,
    h: f64,
    scale: f64,
    focused: bool,
    preedit: String,
    dirty: bool,
    quit: bool,
}

struct Shared {
    p: Mutex<Params>,
    cv: Condvar,
}

/// Wakes a surface's thread: the pane changed.
#[derive(Clone)]
pub struct Waker(Arc<Shared>);

impl Waker {
    pub fn wake(&self) {
        self.0.p.lock().unwrap().dirty = true;
        self.0.cv.notify_one();
    }
}

/// Cell geometry in points, for the host's IME and hit testing.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    pub cell_w: f64,
    pub cell_h: f64,
    pub pad_x: f64,
    pub pad_y: f64,
}

pub struct Surface {
    shared: Arc<Shared>,
    layer: Arc<Layer>,
    thread: Option<JoinHandle<()>>,
    pub metrics: Metrics,
}

impl Surface {
    /// Attaches a renderer to `view` and starts its thread.
    ///
    /// # Safety
    /// `view` is a live NSView (macOS) or UIView (iOS) that outlives the
    /// surface, and this runs on the main thread.
    pub unsafe fn new(
        r: &Renderer,
        view: *mut c_void,
        host: Arc<dyn Host>,
        font_pt: f64,
        scale: f64,
    ) -> Result<Surface, String> {
        let layer = Arc::new(Layer::attach(view, scale));
        let surface = r
            .gpu
            .instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(layer.ptr()))
            .map_err(|e| e.to_string())?;
        let pt = Font::new(font_pt, 1.0);
        let metrics = Metrics {
            cell_w: pt.cell_w,
            cell_h: pt.cell_h,
            pad_x: PAD_X,
            pad_y: PAD_Y,
        };
        let shared = Arc::new(Shared {
            p: Mutex::new(Params {
                w: 0.0,
                h: 0.0,
                scale,
                focused: false,
                preedit: String::new(),
                dirty: true,
                quit: false,
            }),
            cv: Condvar::new(),
        });
        let mut w = Worker::new(Arc::clone(&r.gpu), surface, host, font_pt, scale);
        let sh = Arc::clone(&shared);
        let keep = Arc::clone(&layer);
        let thread = std::thread::Builder::new()
            .name("term-render".into())
            .spawn(move || {
                let _layer = keep;
                w.run(&sh);
            })
            .map_err(|e| e.to_string())?;
        Ok(Surface {
            shared,
            layer,
            thread: Some(thread),
            metrics,
        })
    }

    pub fn waker(&self) -> Waker {
        Waker(Arc::clone(&self.shared))
    }

    fn update(&self, f: impl FnOnce(&mut Params)) {
        let mut p = self.shared.p.lock().unwrap();
        f(&mut p);
        p.dirty = true;
        self.shared.cv.notify_one();
    }

    /// Main thread: the host view is `w`x`h` points. Answers the grid that
    /// fits, in cells.
    pub fn set_size(&self, w: f64, h: f64) -> (u16, u16) {
        let scale = self.shared.p.lock().unwrap().scale;
        self.layer.set_frame(w, h, scale);
        self.update(|p| {
            p.w = w;
            p.h = h;
        });
        let m = self.metrics;
        (
            ((w - 2.0 * m.pad_x) / m.cell_w).floor().max(2.0) as u16,
            ((h - 2.0 * m.pad_y) / m.cell_h).floor().max(1.0) as u16,
        )
    }

    pub fn set_scale(&self, scale: f64) {
        let (w, h) = {
            let p = self.shared.p.lock().unwrap();
            (p.w, p.h)
        };
        self.layer.set_frame(w, h, scale);
        self.update(|p| p.scale = scale);
    }

    pub fn set_focus(&self, focused: bool) {
        self.update(|p| p.focused = focused);
    }

    pub fn set_preedit(&self, text: &str) {
        self.update(|p| p.preedit = text.to_owned());
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        self.update(|p| p.quit = true);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        self.layer.detach();
    }
}

/// The render thread's state.
struct Worker {
    gpu: Arc<Gpu>,
    surface: wgpu::Surface<'static>,
    host: Arc<dyn Host>,
    font_pt: f64,
    scale: f64,
    glyphs: Arc<Mutex<Glyphs>>,
    /// The cell in pixels, from the font.
    cell: (f64, f64),
    /// The atlas generation this surface last drew with.
    generation: u64,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    buf: Option<wgpu::Buffer>,
    insts: Vec<Inst>,
    configured: (u32, u32),
}

fn rgba(c: u32, a: f32) -> [f32; 4] {
    let ch = |s: u32| ((c >> s) & 0xff) as f32 / 255.0;
    [ch(16), ch(8), ch(0), a]
}

fn wide(c: char) -> bool {
    let u = c as u32;
    (0x1100..=0x115f).contains(&u)
        || (0x2e80..=0xa4cf).contains(&u)
        || (0xac00..=0xd7a3).contains(&u)
        || (0xf900..=0xfaff).contains(&u)
        || (0xfe30..=0xfe4f).contains(&u)
        || (0xff00..=0xff60).contains(&u)
        || (0xffe0..=0xffe6).contains(&u)
        || (0x1f300..=0x1faff).contains(&u)
        || (0x20000..=0x3fffd).contains(&u)
}

impl Worker {
    fn new(gpu: Arc<Gpu>, surface: wgpu::Surface<'static>, host: Arc<dyn Host>, font_pt: f64, scale: f64) -> Worker {
        let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let glyphs = gpu.glyphs(font_pt, scale);
        let (bind, cell) = Worker::bind(&gpu, &uniform, &glyphs);
        Worker {
            gpu,
            surface,
            host,
            font_pt,
            scale,
            glyphs,
            cell,
            generation: 0,
            uniform,
            bind,
            buf: None,
            insts: Vec::new(),
            configured: (0, 0),
        }
    }

    fn bind(gpu: &Gpu, uniform: &wgpu::Buffer, glyphs: &Mutex<Glyphs>) -> (wgpu::BindGroup, (f64, f64)) {
        let g = glyphs.lock().unwrap();
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &gpu.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&g.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&gpu.sampler),
                },
            ],
        });
        (bind, (g.font.cell_w, g.font.cell_h))
    }

    fn run(&mut self, sh: &Shared) {
        loop {
            let p = {
                let mut p = sh.p.lock().unwrap();
                while !p.dirty && !p.quit {
                    p = sh.cv.wait(p).unwrap();
                }
                if p.quit {
                    return;
                }
                p.dirty = false;
                p.clone()
            };
            if self.frame(&p) {
                sh.p.lock().unwrap().dirty = true;
            }
        }
    }

    fn configure(&mut self, w: u32, h: u32) {
        let caps = self.surface.get_capabilities(&self.gpu.adapter);
        self.surface.configure(
            &self.gpu.device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: FORMAT,
                color_space: Default::default(),
                width: w,
                height: h,
                present_mode: wgpu::PresentMode::Fifo,
                desired_maximum_frame_latency: 1,
                alpha_mode: caps.alpha_modes[0],
                view_formats: vec![],
            },
        );
        self.configured = (w, h);
    }

    /// Draws one frame; true if it must be drawn again.
    fn frame(&mut self, p: &Params) -> bool {
        if p.scale != self.scale {
            self.scale = p.scale;
            self.glyphs = self.gpu.glyphs(self.font_pt, p.scale);
            (self.bind, self.cell) = Worker::bind(&self.gpu, &self.uniform, &self.glyphs);
        }
        let (w, h) = ((p.w * p.scale).round() as u32, (p.h * p.scale).round() as u32);
        if w == 0 || h == 0 {
            return false;
        }
        let t0 = Instant::now();
        let view = self.host.view();
        let bg = view.as_ref().map_or(0x141416, |v| v.bg);
        self.insts.clear();
        let again = {
            // Builds and uploads under the shared atlas's lock, so the
            // texture holds every glyph this frame's quads point at.
            let glyphs = Arc::clone(&self.glyphs);
            let mut gl = glyphs.lock().unwrap();
            gl.atlas.overflowed = false;
            if let Some(v) = &view {
                self.build(&mut gl, v, p);
            }
            self.upload(&mut gl);
            let again = gl.atlas.overflowed || gl.atlas.generation != self.generation;
            self.generation = gl.atlas.generation;
            again
        };
        if self.configured != (w, h) {
            self.configure(w, h);
        }
        let build_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.configure(w, h);
                return true;
            }
            _ => return true,
        };
        let t1 = Instant::now();
        self.encode(&frame, bg, w, h);
        self.gpu.queue.present(frame);
        let at = Instant::now();
        self.host.presented(Presented {
            at,
            work_ms: build_ms + (at - t1).as_secs_f64() * 1000.0,
            probe_hit: view.as_ref().is_some_and(|v| v.probe_hit),
            had_view: view.is_some(),
        });
        again
    }

    fn upload(&self, gl: &mut Glyphs) {
        let Glyphs { atlas, tex, .. } = gl;
        for (x, y, uw, uh, px) in atlas.uploads.drain(..) {
            self.gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &px,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(uw * 4),
                    rows_per_image: Some(uh),
                },
                wgpu::Extent3d {
                    width: uw,
                    height: uh,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    fn encode(&mut self, frame: &wgpu::SurfaceTexture, bg: u32, w: u32, h: u32) {
        let g = &self.gpu;
        let u: [f32; 4] = [w as f32, h as f32, ATLAS as f32, ATLAS as f32];
        g.queue.write_buffer(&self.uniform, 0, as_bytes(&u));
        let need = (self.insts.len().max(64) * std::mem::size_of::<Inst>()) as u64;
        if self.buf.as_ref().is_none_or(|b| b.size() < need) {
            self.buf = Some(g.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("instances"),
                size: need.next_power_of_two(),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let buf = self.buf.as_ref().unwrap();
        g.queue.write_buffer(buf, 0, as_bytes(&self.insts));
        let target = frame.texture.create_view(&Default::default());
        let mut enc = g.device.create_command_encoder(&Default::default());
        {
            let c = rgba(bg, 1.0);
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: c[0] as f64,
                            g: c[1] as f64,
                            b: c[2] as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            if !self.insts.is_empty() {
                pass.set_pipeline(&g.pipeline);
                pass.set_bind_group(0, &self.bind, &[]);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..4, 0..self.insts.len() as u32);
            }
        }
        g.queue.submit([enc.finish()]);
    }

    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        self.insts.push(Inst {
            rect: [x, y, w, h],
            uv: [0.0; 4],
            color,
            kind: 0,
            _pad: [0; 3],
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn glyph(&mut self, gl: &mut Glyphs, x: f32, y: f32, text: &str, variant: u8, ncols: u8, ascii: bool, color: [f32; 4]) {
        let Glyphs { font, atlas, .. } = gl;
        let g = atlas.get(font, text, variant, ncols, ascii);
        if g.empty {
            return;
        }
        self.insts.push(Inst {
            rect: [x + g.off[0], y + g.off[1], g.uv[2], g.uv[3]],
            uv: g.uv,
            color,
            kind: if g.color { 2 } else { 1 },
            _pad: [0; 3],
        });
    }

    fn build(&mut self, gl: &mut Glyphs, v: &PaneView, p: &Params) {
        let s = self.scale;
        let (cw, ch) = (self.cell.0, self.cell.1.round() as f32);
        let (ox, oy) = (PAD_X * s, PAD_Y * s);
        let x_of = |col: u16| (ox + col as f64 * cw).round() as f32;
        let y_of = |row: u16| (oy + row as f64 * ch as f64).round() as f32;
        for r in &v.runs {
            if r.bg != v.bg {
                let x = x_of(r.col);
                self.rect(x, y_of(r.row), x_of(r.col + r.ncols) - x, ch, rgba(r.bg, 1.0));
            }
        }
        let mut buf = [0u8; 4];
        for r in &v.runs {
            let alpha = if r.flags & flags::FAINT != 0 { 0.6 } else { 1.0 };
            let fg = rgba(r.fg, alpha);
            let variant = (r.flags & flags::BOLD != 0) as u8 + 2 * (r.flags & flags::ITALIC != 0) as u8;
            let (x, y) = (x_of(r.col), y_of(r.row));
            let text = v.run_text(r);
            if r.flags & flags::CLUSTER != 0 {
                let n = if r.flags & flags::WIDE != 0 { 2 } else { 1 };
                self.glyph(gl, x, y, text, variant, n, false, fg);
            } else {
                for (i, b) in text.bytes().enumerate() {
                    if b > 32 && b < 127 {
                        buf[0] = b;
                        let c = std::str::from_utf8(&buf[..1]).unwrap_or(" ");
                        self.glyph(gl, x_of(r.col + i as u16), y, c, variant, 1, true, fg);
                    }
                }
            }
            let w = x_of(r.col + r.ncols) - x;
            let t = s as f32;
            if r.flags & flags::UNDERLINE != 0 {
                self.rect(x, y + ch - 1.5 * t, w, t, fg);
            }
            if r.flags & flags::STRIKE != 0 {
                self.rect(x, y + (ch / 2.0).round(), w, t, fg);
            }
        }
        let (cx, cy) = (x_of(v.cursor_x), y_of(v.cursor_y));
        let cwf = x_of(v.cursor_x + 1) - cx;
        let t = s as f32;
        if !p.preedit.is_empty() {
            // The IME's composition, drawn over the cells at the cursor.
            let cells: Vec<(char, u16)> = p.preedit.chars().map(|c| (c, if wide(c) { 2 } else { 1 })).collect();
            let total: u16 = cells.iter().map(|c| c.1).sum();
            let w = x_of(v.cursor_x + total) - cx;
            self.rect(cx, cy, w, ch, rgba(v.bg, 1.0));
            let mut col = v.cursor_x;
            for (c, n) in cells {
                let mut b = [0u8; 4];
                let s = c.encode_utf8(&mut b);
                self.glyph(gl, x_of(col), cy, s, 0, n as u8, false, rgba(v.fg, 1.0));
                col += n;
            }
            self.rect(cx, cy + ch - t, w, t, rgba(v.fg, 1.0));
        } else if v.cursor_visible {
            let c = rgba(v.cursor_color, 0.75);
            match v.cursor_style {
                2 => self.rect(cx, cy, 2.0 * t, ch, c),
                3 => self.rect(cx, cy + ch - 2.0 * t, cwf, 2.0 * t, c),
                0 if p.focused => self.rect(cx, cy, cwf, ch, c),
                _ => {
                    let c = rgba(v.cursor_color, 1.0);
                    self.rect(cx, cy, cwf, t, c);
                    self.rect(cx, cy + ch - t, cwf, t, c);
                    self.rect(cx, cy, t, ch, c);
                    self.rect(cx + cwf - t, cy, t, ch, c);
                }
            }
        }
    }
}

fn as_bytes<T: Copy>(v: &[T]) -> &[u8] {
    // SAFETY: plain-old-data `repr(C)` values with no padding bytes read
    // as anything but bytes.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast(), std::mem::size_of_val(v)) }
}
