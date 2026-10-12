//! The GPU half of backdrop blur (`blur.wgsl`): a chain of half-size
//! textures the frame under a blur is copied into, halved down, blurred
//! and composited back from. The chain is made the first time a blur is
//! drawn, so a window that never blurs pays nothing for it.
//!
//! Every pass loads texels exactly and writes RGBA8, so the CPU renderer's
//! `backdrop` reproduces it; the two are kept together.

use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};

use crate::cpu::gauss_weights;
use crate::gpu::Gpu;
use crate::scene::Blur;

/// The deepest halving a blur runs at (a factor of 32).
const LEVELS: usize = 5;
/// Uniform slots are this far apart, the alignment wgpu requires of
/// dynamic offsets.
const SLOT: u64 = 256;
/// Blurs drawn in one frame; later ones are skipped. Each takes at most
/// `LEVELS + 3` passes.
pub(crate) const MAX_BLURS: usize = 32;
const MAX_PASSES: usize = MAX_BLURS * (LEVELS + 3);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PassUniform {
    rect: [f32; 4],
    clip: [f32; 4],
    shape: [f32; 4],
    dir_size: [f32; 4],
    weights: [f32; 16],
}

/// One texture of the chain, with the bind group that samples it.
struct Level {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    bind: wgpu::BindGroup,
    size: (u32, u32),
}

/// The textures for one frame size: the copy of the frame, its halvings,
/// and a scratch texture for the Gaussian's first direction.
struct Chain {
    size: (u32, u32),
    levels: Vec<Level>,
    tmp: Level,
}

pub(crate) struct Blurrer {
    down: wgpu::RenderPipeline,
    gauss: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    format: wgpu::TextureFormat,
    chain: Option<Chain>,
    /// This frame's pass uniforms, written once before submitting.
    passes: Vec<PassUniform>,
}

const SHADER: &str = include_str!("blur.wgsl");

impl Blurrer {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat) -> Blurrer {
        let d = &gpu.device;
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vornui blur"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout =
            d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("blur"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: true,
                            min_binding_size: NonZeroU64::new(
                                std::mem::size_of::<PassUniform>() as u64
                            ),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            });
        let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipe = |fs: &str, blend: Option<wgpu::BlendState>| {
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("blur_vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let uniforms = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("blur passes"),
            size: SLOT * MAX_PASSES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Blurrer {
            down: pipe("down_fs", None),
            gauss: pipe("gauss_fs", None),
            composite: pipe(
                "composite_fs",
                Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            ),
            layout,
            uniforms,
            format,
            chain: None,
            passes: Vec::new(),
        }
    }

    /// Starts a frame: no passes recorded yet.
    pub fn begin(&mut self) {
        self.passes.clear();
    }

    /// Whether another blur fits in this frame.
    pub fn has_room(&self) -> bool {
        self.passes.len() + LEVELS + 3 <= MAX_PASSES
    }

    /// Records `blur` over `target` (`size` pixels, which must allow
    /// copying from): copy, halve, blur, composite. The target's render
    /// pass must be ended first.
    pub fn encode(
        &mut self,
        gpu: &Gpu,
        enc: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        size: (u32, u32),
        blur: &Blur,
    ) {
        let frame = [0, 0, size.0 as i32, size.1 as i32];
        let shown = pixel_box(blur.bounds().outset(1.0), frame);
        if shown[0] >= shown[2] || shown[1] >= shown[3] || !self.has_room() {
            return;
        }
        let m = blur.margin().ceil() as i32;
        let src = intersect(
            [shown[0] - m, shown[1] - m, shown[2] + m, shown[3] + m],
            frame,
        );
        self.ensure_chain(gpu, size);
        let Some(chain) = &self.chain else {
            return;
        };
        let target_tex = target.texture();
        enc.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: target_tex,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: src[0] as u32,
                    y: src[1] as u32,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &chain.levels[0].texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: src[0] as u32,
                    y: src[1] as u32,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: (src[2] - src[0]) as u32,
                height: (src[3] - src[1]) as u32,
                depth_or_array_layers: 1,
            },
        );
        let f = blur.factor();
        let k = f.trailing_zeros() as usize;
        let at_level = |i: usize| {
            let s = 1 << i;
            [
                src[0] / s,
                src[1] / s,
                (src[2] + s - 1) / s,
                (src[3] + s - 1) / s,
            ]
        };
        let mut uniform = PassUniform::zeroed();
        for i in 1..=k {
            let from = &chain.levels[i - 1];
            uniform.dir_size = [0.0, 0.0, from.size.0 as f32, from.size.1 as f32];
            let slot = push(&mut self.passes, uniform);
            run(
                enc,
                &self.down,
                &chain.levels[i].view,
                &from.bind,
                slot,
                at_level(i),
            );
        }
        let weights = gauss_weights(blur);
        for (w, v) in uniform.weights.iter_mut().zip(&weights) {
            *w = *v;
        }
        let level = &chain.levels[k];
        let (lw, lh) = (level.size.0 as f32, level.size.1 as f32);
        uniform.shape = [0.0, 0.0, 0.0, (weights.len() - 1) as f32];
        uniform.dir_size = [1.0, 0.0, lw, lh];
        let slot = push(&mut self.passes, uniform);
        run(
            enc,
            &self.gauss,
            &chain.tmp.view,
            &level.bind,
            slot,
            at_level(k),
        );
        uniform.dir_size = [0.0, 1.0, lw, lh];
        let slot = push(&mut self.passes, uniform);
        run(
            enc,
            &self.gauss,
            &level.view,
            &chain.tmp.bind,
            slot,
            at_level(k),
        );
        uniform.rect = blur.rect;
        uniform.clip = blur.clip;
        uniform.shape = [blur.radius, blur.alpha, 1.0 / f as f32, 0.0];
        uniform.dir_size = [0.0, 0.0, lw, lh];
        let slot = push(&mut self.passes, uniform);
        run(enc, &self.composite, target, &level.bind, slot, shown);
    }

    /// Writes this frame's pass uniforms; call before submitting.
    pub fn upload(&self, gpu: &Gpu) {
        if self.passes.is_empty() {
            return;
        }
        let mut bytes = vec![0u8; self.passes.len() * SLOT as usize];
        for (chunk, p) in bytes.chunks_mut(SLOT as usize).zip(&self.passes) {
            chunk[..std::mem::size_of::<PassUniform>()].copy_from_slice(bytemuck::bytes_of(p));
        }
        gpu.queue.write_buffer(&self.uniforms, 0, &bytes);
    }

    fn ensure_chain(&mut self, gpu: &Gpu, size: (u32, u32)) {
        if self.chain.as_ref().is_some_and(|c| c.size == size) {
            return;
        }
        let level = |size: (u32, u32), usage: wgpu::TextureUsages| {
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("blur level"),
                size: wgpu::Extent3d {
                    width: size.0.max(1),
                    height: size.1.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: usage | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.uniforms,
                            offset: 0,
                            size: NonZeroU64::new(std::mem::size_of::<PassUniform>() as u64),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                ],
            });
            Level {
                texture,
                view,
                bind,
                size,
            }
        };
        let halve = |s: (u32, u32), i: u32| (s.0.div_ceil(1 << i), s.1.div_ceil(1 << i));
        let mut levels = vec![level(size, wgpu::TextureUsages::COPY_DST)];
        for i in 1..=LEVELS as u32 {
            levels.push(level(
                halve(size, i),
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            ));
        }
        let tmp = level(halve(size, 1), wgpu::TextureUsages::RENDER_ATTACHMENT);
        self.chain = Some(Chain { size, levels, tmp });
    }
}

fn push(passes: &mut Vec<PassUniform>, p: PassUniform) -> u32 {
    passes.push(p);
    ((passes.len() - 1) as u64 * SLOT) as u32
}

/// One pass: `pipe` reading `bind` (at uniform offset `slot`) into `view`
/// within `area`.
fn run(
    enc: &mut wgpu::CommandEncoder,
    pipe: &wgpu::RenderPipeline,
    view: &wgpu::TextureView,
    bind: &wgpu::BindGroup,
    slot: u32,
    area: [i32; 4],
) {
    if area[0] >= area[2] || area[1] >= area[3] {
        return;
    }
    let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("blur"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    });
    pass.set_pipeline(pipe);
    pass.set_bind_group(0, bind, &[slot]);
    pass.set_scissor_rect(
        area[0] as u32,
        area[1] as u32,
        (area[2] - area[0]) as u32,
        (area[3] - area[1]) as u32,
    );
    pass.draw(0..3, 0..1);
}

/// The whole pixels `r` touches, within `frame`.
fn pixel_box(r: crate::scene::Rect, frame: [i32; 4]) -> [i32; 4] {
    intersect(
        [
            r.x.floor().max(-1.0e6) as i32,
            r.y.floor().max(-1.0e6) as i32,
            r.right().ceil().min(1.0e6) as i32,
            r.bottom().ceil().min(1.0e6) as i32,
        ],
        frame,
    )
}

fn intersect(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ]
}
