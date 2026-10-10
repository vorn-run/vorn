//! Texture atlases: rectangles packed by etagere. Glyphs, icons and images
//! are rasterized on first use and then only referenced by their texel rect.
//!
//! An atlas keeps its texels where its renderer reads them: in a wgpu
//! texture for the GPU renderer, in memory for the CPU one. When an atlas
//! fills up it is not paged: the [`crate::Ui`] empties both atlases and every
//! cache that points into them, and draws the frame again. Screens that cycle
//! through more glyphs than fit are rare, and a reset costs one slow frame.

use etagere::{size2, BucketedAtlasAllocator};

/// Where an atlas's texels live.
enum Store {
    Cpu(Vec<u8>),
    Gpu {
        texture: wgpu::Texture,
        view: wgpu::TextureView,
        queue: wgpu::Queue,
    },
}

/// One atlas: a square of `size` texels, `bytes_per_px` bytes each.
pub struct Atlas {
    pub size: u32,
    bytes_per_px: u32,
    alloc: BucketedAtlasAllocator,
    store: Store,
}

impl Atlas {
    /// An atlas kept in memory, for the CPU renderer.
    pub fn cpu(size: u32, bytes_per_px: u32) -> Atlas {
        Atlas {
            size,
            bytes_per_px,
            alloc: BucketedAtlasAllocator::new(size2(size as i32, size as i32)),
            store: Store::Cpu(vec![0; (size * size * bytes_per_px) as usize]),
        }
    }

    /// An atlas in a texture of `format`, for the GPU renderer.
    pub fn gpu(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
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
        Atlas {
            size,
            bytes_per_px: format.block_copy_size(None).unwrap_or(4),
            alloc: BucketedAtlasAllocator::new(size2(size as i32, size as i32)),
            store: Store::Gpu {
                texture,
                view,
                queue: queue.clone(),
            },
        }
    }

    /// The texture the GPU renderer samples, if this atlas has one.
    pub fn view(&self) -> Option<&wgpu::TextureView> {
        match &self.store {
            Store::Gpu { view, .. } => Some(view),
            Store::Cpu(_) => None,
        }
    }

    /// The texels, row-major, if this atlas is kept in memory.
    pub fn pixels(&self) -> Option<&[u8]> {
        match &self.store {
            Store::Cpu(p) => Some(p),
            Store::Gpu { .. } => None,
        }
    }

    /// Packs and stores a `w`×`h` image (`data` is tightly packed rows);
    /// answers its texel rect, or `None` when the atlas is full. Images too
    /// short for their size get an empty rect: they draw nothing.
    pub fn insert(&mut self, w: u32, h: u32, data: &[u8]) -> Option<[f32; 4]> {
        let row = (w * self.bytes_per_px) as usize;
        if w == 0 || h == 0 || data.len() < row * h as usize {
            return Some([0.0; 4]);
        }
        // A texel of padding keeps linear sampling from bleeding neighbours.
        let a = self.alloc.allocate(size2(w as i32 + 1, h as i32 + 1))?;
        let (x, y) = (a.rectangle.min.x as u32, a.rectangle.min.y as u32);
        self.write(x, y, w, h, data);
        Some([x as f32, y as f32, w as f32, h as f32])
    }

    fn write(&mut self, x: u32, y: u32, w: u32, h: u32, data: &[u8]) {
        let bpp = self.bytes_per_px as usize;
        let row = w as usize * bpp;
        match &mut self.store {
            Store::Cpu(p) => {
                let stride = self.size as usize * bpp;
                for (r, src) in data.chunks_exact(row).take(h as usize).enumerate() {
                    let at = (y as usize + r) * stride + x as usize * bpp;
                    p[at..at + row].copy_from_slice(src);
                }
            }
            Store::Gpu { texture, queue, .. } => queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &data[..row * h as usize],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row as u32),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            ),
        }
    }

    /// Forgets every rectangle and zeroes the texels, so padding stays
    /// transparent for whatever is packed next.
    pub fn reset(&mut self) {
        self.alloc.clear();
        match &mut self.store {
            Store::Cpu(p) => p.fill(0),
            Store::Gpu { .. } => {
                let zeros = vec![0u8; (self.size * self.size * self.bytes_per_px) as usize];
                self.write(0, 0, self.size, self.size, &zeros);
            }
        }
    }
}

/// The two atlases every renderer samples: coverage masks (glyphs, icons)
/// and RGBA images (color emoji, pictures).
pub struct Atlases {
    pub mask: Atlas,
    pub color: Atlas,
    /// Bumped on every reset, so anything that cached texel rects (or
    /// rasterized pixels that came from them) can tell they are stale.
    pub epoch: u64,
    /// An insert failed since the last reset.
    pub full: bool,
}

impl Atlases {
    pub fn cpu(mask_size: u32, color_size: u32) -> Atlases {
        Atlases {
            mask: Atlas::cpu(mask_size, 1),
            color: Atlas::cpu(color_size, 4),
            epoch: 0,
            full: false,
        }
    }

    pub fn gpu(device: &wgpu::Device, queue: &wgpu::Queue) -> Atlases {
        let max = device.limits().max_texture_dimension_2d.min(4096);
        Atlases {
            mask: Atlas::gpu(
                device,
                queue,
                max,
                wgpu::TextureFormat::R8Unorm,
                "mask atlas",
            ),
            color: Atlas::gpu(
                device,
                queue,
                2048.min(max),
                wgpu::TextureFormat::Rgba8Unorm,
                "color atlas",
            ),
            epoch: 0,
            full: false,
        }
    }

    /// Stores a coverage mask; `None` (and [`Atlases::full`]) when full.
    pub fn insert_mask(&mut self, w: u32, h: u32, data: &[u8]) -> Option<[f32; 4]> {
        let r = self.mask.insert(w, h, data);
        self.full |= r.is_none();
        r
    }

    /// Stores a straight-alpha RGBA image; `None` (and [`Atlases::full`]) when full.
    pub fn insert_color(&mut self, w: u32, h: u32, data: &[u8]) -> Option<[f32; 4]> {
        let r = self.color.insert(w, h, data);
        self.full |= r.is_none();
        r
    }

    pub fn reset(&mut self) {
        self.mask.reset();
        self.color.reset();
        self.epoch += 1;
        self.full = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_atlas_stores_rows_and_pads() {
        let mut a = Atlas::cpu(8, 1);
        let r = a.insert(2, 2, &[1, 2, 3, 4]).unwrap();
        assert_eq!(r, [0.0, 0.0, 2.0, 2.0]);
        let p = a.pixels().unwrap();
        assert_eq!(&p[0..3], &[1, 2, 0]);
        assert_eq!(&p[8..11], &[3, 4, 0]);
        let r2 = a.insert(2, 2, &[5; 4]).unwrap();
        assert!(r2[0] >= 3.0 || r2[1] >= 3.0, "a texel of padding between");
    }

    #[test]
    fn fills_then_resets() {
        let mut a = Atlases::cpu(8, 8);
        let mut n = 0;
        while a.insert_mask(3, 3, &[9; 9]).is_some() {
            n += 1;
        }
        assert!(n >= 1);
        assert!(a.full);
        a.reset();
        assert!(!a.full);
        assert_eq!(a.epoch, 1);
        assert!(a.mask.pixels().unwrap().iter().all(|&p| p == 0));
        assert!(a.insert_mask(3, 3, &[9; 9]).is_some());
    }

    #[test]
    fn short_data_draws_nothing() {
        let mut a = Atlas::cpu(8, 4);
        assert_eq!(a.insert(2, 2, &[0; 15]), Some([0.0; 4]));
    }
}
