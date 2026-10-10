//! The app's design tokens: the colours in `src/renderer/theme.css`, the
//! Tailwind v4 defaults the components use (grays, type scale, radii,
//! transitions) and the global rules in `global.css` (focus ring,
//! scrollbar). Widgets read these instead of literals so a port stays a
//! port: a test checks the colours against `theme.css` itself.

use std::time::Duration;

use crate::scene::Rgba;

/// Colours from `theme.css`.
pub mod color {
    use super::Rgba;

    pub const BRONZO: Rgba = Rgba::hex(0xc9972a);
    pub const BRONZO_DARK: Rgba = Rgba::hex(0xb8862a);
    pub const DANGER: Rgba = Rgba::hex(0xd4623f);
    pub const INK: Rgba = Rgba::hex(0xfaf9f7);
    pub const INK_SECONDARY: Rgba = Rgba::white(0.55);
    pub const INK_FAINT: Rgba = Rgba::white(0.35);
    pub const INK_GHOST: Rgba = Rgba::white(0.18);
    pub const STATUS_SLATE: Rgba = Rgba::hex(0x7d8590);
    pub const STATUS_BLUE: Rgba = Rgba::hex(0x6f8faf);
    pub const STATUS_SAGE: Rgba = Rgba::hex(0x7d9471);
    pub const DIFF_ADD: Rgba = Rgba::hex(0x7ea96a);
    pub const DIFF_REMOVE: Rgba = Rgba::hex(0xc96f62);
    pub const SURFACE_BASE: Rgba = Rgba::hex(0x0d0d0f);
    pub const SURFACE_SUNKEN: Rgba = Rgba::hex(0x141416);
    pub const SURFACE_PANEL: Rgba = Rgba::hex(0x101012);
    pub const SURFACE_OVERLAY: Rgba = Rgba::hex(0x1c1c20);
    pub const SURFACE_RAISED: Rgba = SURFACE_SUNKEN;
    pub const SURFACE_NODE: Rgba = SURFACE_SUNKEN;

    /// Tailwind v4's grays (its OKLCH values in sRGB).
    pub const GRAY_100: Rgba = Rgba::hex(0xf3f4f6);
    pub const GRAY_200: Rgba = Rgba::hex(0xe5e7eb);
    pub const GRAY_300: Rgba = Rgba::hex(0xd1d5dc);
    pub const GRAY_400: Rgba = Rgba::hex(0x99a1af);
    pub const GRAY_500: Rgba = Rgba::hex(0x6a7282);
    pub const GRAY_600: Rgba = Rgba::hex(0x4a5565);
    pub const GRAY_700: Rgba = Rgba::hex(0x364153);
    pub const BLUE_500: Rgba = Rgba::hex(0x2b7fff);
    pub const WHITE: Rgba = Rgba::white(1.0);

    /// Text selection: the colour Chromium paints `::selection` with when
    /// the page does not style it.
    pub const SELECTION: Rgba = Rgba::rgba_const(0x33, 0x90, 0xff, 0.4);
}

/// Tailwind's type scale, logical pixels: (size, line height).
pub mod text {
    pub const XS: (f32, f32) = (12.0, 16.0);
    pub const SM: (f32, f32) = (14.0, 20.0);
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

/// Tailwind's `ease-in-out`, `cubic-bezier(0.4, 0, 0.2, 1)`.
pub fn ease_in_out(t: f32) -> f32 {
    match t {
        t if t <= 0.0 => 0.0,
        t if t >= 1.0 => 1.0,
        t => cubic_bezier(0.4, 0.0, 0.2, 1.0, t),
    }
}

/// `y` of a CSS cubic-bezier timing function at `x`.
fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
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
