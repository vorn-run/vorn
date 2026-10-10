//! The terminal grid's automatic layout: `pickAutoLayout` from
//! `src/renderer/lib/auto-grid-layout.ts`, case for case, so the same
//! number of cards in the same window lands in the same rows and columns.

/// Narrowest a card may be before a layout is rejected.
pub const MIN_CARD_W: f32 = 320.0;
/// Rows are only added while each keeps at least this much height.
pub const ROW_FIT_MIN_H: f32 = 280.0;
const HARD_MAX_COLS: usize = 4;
const HARD_MAX_ROWS: usize = 4;

/// Whether the cards fit the window or run past it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Fit,
    /// More cards than fit; rows keep [`fit_max_rows`]'s height.
    Scroll,
}

/// Columns and rows for the cards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub cols: usize,
    pub rows: usize,
    pub mode: Mode,
}

const fn fit(cols: usize, rows: usize) -> Layout {
    Layout {
        cols,
        rows,
        mode: Mode::Fit,
    }
}

fn floor_clamped(v: f32, max: usize) -> usize {
    // f32 to usize saturates, so a huge or negative size cannot wrap.
    (v.floor() as usize).clamp(1, max)
}

/// How many rows fit comfortably in `h`, which sizes rows when scrolling.
pub fn fit_max_rows(h: f32) -> usize {
    floor_clamped(h / ROW_FIT_MIN_H, HARD_MAX_ROWS)
}

/// The layout for `n` cards in a `w` by `h` grid.
pub fn pick(n: usize, w: f32, h: f32) -> Layout {
    match n {
        0 | 1 => return fit(1, 1),
        2 if w / 2.0 >= MIN_CARD_W => return fit(2, 1),
        3 if w / 3.0 >= MIN_CARD_W => return fit(3, 1),
        _ => {}
    }
    let max_cols = floor_clamped(w / MIN_CARD_W, HARD_MAX_COLS);
    let max_rows = fit_max_rows(h);
    if n > max_cols * max_rows {
        return Layout {
            cols: max_cols,
            rows: n.div_ceil(max_cols),
            mode: Mode::Scroll,
        };
    }
    let mut best = (fit(1, n), f32::NEG_INFINITY);
    for cols in 1..=n.min(max_cols) {
        let rows = n.div_ceil(cols);
        if rows > max_rows {
            continue;
        }
        let aspect = ((w / cols as f32) / (h / rows as f32)).ln().abs();
        let score = -aspect - (cols * rows - n) as f32 * 0.25;
        if score > best.1 || (score == best.1 && cols > best.0.cols) {
            best = (fit(cols, rows), score);
        }
    }
    best.0
}

#[cfg(test)]
mod tests {
    use super::*;

    // The cases of `tests/grid-view.test.tsx`'s `pickAutoLayout` block.
    #[test]
    fn matches_the_renderer() {
        let (w, h) = (1920.0, 1080.0);
        assert_eq!(pick(0, w, h), fit(1, 1));
        assert_eq!(pick(1, w, h), fit(1, 1));
        assert_eq!(pick(1, 800.0, 600.0), fit(1, 1));
        assert_eq!(pick(2, w, h), fit(2, 1));
        assert_eq!(pick(3, w, h), fit(3, 1));
        assert_eq!(pick(4, w, h), fit(2, 2));
        assert_eq!(pick(6, w, h), fit(3, 2));
        assert_eq!(pick(6, 2200.0, 1400.0), fit(3, 2));
        assert_eq!(pick(9, w, h), fit(3, 3));
        assert_eq!(pick(10, w, h), fit(4, 3));
        assert_eq!(pick(6, 1230.0, 860.0), fit(3, 2));
        assert_eq!(pick(10, 1230.0, 860.0).mode, Mode::Scroll);
        assert_eq!(pick(10, 1000.0, 700.0).mode, Mode::Scroll);
        assert_eq!(pick(16, 2560.0, 1440.0), fit(4, 4));
        assert_eq!(pick(12, 2560.0, 1440.0), fit(4, 3));
        assert_eq!(pick(17, 2560.0, 1440.0).mode, Mode::Scroll);
    }

    #[test]
    fn narrow_windows_stack_instead_of_squeezing() {
        assert_eq!(pick(2, 500.0, 900.0), fit(1, 2));
        let tiny = pick(5, 0.0, 0.0);
        assert_eq!((tiny.cols, tiny.mode), (1, Mode::Scroll));
    }
}
