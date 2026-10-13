//! The app's design tokens: the colours in `src/renderer/theme.css`, the
//! Tailwind v4 defaults the components use (grays, type scale, radii,
//! transitions) and the global rules in `global.css` (focus ring,
//! scrollbar). Widgets read these instead of literals so a port stays a
//! port: a test checks the colours against `theme.css` itself.

use std::time::Duration;

use crate::scene::Rgba;

/// The palette as `0xRRGGBB`: the one place each value is written.
/// [`color`] wraps these as [`Rgba`]; apps that keep `u32`s read them here.
pub mod rgb {
    pub const BRONZO: u32 = 0xc9972a;
    pub const BRONZO_DARK: u32 = 0xb8862a;
    pub const DANGER: u32 = 0xd4623f;
    pub const INK: u32 = 0xfaf9f7;
    pub const STATUS_SLATE: u32 = 0x7d8590;
    pub const STATUS_BLUE: u32 = 0x6f8faf;
    pub const STATUS_SAGE: u32 = 0x7d9471;
    pub const DIFF_ADD: u32 = 0x7ea96a;
    pub const DIFF_REMOVE: u32 = 0xc96f62;
    pub const SURFACE_BASE: u32 = 0x0d0d0f;
    pub const SURFACE_SUNKEN: u32 = 0x141416;
    pub const SURFACE_PANEL: u32 = 0x101012;
    pub const SURFACE_OVERLAY: u32 = 0x1c1c20;
    pub const SURFACE_RAISED: u32 = SURFACE_SUNKEN;
    pub const SURFACE_NODE: u32 = SURFACE_SUNKEN;
    pub const WHITE: u32 = 0xffffff;
    pub const BLACK: u32 = 0x000000;

    /// Tailwind v4's palette (its OKLCH values in sRGB), the steps in use.
    pub const GRAY_100: u32 = 0xf3f4f6;
    pub const GRAY_200: u32 = 0xe5e7eb;
    pub const GRAY_300: u32 = 0xd1d5dc;
    pub const GRAY_400: u32 = 0x99a1af;
    pub const GRAY_500: u32 = 0x6a7282;
    pub const GRAY_600: u32 = 0x4a5565;
    pub const GRAY_700: u32 = 0x364153;
    pub const GRAY_900: u32 = 0x101828;
    pub const GREEN_300: u32 = 0x7bf1a8;
    pub const GREEN_400: u32 = 0x05df72;
    pub const GREEN_500: u32 = 0x00c758;
    pub const GREEN_600: u32 = 0x00a63e;
    pub const EMERALD_400: u32 = 0x00d492;
    pub const YELLOW_500: u32 = 0xedb200;
    pub const RED_200: u32 = 0xffc9c9;
    pub const RED_300: u32 = 0xffa2a2;
    pub const RED_400: u32 = 0xff6467;
    pub const RED_500: u32 = 0xfb2c36;
    pub const RED_600: u32 = 0xe7000b;
    pub const AMBER_300: u32 = 0xffd230;
    pub const AMBER_400: u32 = 0xffba00;
    pub const AMBER_500: u32 = 0xfe9a00;
    pub const BLUE_300: u32 = 0x8ec5ff;
    pub const BLUE_400: u32 = 0x54a2ff;
    pub const BLUE_500: u32 = 0x2b7fff;
    pub const BLUE_600: u32 = 0x155dfc;
    pub const SKY_400: u32 = 0x00bcff;
}

/// Colours from `theme.css` and Tailwind, ready to paint.
pub mod color {
    use super::{rgb, Rgba};

    pub const BRONZO: Rgba = Rgba::hex(rgb::BRONZO);
    pub const BRONZO_DARK: Rgba = Rgba::hex(rgb::BRONZO_DARK);
    pub const DANGER: Rgba = Rgba::hex(rgb::DANGER);
    pub const INK: Rgba = Rgba::hex(rgb::INK);
    pub const INK_SECONDARY: Rgba = Rgba::white(0.55);
    pub const INK_FAINT: Rgba = Rgba::white(0.35);
    pub const INK_GHOST: Rgba = Rgba::white(0.18);
    pub const STATUS_SLATE: Rgba = Rgba::hex(rgb::STATUS_SLATE);
    pub const STATUS_BLUE: Rgba = Rgba::hex(rgb::STATUS_BLUE);
    pub const STATUS_SAGE: Rgba = Rgba::hex(rgb::STATUS_SAGE);
    pub const DIFF_ADD: Rgba = Rgba::hex(rgb::DIFF_ADD);
    pub const DIFF_REMOVE: Rgba = Rgba::hex(rgb::DIFF_REMOVE);
    pub const SURFACE_BASE: Rgba = Rgba::hex(rgb::SURFACE_BASE);
    pub const SURFACE_SUNKEN: Rgba = Rgba::hex(rgb::SURFACE_SUNKEN);
    pub const SURFACE_PANEL: Rgba = Rgba::hex(rgb::SURFACE_PANEL);
    pub const SURFACE_OVERLAY: Rgba = Rgba::hex(rgb::SURFACE_OVERLAY);
    pub const SURFACE_RAISED: Rgba = SURFACE_SUNKEN;
    pub const SURFACE_NODE: Rgba = SURFACE_SUNKEN;

    pub const GRAY_100: Rgba = Rgba::hex(rgb::GRAY_100);
    pub const GRAY_200: Rgba = Rgba::hex(rgb::GRAY_200);
    pub const GRAY_300: Rgba = Rgba::hex(rgb::GRAY_300);
    pub const GRAY_400: Rgba = Rgba::hex(rgb::GRAY_400);
    pub const GRAY_500: Rgba = Rgba::hex(rgb::GRAY_500);
    pub const GRAY_600: Rgba = Rgba::hex(rgb::GRAY_600);
    pub const GRAY_700: Rgba = Rgba::hex(rgb::GRAY_700);
    pub const GRAY_900: Rgba = Rgba::hex(rgb::GRAY_900);
    pub const GREEN_300: Rgba = Rgba::hex(rgb::GREEN_300);
    pub const GREEN_400: Rgba = Rgba::hex(rgb::GREEN_400);
    pub const GREEN_500: Rgba = Rgba::hex(rgb::GREEN_500);
    pub const GREEN_600: Rgba = Rgba::hex(rgb::GREEN_600);
    pub const EMERALD_400: Rgba = Rgba::hex(rgb::EMERALD_400);
    pub const YELLOW_500: Rgba = Rgba::hex(rgb::YELLOW_500);
    pub const RED_200: Rgba = Rgba::hex(rgb::RED_200);
    pub const RED_300: Rgba = Rgba::hex(rgb::RED_300);
    pub const RED_400: Rgba = Rgba::hex(rgb::RED_400);
    pub const RED_500: Rgba = Rgba::hex(rgb::RED_500);
    pub const RED_600: Rgba = Rgba::hex(rgb::RED_600);
    pub const AMBER_300: Rgba = Rgba::hex(rgb::AMBER_300);
    pub const AMBER_400: Rgba = Rgba::hex(rgb::AMBER_400);
    pub const AMBER_500: Rgba = Rgba::hex(rgb::AMBER_500);
    pub const BLUE_300: Rgba = Rgba::hex(rgb::BLUE_300);
    pub const BLUE_400: Rgba = Rgba::hex(rgb::BLUE_400);
    pub const BLUE_500: Rgba = Rgba::hex(rgb::BLUE_500);
    pub const BLUE_600: Rgba = Rgba::hex(rgb::BLUE_600);
    pub const SKY_400: Rgba = Rgba::hex(rgb::SKY_400);
    pub const WHITE: Rgba = Rgba::white(1.0);

    /// Text selection: the colour Chromium paints `::selection` with when
    /// the page does not style it.
    pub const SELECTION: Rgba = Rgba::rgba_const(0x33, 0x90, 0xff, 0.4);
}

/// The hairlines and washes the components draw with, as `white/[a]`
/// alphas: borders step 0.04 → 0.12, hover washes 0.04 → 0.10.
pub mod line {
    use super::Rgba;
    /// A card header's bottom rule (`border-white/[0.04]`).
    pub const FAINT: Rgba = Rgba::white(0.04);
    /// A pill or chip at rest (`border-white/[0.06]`).
    pub const SOFT: Rgba = Rgba::white(0.06);
    /// Tooltips, popovers and dividers (`border-white/[0.08]`).
    pub const BASE: Rgba = Rgba::white(0.08);
    /// Menus and dialogs (`border-white/10`).
    pub const STRONG: Rgba = Rgba::white(0.10);
    /// A pill or chip under the pointer (`border-white/[0.12]`).
    pub const HOVER: Rgba = Rgba::white(0.12);
}

/// Tailwind's type scale, logical pixels: (size, line height). The
/// bracketed sizes (`text-[11px]`) the components use take their own size
/// as line height under `leading-none`, else Tailwind's inherited 1.5.
pub mod text {
    pub const XS: (f32, f32) = (12.0, 16.0);
    pub const SM: (f32, f32) = (14.0, 20.0);
    pub const BASE: (f32, f32) = (16.0, 24.0);
    pub const LG: (f32, f32) = (18.0, 28.0);
    pub const XL: (f32, f32) = (20.0, 28.0);
    pub const XL2: (f32, f32) = (24.0, 32.0);
    /// `text-[9px]` to `text-[13px]`, the chrome's small sizes.
    pub const PX9: f32 = 9.0;
    pub const PX10: f32 = 10.0;
    pub const PX11: f32 = 11.0;
    pub const PX13: f32 = 13.0;
    /// `font-medium` and `font-semibold`.
    pub const MEDIUM: u16 = 500;
    pub const SEMIBOLD: u16 = 600;
}

/// The faces `global.css`'s stacks resolve to on each platform:
/// `-apple-system, "Segoe UI", Roboto, sans-serif` and the terminal's
/// monospace.
pub mod font {
    /// San Francisco's family name; fontdb's scan skips its file, so the
    /// text system loads [`SF_FILE`] itself.
    pub const SF: &str = "System Font";
    pub const SF_FILE: &str = "/System/Library/Fonts/SFNS.ttf";

    pub const UI: &str = if cfg!(target_os = "macos") {
        SF
    } else if cfg!(windows) {
        "Segoe UI"
    } else {
        "DejaVu Sans"
    };

    /// Tracking in em that widens [`SF`]'s default instance to the width
    /// browsers draw it at `size` px: they pick the optical size (`opsz`)
    /// for the size, an axis cosmic-text cannot set. Fitted to Chromium's
    /// widths of the same text at weights 400 to 600.
    pub fn sf_tracking(size: f32, weight: u16) -> f32 {
        const PX: [(f32, f32); 12] = [
            (9.0, 0.683),
            (10.0, 0.691),
            (11.0, 0.695),
            (12.0, 0.688),
            (13.0, 0.669),
            (14.0, 0.652),
            (15.0, 0.626),
            (16.0, 0.605),
            (18.0, 0.545),
            (20.0, 0.427),
            (24.0, 0.320),
            (40.0, 0.0),
        ];
        let px = match PX.iter().position(|&(s, _)| s >= size) {
            None => 0.0,
            Some(0) => PX[0].1,
            Some(i) => {
                let ((s0, p0), (s1, p1)) = (PX[i - 1], PX[i]);
                p0 + (p1 - p0) * (size - s0) / (s1 - s0)
            }
        };
        let px = px - 0.00024 * (f32::from(weight) - 400.0);
        (px / size).max(0.0)
    }

    pub const MONO: &str = if cfg!(target_os = "macos") {
        "Menlo"
    } else if cfg!(windows) {
        "Consolas"
    } else {
        "DejaVu Sans Mono"
    };
}

/// Tailwind's spacing: `p-1` is one unit of 4 logical pixels.
pub const fn space(units: f32) -> f32 {
    units * 4.0
}

/// Tailwind's `box-shadow` scale and the glass panels' shadow, as
/// (y offset, blur, spread, alpha) of black layers, for what floats over
/// the page (menus, popovers, dialogs, tooltips).
pub mod shadow {
    /// `shadow-sm`.
    pub const SM: [(f32, f32, f32, f32); 2] = [(1.0, 3.0, 0.0, 0.1), (1.0, 2.0, -1.0, 0.1)];
    /// `shadow-md`.
    pub const MD: [(f32, f32, f32, f32); 2] = [(4.0, 6.0, -1.0, 0.1), (2.0, 4.0, -2.0, 0.1)];
    /// `shadow-lg`.
    pub const LG: [(f32, f32, f32, f32); 2] = [(10.0, 15.0, -3.0, 0.1), (4.0, 6.0, -4.0, 0.1)];
    /// `shadow-xl`.
    pub const XL: [(f32, f32, f32, f32); 2] = [(20.0, 25.0, -5.0, 0.1), (8.0, 10.0, -6.0, 0.1)];
    /// `shadow-2xl`.
    pub const XL2: [(f32, f32, f32, f32); 1] = [(25.0, 50.0, -12.0, 0.25)];
    /// `--glass-shadow`'s outer part.
    pub const GLASS: [(f32, f32, f32, f32); 1] = [(0.0, 25.0, 0.0, 0.15)];
}

/// Tailwind's radii, logical pixels.
pub mod radius {
    pub const SM: f32 = 4.0;
    pub const MD: f32 = 6.0;
    pub const LG: f32 = 8.0;
    pub const XL: f32 = 12.0;
    pub const FULL: f32 = 9999.0;
}

/// `transition-colors`: Tailwind's default duration and easing.
pub const TRANSITION: Duration = Duration::from_millis(150);

/// Motion beyond hover fades.
pub mod motion {
    use std::time::Duration;
    /// `animate-pulse`: opacity 1 → 0.5 → 1, `cubic-bezier(0.4, 0, 0.6, 1)`.
    pub const PULSE: Duration = Duration::from_secs(2);
    /// `animate-spin`: one turn, linear.
    pub const SPIN: Duration = Duration::from_secs(1);
    /// How often a view that moves on its own asks for a frame: opacity
    /// steps this small read as continuous.
    pub const FRAME: Duration = Duration::from_millis(33);
}

/// Tailwind's `ease-in-out`, `cubic-bezier(0.4, 0, 0.2, 1)`.
pub fn ease_in_out(t: f32) -> f32 {
    match t {
        t if t <= 0.0 => 0.0,
        t if t >= 1.0 => 1.0,
        t => cubic_bezier(0.4, 0.0, 0.2, 1.0, t),
    }
}

/// `y` of a CSS `cubic-bezier(x1, y1, x2, y2)` timing function at `x`.
pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    let curve = |a: f32, b: f32, t: f32| {
        let u = 1.0 - t;
        3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
    };
    // Bisection: the curve is monotonic in x for CSS timing functions.
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if curve(x1, x2, mid) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    curve(y1, y2, (lo + hi) / 2.0)
}

/// `backdrop-filter: blur()` standard deviations, logical pixels.
pub mod blur {
    /// `backdrop-blur-sm`.
    pub const SM: f32 = 4.0;
    /// `--glass-blur`.
    pub const GLASS: f32 = 16.0;
}

/// `:focus-visible` from `global.css`: a 1px outline, 1px out, and the
/// element's corners forced to 4px.
pub mod focus {
    use super::Rgba;
    pub const RING: Rgba = Rgba::white(0.45);
    pub const WIDTH: f32 = 1.0;
    pub const OFFSET: f32 = 1.0;
    pub const RADIUS: f32 = 4.0;
}

/// `::-webkit-scrollbar` from `global.css`.
pub mod scrollbar {
    use super::Rgba;
    pub const WIDTH: f32 = 6.0;
    pub const RADIUS: f32 = 3.0;
    pub const THUMB: Rgba = Rgba::white(0.08);
    pub const THUMB_HOVER: Rgba = Rgba::white(0.2);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(css: &str, name: &str) -> Option<Rgba> {
        let line = css.lines().find(|l| l.trim_start().starts_with(name))?;
        let v = line.split(':').nth(1)?.split(';').next()?.trim();
        if let Some(hex) = v.strip_prefix('#') {
            return u32::from_str_radix(hex, 16).ok().map(Rgba::hex);
        }
        let inner = v.strip_prefix("rgba(")?.strip_suffix(')')?;
        let n: Vec<f32> = inner
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        Some(Rgba::rgba_const(n[0] as u8, n[1] as u8, n[2] as u8, n[3]))
    }

    #[test]
    fn colors_match_theme_css() {
        let css = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../../src/renderer/theme.css"
        ));
        let pairs = [
            ("--color-bronzo:", color::BRONZO),
            ("--color-bronzo-dark:", color::BRONZO_DARK),
            ("--color-danger:", color::DANGER),
            ("--color-ink:", color::INK),
            ("--color-ink-secondary:", color::INK_SECONDARY),
            ("--color-ink-faint:", color::INK_FAINT),
            ("--color-ink-ghost:", color::INK_GHOST),
            ("--color-status-slate:", color::STATUS_SLATE),
            ("--color-status-blue:", color::STATUS_BLUE),
            ("--color-status-sage:", color::STATUS_SAGE),
            ("--color-diff-add:", color::DIFF_ADD),
            ("--color-diff-remove:", color::DIFF_REMOVE),
            ("--color-surface-base:", color::SURFACE_BASE),
            ("--color-surface-sunken:", color::SURFACE_SUNKEN),
            ("--color-surface-panel:", color::SURFACE_PANEL),
            ("--color-surface-overlay:", color::SURFACE_OVERLAY),
        ];
        for (name, ours) in pairs {
            let theirs = parse(css, name).unwrap_or_else(|| panic!("{name} not in theme.css"));
            for c in 0..4 {
                assert!(
                    (theirs.0[c] - ours.0[c]).abs() < 1e-3,
                    "{name}: {theirs:?} != {ours:?}"
                );
            }
        }
        assert!(css.contains("--color-surface-raised: var(--color-surface-sunken)"));
    }

    #[test]
    fn easing_matches_css_endpoints_and_shape() {
        assert_eq!(ease_in_out(0.0), 0.0);
        assert!((ease_in_out(1.0) - 1.0).abs() < 1e-4);
        // cubic-bezier(.4,0,.2,1) at x=.5 is about .78 (slow start, fast end).
        assert!(
            (ease_in_out(0.5) - 0.78).abs() < 0.02,
            "{}",
            ease_in_out(0.5)
        );
    }
}
