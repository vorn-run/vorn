//! A terminal selection, held by absolute line so it stays on its text
//! while output scrolls; vornd turns it into text.

use vorn_term_proto::msg::GridPoint;

use crate::view::PaneView;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sel {
    /// The first cell, inclusive.
    pub from: GridPoint,
    /// The last cell, inclusive.
    pub to: GridPoint,
    /// Made on the primary screen; it vanishes on the other.
    pub primary: bool,
}

impl Sel {
    /// From `a` to `b`, in either order.
    pub fn new(a: GridPoint, b: GridPoint, primary: bool) -> Sel {
        let (from, to) = if (a.line, a.col) <= (b.line, b.col) {
            (a, b)
        } else {
            (b, a)
        };
        Sel { from, to, primary }
    }

    /// The screen and the history above it.
    pub fn all(v: &PaneView) -> Sel {
        let from = GridPoint {
            line: v.oldest,
            col: 0,
            sb_epoch: v.sb_epoch,
        };
        let to = v.point(v.cols.saturating_sub(1), v.rows.saturating_sub(1));
        Sel::new(from, to, v.primary)
    }

    /// Whether `v` still shows the lines this was made on.
    pub fn holds(&self, v: &PaneView) -> bool {
        self.primary == v.primary && self.from.sb_epoch == v.sb_epoch
    }

    /// What to highlight: `(row, from col, to col exclusive)` per visible row.
    pub fn rows(&self, v: &PaneView) -> Vec<(u16, u16, u16)> {
        if !self.holds(v) || v.rows == 0 {
            return Vec::new();
        }
        let first = self.from.line.max(v.line0);
        let last = self.to.line.min(v.line0 + u64::from(v.rows) - 1);
        (first..=last)
            .filter_map(|line| {
                let row = u16::try_from(line - v.line0).ok()?;
                let a = if line == self.from.line {
                    self.from.col
                } else {
                    0
                };
                let b = if line == self.to.line {
                    self.to.col.saturating_add(1)
                } else {
                    v.cols
                };
                let (a, b) = (a.min(v.cols), b.min(v.cols));
                (a < b).then_some((row, a, b))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(line0: u64) -> PaneView {
        PaneView {
            cols: 10,
            rows: 3,
            line0,
            oldest: 2,
            sb_epoch: 1,
            primary: true,
            ..PaneView::default()
        }
    }

    #[test]
    fn a_selection_highlights_whole_middle_rows_and_follows_scrolling() {
        let v = view(100);
        let s = Sel::new(v.point(4, 2), v.point(2, 0), true);
        assert_eq!(s.from, v.point(2, 0));
        assert_eq!(s.rows(&v), [(0, 2, 10), (1, 0, 10), (2, 0, 5)]);
        // Two lines of output later the same text sits two rows higher.
        assert_eq!(s.rows(&view(102)), [(0, 0, 5)]);
        assert!(s.rows(&view(103)).is_empty());
    }

    #[test]
    fn select_all_reaches_back_through_history_and_ends_with_the_screen() {
        let v = view(100);
        let s = Sel::all(&v);
        assert_eq!((s.from.line, s.from.col), (2, 0));
        assert_eq!((s.to.line, s.to.col), (102, 9));
        assert_eq!(s.rows(&v), [(0, 0, 10), (1, 0, 10), (2, 0, 10)]);
    }

    #[test]
    fn a_selection_vanishes_on_the_other_screen_or_a_new_epoch() {
        let v = view(100);
        let s = Sel::all(&v);
        let alt = PaneView {
            primary: false,
            line0: 0,
            ..view(0)
        };
        assert!(s.rows(&alt).is_empty());
        let cleared = PaneView {
            sb_epoch: 2,
            ..view(100)
        };
        assert!(s.rows(&cleared).is_empty());
    }
}
