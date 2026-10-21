//! Themes for Vorn: the colour [`Token`]s every surface draws with, the
//! data-only `[theme]` section of an extension's `vorn-extension.toml` that
//! sets them and the terminal's palette, the themes Vorn ships
//! ([`bundled`]), how a person's appearance settings pick one ([`Prefs`]),
//! and [`contrast`] correction.
//!
//! A plain crate: vornd checks the section when a theme installs, and the
//! app on every surface (native, the web and the terminal UI) resolves the
//! same [`Theme`] from the same settings.

mod bundled;
mod color;
pub mod contrast;
mod derive;
mod prefs;
mod token;

use std::fmt;

pub use bundled::{bundled, find_bundled, manifests, vorn_dark, vorn_light, VORN_DARK, VORN_LIGHT};
pub use color::{Oklch, Rgba8};
pub use prefs::{Chrome, Materials, Prefs, Resolved, Scheme};
pub use token::Token;

/// A `[theme]` section as written, for clients that receive it as data.
pub type Section = toml::Value;

/// Whether a theme is drawn on a dark window or a light one: the colour
/// scheme setting picks between the person's dark and light theme by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Appearance {
    Dark,
    Light,
}

impl Appearance {
    pub fn key(self) -> &'static str {
        match self {
            Appearance::Dark => "dark",
            Appearance::Light => "light",
        }
    }
}

/// The terminal's palette: the 16 ANSI colours a program names, its
/// default foreground and background, the cursor and the selection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Terminal {
    pub foreground: Rgba8,
    pub background: Rgba8,
    pub cursor: Rgba8,
    pub selection: Rgba8,
    /// Black, red, green, yellow, blue, magenta, cyan, white, then the
    /// bright eight.
    pub ansi: [Rgba8; 16],
}

/// One theme, every token resolved.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Theme {
    /// The extension's id; the settings name a theme by it.
    pub id: String,
    pub name: String,
    pub appearance: Appearance,
    pub terminal: Terminal,
    ui: [Rgba8; Token::COUNT],
}

impl Theme {
    /// The colour of `token`.
    pub fn get(&self, token: Token) -> Rgba8 {
        self.ui[token as usize]
    }

    /// Every token's colour, indexed by `Token as usize`.
    pub fn tokens(&self) -> &[Rgba8; Token::COUNT] {
        &self.ui
    }

    /// Reads a manifest's `[theme]` section for the extension `id` named
    /// `name`. Errors carry the dotted path of the field, from `theme`, for
    /// the manifest reader to turn into a line.
    pub fn from_section(id: &str, name: &str, section: &toml::Value) -> Result<Theme, ThemeError> {
        let table = section
            .as_table()
            .ok_or_else(|| ThemeError::at("theme", "is a table"))?;
        for key in table.keys() {
            if !matches!(key.as_str(), "appearance" | "ui" | "terminal") {
                return Err(ThemeError::at(
                    &format!("theme.{key}"),
                    "is not a theme field; a theme sets appearance, [theme.ui] and [theme.terminal]",
                ));
            }
        }
        let appearance = match table.get("appearance").map(|v| (v, v.as_str())) {
            Some((_, Some("dark"))) => Appearance::Dark,
            Some((_, Some("light"))) => Appearance::Light,
            Some(_) => {
                return Err(ThemeError::at(
                    "theme.appearance",
                    "is \"dark\" or \"light\"",
                ))
            }
            None => {
                return Err(ThemeError::at(
                    "theme.appearance",
                    "is missing; say \"dark\" or \"light\"",
                ))
            }
        };
        let terminal = read_terminal(table.get("terminal").ok_or_else(|| {
            ThemeError::at(
                "theme.terminal",
                "is missing; a theme sets the terminal's palette",
            )
        })?)?;
        let mut given = [None; Token::COUNT];
        if let Some(ui) = table.get("ui") {
            let ui = ui
                .as_table()
                .ok_or_else(|| ThemeError::at("theme.ui", "is a table of token = \"#rrggbb\""))?;
            for (key, value) in ui {
                let field = format!("theme.ui.{key}");
                let token = Token::from_key(key).ok_or_else(|| {
                    ThemeError::at(
                        &field,
                        "is not a token; see the theme tokens in the extension docs",
                    )
                })?;
                given[token as usize] = Some(read_color(&field, value)?);
            }
        }
        Ok(Theme {
            id: id.to_owned(),
            name: name.to_owned(),
            appearance,
            ui: derive::tokens(appearance, &terminal, &given),
            terminal,
        })
    }

    /// The theme with Vorn's own chrome for its appearance (surfaces, ink,
    /// hairlines, grays) and its own meaningful colours and terminal: the
    /// "neutral" interface style.
    pub fn with_neutral_chrome(&self) -> Theme {
        let vorn = match self.appearance {
            Appearance::Dark => vorn_dark(),
            Appearance::Light => vorn_light(),
        };
        let mut out = self.clone();
        for t in Token::ALL.iter().filter(|t| t.is_chrome()) {
            out.ui[*t as usize] = vorn.get(*t);
        }
        out
    }

    /// The theme with every text token and ANSI colour lifted to
    /// [`contrast::MIN_RATIO`] on the surfaces it is drawn on, hues kept.
    pub fn with_contrast(&self) -> Theme {
        let mut out = self.clone();
        let base = self.get(Token::SurfaceBase);
        let sunken = self.get(Token::SurfaceSunken);
        for t in Token::ALL.iter().filter(|t| t.is_text()) {
            let c = contrast::correct(self.get(*t), base, contrast::MIN_RATIO);
            out.ui[*t as usize] = contrast::correct(c, sunken, contrast::MIN_RATIO);
        }
        let bg = self.terminal.background;
        out.terminal.foreground =
            contrast::correct(self.terminal.foreground, bg, contrast::MIN_RATIO);
        for (i, c) in out.terminal.ansi.iter_mut().enumerate() {
            // Black on a dark window and white on a light one are fills.
            let fill = match self.appearance {
                Appearance::Dark => matches!(i, 0 | 8),
                Appearance::Light => matches!(i, 7 | 15),
            };
            if !fill {
                *c = contrast::correct(*c, bg, contrast::MIN_RATIO);
            }
        }
        out
    }
}

/// A `[theme]` section that does not read, and which field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeError {
    /// The dotted path from `theme`, as `theme.terminal.ansi[3]`.
    pub field: String,
    pub message: String,
}

impl ThemeError {
    fn at(field: &str, message: &str) -> ThemeError {
        ThemeError {
            field: field.to_owned(),
            message: message.to_owned(),
        }
    }
}

impl fmt::Display for ThemeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

impl std::error::Error for ThemeError {}

fn read_color(field: &str, value: &toml::Value) -> Result<Rgba8, ThemeError> {
    value
        .as_str()
        .and_then(Rgba8::parse)
        .ok_or_else(|| ThemeError::at(field, "is a colour as \"#rrggbb\" or \"#rrggbbaa\""))
}

fn read_terminal(value: &toml::Value) -> Result<Terminal, ThemeError> {
    let table = value
        .as_table()
        .ok_or_else(|| ThemeError::at("theme.terminal", "is a table"))?;
    for key in table.keys() {
        if !matches!(
            key.as_str(),
            "foreground" | "background" | "cursor" | "selection" | "ansi"
        ) {
            return Err(ThemeError::at(
                &format!("theme.terminal.{key}"),
                "is not a terminal colour; a theme sets foreground, background, cursor, selection and ansi",
            ));
        }
    }
    let get = |key: &str| -> Result<Option<Rgba8>, ThemeError> {
        table
            .get(key)
            .map(|v| read_color(&format!("theme.terminal.{key}"), v))
            .transpose()
    };
    let need = |key: &str| -> Result<Rgba8, ThemeError> {
        get(key)?.ok_or_else(|| ThemeError::at(&format!("theme.terminal.{key}"), "is missing"))
    };
    let foreground = need("foreground")?;
    let background = need("background")?;
    let ansi = table
        .get("ansi")
        .ok_or_else(|| {
            ThemeError::at(
                "theme.terminal.ansi",
                "is missing; list the 16 ANSI colours",
            )
        })?
        .as_array()
        .ok_or_else(|| ThemeError::at("theme.terminal.ansi", "is a list of 16 colours"))?;
    if ansi.len() != 16 {
        return Err(ThemeError::at(
            "theme.terminal.ansi",
            &format!("has {} colours; a terminal names 16", ansi.len()),
        ));
    }
    let mut colors = [Rgba8::BLACK; 16];
    for (i, v) in ansi.iter().enumerate() {
        colors[i] = read_color(&format!("theme.terminal.ansi[{i}]"), v)?;
    }
    Ok(Terminal {
        cursor: get("cursor")?.unwrap_or(foreground),
        selection: get("selection")?.unwrap_or_else(|| background.mix(foreground, 0.25)),
        foreground,
        background,
        ansi: colors,
    })
}

/// Reads a whole manifest's text for its `[theme]`, as the themes Vorn
/// ships are read: their id and name are the manifest's.
pub fn from_manifest(text: &str) -> Result<Option<Theme>, ThemeError> {
    let table: toml::Table = toml::from_str(text).map_err(|e| ThemeError {
        field: String::new(),
        message: e.message().trim_end().to_owned(),
    })?;
    let Some(section) = table.get("theme") else {
        return Ok(None);
    };
    let text_of = |key: &str| {
        table
            .get(key)
            .and_then(toml::Value::as_str)
            .unwrap_or_default()
    };
    Theme::from_section(text_of("id"), text_of("name"), section).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r##"
id = "acme-dusk"
name = "Acme Dusk"
version = "1.0.0"
api = "vorn:extension@1.0"

[theme]
appearance = "dark"

[theme.terminal]
foreground = "#dddddd"
background = "#111111"
ansi = ["#000000", "#cc3333", "#33cc33", "#cccc33", "#3333cc", "#cc33cc", "#33cccc", "#cccccc",
        "#555555", "#ff5555", "#55ff55", "#ffff55", "#5555ff", "#ff55ff", "#55ffff", "#ffffff"]
"##;

    fn err(text: &str) -> ThemeError {
        from_manifest(text).expect_err("should not read")
    }

    #[test]
    fn a_minimal_theme_derives_every_token() {
        let theme = from_manifest(MINIMAL).unwrap().unwrap();
        assert_eq!(
            (theme.id.as_str(), theme.name.as_str()),
            ("acme-dusk", "Acme Dusk")
        );
        assert_eq!(theme.get(Token::SurfaceBase), Rgba8::rgb(0x111111));
        assert_eq!(theme.get(Token::Ink), Rgba8::rgb(0xdddddd));
        assert_eq!(theme.get(Token::Danger), Rgba8::rgb(0xcc3333));
        assert_eq!(theme.terminal.cursor, Rgba8::rgb(0xdddddd));
        // Surfaces step towards the ink, as Vorn's do.
        let l = |t| theme.get(t).luminance();
        assert!(l(Token::SurfaceBase) < l(Token::SurfaceSunken));
        assert!(l(Token::SurfaceSunken) < l(Token::SurfaceOverlay));
    }

    #[test]
    fn names_the_field_of_each_mistake() {
        let cases = [
            (
                MINIMAL.replace("appearance = \"dark\"", "appearance = \"dim\""),
                "theme.appearance",
            ),
            (
                MINIMAL.replace("appearance = \"dark\"\n", ""),
                "theme.appearance",
            ),
            (
                MINIMAL.replace("\"#cc3333\"", "\"red\""),
                "theme.terminal.ansi[1]",
            ),
            (
                MINIMAL.replace(", \"#ffffff\"]", "]"),
                "theme.terminal.ansi",
            ),
            (MINIMAL.replace("foreground", "fg"), "theme.terminal.fg"),
            (
                MINIMAL.replace(
                    "[theme.terminal]",
                    "[theme.ui]\nsurface = \"#000000\"\n[theme.terminal]",
                ),
                "theme.ui.surface",
            ),
            (
                MINIMAL.replace(
                    "[theme.terminal]",
                    "[theme.ui]\naccent = \"#12\"\n[theme.terminal]",
                ),
                "theme.ui.accent",
            ),
            (
                MINIMAL.replace("appearance", "mood = 1\nappearance"),
                "theme.mood",
            ),
        ];
        for (text, field) in cases {
            assert_eq!(err(&text).field, field, "{text}");
        }
    }

    #[test]
    fn neutral_chrome_keeps_the_theme_s_meaning() {
        let theme = from_manifest(MINIMAL).unwrap().unwrap();
        let neutral = theme.with_neutral_chrome();
        let dark = find_bundled(VORN_DARK).unwrap();
        assert_eq!(
            neutral.get(Token::SurfaceBase),
            dark.get(Token::SurfaceBase)
        );
        assert_eq!(neutral.get(Token::Danger), theme.get(Token::Danger));
        assert_eq!(neutral.terminal, theme.terminal);
    }

    #[test]
    fn contrast_lifts_every_text_token_and_ansi_colour() {
        for theme in bundled() {
            let fixed = theme.with_contrast();
            let base = fixed.get(Token::SurfaceBase);
            for t in Token::ALL.iter().filter(|t| t.is_text()) {
                let c = fixed.get(*t).over(base);
                assert!(
                    contrast::ratio(c, base) >= contrast::MIN_RATIO - 0.01,
                    "{} {}",
                    theme.id,
                    t.key()
                );
            }
            let bg = fixed.terminal.background;
            assert!(contrast::ratio(fixed.terminal.foreground, bg) >= contrast::MIN_RATIO - 0.01);
        }
    }
}
