//! Kitty graphics: the switches that let a terminal decode PNG and load images from files, and
//! what a renderer reads of the images a program stored and where it placed them.
//!
//! The terminal library parses the protocol and keeps the images, their placements and its
//! storage limit; nothing here stores pixels. A renderer reads [`Emulator::placements`] after an
//! update and fetches an image's pixels once per generation with [`Emulator::picture`].

use std::cell::Cell;

use libghostty_vt::alloc::Allocator;
use libghostty_vt::ffi;
use libghostty_vt::kitty::graphics::{
    self, DecodePng, DecodedImage, ImageFormat, PlacementIterator, RustPngDecoder,
};

use crate::{Emulator, Result};

/// One image's pixels, as straight-alpha RGBA rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub id: u32,
    /// Changes whenever the pixels do (a retransmit, an animation frame).
    pub gen: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// One placement on the grid, in the viewport's cells and the terminal's cell pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    pub image: u32,
    /// The generation of the image's pixels it shows.
    pub gen: u64,
    /// The viewport row of its top left cell: negative once it scrolled into history.
    pub row: i32,
    pub col: i32,
    /// Pixels from the top left of that cell.
    pub x_off: u32,
    pub y_off: u32,
    /// The size it is drawn at, in pixels.
    pub width: u32,
    pub height: u32,
    /// The part of the image drawn: x, y, width, height in image pixels.
    pub src: [u32; 4],
    /// Below text when negative; below cell backgrounds under `i32::MIN / 2`.
    pub z: i32,
}

/// A virtual placement, which unicode placeholder cells draw, with its grid size resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Virtual {
    pub image: u32,
    pub gen: u64,
    pub placement: u32,
    pub cols: u32,
    pub rows: u32,
}

thread_local! {
    /// Whether the terminal being fed on this thread may decode PNG.
    static PNG_ON: Cell<bool> = const { Cell::new(false) };
    /// Whether this thread's decoder is installed: the terminal library's hook is global, the decoder per thread.
    static PNG_SET: Cell<bool> = const { Cell::new(false) };
}

/// PNG decoding gated by the switch, so a terminal with images off refuses PNG as it did before.
struct Gated;

impl DecodePng for Gated {
    fn decode_png<'a>(
        &mut self,
        alloc: &'a Allocator<'_>,
        data: &[u8],
    ) -> Option<DecodedImage<'a>> {
        if !PNG_ON.get() {
            return None;
        }
        RustPngDecoder::new().decode_png(alloc, data)
    }
}

/// Lets the next feed on this thread decode PNG, or not.
pub(crate) fn allow_png(on: bool) {
    PNG_ON.set(on);
    if on && !PNG_SET.get() && graphics::set_png_decoder(Some(Box::new(Gated))).is_ok() {
        PNG_SET.set(true);
    }
}

impl Emulator {
    /// Turns images on or off: with them on, PNG decodes and images sent in the stream itself
    /// load, and a checkpoint keeps the live terminal, since it cannot carry them. Off is how a
    /// terminal always behaved. Loading from files is its own switch, [`Emulator::set_image_files`].
    pub fn set_images(&mut self, on: bool) {
        self.images = on;
    }

    /// Lets images load from files, temp files and shared memory, which only a program on this
    /// machine can name: on for a local terminal, off for one whose program runs elsewhere. The
    /// terminal library's own checks on paths apply, temp files only under the OS temp dir with
    /// the protocol's name, and every medium refused gets the same error reply.
    pub fn set_image_files(&mut self, on: bool) {
        if self.image_files == on {
            return;
        }
        self.image_files = on;
        let tmp = std::env::temp_dir();
        let _ = self.term.set_kitty_image_from_file_allowed(on);
        let _ = self.term.set_kitty_image_from_shared_mem_allowed(on);
        let _ = self
            .term
            .set_kitty_image_temp_file_dir(on.then_some(tmp.as_path()));
    }

    /// Whether images may load from files (see [`Emulator::set_image_files`]).
    pub fn image_files(&self) -> bool {
        self.image_files
    }

    /// Whether images are on (see [`Emulator::set_images`]).
    pub fn images(&self) -> bool {
        self.images
    }

    /// Sets the bytes of images each screen may hold, past which the oldest are evicted; the
    /// terminal library starts at 10 MB.
    pub fn set_image_limit(&mut self, bytes: u64) -> Result<()> {
        self.term.set_kitty_image_storage_limit(bytes)?;
        Ok(())
    }

    /// Bytes of image pixels either screen holds.
    pub fn image_bytes(&self) -> u64 {
        let mut usage = ffi::TerminalMemoryUsage {
            size: std::mem::size_of::<ffi::TerminalMemoryUsage>(),
            ..Default::default()
        };
        // SAFETY: `raw` is this emulator's live terminal, and `usage` is the sized struct the
        // option writes.
        let r = unsafe {
            ffi::ghostty_terminal_get(
                self.raw,
                ffi::TerminalData::MEMORY_USAGE,
                (&raw mut usage).cast(),
            )
        };
        if r != ffi::Result::SUCCESS {
            return 0;
        }
        usage.primary_image_bytes + usage.alternate_image_bytes
    }

    /// Images are on and the terminal holds some: it must stay live to keep them.
    pub fn keeps_images(&self) -> bool {
        self.images && self.image_bytes() > 0
    }

    /// The active screen's storage generation: unchanged means the same images and placements,
    /// though a scroll or resize may still have moved them. Zero for a storage never written.
    pub fn images_gen(&self) -> u64 {
        self.term
            .kitty_graphics()
            .and_then(|g| g.generation())
            .unwrap_or(0)
    }

    /// The active screen's placements into `placed`, and its virtual ones into `virtuals`, both
    /// cleared first. Placements whose spot was pruned from history are left out.
    pub fn placements(&self, placed: &mut Vec<Placed>, virtuals: &mut Vec<Virtual>) {
        placed.clear();
        virtuals.clear();
        let Ok(g) = self.term.kitty_graphics() else {
            return;
        };
        let Ok(mut it) = PlacementIterator::new() else {
            return;
        };
        let Ok(mut each) = it.update(&g) else {
            return;
        };
        let (cw, ch) = self.cell_px();
        while let Some(p) = each.next() {
            let Some(img) = p.image_id().ok().and_then(|id| g.image(id)) else {
                continue;
            };
            let (Ok(id), Ok(gen), Ok(w), Ok(h)) =
                (img.id(), img.generation(), img.width(), img.height())
            else {
                continue;
            };
            if p.is_virtual().unwrap_or(false) {
                let (cols, rows) = (p.columns().unwrap_or(0), p.rows().unwrap_or(0));
                virtuals.push(Virtual {
                    image: id,
                    gen,
                    placement: p.placement_id().unwrap_or(0),
                    cols: if cols == 0 { w.div_ceil(cw) } else { cols },
                    rows: if rows == 0 { h.div_ceil(ch) } else { rows },
                });
                continue;
            }
            let Ok(info) = p.placement_render_info(&img, &self.term) else {
                continue;
            };
            // A pin pruned from history reads as the top left corner, hidden.
            if !info.viewport_visible && info.viewport_col == 0 && info.viewport_row == 0 {
                continue;
            }
            if info.pixel_width == 0 || info.pixel_height == 0 {
                continue;
            }
            placed.push(Placed {
                image: id,
                gen,
                row: info.viewport_row,
                col: info.viewport_col,
                x_off: p.x_offset().unwrap_or(0),
                y_off: p.y_offset().unwrap_or(0),
                width: info.pixel_width,
                height: info.pixel_height,
                src: [
                    info.source_x,
                    info.source_y,
                    info.source_width,
                    info.source_height,
                ],
                z: p.z().unwrap_or(0),
            });
        }
    }

    /// Image `id`'s pixels on the active screen, as RGBA, if it holds them at generation `gen`.
    pub fn picture(&self, id: u32, gen: u64) -> Option<Picture> {
        let g = self.term.kitty_graphics().ok()?;
        let img = g.image(id)?;
        if img.generation().ok()? != gen {
            return None;
        }
        let (width, height) = (img.width().ok()?, img.height().ok()?);
        let data = img.data().ok()??;
        let px = (width as usize).checked_mul(height as usize)?;
        let rgba = to_rgba(img.format().ok()?, data, px)?;
        Some(Picture {
            id,
            gen,
            width,
            height,
            rgba,
        })
    }
}

/// `px` pixels of `data` in `format` as RGBA; `None` when `data` is short.
fn to_rgba(format: ImageFormat, data: &[u8], px: usize) -> Option<Vec<u8>> {
    let channels = match format {
        ImageFormat::Rgba | ImageFormat::Png => 4,
        ImageFormat::Rgb => 3,
        ImageFormat::GrayAlpha => 2,
        ImageFormat::Gray => 1,
        _ => return None,
    };
    let data = data.get(..px.checked_mul(channels)?)?;
    if channels == 4 {
        return Some(data.to_vec());
    }
    let mut out = Vec::with_capacity(px * 4);
    for p in data.chunks_exact(channels) {
        out.extend_from_slice(&match *p {
            [r, g, b] => [r, g, b, 255],
            [l, a] => [l, l, l, a],
            [l] => [l, l, l, 255],
            _ => return None,
        });
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_format_widens_to_rgba() {
        assert_eq!(
            to_rgba(ImageFormat::Rgb, &[1, 2, 3, 4, 5, 6], 2),
            Some(vec![1, 2, 3, 255, 4, 5, 6, 255])
        );
        assert_eq!(
            to_rgba(ImageFormat::GrayAlpha, &[7, 8], 1),
            Some(vec![7, 7, 7, 8])
        );
        assert_eq!(
            to_rgba(ImageFormat::Gray, &[9], 1),
            Some(vec![9, 9, 9, 255])
        );
        assert_eq!(
            to_rgba(ImageFormat::Rgba, &[1, 2, 3, 4], 1),
            Some(vec![1, 2, 3, 4])
        );
        assert_eq!(to_rgba(ImageFormat::Rgb, &[1, 2], 1), None);
    }
}
