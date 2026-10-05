//! Viewers fit, they never clip (TP §10).
//!
//! Every client draws the session's whole grid, whatever size it is. One
//! whose box is smaller scales its font down, to a floor, and pans what
//! still does not fit; one whose box is larger draws at its own font with
//! room to spare. The renderer and the web client follow the same arithmetic
//! (`src/renderer/lib/grid-fit.ts`).

use crate::Size;

/// How a client draws a grid that is not its own size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    /// The font size to draw at, in the unit `font` was given in.
    pub font: f32,
    /// Columns and rows on screen at that font, at most the grid's.
    pub shown: Size,
    /// Columns and rows that are panned to rather than shown at once:
    /// `shown + pan` is always the whole grid.
    pub pan: Size,
}

/// How to draw `grid` in a box that holds `room` cells at font size `font`,
/// scaling the font no lower than `min_font`.
///
/// The cell is taken to scale with the font, which is how a monospace font
/// behaves to within a pixel; the client measures again after it applies the
/// font, so the rounding is never the difference between fitting and not.
pub fn fit(grid: Size, room: Size, font: f32, min_font: f32) -> Fit {
    if grid.cols == 0 || grid.rows == 0 || room.cols == 0 || room.rows == 0 || font <= 0.0 {
        return Fit {
            font,
            shown: grid,
            pan: Size::default(),
        };
    }
    let scale = (f32::from(room.cols) / f32::from(grid.cols))
        .min(f32::from(room.rows) / f32::from(grid.rows))
        .min(1.0);
    let scaled = (font * scale).max(min_font.min(font));
    // The box holds `room` cells of the original font, so this many of the
    // scaled one. The epsilon keeps an exact fit from rounding down.
    let holds = |cells: u16, of: u16| -> u16 {
        let n = (f32::from(cells) * font / scaled + 1e-3).floor();
        if n >= f32::from(of) {
            of
        } else {
            // Below `of`, which is a u16, so the cast cannot truncate.
            n.max(0.0) as u16
        }
    };
    let shown = Size::new(holds(room.cols, grid.cols), holds(room.rows, grid.rows));
    Fit {
        font: scaled,
        shown,
        pan: Size::new(grid.cols - shown.cols, grid.rows - shown.rows),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_larger_box_draws_at_its_own_font() {
        let f = fit(Size::new(80, 24), Size::new(120, 40), 13.0, 9.0);
        assert_eq!(f.font, 13.0);
        assert_eq!((f.shown, f.pan), (Size::new(80, 24), Size::new(0, 0)));
    }

    #[test]
    fn a_smaller_box_scales_then_pans() {
        // 120 columns on a phone that holds 60 at 13 px: 6.5 px is below the
        // floor, so 9 px, which shows 86 columns and pans the other 34.
        let f = fit(Size::new(120, 30), Size::new(60, 40), 13.0, 9.0);
        assert_eq!(f.font, 9.0);
        assert_eq!(f.shown, Size::new(86, 30));
        assert_eq!(f.pan, Size::new(34, 0));
        // 100 columns where 90 fit: scaled, nothing panned.
        let f = fit(Size::new(100, 24), Size::new(90, 30), 13.0, 9.0);
        assert!((f.font - 11.7).abs() < 1e-3, "{f:?}");
        assert_eq!((f.shown, f.pan), (Size::new(100, 24), Size::new(0, 0)));
    }

    #[test]
    fn a_preferred_font_below_the_floor_is_kept() {
        let f = fit(Size::new(80, 24), Size::new(40, 24), 8.0, 9.0);
        assert_eq!(f.font, 8.0);
        assert_eq!(f.shown.cols + f.pan.cols, 80);
    }
}
