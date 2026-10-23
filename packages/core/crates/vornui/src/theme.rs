//! The app's design tokens: the theme's colours, type scale, radii,
//! transitions, focus ring and scrollbar. Widgets read these instead of
//! literals; colours come from the installed theme (`crate::palette`).

use web_time::Duration;

pub use crate::palette::{bright, shadow, Color};
pub use vorn_theme::Token;

/// The theme's colours by the names the widgets use: each a token the
/// installed theme gives a value (`crate::palette`), never a literal.
pub mod color {
    use super::{Color, Token};

    const fn t(token: Token) -> Color {
        Color::Token(token)
    }

    pub const BRONZO: Color = t(Token::Accent);
    pub const BRONZO_DARK: Color = t(Token::AccentStrong);
    pub const DANGER: Color = t(Token::Danger);
    pub const INK: Color = t(Token::Ink);
    pub const INK_SECONDARY: Color = t(Token::InkSecondary);
    pub const INK_FAINT: Color = t(Token::InkFaint);
    pub const INK_GHOST: Color = t(Token::InkGhost);
    pub const STATUS_SLATE: Color = t(Token::StatusSlate);
    pub const STATUS_BLUE: Color = t(Token::StatusBlue);
    pub const STATUS_SAGE: Color = t(Token::StatusSage);
    pub const DIFF_ADD: Color = t(Token::DiffAdd);
    pub const DIFF_REMOVE: Color = t(Token::DiffRemove);
    pub const SURFACE_BASE: Color = t(Token::SurfaceBase);
    pub const SURFACE_SUNKEN: Color = t(Token::SurfaceSunken);
    pub const SURFACE_PANEL: Color = t(Token::SurfacePanel);
    pub const SURFACE_OVERLAY: Color = t(Token::SurfaceOverlay);
    pub const SURFACE_RAISED: Color = SURFACE_SUNKEN;
    pub const SURFACE_NODE: Color = SURFACE_SUNKEN;
    /// The brightest ink: white on a dark theme, black on a light one.
    pub const WHITE: Color = t(Token::InkBright);
    /// Scrims and shadows.
    pub const SHADOW: Color = t(Token::Shadow);
    /// The groove a slider runs in.
    pub const SLIDER_TRACK: Color = t(Token::SliderTrack);

    /// Tailwind v4's steps in use, as each theme maps them.
    pub const GRAY_100: Color = t(Token::Gray100);
    pub const GRAY_200: Color = t(Token::Gray200);
    pub const GRAY_300: Color = t(Token::Gray300);
    pub const GRAY_400: Color = t(Token::Gray400);
    pub const GRAY_500: Color = t(Token::Gray500);
    pub const GRAY_600: Color = t(Token::Gray600);
    pub const GRAY_700: Color = t(Token::Gray700);
    pub const GRAY_800: Color = t(Token::Gray800);
    pub const GRAY_900: Color = t(Token::Gray900);
    pub const GREEN_300: Color = t(Token::Green300);
    pub const GREEN_400: Color = t(Token::Green400);
    pub const GREEN_500: Color = t(Token::Green500);
    pub const GREEN_600: Color = t(Token::Green600);
    pub const EMERALD_400: Color = t(Token::Emerald400);
    pub const YELLOW_400: Color = t(Token::Yellow400);
    pub const YELLOW_500: Color = t(Token::Yellow500);
    pub const RED_200: Color = t(Token::Red200);
    pub const RED_300: Color = t(Token::Red300);
    pub const RED_400: Color = t(Token::Red400);
    pub const RED_500: Color = t(Token::Red500);
    pub const RED_600: Color = t(Token::Red600);
    pub const ORANGE_400: Color = t(Token::Orange400);
    pub const ORANGE_500: Color = t(Token::Orange500);
    pub const AMBER_300: Color = t(Token::Amber300);
    pub const AMBER_400: Color = t(Token::Amber400);
    pub const AMBER_500: Color = t(Token::Amber500);
    pub const CYAN_400: Color = t(Token::Cyan400);
    pub const SKY_400: Color = t(Token::Sky400);
    pub const BLUE_300: Color = t(Token::Blue300);
    pub const BLUE_400: Color = t(Token::Blue400);
    pub const BLUE_500: Color = t(Token::Blue500);
    pub const BLUE_600: Color = t(Token::Blue600);
    pub const VIOLET_400: Color = t(Token::Violet400);
    pub const VIOLET_500: Color = t(Token::Violet500);
    pub const PURPLE_400: Color = t(Token::Purple400);
    pub const PINK_400: Color = t(Token::Pink400);
    pub const PINK_500: Color = t(Token::Pink500);

    /// Text selection.
    pub const SELECTION: Color = t(Token::Selection);
}

/// The hairlines the components draw with, faint to strong.
pub mod line {
    use super::{Color, Token};
    /// A card header's bottom rule.
    pub const FAINT: Color = Color::Token(Token::LineFaint);
    /// A pill or chip at rest.
    pub const SOFT: Color = Color::Token(Token::LineSoft);
    /// Tooltips, popovers and dividers.
    pub const BASE: Color = Color::Token(Token::LineBase);
    /// Menus and dialogs.
    pub const STRONG: Color = Color::Token(Token::LineStrong);
    /// A pill or chip under the pointer.
    pub const HOVER: Color = Color::Token(Token::LineHover);
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

    /// The faces a browser build bundles (`use_bundled_fonts`).
    pub const WEB_UI: &str = "Inter";
    pub const WEB_MONO: &str = "JetBrains Mono";

    pub const UI: &str = if cfg!(target_arch = "wasm32") {
        WEB_UI
    } else if cfg!(target_os = "macos") {
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

    pub const MONO: &str = if cfg!(target_arch = "wasm32") {
        WEB_MONO
    } else if cfg!(target_os = "macos") {
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

/// Which layout a window gets, chosen by its width rather than its platform.
pub mod layout {
    /// Below this many logical pixels wide a window lays out in one column.
    pub const COMPACT_BELOW: f32 = 600.0;
    /// The smallest box a finger can hit, in points.
    pub const TOUCH: f32 = 44.0;

    /// Whether a window `width` logical pixels wide gets the compact layout.
    pub fn compact(width: f32) -> bool {
        width < COMPACT_BELOW
    }
}

/// `transition-colors`: Tailwind's default duration and easing.
pub const TRANSITION: Duration = Duration::from_millis(150);

/// Motion beyond hover fades.
pub mod motion {
    use web_time::Duration;
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

/// How floating surfaces blur what is under them when materials are on.
pub mod material {
    /// A backdrop blur and how opaque the surface over it stays.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct Tuning {
        /// Standard deviation of the blur, logical pixels.
        pub sigma: f32,
        /// Alpha of the surface drawn over the blur.
        pub opacity: f32,
    }

    /// Close to the macOS menu material: a light frost, mostly surface.
    pub const APPLE: Tuning = Tuning {
        sigma: 20.0,
        opacity: 0.78,
    };
    /// Close to Acrylic: a wider blur under a thinner tint.
    pub const ACRYLIC: Tuning = Tuning {
        sigma: 30.0,
        opacity: 0.65,
    };

    /// How much of the theme's panel colour lies over the system's sidebar
    /// material: a wash with Native, so the theme still shows, and a tint
    /// with Themed.
    pub const NATIVE_TINT: f32 = 0.3;
    pub const THEMED_TINT: f32 = 0.6;

    /// The sidebar's fill: its panel colour, or over the system's material
    /// the active theme's panel thinned, so switching themes recolours it.
    pub fn sidebar(see_through: bool) -> crate::scene::Rgba {
        use vorn_theme::{Materials, Token};
        let panel = crate::palette::get(Token::SurfacePanel);
        match crate::palette::materials() {
            Materials::Solid => panel,
            _ if !see_through => panel,
            Materials::Native => panel.alpha(NATIVE_TINT),
            Materials::Themed => panel.alpha(THEMED_TINT),
        }
    }

    /// The content area's fill: the base colour, or over the system's
    /// material the base mostly opaque, so the material shows faintly.
    pub const NATIVE_CONTENT: f32 = 0.82;
    pub const THEMED_CONTENT: f32 = 0.9;

    pub fn content(see_through: bool) -> crate::scene::Rgba {
        use vorn_theme::{Materials, Token};
        let base = crate::palette::get(Token::SurfaceBase);
        match crate::palette::materials() {
            Materials::Solid => base,
            _ if !see_through => base,
            Materials::Native => base.alpha(NATIVE_CONTENT),
            Materials::Themed => base.alpha(THEMED_CONTENT),
        }
    }

    /// This platform's tuning.
    pub fn tuning() -> Tuning {
        if cfg!(any(target_os = "macos", target_os = "ios")) {
            APPLE
        } else {
            ACRYLIC
        }
    }
}

/// `:focus-visible` from `global.css`: a 1px outline, 1px out, and the
/// element's corners forced to 4px.
pub mod focus {
    use super::{Color, Token};
    pub const RING: Color = Color::Token(Token::FocusRing);
    pub const WIDTH: f32 = 1.0;
    pub const OFFSET: f32 = 1.0;
    pub const RADIUS: f32 = 4.0;
}

/// `::-webkit-scrollbar` from `global.css`.
pub mod scrollbar {
    use super::{Color, Token};
    pub const WIDTH: f32 = 6.0;
    pub const RADIUS: f32 = 3.0;
    pub const THUMB: Color = Color::Token(Token::ScrollThumb);
    pub const THUMB_HOVER: Color = Color::Token(Token::ScrollThumbHover);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::{self, Palette};
    use crate::scene::Rgba;
    use vorn_theme::{find_bundled, Materials, Token};

    #[test]
    fn switching_themes_recolours_a_see_through_sidebar() {
        let sidebar = |id: &str, m: Materials, see_through: bool| {
            palette::install(Palette::new(find_bundled(id).unwrap().clone(), m));
            material::sidebar(see_through)
        };
        for m in [Materials::Native, Materials::Themed] {
            let (nord, dracula) = (sidebar("nord", m, true), sidebar("dracula", m, true));
            assert_ne!(nord, dracula, "{m:?}");
            assert!(nord.0[3] > 0.0, "{m:?}: the theme shows over the material");
        }
        assert!(
            sidebar("nord", Materials::Native, true).0[3]
                < sidebar("nord", Materials::Themed, true).0[3],
            "Native shows more of the system's material than Themed"
        );
        let panel = |id: &str| Rgba::from(find_bundled(id).unwrap().get(Token::SurfacePanel));
        for m in [Materials::Solid, Materials::Native, Materials::Themed] {
            assert_eq!(sidebar("nord", m, false), panel("nord"), "{m:?}: opaque");
        }
        assert_eq!(sidebar("nord", Materials::Solid, true), panel("nord"));
        palette::install(Palette::default());
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
