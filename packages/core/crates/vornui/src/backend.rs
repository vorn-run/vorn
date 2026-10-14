//! Where a laid-out, painted frame goes. One [`Ui`] paints one list; a
//! backend turns it into pixels ([`Pixels`], today's wgpu or CPU raster) or
//! terminal cells ([`Cells`]).

use crate::cells::Buffer;
use crate::render::Offscreen;
use crate::ui::Ui;

/// Presents the frame `ui` last painted.
pub trait Backend {
    /// Draws the frame; answers whether anything was drawn.
    fn present(&mut self, ui: &mut Ui) -> Result<bool, String>;
}

/// The pixel backend: the scene rasterized into an offscreen target, as the
/// window and the screenshot tools draw it.
pub struct Pixels {
    pub target: Offscreen,
}

impl Pixels {
    pub fn new(ui: &Ui, size: (u32, u32)) -> Pixels {
        Pixels {
            target: ui.offscreen(size),
        }
    }
}

impl Backend for Pixels {
    fn present(&mut self, ui: &mut Ui) -> Result<bool, String> {
        Ok(ui.render_offscreen(&self.target))
    }
}

/// The cells backend: the paint list rasterized into a cell grid and
/// written to the terminal as a diff.
pub struct Cells {
    pub buffer: Buffer,
    #[cfg(not(target_os = "wasi"))]
    pub terminal: Option<crate::cells::term::Terminal>,
}

impl Cells {
    /// A cells backend that only keeps the grid (snapshots and tests).
    pub fn offscreen(cols: u16, rows: u16) -> Cells {
        Cells {
            buffer: Buffer::new(cols, rows),
            #[cfg(not(target_os = "wasi"))]
            terminal: None,
        }
    }

    #[cfg(not(target_os = "wasi"))]
    /// A cells backend drawing to the controlling terminal, sized to it.
    pub fn terminal(terminal: crate::cells::term::Terminal) -> Cells {
        let (cols, rows) = crate::cells::term::Terminal::size().unwrap_or((80, 24));
        Cells {
            buffer: Buffer::new(cols, rows),
            terminal: Some(terminal),
        }
    }

    /// Resizes the grid; the next frame is written in full.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        if (cols, rows) != (self.buffer.cols, self.buffer.rows) {
            self.buffer = Buffer::new(cols, rows);
            #[cfg(not(target_os = "wasi"))]
            if let Some(t) = &mut self.terminal {
                t.invalidate();
            }
        }
    }
}

impl Backend for Cells {
    fn present(&mut self, ui: &mut Ui) -> Result<bool, String> {
        let Some(list) = ui.paint_list() else {
            return Err("a cells backend needs a cells Ui (Ui::cells)".into());
        };
        self.buffer.draw(list);
        #[cfg(not(target_os = "wasi"))]
        if let Some(t) = &mut self.terminal {
            t.draw(&self.buffer).map_err(|e| e.to_string())?;
        }
        Ok(true)
    }
}
