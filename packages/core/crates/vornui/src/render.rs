//! Which renderer draws a frame, and where the frame goes.
//!
//! A [`Ui`](crate::Ui) draws with the GPU renderer when it has a hardware
//! adapter and with the CPU renderer when the only adapter is a software
//! rasterizer or there is none. Either way the frame ends in a texture (an
//! [`Offscreen`] target or a window's surface texture) when there is a
//! device, so presenting is the same; the CPU renderer only uploads the rows
//! that changed into a target that keeps its contents.

use crate::atlas::Atlases;
use crate::cpu::CpuRaster;
use crate::gpu::{settle, Gpu, GpuRenderer};
use crate::scene::{Rgba, Scene};

/// How a [`Ui`](crate::Ui) chooses its renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderMode {
    /// The GPU unless its adapter is a software rasterizer.
    #[default]
    Auto,
    Gpu,
    Cpu,
}

impl RenderMode {
    /// `VORNUI_RENDERER=gpu|cpu|auto`, `Auto` when unset or unknown.
    pub fn from_env() -> RenderMode {
        match std::env::var("VORNUI_RENDERER").as_deref() {
            Ok("gpu") => RenderMode::Gpu,
            Ok("cpu") => RenderMode::Cpu,
            _ => RenderMode::Auto,
        }
    }

    /// Whether this mode draws on the CPU with `gpu` available (or not).
    pub fn uses_cpu(self, gpu: Option<&Gpu>) -> bool {
        match (self, gpu) {
            (_, None) | (RenderMode::Cpu, _) => true,
            (RenderMode::Gpu, Some(_)) => false,
            (RenderMode::Auto, Some(g)) => g.is_software(),
        }
    }
}

/// The format offscreen targets use, and screenshots are read in.
pub const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

pub(crate) enum Renderer {
    Gpu(GpuRenderer),
    Cpu {
        raster: CpuRaster,
        in_flight: Option<wgpu::SubmissionIndex>,
    },
}

impl Renderer {
    /// A renderer for targets of `format`, and the atlases it samples.
    /// Falls back to the CPU when the GPU pipelines cannot be built or the
    /// format is not one the CPU renderer writes.
    pub fn new(
        mode: RenderMode,
        gpu: Option<&Gpu>,
        format: wgpu::TextureFormat,
    ) -> (Renderer, Atlases) {
        let cpu_format = cpu_bgra(format);
        let want_cpu = mode.uses_cpu(gpu) && cpu_format.is_some();
        if let (false, Some(g)) = (want_cpu, gpu) {
            let atlases = Atlases::gpu(&g.device, &g.queue);
            if let Some(r) = GpuRenderer::new(g, format, &atlases) {
                return (Renderer::Gpu(r), atlases);
            }
        }
        let raster = CpuRaster::new(cpu_format.unwrap_or(false));
        (
            Renderer::Cpu {
                raster,
                in_flight: None,
            },
            Atlases::cpu(4096, 2048),
        )
    }

    pub fn is_cpu(&self) -> bool {
        matches!(self, Renderer::Cpu { .. })
    }

    /// Draws `scene` into `target` (`size` physical pixels). `keeps` says
    /// the target still holds the last frame, so the CPU renderer may
    /// upload only what changed.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        gpu: Option<&Gpu>,
        scene: &Scene,
        atlases: &Atlases,
        clear: Rgba,
        target: Option<(&wgpu::Texture, &wgpu::TextureView)>,
        size: (u32, u32),
        keeps: bool,
    ) {
        match self {
            Renderer::Gpu(r) => {
                if let (Some(g), Some((_, view))) = (gpu, target) {
                    r.render(g, scene, atlases, clear, view, size);
                }
            }
            Renderer::Cpu { raster, in_flight } => {
                raster.render(scene, atlases, clear, size);
                let (Some(g), Some((texture, _))) = (gpu, target) else {
                    return;
                };
                let whole = [(0, size.1)];
                let rows = if keeps { raster.damage() } else { &whole };
                for &(y0, y1) in rows {
                    upload_rows(g, texture, raster, y0, y1);
                }
                let submitted = g.queue.submit(None);
                settle(g, in_flight, submitted);
            }
        }
    }

    /// The CPU renderer's last frame, if this is the CPU renderer.
    pub fn cpu_frame(&self) -> Option<&CpuRaster> {
        match self {
            Renderer::Cpu { raster, .. } => Some(raster),
            Renderer::Gpu(_) => None,
        }
    }

    /// Makes the next CPU frame whole (after the target lost its contents).
    pub fn invalidate(&mut self) {
        if let Renderer::Cpu { raster, .. } = self {
            raster.invalidate();
        }
    }
}

/// Whether the CPU renderer can write `format`, and in which byte order.
fn cpu_bgra(format: wgpu::TextureFormat) -> Option<bool> {
    match format {
        wgpu::TextureFormat::Rgba8Unorm => Some(false),
        wgpu::TextureFormat::Bgra8Unorm => Some(true),
        _ => None,
    }
}

fn upload_rows(gpu: &Gpu, texture: &wgpu::Texture, raster: &CpuRaster, y0: u32, y1: u32) {
    let w = raster.size().0;
    let row = w as usize * 4;
    let bytes = &raster.bytes()[y0 as usize * row..y1 as usize * row];
    gpu.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d { x: 0, y: y0, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w * 4),
            rows_per_image: Some(y1 - y0),
        },
        wgpu::Extent3d {
            width: w,
            height: y1 - y0,
            depth_or_array_layers: 1,
        },
    );
}

/// A target that is not a window: what benches draw into and screenshots
/// are read from. Without a device it is only the CPU renderer's frame.
pub struct Offscreen {
    pub size: (u32, u32),
    pub(crate) texture: Option<(wgpu::Texture, wgpu::TextureView)>,
}

impl Offscreen {
    pub(crate) fn new(gpu: Option<&Gpu>, size: (u32, u32)) -> Offscreen {
        let texture = gpu.map(|g| {
            let t = g.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("offscreen"),
                size: wgpu::Extent3d {
                    width: size.0.max(1),
                    height: size.1.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: OFFSCREEN_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let v = t.create_view(&Default::default());
            (t, v)
        });
        Offscreen { size, texture }
    }

    pub(crate) fn target(&self) -> Option<(&wgpu::Texture, &wgpu::TextureView)> {
        self.texture.as_ref().map(|(t, v)| (t, v))
    }
}

/// Waits for the GPU and copies `texture` back as RGBA rows.
pub(crate) fn read_texture(
    gpu: &Gpu,
    texture: &wgpu::Texture,
    (w, h): (u32, u32),
) -> Result<Vec<u8>, String> {
    let row = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(row) * u64::from(h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        texture.as_image_copy(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_device_means_cpu() {
        assert!(RenderMode::Gpu.uses_cpu(None));
        assert!(RenderMode::Auto.uses_cpu(None));
        let (r, a) = Renderer::new(RenderMode::Gpu, None, OFFSCREEN_FORMAT);
        assert!(r.is_cpu());
        assert!(a.mask.pixels().is_some());
    }
}
