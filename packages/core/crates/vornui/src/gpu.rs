//! The GPU renderer: one wgpu device, two instanced pipelines (rounded quads,
//! and sprites from the mask or color atlas) and a scene drawn layer by
//! layer. Everything the UI shows is one of those two primitives, so a frame
//! is a handful of draw calls whatever it contains.

use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};

use crate::atlas::Atlases;
use crate::scene::{Quad, Rgba, Scene, Sprite};

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

    /// A device with no window, for offscreen drawing.
    pub fn headless() -> Result<Gpu, String> {
        Gpu::new(wgpu::Instance::new(instance_desc()), None)
    }

    pub fn adapter_name(&self) -> String {
        let i = self.adapter.get_info();
        format!("{} ({:?}, {:?})", i.name, i.backend, i.device_type)
    }

    /// The adapter rasterizes on the CPU (WARP, llvmpipe, SwiftShader).
    pub fn is_software(&self) -> bool {
        self.adapter.get_info().device_type == wgpu::DeviceType::Cpu
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

/// The pipelines and instance buffers for one target format.
pub struct GpuRenderer {
    quad_pipe: wgpu::RenderPipeline,
    sprite_pipe: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    bind: wgpu::BindGroup,
    quads: wgpu::Buffer,
    sprites: wgpu::Buffer,
    /// Reused across frames so a frame allocates nothing.
    quad_scratch: Vec<Quad>,
    sprite_scratch: Vec<Sprite>,
    /// The last frame submitted, waited on before the next is queued.
    in_flight: Option<wgpu::SubmissionIndex>,
}

const SHADER: &str = include_str!("shader.wgsl");

impl GpuRenderer {
    /// Pipelines drawing into `format`, sampling `atlases` (which must be
    /// GPU atlases on the same device).
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat, atlases: &Atlases) -> Option<GpuRenderer> {
        let d = &gpu.device;
        let mask_view = atlases.mask.view()?;
        let color_view = atlases.color.view()?;
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
                    resource: wgpu::BindingResource::TextureView(mask_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(color_view),
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
        let attrs = wgpu::vertex_attr_array![
            0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4
        ];
        let quad_pipe = pipe("quad_vs", "quad_fs", &attrs, std::mem::size_of::<Quad>());
        let sprite_pipe = pipe(
            "sprite_vs",
            "sprite_fs",
            &attrs,
            std::mem::size_of::<Sprite>(),
        );
        Some(GpuRenderer {
            quad_pipe,
            sprite_pipe,
            globals,
            bind,
            quads: vertex_buffer(d, "quads", 1 << 16),
            sprites: vertex_buffer(d, "sprites", 1 << 20),
            quad_scratch: Vec::new(),
            sprite_scratch: Vec::new(),
            in_flight: None,
        })
    }

    /// Encodes `scene` into `view` (`size` physical pixels) and submits it.
    /// Keeps at most two frames queued, as a swapchain would; without that a
    /// software adapter falls ever further behind until wgpu gives up.
    pub fn render(
        &mut self,
        gpu: &Gpu,
        scene: &Scene,
        atlases: &Atlases,
        clear: Rgba,
        view: &wgpu::TextureView,
        size: (u32, u32),
    ) {
        let g = Globals {
            viewport: [size.0 as f32, size.1 as f32],
            mask_size: [atlases.mask.size as f32; 2],
            color_size: [atlases.color.size as f32; 2],
            _pad: [0.0; 2],
        };
        gpu.queue
            .write_buffer(&self.globals, 0, bytemuck::bytes_of(&g));
        self.quad_scratch.clear();
        self.sprite_scratch.clear();
        for l in &scene.layers {
            self.quad_scratch.extend_from_slice(&l.quads);
            self.sprite_scratch.extend_from_slice(&l.sprites);
        }
        upload(
            gpu,
            &mut self.quads,
            bytemuck::cast_slice(&self.quad_scratch),
            "quads",
        );
        upload(
            gpu,
            &mut self.sprites,
            bytemuck::cast_slice(&self.sprite_scratch),
            "sprites",
        );
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let c = clear.0;
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
        let submitted = gpu.queue.submit(Some(enc.finish()));
        settle(gpu, &mut self.in_flight, submitted);
    }
}

/// Records `submitted` as in flight and waits for the one before it.
pub(crate) fn settle(
    gpu: &Gpu,
    in_flight: &mut Option<wgpu::SubmissionIndex>,
    submitted: wgpu::SubmissionIndex,
) {
    let poll = match in_flight.replace(submitted) {
        Some(prev) => wgpu::PollType::Wait {
            submission_index: Some(prev),
            timeout: None,
        },
        None => wgpu::PollType::Poll,
    };
    // A lost device shows up on the next submit; nothing to do here.
    let _ = gpu.device.poll(poll);
}

fn vertex_buffer(d: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    d.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Writes `data` to `buf`, replacing it with a larger one first if needed.
fn upload(gpu: &Gpu, buf: &mut wgpu::Buffer, data: &[u8], label: &str) {
    if data.is_empty() {
        return;
    }
    if (data.len() as u64) > buf.size() {
        *buf = vertex_buffer(&gpu.device, label, (data.len() as u64).next_power_of_two());
    }
    gpu.queue.write_buffer(buf, 0, data);
}
