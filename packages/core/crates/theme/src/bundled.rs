//! The themes Vorn ships: its own dark and light first, then a small
//! reviewed set of popular themes under licences that allow it (each file
//! names its licence; `THIRD_PARTY_NOTICES.md` carries the texts). They are
//! ordinary `vorn-extension.toml` files, read by the same reader as any
//! installed theme.

use std::sync::OnceLock;

use crate::Theme;

/// Vorn's own themes' ids: today's look, and its light counterpart.
pub const VORN_DARK: &str = "vorn-dark";
pub const VORN_LIGHT: &str = "vorn-light";

/// The manifests, Vorn's first.
const FILES: [&str; 8] = [
    include_str!("../themes/vorn-dark.toml"),
    include_str!("../themes/vorn-light.toml"),
    include_str!("../themes/solarized-dark.toml"),
    include_str!("../themes/solarized-light.toml"),
    include_str!("../themes/catppuccin-mocha.toml"),
    include_str!("../themes/catppuccin-latte.toml"),
    include_str!("../themes/nord.toml"),
    include_str!("../themes/dracula.toml"),
];

/// The themes Vorn ships, in the order the settings show them.
pub fn bundled() -> &'static [Theme] {
    static THEMES: OnceLock<Vec<Theme>> = OnceLock::new();
    THEMES.get_or_init(|| {
        FILES
            .iter()
            // The tests read every file; one that does not read is left out, never fatal.
            .filter_map(|text| crate::from_manifest(text).ok().flatten())
            .collect()
    })
}

/// The manifests' text, for the extension reader's own checks.
pub fn manifests() -> &'static [&'static str] {
    &FILES
}

/// The shipped theme with `id`.
pub fn find_bundled(id: &str) -> Option<&'static Theme> {
    bundled().iter().find(|t| t.id == id)
}

/// Vorn Dark: the default, and what a missing theme falls back to.
pub fn vorn_dark() -> &'static Theme {
    #[allow(clippy::expect_used)]
    find_bundled(VORN_DARK).expect("vorn-dark.toml is compiled in and its test reads it")
}

/// Vorn Light, the light default.
pub fn vorn_light() -> &'static Theme {
    #[allow(clippy::expect_used)]
    find_bundled(VORN_LIGHT).expect("vorn-light.toml is compiled in and its test reads it")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Appearance, Rgba8, Token};

    #[test]
    fn every_file_reads() {
        for text in FILES {
            crate::from_manifest(text).unwrap().unwrap();
        }
        assert_eq!(bundled().len(), FILES.len());
        assert_eq!(bundled()[0].id, VORN_DARK);
        assert_eq!(bundled()[1].id, VORN_LIGHT);
    }

    #[test]
    fn every_file_names_its_licence() {
        for text in FILES {
            assert!(text.lines().any(|l| l.starts_with("# Licence: ")), "{text}");
        }
    }

    #[test]
    fn each_appearance_has_vorn_s_own_and_another() {
        for a in [Appearance::Dark, Appearance::Light] {
            assert!(bundled().iter().filter(|t| t.appearance == a).count() >= 2);
        }
    }

    /// Vorn Dark is today's look, value for value.
    #[test]
    fn vorn_dark_is_today_s_palette() {
        let t = find_bundled(VORN_DARK).unwrap();
        let rgb = |c| Rgba8::rgb(c);
        assert_eq!(t.get(Token::SurfaceBase), rgb(0x0d0d0f));
        assert_eq!(t.get(Token::SurfacePanel), rgb(0x101012));
        assert_eq!(t.get(Token::SurfaceSunken), rgb(0x141416));
        assert_eq!(t.get(Token::SurfaceOverlay), rgb(0x1c1c20));
        assert_eq!(t.get(Token::Accent), rgb(0xc9972a));
        assert_eq!(t.get(Token::Gray400), rgb(0x99a1af));
        assert_eq!(t.get(Token::SliderTrack), rgb(0xefefef));
        assert_eq!(t.get(Token::InkSecondary), Rgba8::WHITE.with_alpha(0.55));
        assert_eq!(t.terminal.background, rgb(0x141416));
        assert_eq!(t.terminal.ansi[1], rgb(0xef4444));
    }
}
