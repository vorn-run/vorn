//! A texture atlas: rectangles packed by etagere and uploaded once. Glyphs,
//! icons and images are rasterized on first use and then only referenced.

use etagere::{size2, BucketedAtlasAllocator};

pub struct Atlas {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub size: u32,
    bytes_per_px: u32,
    alloc: BucketedAtlasAllocator,
}

impl Atlas {
    pub fn new(
        device: &wgpu::Device,
        size: u32,
        format: wgpu::TextureFormat,
        label: &str,
    ) -> Atlas {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let bytes_per_px = format.block_copy_size(None).unwrap_or(4);
        Atlas {
            texture,
            view,
            size,
            bytes_per_px,
            alloc: BucketedAtlasAllocator::new(size2(size as i32, size as i32)),
        }
    }

    /// Packs and uploads a `w`×`h` image; returns its texel rect, or `None`
    /// when the atlas is full (the spike sizes atlases so that never happens
    /// in its scenes; a complete layer would add pages).
    pub fn insert(&mut self, queue: &wgpu::Queue, w: u32, h: u32, data: &[u8]) -> Option<[f32; 4]> {
        if w == 0 || h == 0 {
            return Some([0.0; 4]);
        }
        // One texel of padding keeps linear sampling from bleeding neighbours.
        let a = self.alloc.allocate(size2(w as i32 + 1, h as i32 + 1))?;
        let (x, y) = (a.rectangle.min.x as u32, a.rectangle.min.y as u32);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * self.bytes_per_px),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        Some([x as f32, y as f32, w as f32, h as f32])
    }
}
