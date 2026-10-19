//! From a laid-out [`Ui`] to the bytes that show it as kitty images,
//! without a terminal: what the pixel terminal sends, and what tests and
//! benchmarks look at.

use super::kitty::Wrap;
use super::present::{Frame, Placing, Presenter, Rect, Sent, Transport};
use crate::cpu::{layers_bounds, CpuRaster};
use crate::ui::Ui;

/// Draws frames with the CPU renderer and turns each into the images
/// that show what changed since the one before.
pub struct PixelFrames {
    presenter: Presenter,
    /// What is under the overlays, and (while there are any) the whole frame.
    under: CpuRaster,
    all: CpuRaster,
    /// The last frame had overlays, drawn whole into `all`.
    split: bool,
}

impl PixelFrames {
    pub fn new(wrap: Wrap, placing: Placing, cell: (u32, u32), transport: Transport) -> Self {
        PixelFrames {
            presenter: Presenter::new(wrap, placing, cell, transport),
            under: CpuRaster::new(false),
            all: CpuRaster::new(false),
            split: false,
        }
    }

    /// Starts over with a new cell size: the next frame is sent whole.
    pub fn set_cell(&mut self, cell: (u32, u32)) {
        self.presenter.set_cell(cell);
    }

    /// Forgets what the terminal shows, so the next frame is sent whole.
    pub fn invalidate(&mut self) {
        self.presenter.invalidate();
        self.under.invalidate();
        self.all.invalidate();
    }

    /// Deletes every image sent.
    pub fn clear(&mut self, out: &mut Vec<u8>) {
        self.presenter.clear(out);
    }

    /// Draws the frame `ui` last laid out at `px` pixels and appends to
    /// `out` what shows it. Answers `None` when it was not drawn because
    /// the atlases filled up: lay the frame out again and call this again.
    pub fn draw(&mut self, ui: &mut Ui, px: (u32, u32), out: &mut Vec<u8>) -> Option<Sent> {
        let overlays = match self.presenter.placing() {
            Placing::Cursor => ui.overlay_layers(),
            // Placeholder cells cannot stack: one image for everything.
            Placing::Cells => 0..0,
        };
        let split = !overlays.is_empty();
        let drawn = if split {
            ui.render_rasters(
                &mut [(&mut self.under, overlays.clone()), (&mut self.all, 0..0)],
                px,
            )
        } else {
            ui.render_rasters(&mut [(&mut self.under, 0..0)], px)
        };
        if !drawn {
            return None;
        }
        self.split = split;
        let rect: Option<Rect> = split
            .then(|| layers_bounds(&ui.scene, overlays, px))
            .flatten()
            .map(|b| b.map(|v| v.max(0) as u32));
        let under = frame_of(&self.under);
        let overlay = rect.map(|r| (frame_of(&self.all), r));
        Some(self.presenter.present(under, overlay, out))
    }

    /// The last frame's pixels with its overlays (RGBA, row by row).
    pub fn pixels(&self) -> (&[u8], (u32, u32)) {
        let r = if self.split { &self.all } else { &self.under };
        (r.bytes(), r.size())
    }
}

fn frame_of(r: &CpuRaster) -> Frame<'_> {
    Frame {
        px: r.pixels(),
        size: r.size(),
        dirty: r.dirty_tiles(),
        tiles_x: r.tiles_x(),
        tile: CpuRaster::TILE_PX,
    }
}
