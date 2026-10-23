//! Settings › Appearance applied: the stored choices, the system's light or
//! dark and its transparency setting, and the themes extensions bring,
//! resolved to the palette this thread draws with.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use vorn_theme::{Appearance as Look, Chrome, Materials, Prefs, Scheme, Section, Theme};
use vornui::palette::{self, Palette};

/// The `defaults` keys Appearance stores.
pub const SCHEME: &str = "colorScheme";
pub const LIGHT: &str = "lightTheme";
pub const DARK: &str = "darkTheme";
pub const CHROME: &str = "interfaceStyle";
pub const CONTRAST: &str = "contrastCorrection";
pub const MATERIALS: &str = "materials";
/// The terminal's font family; the bundled monospace face when unset.
pub const FONT_FAMILY: &str = "terminalFontFamily";

/// One theme row of Settings › Appearance: its label, the side of the
/// scheme it picks for, and the key that choice is stored under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeRow {
    pub label: &'static str,
    pub look: Look,
    pub key: &'static str,
}

const LIGHT_ROW: ThemeRow = ThemeRow {
    label: "Light Theme",
    look: Look::Light,
    key: LIGHT,
};
const DARK_ROW: ThemeRow = ThemeRow {
    label: "Dark Theme",
    look: Look::Dark,
    key: DARK,
};

/// The theme rows `scheme` shows: one Theme row for a fixed scheme, a
/// light and a dark one when it follows the system.
pub fn theme_rows(scheme: Scheme) -> &'static [ThemeRow] {
    match scheme {
        Scheme::System => &[LIGHT_ROW, DARK_ROW],
        Scheme::Light => &[ThemeRow {
            label: "Theme",
            ..LIGHT_ROW
        }],
        Scheme::Dark => &[ThemeRow {
            label: "Theme",
            ..DARK_ROW
        }],
    }
}

/// What picking theme `id`, drawn on a `look` window, writes: the theme
/// under its side's key, and under a fixed scheme of the other side the
/// scheme too, so the one Theme row shows the theme picked.
pub fn pick_theme(scheme: Scheme, id: &str, look: Look) -> Vec<(&'static str, Value)> {
    let (key, side) = match look {
        Look::Light => (LIGHT, Scheme::Light),
        Look::Dark => (DARK, Scheme::Dark),
    };
    let mut out = vec![(key, Value::from(id))];
    if scheme != Scheme::System && scheme != side {
        out.push((SCHEME, Value::from(side.key())));
    }
    out
}

/// The theme picker's filter chips.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThemeFilter {
    Dark,
    Light,
    /// The themes extensions brought.
    Installed,
}

impl ThemeFilter {
    pub const ALL: [ThemeFilter; 3] = [ThemeFilter::Dark, ThemeFilter::Light, ThemeFilter::Installed];

    pub fn label(self) -> &'static str {
        match self {
            ThemeFilter::Dark => "Dark",
            ThemeFilter::Light => "Light",
            ThemeFilter::Installed => "Installed",
        }
    }

    /// The chip a picker for a `look` row opens on.
    pub fn of(look: Look) -> ThemeFilter {
        match look {
            Look::Dark => ThemeFilter::Dark,
            Look::Light => ThemeFilter::Light,
        }
    }
}

/// The themes the picker lists: Vorn's, then the installed, kept when they
/// pass `filter` and their name holds every word of `query`.
pub fn picker_themes<'a>(
    installed: &'a [Theme],
    filter: Option<ThemeFilter>,
    query: &str,
) -> Vec<&'a Theme> {
    let q = query.to_lowercase();
    let words: Vec<&str> = q.split_whitespace().collect();
    vorn_theme::bundled()
        .iter()
        .map(|t| (t, false))
        .chain(installed.iter().map(|t| (t, true)))
        .filter(|&(t, extra)| match filter {
            None => true,
            Some(ThemeFilter::Dark) => t.appearance == Look::Dark,
            Some(ThemeFilter::Light) => t.appearance == Look::Light,
            Some(ThemeFilter::Installed) => extra,
        })
        .filter(|(t, _)| {
            let name = t.name.to_lowercase();
            words.iter().all(|w| name.contains(w))
        })
        .map(|(t, _)| t)
        .collect()
}

/// A theme by id, Vorn's before the installed.
pub fn find<'a>(installed: &'a [Theme], id: &str) -> Option<&'a Theme> {
    vorn_theme::bundled()
        .iter()
        .chain(installed)
        .find(|t| t.id == id)
}

/// The appearance settings stored in `defaults`; the default for any value
/// missing or unknown.
pub fn prefs(defaults: Option<&Value>) -> Prefs {
    let mut p = Prefs::default();
    let Some(d) = defaults else {
        return p;
    };
    let text = |k: &str| d.get(k).and_then(Value::as_str);
    if let Some(v) = text(SCHEME).and_then(Scheme::from_key) {
        p.scheme = v;
    }
    if let Some(v) = text(LIGHT) {
        v.clone_into(&mut p.light);
    }
    if let Some(v) = text(DARK) {
        v.clone_into(&mut p.dark);
    }
    if let Some(v) = text(CHROME).and_then(Chrome::from_key) {
        p.chrome = v;
    }
    if let Some(v) = text(MATERIALS).and_then(Materials::from_key) {
        p.materials = v;
    }
    p.contrast = d.get(CONTRAST).and_then(Value::as_bool) == Some(true);
    p
}

/// Where the Materials choice is kept for the next launch: the window builds
/// its renderer from it before vornd has answered.
pub fn launch_file(data_dir: &Path) -> PathBuf {
    data_dir.join("desktop").join("materials")
}

/// The Materials choice the last launch kept in `file`; Solid when none.
pub fn launch_materials(file: &Path) -> Materials {
    crate::keep::read(file)
        .and_then(|s| Materials::from_key(s.trim()))
        .unwrap_or_default()
}

/// A theme an extension contributes, from its `extension:contributions`
/// entry of kind `themes`; `None` when it does not read.
pub fn listed_theme(entry: &Value) -> Option<Theme> {
    let id = entry.get("id")?.as_str()?;
    let name = entry.get("title").and_then(Value::as_str).unwrap_or(id);
    let section = Section::deserialize(entry.get("theme")?).ok()?;
    let mut theme = Theme::from_section(id, name, &section).ok()?;
    let text = |k: &str| entry.get(k).and_then(Value::as_str).unwrap_or_default();
    text("publisher").clone_into(&mut theme.publisher);
    text("version").clone_into(&mut theme.version);
    Some(theme)
}

/// Everything the palette depends on, so it is resolved again only when one changes.
#[derive(Debug, Clone, PartialEq)]
struct Inputs {
    prefs: Prefs,
    system_dark: bool,
    reduced_transparency: bool,
    installed: u64,
    preview: Option<String>,
}

/// The themes extensions bring and what the palette was last resolved from.
#[derive(Debug, Default)]
pub struct Appearance {
    installed: Vec<Theme>,
    /// Counts changes to `installed`.
    rev: u64,
    last: Option<Inputs>,
    /// The Materials choice last written to the launch file.
    kept: Option<Materials>,
}

impl Appearance {
    /// The installed themes, as the settings list them after Vorn's own.
    pub fn installed(&self) -> &[Theme] {
        &self.installed
    }

    /// Takes the themes from a fresh contributions list.
    pub fn listed(&mut self, list: &[Value]) {
        let themes: Vec<Theme> = list
            .iter()
            .filter(|v| v.get("kind").and_then(Value::as_str) == Some("themes"))
            .filter_map(listed_theme)
            .collect();
        if themes != self.installed {
            self.installed = themes;
            self.rev += 1;
        }
    }

    /// Writes the stored Materials choice to `file` when it changed, for the
    /// next launch; nothing before the settings have loaded.
    pub fn keep_materials(&mut self, defaults: Option<&Value>, file: &Path) {
        let Some(d) = defaults else {
            return;
        };
        let m = d
            .get(MATERIALS)
            .and_then(Value::as_str)
            .and_then(Materials::from_key)
            .unwrap_or_default();
        if self.kept != Some(m) && crate::keep::write(file, m.key()).is_ok() {
            self.kept = Some(m);
        }
    }

    /// Installs the palette these inputs resolve to; `true` when the
    /// colours changed, so cached drawings must be made again.
    pub fn apply(
        &mut self,
        defaults: Option<&Value>,
        system_dark: bool,
        reduced_transparency: bool,
        preview: Option<&str>,
    ) -> bool {
        let inputs = Inputs {
            prefs: prefs(defaults),
            system_dark,
            reduced_transparency,
            installed: self.rev,
            preview: preview.map(str::to_owned),
        };
        if self.last.as_ref() == Some(&inputs) {
            return false;
        }
        let r = inputs.prefs.resolve(
            system_dark,
            reduced_transparency,
            &self.installed,
            inputs.preview.as_deref(),
        );
        self.last = Some(inputs);
        palette::install(Palette::new(r.theme, r.materials))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use vorn_theme::Token;

    #[test]
    fn a_fixed_scheme_shows_one_theme_row_and_system_shows_both() {
        let rows = |s| -> Vec<(&str, Look, &str)> {
            theme_rows(s).iter().map(|r| (r.label, r.look, r.key)).collect()
        };
        assert_eq!(rows(Scheme::Dark), [("Theme", Look::Dark, DARK)]);
        assert_eq!(rows(Scheme::Light), [("Theme", Look::Light, LIGHT)]);
        assert_eq!(
            rows(Scheme::System),
            [("Light Theme", Look::Light, LIGHT), ("Dark Theme", Look::Dark, DARK)]
        );
    }

    #[test]
    fn picking_a_theme_of_the_other_side_moves_a_fixed_scheme_to_it() {
        assert_eq!(
            pick_theme(Scheme::Dark, "nord", Look::Dark),
            [(DARK, json!("nord"))]
        );
        assert_eq!(
            pick_theme(Scheme::Dark, "solarized-light", Look::Light),
            [(LIGHT, json!("solarized-light")), (SCHEME, json!("light"))]
        );
        assert_eq!(
            pick_theme(Scheme::System, "solarized-light", Look::Light),
            [(LIGHT, json!("solarized-light"))]
        );
    }

    #[test]
    fn the_picker_filters_by_side_origin_and_name() {
        let acme = listed_theme(&json!({
            "kind": "themes", "id": "acme", "title": "Acme Night",
            "theme": { "appearance": "dark", "terminal": {
                "foreground": "#eeeeee", "background": "#101010",
                "ansi": ["#000000", "#cc0000", "#00cc00", "#cccc00", "#0000cc", "#cc00cc", "#00cccc", "#cccccc",
                         "#555555", "#ff5555", "#55ff55", "#ffff55", "#5555ff", "#ff55ff", "#55ffff", "#ffffff"] } }
        }))
        .unwrap();
        let installed = [acme];
        let ids = |f, q| -> Vec<String> {
            picker_themes(&installed, f, q).iter().map(|t| t.id.clone()).collect()
        };
        let all = ids(None, "");
        assert_eq!(all.len(), vorn_theme::bundled().len() + 1);
        assert_eq!(ids(Some(ThemeFilter::Installed), ""), ["acme"]);
        assert!(ids(Some(ThemeFilter::Light), "")
            .iter()
            .all(|id| find(&installed, id).unwrap().appearance == Look::Light));
        assert!(ids(Some(ThemeFilter::Dark), "").contains(&"acme".to_owned()));
        assert_eq!(ids(None, "NIGHT acme"), ["acme"]);
        assert!(ids(Some(ThemeFilter::Light), "acme").is_empty());
    }

    #[test]
    fn stored_choices_are_read() {
        let d = json!({ "colorScheme": "light", "darkTheme": "nord", "materials": "native" });
        let p = prefs(Some(&d));
        assert_eq!(p.scheme, Scheme::Light);
        assert_eq!(p.dark, "nord");
        assert_eq!(p.materials, Materials::Native);
        assert_eq!(prefs(None), Prefs::default());
    }

    #[test]
    fn unknown_values_keep_the_default() {
        let d = json!({ "colorScheme": "sepia", "interfaceStyle": 3 });
        assert_eq!(prefs(Some(&d)), Prefs::default());
    }

    #[test]
    fn a_listed_theme_reads_as_vornd_sent_it() {
        let entry = json!({
            "kind": "themes", "id": "acme", "title": "Acme", "publisher": "Acme Co", "version": "2.1.0",
            "theme": { "appearance": "dark", "terminal": {
                "foreground": "#eeeeee", "background": "#101010",
                "ansi": ["#000000", "#cc0000", "#00cc00", "#cccc00", "#0000cc", "#cc00cc", "#00cccc", "#cccccc",
                         "#555555", "#ff5555", "#55ff55", "#ffff55", "#5555ff", "#ff55ff", "#55ffff", "#ffffff"] } }
        });
        let t = listed_theme(&entry).expect("reads");
        assert_eq!((t.id.as_str(), t.name.as_str()), ("acme", "Acme"));
        assert_eq!(t.terminal.background.to_u32(), 0x101010);
        assert_eq!((t.publisher.as_str(), t.version.as_str()), ("Acme Co", "2.1.0"));
    }

    #[test]
    fn changing_a_setting_installs_a_new_palette_once() {
        let mut a = Appearance::default();
        let d = json!({ "darkTheme": "dracula", "interfaceStyle": "tinted" });
        assert!(a.apply(Some(&d), true, false, None));
        assert!(!a.apply(Some(&d), true, false, None));
        let dracula = vorn_theme::find_bundled("dracula").unwrap();
        assert_eq!(
            palette::get(Token::SurfaceBase),
            dracula.get(Token::SurfaceBase).into()
        );
        // The system going light does not matter to a Dark scheme.
        assert!(!a.apply(Some(&d), false, false, None));
        assert!(a.apply(Some(&d), true, false, Some("nord")));
    }

    #[test]
    fn switching_the_theme_recolours_the_native_sidebar() {
        use crate::ui::theme::material;
        let mut a = Appearance::default();
        let with = |theme: &str| json!({ "darkTheme": theme, "interfaceStyle": "tinted", "materials": "native" });
        a.apply(Some(&with("nord")), true, false, None);
        let nord = material::sidebar(true);
        a.apply(Some(&with("dracula")), true, false, None);
        let dracula = material::sidebar(true);
        assert_ne!(nord, dracula);
        let panel = vorn_theme::find_bundled("dracula")
            .unwrap()
            .get(Token::SurfacePanel);
        assert_eq!(
            dracula,
            crate::ui::Rgba::from(panel).alpha(material::NATIVE_TINT)
        );
        palette::install(Palette::default());
    }

    #[test]
    fn reduce_transparency_keeps_materials_solid() {
        let mut a = Appearance::default();
        let d = json!({ "materials": "native" });
        a.apply(Some(&d), true, true, None);
        assert_eq!(palette::materials(), Materials::Solid);
        a.apply(Some(&d), true, false, None);
        assert_eq!(palette::materials(), Materials::Native);
    }

    #[test]
    fn the_next_launch_reads_the_last_stored_materials() {
        let dir = std::env::temp_dir().join(format!("vorn-materials-{}", std::process::id()));
        let f = launch_file(&dir);
        assert_eq!(launch_materials(&f), Materials::Native);
        let mut a = Appearance::default();
        a.keep_materials(None, &f);
        assert!(!f.exists(), "nothing is kept before the settings load");
        a.keep_materials(Some(&json!({ "materials": "native" })), &f);
        assert_eq!(launch_materials(&f), Materials::Native);
        a.keep_materials(Some(&json!({ "materials": "solid" })), &f);
        assert_eq!(launch_materials(&f), Materials::Solid);
        a.keep_materials(Some(&json!({})), &f);
        assert_eq!(launch_materials(&f), Materials::Native);
        std::fs::write(&f, "glass").unwrap();
        assert_eq!(launch_materials(&f), Materials::Native);
        let _ = std::fs::remove_dir_all(dir);
    }
}
