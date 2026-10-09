//! Absolute line numbers for the primary screen (Terminal State Protocol
//! §9). libghostty-vt has no stable row identity, so they are counted: a
//! tracked grid reference sits on active row 0, and at the next count the
//! number of history rows above where that row now is says how many lines
//! scrolled into history since. Pruning at the top lowers both sides alike,
//! so the difference survives a full scrollback.
//!
//! When the count cannot be known the scrollback epoch (`sb_epoch`) is
//! raised and numbering starts again: the tracked row was pruned (more lines
//! than the scrollback holds in one frame), the history was cleared (ED 3,
//! RIS) or the screen was resized (the primary screen reflows). Clients drop
//! their cached history when it changes.
//!
//! Only the primary screen has line numbers. While the alternate screen is
//! active nothing is counted and the reference stays on the primary
//! screen's rows, so leaving vim needs no fetch.

use libghostty_vt::screen::Screen as Which;
use libghostty_vt::screen::TrackedGridRef;
use libghostty_vt::terminal::{Point, PointCoordinate, PointSpace, Terminal};

type Term = Terminal<'static, 'static>;

/// What a count found since the last one was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Moved {
    /// Lines that entered history.
    pub scrolled: u64,
    /// The count was lost and a new epoch began: no shift is known.
    pub new_epoch: bool,
}

/// The line count of one grid: where active row 0 is in absolute lines,
/// and the epoch those lines belong to.
#[derive(Debug, Default)]
pub struct Lines {
    /// Active row 0 at the last count, on the primary screen's page list.
    anchor: Option<TrackedGridRef>,
    /// History rows at the last count, once one was taken.
    h: Option<u64>,
    /// Absolute line of active row 0.
    top_line: u64,
    sb_epoch: u32,
    /// Counted and not yet reported by a frame.
    pending: Moved,
}

impl Lines {
    pub fn sb_epoch(&self) -> u32 {
        self.sb_epoch
    }

    pub fn top_line(&self) -> u64 {
        self.top_line
    }

    /// History rows at the last count.
    pub fn history_lines(&self) -> u64 {
        self.h.unwrap_or(0)
    }

    /// The absolute line of the oldest row history holds.
    pub fn oldest_line(&self) -> u64 {
        self.top_line.saturating_sub(self.history_lines())
    }

    /// Counts what scrolled since the last count and moves the reference
    /// back to active row 0. Costs one tracked reference; nothing on the
    /// alternate screen.
    pub fn settle(&mut self, t: &Term) {
        if !matches!(t.active_screen(), Ok(Which::Primary)) {
            return;
        }
        let h = t.scrollback_rows().map_or(0, |h| h as u64);
        match (self.h, &self.anchor) {
            // The first count: numbering starts with the oldest line at 0.
            (None, _) => self.top_line = h,
            (Some(before), Some(a)) => match a.point(PointSpace::Screen) {
                Ok(Some(p)) => {
                    let scrolled = h.saturating_sub(u64::from(p.y));
                    // History shrank to nothing without the anchor being
                    // pruned: ED 3 or RIS.
                    if h == 0 && before + scrolled > 0 {
                        self.lose(h);
                    } else {
                        self.top_line += scrolled;
                        self.pending.scrolled += scrolled;
                    }
                }
                _ => self.lose(h),
            },
            // No reference since a terminal swap or a screen switch: the
            // primary screen has been still unless its history moved.
            (Some(before), None) => {
                if before != h {
                    self.lose(h);
                }
            }
        }
        self.h = Some(h);
        self.anchor = t
            .track_grid_ref(Point::Active(PointCoordinate { x: 0, y: 0 }))
            .ok();
    }

    /// Starts a new epoch because line numbers stopped being valid for a
    /// reason the count does not see: a resize.
    pub fn new_epoch(&mut self, t: &Term) {
        let primary = matches!(t.active_screen(), Ok(Which::Primary));
        let h = if primary {
            t.scrollback_rows().map_or(0, |h| h as u64)
        } else {
            self.h.unwrap_or(0)
        };
        self.lose(h);
        if primary {
            self.h = Some(h);
            self.anchor = t
                .track_grid_ref(Point::Active(PointCoordinate { x: 0, y: 0 }))
                .ok();
        }
    }

    /// The terminal is about to be replaced by an exact rebuild (a
    /// checkpoint cut): count against the old one while its reference
    /// still points somewhere.
    pub fn before_swap(&mut self, t: &Term) {
        self.settle(t);
    }

    /// The rebuild is in place. Its history has the same rows, so numbering
    /// carries on unless the row count says otherwise.
    pub fn after_swap(&mut self, t: &Term) {
        self.anchor = None;
        if matches!(t.active_screen(), Ok(Which::Primary)) {
            self.settle(t);
        }
    }

    /// What was counted since the last frame, and the count reset.
    pub fn take(&mut self) -> Moved {
        std::mem::take(&mut self.pending)
    }

    fn lose(&mut self, h: u64) {
        self.sb_epoch = self.sb_epoch.wrapping_add(1);
        self.top_line = h;
        self.pending = Moved {
            scrolled: 0,
            new_epoch: true,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(scrollback: usize) -> Term {
        let mut t = Terminal::new(10, 4).unwrap();
        t.set_scrollback_max_bytes(Some(scrollback)).unwrap();
        t
    }

    fn lines(t: &mut Term, from: u32, n: u32) {
        for i in from..from + n {
            t.vt_write(format!("\r\n{i}").as_bytes());
        }
    }

    #[test]
    fn counts_lines_into_history_and_keeps_counting_once_it_is_full() {
        let mut t = term(4096);
        let mut l = Lines::default();
        l.settle(&t);
        assert_eq!((l.top_line(), l.take()), (0, Moved::default()));
        lines(&mut t, 1, 3);
        l.settle(&t);
        // Three line feeds on a four-row screen that started at row 0.
        assert_eq!(l.take().scrolled, 0);
        lines(&mut t, 4, 10);
        l.settle(&t);
        assert_eq!(l.take().scrolled, 10);
        assert_eq!(l.top_line(), 10);
        // Far past what the scrollback holds, a little at a time: pruning
        // never loses the count.
        let mut pruned_at = None;
        let mut i = 0;
        while pruned_at.is_none_or(|p| i < p + 200) {
            assert!(i < 100_000, "the scrollback was never pruned");
            lines(&mut t, 100 + i * 5, 5);
            l.settle(&t);
            let m = l.take();
            assert_eq!((m.scrolled, m.new_epoch), (5, false), "round {i}");
            if pruned_at.is_none() && l.oldest_line() > 0 {
                pruned_at = Some(i);
            }
            i += 1;
        }
        assert_eq!(l.top_line(), 10 + 5 * u64::from(i));
        assert_eq!(l.sb_epoch(), 0);
    }

    #[test]
    fn clearing_history_or_outrunning_it_starts_an_epoch() {
        let mut t = term(4096);
        let mut l = Lines::default();
        l.settle(&t);
        lines(&mut t, 1, 20);
        l.settle(&t);
        l.take();
        // ED 3.
        t.vt_write(b"\x1b[3J");
        l.settle(&t);
        assert!(l.take().new_epoch);
        assert_eq!((l.sb_epoch(), l.top_line()), (1, 0));
        // RIS.
        lines(&mut t, 1, 20);
        l.settle(&t);
        l.take();
        t.vt_write(b"\x1bc");
        l.settle(&t);
        assert!(l.take().new_epoch);
        assert_eq!(l.sb_epoch(), 2);
        // More lines in one go than the scrollback holds.
        l.settle(&t);
        lines(&mut t, 1, 100_000);
        l.settle(&t);
        assert!(l.take().new_epoch);
        assert_eq!(l.sb_epoch(), 3);
    }

    #[test]
    fn the_alternate_screen_leaves_primary_numbering_alone() {
        let mut t = term(4096);
        let mut l = Lines::default();
        l.settle(&t);
        lines(&mut t, 1, 10);
        l.settle(&t);
        l.take();
        let top = l.top_line();
        t.vt_write(b"\x1b[?1049h");
        lines(&mut t, 1, 50);
        l.settle(&t);
        assert_eq!((l.take(), l.top_line()), (Moved::default(), top));
        t.vt_write(b"\x1b[?1049l");
        l.settle(&t);
        assert_eq!((l.take(), l.top_line()), (Moved::default(), top));
        lines(&mut t, 1, 2);
        l.settle(&t);
        assert_eq!(l.take().scrolled, 2);
    }
}
