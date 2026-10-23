//! Settings › Appearance applied: the stored choices, the system's light or
//! dark and its transparency setting, and the themes extensions bring,
//! resolved to the palette this thread draws with.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use vorn_theme::{Chrome, Materials, Prefs, Scheme, Section, Theme};
use vornui::palette::{self, Palette};

/// The `defaults` keys Appearance stores.
pub const SCHEME: &str = "colorScheme";
pub const LIGHT: &str = "lightTheme";
pub const DARK: &str = "darkTheme";
pub const CHROME: &str = "interfaceStyle";
pub const CONTRAST: &str = "contrastCorrection";
pub const MATERIALS: &str = "materials";

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
    Theme::from_section(id, name, &section).ok()
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
            "kind": "themes", "id": "acme", "title": "Acme",
            "theme": { "appearance": "dark", "terminal": {
                "foreground": "#eeeeee", "background": "#101010",
                "ansi": ["#000000", "#cc0000", "#00cc00", "#cccc00", "#0000cc", "#cc00cc", "#00cccc", "#cccccc",
                         "#555555", "#ff5555", "#55ff55", "#ffff55", "#5555ff", "#ff55ff", "#55ffff", "#ffffff"] } }
        });
        let t = listed_theme(&entry).expect("reads");
        assert_eq!((t.id.as_str(), t.name.as_str()), ("acme", "Acme"));
        assert_eq!(t.terminal.background.to_u32(), 0x101010);
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
        assert_eq!(launch_materials(&f), Materials::Solid);
        let mut a = Appearance::default();
        a.keep_materials(None, &f);
        assert!(!f.exists(), "nothing is kept before the settings load");
        a.keep_materials(Some(&json!({ "materials": "native" })), &f);
        assert_eq!(launch_materials(&f), Materials::Native);
        a.keep_materials(Some(&json!({})), &f);
        assert_eq!(launch_materials(&f), Materials::Solid);
        std::fs::write(&f, "glass").unwrap();
        assert_eq!(launch_materials(&f), Materials::Solid);
        let _ = std::fs::remove_dir_all(dir);
    }
}
