//! A person's appearance settings and the one theme they resolve to on a
//! window: the colour scheme picks the light or the dark theme, the
//! interface style keeps the theme's chrome or Vorn's, and contrast
//! correction lifts what does not read.

use crate::{bundled, vorn_dark, vorn_light, Theme, VORN_DARK, VORN_LIGHT};

/// Settings › Appearance › Colour scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scheme {
    /// Follows the system's light or dark appearance.
    System,
    Light,
    /// Today's look, so the default.
    #[default]
    Dark,
}

/// Settings › Appearance › Interface style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Chrome {
    /// Vorn's own surfaces and ink; the theme colours the terminal and what
    /// means something.
    #[default]
    Neutral,
    /// The theme's own surfaces too.
    Tinted,
}

/// Settings › Appearance › Materials.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Materials {
    /// Opaque surfaces everywhere.
    #[default]
    Solid,
    /// The system's translucent sidebar, and menus, popovers and the palette
    /// over a blur of what is behind them.
    Native,
    /// As native, the translucency tinted with the theme's surfaces.
    Themed,
}

macro_rules! keyed {
    ($ty:ident { $($v:ident = $k:literal),* }) => {
        impl $ty {
            /// The value the settings store.
            pub fn key(self) -> &'static str {
                match self { $($ty::$v => $k,)* }
            }

            pub fn from_key(key: &str) -> Option<$ty> {
                match key { $($k => Some($ty::$v),)* _ => None }
            }
        }
    };
}

keyed!(Scheme { System = "system", Light = "light", Dark = "dark" });
keyed!(Chrome { Neutral = "neutral", Tinted = "tinted" });
keyed!(Materials { Solid = "solid", Native = "native", Themed = "themed" });

/// Everything Settings › Appearance stores about colour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefs {
    pub scheme: Scheme,
    /// The theme ids drawn when the scheme is light and when it is dark.
    pub light: String,
    pub dark: String,
    pub chrome: Chrome,
    pub contrast: bool,
    pub materials: Materials,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs {
            scheme: Scheme::default(),
            light: VORN_LIGHT.to_owned(),
            dark: VORN_DARK.to_owned(),
            chrome: Chrome::default(),
            contrast: false,
            materials: Materials::default(),
        }
    }
}

/// What a window draws with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub theme: Theme,
    /// [`Materials::Solid`] whenever the system asks for less transparency.
    pub materials: Materials,
}

impl Prefs {
    /// Whether the window is dark under these settings.
    pub fn is_dark(&self, system_dark: bool) -> bool {
        match self.scheme {
            Scheme::System => system_dark,
            Scheme::Light => false,
            Scheme::Dark => true,
        }
    }

    /// The id of the theme the scheme picks.
    pub fn chosen(&self, system_dark: bool) -> &str {
        if self.is_dark(system_dark) {
            &self.dark
        } else {
            &self.light
        }
    }

    /// The theme a window draws: `preview` (a tile under the pointer) or
    /// the chosen one, found among Vorn's then `installed` (so an extension
    /// can never replace a bundled theme), falling back to Vorn's own for
    /// the scheme when the id is gone.
    pub fn resolve(
        &self,
        system_dark: bool,
        reduce_transparency: bool,
        installed: &[Theme],
        preview: Option<&str>,
    ) -> Resolved {
        let id = preview.unwrap_or_else(|| self.chosen(system_dark));
        let fallback = if self.is_dark(system_dark) {
            vorn_dark()
        } else {
            vorn_light()
        };
        let base = bundled()
            .iter()
            .chain(installed)
            .find(|t| t.id == id)
            .unwrap_or(fallback)
            .clone();
        let theme = match self.chrome {
            Chrome::Neutral => base.with_neutral_chrome(),
            Chrome::Tinted => base,
        };
        let theme = if self.contrast {
            theme.with_contrast()
        } else {
            theme
        };
        Resolved {
            theme,
            materials: if reduce_transparency {
                Materials::Solid
            } else {
                self.materials
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{find_bundled, Token};

    #[test]
    fn defaults_draw_today_s_look() {
        let r = Prefs::default().resolve(false, false, &[], None);
        assert_eq!(&r.theme, find_bundled(VORN_DARK).unwrap());
        assert_eq!(r.materials, Materials::Solid);
    }

    #[test]
    fn system_scheme_follows_the_system() {
        let prefs = Prefs {
            scheme: Scheme::System,
            ..Prefs::default()
        };
        assert_eq!(prefs.resolve(true, false, &[], None).theme.id, VORN_DARK);
        assert_eq!(prefs.resolve(false, false, &[], None).theme.id, VORN_LIGHT);
    }

    #[test]
    fn a_missing_theme_falls_back_to_vorn_s_for_the_scheme() {
        let prefs = Prefs {
            scheme: Scheme::Light,
            light: "gone".into(),
            ..Prefs::default()
        };
        assert_eq!(prefs.resolve(true, false, &[], None).theme.id, VORN_LIGHT);
    }

    #[test]
    fn tinted_keeps_the_theme_s_surfaces_and_neutral_does_not() {
        let mut prefs = Prefs {
            dark: "nord".into(),
            ..Prefs::default()
        };
        let nord = find_bundled("nord").unwrap();
        let vorn = find_bundled(VORN_DARK).unwrap();
        let neutral = prefs.resolve(true, false, &[], None).theme;
        assert_eq!(
            neutral.get(Token::SurfaceBase),
            vorn.get(Token::SurfaceBase)
        );
        assert_eq!(neutral.terminal, nord.terminal);
        prefs.chrome = Chrome::Tinted;
        let tinted = prefs.resolve(true, false, &[], None).theme;
        assert_eq!(tinted.get(Token::SurfaceBase), nord.get(Token::SurfaceBase));
    }

    #[test]
    fn reduce_transparency_makes_materials_solid() {
        let prefs = Prefs {
            materials: Materials::Native,
            ..Prefs::default()
        };
        assert_eq!(
            prefs.resolve(true, false, &[], None).materials,
            Materials::Native
        );
        assert_eq!(
            prefs.resolve(true, true, &[], None).materials,
            Materials::Solid
        );
    }

    #[test]
    fn an_installed_theme_never_replaces_a_bundled_one() {
        let mut impostor = find_bundled("nord").unwrap().clone();
        impostor.id = VORN_DARK.into();
        let r = Prefs::default().resolve(true, false, &[impostor], None);
        assert_eq!(&r.theme, find_bundled(VORN_DARK).unwrap());
    }

    #[test]
    fn a_preview_wins_over_the_choice() {
        let r = Prefs::default().resolve(true, false, &[], Some("dracula"));
        assert_eq!(r.theme.id, "dracula");
    }

    #[test]
    fn keys_round_trip() {
        for s in [Scheme::System, Scheme::Light, Scheme::Dark] {
            assert_eq!(Scheme::from_key(s.key()), Some(s));
        }
        for m in [Materials::Solid, Materials::Native, Materials::Themed] {
            assert_eq!(Materials::from_key(m.key()), Some(m));
        }
        assert_eq!(Chrome::from_key("tinted"), Some(Chrome::Tinted));
        assert_eq!(Chrome::from_key("loud"), None);
    }
}
