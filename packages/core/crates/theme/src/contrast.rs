//! Contrast correction: lifting a colour until text drawn in it reads at
//! 4.5:1 (WCAG 2's AA for body text) on its background, without moving it
//! out of its hue family, so a program's red still reads as red.
//!
//! Lightness moves in OKLCH with the hue held; chroma is given up only
//! where the lighter or darker colour leaves sRGB. Status colours keep their
//! meaning because their hue does not move.

use crate::Rgba8;

/// WCAG 2 AA for body text.
pub const MIN_RATIO: f32 = 4.5;

/// Below this OKLCH chroma a colour reads as a gray and has no hue family.
pub const GRAY_CHROMA: f32 = 0.03;

/// WCAG 2's contrast ratio of two opaque colours, 1 to 21.
pub fn ratio(a: Rgba8, b: Rgba8) -> f32 {
    let (la, lb) = (a.luminance(), b.luminance());
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// `fg` lifted until it reads at `min` on the opaque `bg`, or `fg` itself
/// when it already does. A translucent `fg` first gains opacity, so it keeps
/// its relation to the surfaces around; only then does its colour move.
pub fn correct(fg: Rgba8, bg: Rgba8, min: f32) -> Rgba8 {
    let bg = Rgba8 { a: 255, ..bg };
    if ratio(fg.over(bg), bg) >= min {
        return fg;
    }
    if fg.a < 255 {
        let opaque = Rgba8 { a: 255, ..fg };
        if ratio(opaque, bg) >= min {
            let (mut lo, mut hi) = (fg.alpha(), 1.0f32);
            for _ in 0..12 {
                let mid = (lo + hi) / 2.0;
                if ratio(fg.with_alpha(mid).over(bg), bg) >= min {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            return fg.with_alpha(hi);
        }
        return correct(opaque, bg, min);
    }
    let start = fg.oklch();
    let away_up = bg.luminance() < 0.18;
    // Away from the background first; the other way only if that cannot reach `min`.
    for up in [away_up, !away_up] {
        let end = if up { 1.0 } else { 0.0 };
        let at = |l: f32| {
            crate::color::Oklch {
                l: start.l + (end - start.l) * l,
                ..start
            }
            .to_rgb_clipped(255)
        };
        if ratio(at(1.0), bg) < min {
            continue;
        }
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..16 {
            let mid = (lo + hi) / 2.0;
            if ratio(at(mid), bg) >= min {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        return at(hi);
    }
    // No colour reaches `min` on a mid gray: the extreme that comes closest.
    let (white, black) = (ratio(Rgba8::WHITE, bg), ratio(Rgba8::BLACK, bg));
    if white >= black {
        Rgba8::WHITE
    } else {
        Rgba8::BLACK
    }
}

/// The broad hue a colour reads as, or `None` for a gray. Correction keeps
/// a colour in its family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Red,
    Orange,
    Yellow,
    Green,
    Cyan,
    Blue,
    Purple,
    Pink,
}

/// The hue family of `c` by its OKLCH hue.
pub fn family(c: Rgba8) -> Option<Family> {
    let lch = c.oklch();
    if lch.c < GRAY_CHROMA {
        return None;
    }
    Some(match lch.h {
        h if !(10.0..350.0).contains(&h) => Family::Pink,
        h if h < 45.0 => Family::Red,
        h if h < 65.0 => Family::Orange,
        h if h < 115.0 => Family::Yellow,
        h if h < 175.0 => Family::Green,
        h if h < 230.0 => Family::Cyan,
        h if h < 290.0 => Family::Blue,
        h if h < 325.0 => Family::Purple,
        _ => Family::Pink,
    })
}

/// Corrected foregrounds by `(fg, bg)`, so a terminal's cells cost a lookup
/// rather than a search: direct-mapped, a fixed size, no allocation after
/// [`Cache::new`].
pub struct Cache {
    slots: Box<[(u32, u32, u32); Cache::SLOTS]>,
}

impl Cache {
    const SLOTS: usize = 1024;
    const EMPTY: (u32, u32, u32) = (u32::MAX, u32::MAX, 0);

    pub fn new() -> Cache {
        Cache {
            slots: Box::new([Cache::EMPTY; Cache::SLOTS]),
        }
    }

    /// `fg` (`0xRRGGBB`) lifted to [`MIN_RATIO`] on `bg`.
    pub fn get(&mut self, fg: u32, bg: u32) -> u32 {
        // Fibonacci hashing of the pair; collisions only cost a recompute.
        let key = (u64::from(fg) << 24 | u64::from(bg)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let slot = &mut self.slots[(key >> 54) as usize % Cache::SLOTS];
        if slot.0 == fg && slot.1 == bg {
            return slot.2;
        }
        let out = correct(Rgba8::rgb(fg), Rgba8::rgb(bg), MIN_RATIO).to_u32();
        *slot = (fg, bg, out);
        out
    }
}

impl Default for Cache {
    fn default() -> Cache {
        Cache::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// xorshift64*, so a failure names a seed that reproduces it.
    fn colors(seed: u64, n: usize) -> impl Iterator<Item = (Rgba8, Rgba8)> {
        let mut s = seed;
        let mut next = move || {
            s ^= s >> 12;
            s ^= s << 25;
            s ^= s >> 27;
            (s.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as u32
        };
        (0..n).map(move |_| (Rgba8::rgb(next()), Rgba8::rgb(next())))
    }

    #[test]
    fn ratio_matches_known_pairs() {
        assert!((ratio(Rgba8::WHITE, Rgba8::BLACK) - 21.0).abs() < 0.01);
        assert!((ratio(Rgba8::rgb(0x777777), Rgba8::WHITE) - 4.48).abs() < 0.01);
    }

    #[test]
    fn leaves_readable_text_alone() {
        let (fg, bg) = (Rgba8::rgb(0xd4d4d8), Rgba8::rgb(0x141416));
        assert_eq!(correct(fg, bg, MIN_RATIO), fg);
    }

    #[test]
    fn lifts_dark_blue_on_black_and_keeps_it_blue() {
        let (fg, bg) = (Rgba8::rgb(0x0000aa), Rgba8::rgb(0x000000));
        let out = correct(fg, bg, MIN_RATIO);
        assert!(ratio(out, bg) >= MIN_RATIO, "{out}");
        assert_eq!(family(out), Some(Family::Blue));
    }

    #[test]
    fn darkens_yellow_on_white_and_keeps_its_family() {
        let (fg, bg) = (Rgba8::rgb(0xf1fa8c), Rgba8::WHITE);
        let out = correct(fg, bg, MIN_RATIO);
        assert!(ratio(out, bg) >= MIN_RATIO, "{out}");
        assert_eq!(family(out), family(fg));
    }

    #[test]
    fn status_colours_keep_their_meaning_on_every_bundled_background() {
        for theme in crate::bundled() {
            let bg = theme.terminal.background;
            let mut seen = Vec::new();
            for i in [1, 2, 4] {
                let fg = theme.terminal.ansi[i];
                let out = correct(fg, bg, MIN_RATIO);
                assert!(ratio(out, bg) >= MIN_RATIO, "{} ansi {i}", theme.id);
                assert!(family(fg).is_some(), "{} ansi {i} is a gray", theme.id);
                assert_eq!(
                    family(out),
                    family(fg),
                    "{} ansi {i}: {fg} -> {out}",
                    theme.id
                );
                seen.push(family(out));
            }
            // Red, green and blue still read as three different things.
            assert!(
                seen[0] != seen[1] && seen[1] != seen[2] && seen[0] != seen[2],
                "{}",
                theme.id
            );
        }
    }

    #[test]
    fn translucent_text_gains_opacity_first() {
        let bg = Rgba8::rgb(0x0d0d0f);
        let faint = Rgba8::rgb(0xfaf9f7).with_alpha(0.35);
        let out = correct(faint, bg, MIN_RATIO);
        assert_eq!(out.to_u32(), faint.to_u32());
        assert!(out.a > faint.a);
        assert!(ratio(out.over(bg), bg) >= MIN_RATIO);
    }

    /// Every pair reaches 4.5:1 where any colour can, and a coloured
    /// foreground keeps its hue family. Failing seeds join `SEEDS`.
    #[test]
    fn property_every_cell_reads_and_keeps_its_hue() {
        const SEEDS: [u64; 4] = [0x5eed, 0x00c0_ffee, 42, 0xdead_beef];
        for seed in SEEDS {
            for (fg, bg) in colors(seed, 2000) {
                let out = correct(fg, bg, MIN_RATIO);
                let best = ratio(Rgba8::WHITE, bg).max(ratio(Rgba8::BLACK, bg));
                if best >= MIN_RATIO {
                    assert!(
                        ratio(out, bg) >= MIN_RATIO - 0.01,
                        "seed {seed:#x}: {fg} on {bg} -> {out} at {}",
                        ratio(out, bg)
                    );
                }
                if ratio(fg, bg) >= MIN_RATIO {
                    assert_eq!(out, fg);
                }
                let lch = fg.oklch();
                // Very dark or light colours carry hue the eye cannot see.
                if lch.c >= 0.08 && (0.25..0.92).contains(&lch.l) {
                    let moved = out.oklch();
                    if moved.c >= GRAY_CHROMA && out != Rgba8::WHITE && out != Rgba8::BLACK {
                        let dh = (moved.h - lch.h).abs();
                        assert!(
                            dh.min(360.0 - dh) < 12.0,
                            "seed {seed:#x}: {fg} -> {out} moved hue {dh}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn cache_answers_as_the_search_does() {
        let mut cache = Cache::new();
        for (fg, bg) in colors(7, 500) {
            let want = correct(fg, bg, MIN_RATIO).to_u32();
            assert_eq!(cache.get(fg.to_u32(), bg.to_u32()), want);
            assert_eq!(cache.get(fg.to_u32(), bg.to_u32()), want);
        }
    }
}
