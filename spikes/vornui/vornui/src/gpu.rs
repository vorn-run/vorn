//! Drawing: one wgpu device, two instanced pipelines (rounded quads, and
//! sprites from the glyph/icon mask atlas or the color atlas) and a scene of
//! layers drawn in order. Everything the UI shows is one of those two
//! primitives, so a frame is a handful of draw calls whatever it contains.

use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::atlas::Atlas;

/// A color with straight alpha, components 0..=1, in sRGB: blending happens
/// in sRGB space, as a browser composites CSS colors.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rgba(pub [f32; 4]);

impl Rgba {
    pub fn hex(c: u32) -> Rgba {
        Rgba::hexa(c, 1.0)
    }

    pub fn hexa(c: u32, a: f32) -> Rgba {
        let ch = |s: u32| ((c >> s) & 0xff) as f32 / 255.0;
        Rgba([ch(16), ch(8), ch(0), a])
    }

    pub fn alpha(self, a: f32) -> Rgba {
        Rgba([self.0[0], self.0[1], self.0[2], self.0[3] * a])
    }
}

/// A rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Quad {
    pub rect: [f32; 4],
    pub color: [f32; 4],
    pub border_color: [f32; 4],
    /// Corner radius, border width.
    pub params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Sprite {
    pub rect: [f32; 4],
    /// Atlas texels: x, y, w, h.
    pub uv: [f32; 4],
    pub color: [f32; 4],
    /// 0: tint the mask atlas; 1: the color atlas times alpha.
    pub kind: [f32; 4],
}

#[derive(Default)]
pub struct Layer {
    pub quads: Vec<Quad>,
    pub sprites: Vec<Sprite>,
}

/// What a frame draws: layers in order, each its quads then its sprites.
#[derive(Default)]
pub struct Scene {
    pub layers: Vec<Layer>,
}

impl Scene {
    pub fn clear(&mut self) {
        self.layers.clear();
        self.layers.push(Layer::default());
    }

    /// Starts a layer drawn over everything so far.
    pub fn layer(&mut self) {
        self.layers.push(Layer::default());
    }

    fn top(&mut self) -> &mut Layer {
        if self.layers.is_empty() {
            self.layers.push(Layer::default());
        }
        let n = self.layers.len();
        &mut self.layers[n - 1]
    }

    pub fn quad(&mut self, r: Rect, fill: Rgba, radius: f32, border: Option<(f32, Rgba)>) {
        let (bw, bc) = border.unwrap_or((0.0, fill));
        self.top().quads.push(Quad {
            rect: [r.x, r.y, r.w, r.h],
            color: fill.0,
            border_color: bc.0,
            params: [radius, bw, 0.0, 0.0],
        });
    }

    pub fn sprite(&mut self, r: Rect, uv: [f32; 4], color: Rgba, colored: bool) {
        self.top().sprites.push(Sprite {
            rect: [r.x, r.y, r.w, r.h],
            uv,
            color: color.0,
            kind: [f32::from(u8::from(colored)), 0.0, 0.0, 0.0],
        });
    }

    pub fn counts(&self) -> (usize, usize) {
        self.layers
            .iter()
            .fold((0, 0), |(q, s), l| (q + l.quads.len(), s + l.sprites.len()))
    }
}

/// The device and queue, shared by every target.
pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Gpu {
    /// A device compatible with `surface`, or any device when there is none.
    pub fn new(
        instance: wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
    ) -> Result<Gpu, String> {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: surface,
            force_fallback_adapter: false,
        }))
        .map_err(|e| format!("no GPU adapter: {e}"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("vornui"),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| format!("no GPU device: {e}"))?;
        Ok(Gpu {
            instance,
            adapter,
            device,
            queue,
        })
    }

    pub fn headless() -> Result<Gpu, String> {
        Gpu::new(wgpu::Instance::new(instance_desc()), None)
    }

    pub fn adapter_name(&self) -> String {
        let i = self.adapter.get_info();
        format!("{} ({:?})", i.name, i.backend)
    }
}

pub fn instance_desc() -> wgpu::InstanceDescriptor {
    let mut d = wgpu::InstanceDescriptor::new_without_display_handle();
    d.backends = wgpu::Backends::PRIMARY;
    d
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    viewport: [f32; 2],
    mask_size: [f32; 2],
    color_size: [f32; 2],
    _pad: [f32; 2],
}

/// The pipelines, atlases and instance buffers for one target format.
pub struct Renderer {
    quad_pipe: wgpu::RenderPipeline,
    sprite_pipe: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    bind: wgpu::BindGroup,
    quads: wgpu::Buffer,
    sprites: wgpu::Buffer,
    pub mask: Atlas,
    pub color: Atlas,
    /// Background of every frame.
    pub clear: Rgba,
}

const SHADER: &str = include_str!("shader.wgsl");

impl Renderer {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat) -> Renderer {
        let d = &gpu.device;
        let max = d.limits().max_texture_dimension_2d.min(4096);
        let mask = Atlas::new(d, max, wgpu::TextureFormat::R8Unorm, "mask atlas");
        let color = Atlas::new(
            d,
            2048.min(max),
            wgpu::TextureFormat::Rgba8Unorm,
            "color atlas",
        );
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vornui"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(std::mem::size_of::<Globals>() as u64),
                    },
                    count: None,
                },
                tex(1),
                tex(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let globals = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&mask.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&color.view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipe = |vs: &str, fs: &str, attrs: &[wgpu::VertexAttribute], stride: usize| {
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(vs),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: stride as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: attrs,
                    }],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let attrs4 = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4];
        let quad_pipe = pipe("quad_vs", "quad_fs", &attrs4, std::mem::size_of::<Quad>());
        let sprite_pipe = pipe(
            "sprite_vs",
            "sprite_fs",
            &attrs4,
            std::mem::size_of::<Sprite>(),
        );
        let buf = |label, size| {
            d.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        Renderer {
            quad_pipe,
            sprite_pipe,
            globals,
            bind,
            quads: buf("quads", 1 << 16),
            sprites: buf("sprites", 1 << 20),
            mask,
            color,
            clear: Rgba::hex(0),
        }
    }

    /// Encodes `scene` into `view` (`size` physical pixels) and submits it.
    /// Returns without waiting for the GPU, as a presented frame would.
    pub fn render(&mut self, gpu: &Gpu, scene: &Scene, view: &wgpu::TextureView, size: (u32, u32)) {
        let g = Globals {
            viewport: [size.0 as f32, size.1 as f32],
            mask_size: [self.mask.size as f32; 2],
            color_size: [self.color.size as f32; 2],
            _pad: [0.0; 2],
        };
        gpu.queue
            .write_buffer(&self.globals, 0, bytemuck::bytes_of(&g));
        let quads: Vec<Quad> = scene
            .layers
            .iter()
            .flat_map(|l| l.quads.iter().copied())
            .collect();
        let sprites: Vec<Sprite> = scene
            .layers
            .iter()
            .flat_map(|l| l.sprites.iter().copied())
            .collect();
        grow(gpu, &mut self.quads, bytemuck::cast_slice(&quads), "quads");
        grow(
            gpu,
            &mut self.sprites,
            bytemuck::cast_slice(&sprites),
            "sprites",
        );
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let c = self.clear.0;
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(c[0]),
                            g: f64::from(c[1]),
                            b: f64::from(c[2]),
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_bind_group(0, &self.bind, &[]);
            let qs = std::mem::size_of::<Quad>() as u64;
            let ss = std::mem::size_of::<Sprite>() as u64;
            let (mut q0, mut s0) = (0u32, 0u32);
            for l in &scene.layers {
                let (nq, ns) = (l.quads.len() as u32, l.sprites.len() as u32);
                if nq > 0 {
                    pass.set_pipeline(&self.quad_pipe);
                    pass.set_vertex_buffer(0, self.quads.slice(u64::from(q0) * qs..));
                    pass.draw(0..4, 0..nq);
                }
                if ns > 0 {
                    pass.set_pipeline(&self.sprite_pipe);
                    pass.set_vertex_buffer(0, self.sprites.slice(u64::from(s0) * ss..));
                    pass.draw(0..4, 0..ns);
                }
                q0 += nq;
                s0 += ns;
            }
        }
        gpu.queue.submit(Some(enc.finish()));
        // Retires finished frames' resources without waiting on the GPU.
        let _ = gpu.device.poll(wgpu::PollType::Poll);
    }
}

/// Writes `data` to `buf`, replacing it with a larger one first if needed.
fn grow(gpu: &Gpu, buf: &mut wgpu::Buffer, data: &[u8], label: &str) {
    if data.is_empty() {
        return;
    }
    if (data.len() as u64) > buf.size() {
        *buf = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: &vec![0u8; data.len().next_power_of_two()],
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
    }
    gpu.queue.write_buffer(buf, 0, data);
}

/// An offscreen target: what the benches render to, and what screenshots
/// are read back from.
pub struct Offscreen {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub size: (u32, u32),
}

pub const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl Offscreen {
    pub fn new(gpu: &Gpu, size: (u32, u32)) -> Offscreen {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OFFSCREEN_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Offscreen {
            texture,
            view,
            size,
        }
    }

    /// Waits for the GPU and copies the target back as RGBA rows.
    pub fn read(&self, gpu: &Gpu) -> Result<Vec<u8>, String> {
        let (w, h) = self.size;
        let row = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(row) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit(Some(enc.finish()));
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| e.to_string())?;
        let data = slice.get_mapped_range();
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            let start = y * row as usize;
            out.extend_from_slice(&data[start..start + w as usize * 4]);
        }
        Ok(out)
    }

    pub fn save_png(&self, gpu: &Gpu, path: &str) -> Result<(), String> {
        let rgba = self.read(gpu)?;
        crate::write_png(path, self.size, &rgba)
    }
}
