//! Correctness against the baseline: libghostty-vt (through vorn-screen, as
//! vornd runs it) and the candidate fed the same reads in lockstep, their
//! visible grids compared cell for cell at checkpoints.
//!
//! vorn-recovery's comparator diffs Ghostty's own VT serialization, which
//! only Ghostty can write, so this compares the engine-neutral
//! [`vt_api::Grid`] instead and sorts each difference into a kind.
//!
//! A cell is compared by what it shows: a blank cell's foreground and text
//! attributes are invisible, and a spacer is only checked for being one.

use std::collections::BTreeMap;
use std::panic::{self, AssertUnwindSafe};

use vorn_recovery::Profile;
use vt_api::{Attrs, Cell, Engine, Grid, Width};
use vt_ghostty::Ghostty;

use crate::corpus::{self, Corpus, Op};
use crate::json::{str_array, Obj};
use crate::{play, SCROLLBACK};

/// Examples kept per kind of difference.
const EXAMPLES: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// The candidate panicked on the input; the session stops there.
    Panicked,
    Dimensions,
    /// A row's text differs after a resize the screens have not converged
    /// from: the engines reflow differently.
    Reflow,
    /// The candidate's row matches a nearby baseline row: a scroll or wrap
    /// landed one line off.
    RowsShifted,
    /// Text differs in a row holding wide or non-ASCII characters.
    WideText,
    Text,
    /// Same text, but one engine has a wide cell or spacer where the other
    /// has not.
    WideCells,
    Colours,
    Attributes,
    Hyperlinks,
    CursorPosition,
    CursorVisibility,
    AltScreen,
    Title,
    Scrollback,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Panicked => "engine panicked",
            Kind::Dimensions => "dimensions",
            Kind::Reflow => "text after resize (reflow)",
            Kind::RowsShifted => "rows shifted (scroll/wrap)",
            Kind::WideText => "text with wide/non-ASCII chars",
            Kind::Text => "text",
            Kind::WideCells => "wide cell/spacer layout",
            Kind::Colours => "colours",
            Kind::Attributes => "attributes",
            Kind::Hyperlinks => "hyperlinks",
            Kind::CursorPosition => "cursor position",
            Kind::CursorVisibility => "cursor visibility",
            Kind::AltScreen => "alternate screen",
            Kind::Title => "title",
            Kind::Scrollback => "scrollback line count",
        }
    }
}

#[derive(Default)]
struct KindTally {
    checkpoints: u64,
    cells: u64,
    examples: Vec<String>,
}

#[derive(Default)]
pub struct Tally {
    checkpoints: u64,
    identical: u64,
    cells: u64,
    cells_equal: u64,
    text_equal: u64,
    kinds: BTreeMap<Kind, KindTally>,
}

impl Tally {
    fn note(&mut self, kind: Kind, cells: u64, example: impl FnOnce() -> String) {
        let k = self.kinds.entry(kind).or_default();
        k.cells += cells;
        if k.examples.len() < EXAMPLES {
            k.examples.push(example());
        }
    }

    fn to_json(&self, engine: &str, corpus: &str) -> String {
        let pct = |a: u64, b: u64| {
            if b == 0 {
                100.0
            } else {
                100.0 * a as f64 / b as f64
            }
        };
        let mut kinds = String::from("[");
        for (i, (k, t)) in self.kinds.iter().enumerate() {
            if i > 0 {
                kinds.push(',');
            }
            let o = Obj::new()
                .str("kind", k.name())
                .num("checkpoints", t.checkpoints as f64)
                .num("cells", t.cells as f64)
                .raw(
                    "examples",
                    &str_array(t.examples.iter().map(String::as_str)),
                );
            kinds.push_str(&o.to_string());
        }
        kinds.push(']');
        Obj::new()
            .str("engine", engine)
            .str("corpus", corpus)
            .num("checkpoints", self.checkpoints as f64)
            .num("identical_checkpoints", self.identical as f64)
            .num("identical_pct", pct(self.identical, self.checkpoints))
            .num("cells", self.cells as f64)
            .num("cell_match_pct", pct(self.cells_equal, self.cells))
            .num("text_match_pct", pct(self.text_equal, self.cells))
            .raw("kinds", &kinds)
            .to_string()
    }
}

fn shows_text(c: &Cell) -> bool {
    !c.text.is_empty()
}

/// Whether two cells look the same on screen.
fn same_look(a: &Cell, b: &Cell) -> bool {
    if a.width == Width::Spacer || b.width == Width::Spacer {
        return a.width == b.width;
    }
    if !shows_text(a) && !shows_text(b) {
        let marks = Attrs::INVERSE;
        return a.bg == b.bg
            && a.attrs.contains(marks) == b.attrs.contains(marks)
            && a.link == b.link;
    }
    a == b
}

fn clip(s: &str) -> String {
    s.chars().take(100).collect()
}

/// Compares one checkpoint and adds it to `t`. Returns whether the screens
/// were identical.
fn check(
    t: &mut Tally,
    at: &str,
    base: &mut Ghostty,
    cand: &mut impl Engine,
    since_resize: bool,
) -> bool {
    let (g, c) = (base.grid(), cand.grid());
    let mut seen: Vec<Kind> = Vec::new();
    let mut note = |t: &mut Tally, k: Kind, cells: u64, ex: &dyn Fn() -> String| {
        t.note(k, cells, ex);
        if !seen.contains(&k) {
            seen.push(k);
        }
    };
    t.checkpoints += 1;
    if (g.cols, g.rows) != (c.cols, c.rows) {
        note(t, Kind::Dimensions, 0, &|| {
            format!("{at}: {}x{} vs {}x{}", g.cols, g.rows, c.cols, c.rows)
        });
    } else {
        compare_cells(t, at, &g, &c, since_resize, &mut note);
    }
    let (gc, cc) = (base.cursor(), cand.cursor());
    if (gc.x, gc.y) != (cc.x, cc.y) {
        note(t, Kind::CursorPosition, 0, &|| {
            format!("{at}: ({},{}) vs ({},{})", gc.x, gc.y, cc.x, cc.y)
        });
    }
    if gc.visible != cc.visible {
        note(t, Kind::CursorVisibility, 0, &|| {
            format!("{at}: {} vs {}", gc.visible, cc.visible)
        });
    }
    if base.alt_screen() != cand.alt_screen() {
        note(t, Kind::AltScreen, 0, &|| {
            format!("{at}: {} vs {}", base.alt_screen(), cand.alt_screen())
        });
    }
    let (gt, ct) = (base.title(), cand.title());
    if gt != ct {
        note(t, Kind::Title, 0, &|| format!("{at}: {gt:?} vs {ct:?}"));
    }
    // Ghostty's byte budget can keep a little more than asked; compare up to
    // what was asked.
    let (gs, cs) = (
        base.scrollback_lines().min(SCROLLBACK),
        cand.scrollback_lines().min(SCROLLBACK),
    );
    if gs != cs {
        note(t, Kind::Scrollback, 0, &|| format!("{at}: {gs} vs {cs}"));
    }
    for k in &seen {
        t.kinds.get_mut(k).expect("noted").checkpoints += 1;
    }
    let identical = seen.is_empty();
    t.identical += u64::from(identical);
    identical
}

type Note<'a> = dyn FnMut(&mut Tally, Kind, u64, &dyn Fn() -> String) + 'a;

fn compare_cells(t: &mut Tally, at: &str, g: &Grid, c: &Grid, since_resize: bool, note: &mut Note) {
    for y in 0..g.rows {
        let (gr, cr) = (g.row(y), c.row(y));
        let (gt, ct) = (g.row_text(y), c.row_text(y));
        let mut text_off = 0u64;
        for (a, b) in gr.iter().zip(cr) {
            t.cells += 1;
            let text_same =
                a.width == Width::Spacer && b.width == Width::Spacer || a.text == b.text;
            t.text_equal += u64::from(text_same);
            text_off += u64::from(!text_same);
            if same_look(a, b) {
                t.cells_equal += 1;
            } else if text_same {
                let kind = if a.width != b.width {
                    Kind::WideCells
                } else if a.fg != b.fg || a.bg != b.bg {
                    Kind::Colours
                } else if a.attrs != b.attrs || a.underline != b.underline {
                    Kind::Attributes
                } else {
                    Kind::Hyperlinks
                };
                note(t, kind, 1, &|| {
                    format!("{at} row {y} {:?}: {a:?} vs {b:?}", clip(&gt))
                });
            }
        }
        if text_off > 0 {
            let kind = if since_resize {
                Kind::Reflow
            } else if (1..=3).any(|d| {
                (y >= d && g.row_text(y - d) == ct) || (y + d < g.rows && g.row_text(y + d) == ct)
            }) && !ct.is_empty()
            {
                Kind::RowsShifted
            } else if gr.iter().chain(cr).any(|x| x.width != Width::Narrow)
                || !gt.is_ascii()
                || !ct.is_ascii()
            {
                Kind::WideText
            } else {
                Kind::Text
            };
            note(t, kind, text_off, &|| {
                format!("{at} row {y}: {:?} vs {:?}", clip(&gt), clip(&ct))
            });
        }
    }
}

/// Feeds `corpus` to both engines, checking every `every` ops and at the end.
/// A candidate that panics ends the session there, counted as a difference.
fn lockstep<E: Engine>(t: &mut Tally, label: &str, c: &Corpus, every: usize) {
    let mut base = Ghostty::new(c.cols, c.rows, SCROLLBACK);
    let mut cand = E::new(c.cols, c.rows, SCROLLBACK);
    let mut since_resize = false;
    for (i, op) in c.ops.iter().enumerate() {
        play(&mut base, std::slice::from_ref(op));
        since_resize |= matches!(op, Op::Resize(..));
        let fed = panic::catch_unwind(AssertUnwindSafe(|| {
            play(&mut cand, std::slice::from_ref(op));
        }));
        if fed.is_err() {
            t.checkpoints += 1;
            t.note(Kind::Panicked, 0, || format!("{label}@{}", i + 1));
            t.kinds.get_mut(&Kind::Panicked).expect("noted").checkpoints += 1;
            return;
        }
        if (i + 1) % every == 0 || i + 1 == c.ops.len() {
            base.take_replies();
            cand.take_replies();
            if check(
                t,
                &format!("{label}@{}", i + 1),
                &mut base,
                &mut cand,
                since_resize,
            ) {
                since_resize = false;
            }
        }
    }
}

/// Seeds of the generator corpus; each is a separate session.
const SEEDS: u64 = 8;

/// One JSON line per corpus.
pub fn run<E: Engine>(name: &str) -> Vec<String> {
    let mut t = Tally::default();
    match name {
        "vim" | "htop" | "agent" => lockstep::<E>(&mut t, name, &corpus::load_once(name, 0), 1),
        "build-log" => lockstep::<E>(&mut t, name, &corpus::build_log(7, 2 << 20), 16),
        "seeded" | "seeded-fixed" => {
            let profile = if name == "seeded" {
                Profile::mixed()
            } else {
                Profile::mixed().mix(corpus::NO_RESIZE)
            };
            for seed in 1..=SEEDS {
                let c = corpus::seeded(seed, profile, 256 << 10);
                lockstep::<E>(&mut t, &format!("seed{seed}"), &c, 16);
            }
        }
        other => panic!("unknown corpus {other}"),
    }
    vec![t.to_json(E::NAME, name)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use vt_ghostty::GhosttyRaw;

    #[test]
    fn blank_cells_compare_by_background_only() {
        let a = Cell::default();
        let b = Cell {
            fg: vt_api::Color::Indexed(3),
            ..Cell::default()
        };
        assert!(same_look(&a, &b));
        let c = Cell {
            bg: vt_api::Color::Indexed(3),
            ..Cell::default()
        };
        assert!(!same_look(&a, &c));
    }

    #[test]
    fn the_baseline_matches_itself_without_vorn_screen() {
        let mut t = Tally::default();
        lockstep::<GhosttyRaw>(&mut t, "vim", &corpus::load_once("vim", 0), 1);
        assert_eq!(t.cells_equal, t.cells);
        assert_eq!(t.identical, t.checkpoints);
    }

    #[test]
    fn sorts_a_one_line_scroll_as_shifted_rows() {
        let mut t = Tally::default();
        let mut base = Ghostty::new(10, 4, 10);
        let mut cand = Ghostty::new(10, 4, 10);
        base.feed(b"a\r\nb\r\nc\r\nd");
        cand.feed(b"a\r\nb\r\nc\r\nd\r\n");
        assert!(!check(&mut t, "t", &mut base, &mut cand, false));
        assert!(t.kinds.contains_key(&Kind::RowsShifted));
        assert!(t.kinds.contains_key(&Kind::CursorPosition));
    }
}
