//! Windowed lists: a scroll view that builds only the items in or near
//! view, the rest standing in as spacers of the height they last measured.

use std::collections::HashMap;

use crate::cells::units;
use crate::element::{div, El, Id};
use crate::ui::Ui;

/// What a windowed scroll view learnt from its last layout, in layout
/// units (logical pixels, or cells).
#[derive(Debug, Default)]
pub(crate) struct Virt {
    /// Each item's height when it was last built.
    heights: HashMap<Id, f32>,
    /// The sum of `heights`, for what an item never built is guessed at.
    total: f64,
    pub gap: f32,
    /// Where the first item starts: the view's top padding.
    pub top: f32,
    pub view: f32,
    /// The first item in view and where it started in the content, which
    /// the next layout keeps in place.
    pub anchor: Option<(Id, f32)>,
}

impl Virt {
    pub fn measured(&mut self, id: Id, h: f32) {
        if let Some(old) = self.heights.insert(id, h) {
            self.total -= f64::from(old);
        }
        self.total += f64::from(h);
    }

    fn estimate(&self) -> Option<f32> {
        (!self.heights.is_empty()).then(|| (self.total / self.heights.len() as f64) as f32)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Row {
    Item(usize),
    Space(f32),
}

/// Which items of a list [`Ui::windowed`] builds, in order, and the
/// spacers that hold the place of the rest.
#[derive(Debug, Clone, PartialEq)]
pub struct Windowed {
    rows: Vec<Row>,
    cells: bool,
}

impl Windowed {
    /// The indices of the items built, in order.
    pub fn items(&self) -> impl Iterator<Item = usize> + '_ {
        self.rows.iter().filter_map(|r| match r {
            Row::Item(i) => Some(*i),
            Row::Space(_) => None,
        })
    }

    /// `list` (the scroll view [`Ui::windowed`] was asked about) with the
    /// built items, each made by `item`, and the spacers between them.
    pub fn fill(self, list: El, mut item: impl FnMut(usize) -> El) -> El {
        let mut list = list;
        list.windowed = true;
        list.children.reserve(self.rows.len());
        for row in self.rows {
            list.children.push(match row {
                Row::Item(i) => item(i),
                Row::Space(h) => spacer(h, self.cells),
            });
        }
        list
    }
}

/// An empty box `h` layout units tall that does not shrink.
fn spacer(h: f32, cells: bool) -> El {
    // On cells an empty box is spacing, which counts a row per ROW_GAP pixels.
    let px = if cells {
        h.round() * units::ROW_GAP
    } else {
        h
    };
    div().h(px.max(0.0)).shrink0()
}

impl Ui {
    /// Plans a long list in the scroll view `scroll`, whose items are the
    /// direct children the ids `keys` name, in order. Built are the items
    /// in view and one either side, the focused item and its neighbours (so
    /// Tab walks on), and the items `keep` names (one being dragged, one
    /// whose menu is open). Each run of the others becomes one spacer of
    /// their height when last built, or of the measured items' mean (`est`
    /// logical pixels before any was measured). Scrolling keeps the first
    /// item in view where it was as the heights above it settle.
    pub fn windowed(&self, scroll: Id, keys: &[Id], est: f32, keep: &[Id]) -> Windowed {
        let cells = self.is_cells();
        let n = keys.len();
        if n == 0 {
            return Windowed {
                rows: Vec::new(),
                cells,
            };
        }
        let v = self.st.virt.get(&scroll);
        let guess = v.and_then(Virt::estimate).unwrap_or(if cells {
            units::height(est)
        } else {
            est
        });
        let (gap, top) = v.map_or((0.0, 0.0), |v| (v.gap, v.top));
        // Before the first layout, the window's height bounds the view's.
        let view = v.map_or(
            if cells {
                self.viewport().1 / units::ROW
            } else {
                self.viewport().1
            },
            |v| v.view,
        );
        let hs: Vec<f32> = keys
            .iter()
            .map(|k| v.and_then(|v| v.heights.get(k)).copied().unwrap_or(guess))
            .collect();
        let total = top + hs.iter().sum::<f32>() + gap * (n - 1) as f32;
        let offset = self.scroll_offset(scroll).min((total - view).max(0.0));
        let (mut first, mut last) = (None, 0);
        let mut y = top;
        for (i, h) in hs.iter().enumerate() {
            if y >= offset + view {
                break;
            }
            if y + h > offset {
                first.get_or_insert(i);
                last = i;
            }
            y += h + gap;
        }
        let (lo, hi) = match first {
            Some(f) => (f.saturating_sub(1), (last + 1).min(n - 1)),
            None => (0, 0),
        };
        let focus = self.st.focus;
        let mut pins: Vec<usize> = Vec::new();
        if focus.is_some() || !keep.is_empty() {
            for (i, k) in keys.iter().enumerate() {
                if Some(*k) == focus {
                    pins.extend([i.saturating_sub(1), i, (i + 1).min(n - 1)]);
                } else if keep.contains(k) {
                    pins.push(i);
                }
            }
            pins.sort_unstable();
            pins.dedup();
        }
        let mut rows = Vec::with_capacity(hi + 3 - lo + 2 * pins.len());
        let mut space: Option<f32> = None;
        let mut pin = pins.iter().peekable();
        for (i, h) in hs.iter().enumerate() {
            while pin.next_if(|p| **p < i).is_some() {}
            let built = (lo..=hi).contains(&i) || pin.peek() == Some(&&i);
            if built {
                if let Some(s) = space.take() {
                    rows.push(Row::Space(s - gap));
                }
                rows.push(Row::Item(i));
            } else {
                *space.get_or_insert(0.0) += h + gap;
            }
        }
        if let Some(s) = space {
            rows.push(Row::Space(s - gap));
        }
        Windowed { rows, cells }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{text, Rgba, UiConfig};

    fn keys(n: usize) -> Vec<Id> {
        (0..n).map(|i| Id::new("item").child(i)).collect()
    }

    fn items(w: &Windowed) -> Vec<usize> {
        w.items().collect()
    }

    /// A list of `n` items 50 px tall with an 8 px gap in a 200 px view.
    fn frame(ui: &mut Ui, n: usize, keep: &[Id]) -> Windowed {
        let scroll = Id::new("list");
        let keys = keys(n);
        let w = ui.windowed(scroll, &keys, 40.0, keep);
        let list = div().id(scroll).col().gap(8.0).h(200.0).scroll_y();
        let el = w.clone().fill(list, |i| {
            div()
                .id(keys[i])
                .h(50.0)
                .shrink0()
                .bg(Rgba::white(1.0))
                .child(text(format!("{i}"), 12.0, Rgba::white(1.0)))
        });
        ui.layout(div().w(300.0).h(400.0).child(el), (300.0, 400.0));
        w
    }

    fn ui() -> Ui {
        Ui::headless(crate::RenderMode::Cpu, &UiConfig::system(1.0, 13.0))
    }

    #[test]
    fn builds_only_what_is_in_view_once_measured() {
        let mut ui = ui();
        frame(&mut ui, 1000, &[]);
        let w = frame(&mut ui, 1000, &[]);
        // 200 px over 58 px a row: four in view and one past it.
        assert_eq!(items(&w), vec![0, 1, 2, 3, 4]);
        let scroll = Id::new("list");
        ui.set_scroll_offset(scroll, 58.0 * 500.0);
        let w = frame(&mut ui, 1000, &[]);
        assert_eq!(items(&w), (499..=504).collect::<Vec<_>>());
        // The spacers keep the content as tall as every item built.
        let max = ui.st.scroll[&scroll].max;
        assert!((max - (1000.0 * 58.0 - 8.0 - 200.0)).abs() < 1.0, "{max}");
        let laid = ui.rect_of(Id::new("list")).map(|r| r.h);
        assert_eq!(laid, Some(200.0));
    }

    #[test]
    fn keeps_pinned_and_focused_items_with_spacers_between() {
        let mut ui = ui();
        frame(&mut ui, 100, &[]);
        let ks = keys(100);
        ui.set_focus(Some(ks[50]));
        let w = frame(&mut ui, 100, &[ks[90]]);
        assert_eq!(items(&w), vec![0, 1, 2, 3, 4, 49, 50, 51, 90]);
        let spaces: Vec<f32> = w
            .rows
            .iter()
            .filter_map(|r| match r {
                Row::Space(h) => Some(*h),
                Row::Item(_) => None,
            })
            .collect();
        // 44 skipped rows of 58 px, less the gap the spacer brings itself.
        assert_eq!(spaces, vec![44.0 * 58.0 - 8.0, 38.0 * 58.0 - 8.0, 9.0 * 58.0 - 8.0]);
    }

    #[test]
    fn anchors_the_first_item_in_view_as_heights_above_settle() {
        let mut ui = ui();
        let scroll = Id::new("list");
        // Guessed at 40 px, the items measure 50: scrolled down before any
        // is measured, the item first in view must not jump.
        frame(&mut ui, 1000, &[]);
        ui.set_scroll_offset(scroll, 4800.0);
        frame(&mut ui, 1000, &[]);
        let anchor = ui.st.virt[&scroll].anchor.expect("an item in view");
        let before = ui.st.scroll[&scroll].offset;
        frame(&mut ui, 1000, &[]);
        let after = ui.st.scroll[&scroll].offset;
        let now = ui.st.virt[&scroll].anchor.expect("an item in view");
        assert_eq!(anchor.0, now.0, "the same item stays first in view");
        assert!(
            ((anchor.1 - before) - (now.1 - after)).abs() < 0.5,
            "{anchor:?} at {before}, {now:?} at {after}"
        );
    }

    #[test]
    fn spacers_count_rows_on_cells() {
        let el = spacer(7.0, true);
        let mut ui = Ui::cells(&UiConfig::system(1.0, 13.0));
        let list = div().col().h(28.0 * 20.0).w(80.0).child(el).child(
            div()
                .id(Id::new("after"))
                .on(crate::Action::Click)
                .child(text("x", 12.0, Rgba::white(1.0))),
        );
        ui.layout(list, crate::cells::units::px(20, 20));
        assert_eq!(ui.rect_of(Id::new("after")).map(|r| r.y), Some(7.0));
    }
}
