//! The Seti glyph and colour the app gives a file by its
//! name, drawn from the glyph's outline rather than from the icon font.

mod tables;

use tables::{DEFS, EXT, GLYPHS, LANG_EXT, NAMES};

fn lookup<'a>(table: &[(&str, &'a str)], key: &str) -> Option<&'a str> {
    table
        .binary_search_by(|(k, _)| (*k).cmp(key))
        .ok()
        .map(|i| table[i].1)
}

/// An icon by its name in `D`: its glyph and colour.
fn def(name: &str) -> Option<(u32, u32)> {
    DEFS.binary_search_by(|(k, _, _)| (*k).cmp(name))
        .ok()
        .map(|i| (DEFS[i].1, DEFS[i].2))
}

const DEFAULT: &str = "_default";

/// The whole name, then a compound extension, then the last
/// one, then dotfiles as config. Returns the glyph's code point and colour.
pub fn resolve(name: &str) -> (u32, u32) {
    let lower = name.to_lowercase();
    let named = |icon: &str| {
        def(icon)
            .or_else(|| def(DEFAULT))
            // literal-colour: the default file mark's own colour.
            .unwrap_or((0xE023, 0xd4d7d6))
    };
    if let Some(icon) = lookup(NAMES, &lower) {
        return named(icon);
    }
    if let Some(first) = lower.find('.') {
        let after = &lower[first + 1..];
        if let Some(icon) = lookup(EXT, after) {
            return named(icon);
        }
        if let Some(last) = lower.rfind('.').filter(|&l| l != first) {
            if let Some(prev) = lower[..last].rfind('.') {
                if let Some(icon) = lookup(EXT, &lower[prev + 1..]) {
                    return named(icon);
                }
            }
        }
    }
    if let Some((_, ext)) = lower.rsplit_once('.') {
        if let Some(icon) = lookup(EXT, ext).or_else(|| lookup(LANG_EXT, ext)) {
            return named(icon);
        }
    }
    if lower.starts_with('.') {
        return named("_config");
    }
    named(DEFAULT)
}

/// The glyph `ch` as a filled SVG in a 1000-unit square, for tinting.
pub fn svg(ch: u32) -> Option<String> {
    let i = GLYPHS.binary_search_by(|(c, _)| c.cmp(&ch)).ok()?;
    Some(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1000 1000\"><path fill=\"#ffffff\" d=\"{}\"/></svg>",
        GLYPHS[i].1
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_sorted_for_the_search() {
        let sorted = |t: &[(&str, &str)]| t.windows(2).all(|w| w[0].0 < w[1].0);
        assert!(sorted(EXT) && sorted(LANG_EXT) && sorted(NAMES));
        assert!(DEFS.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(GLYPHS.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn names_resolve_as_the_renderer_resolves_them() {
        let icon = |n: &str| def(n).expect(n);
        assert_eq!(resolve("app.ts"), icon("_typescript"));
        assert_eq!(resolve("README.md"), resolve("readme.md"));
        assert_eq!(
            resolve("package.json"),
            icon(lookup(NAMES, "package.json").unwrap())
        );
        assert_eq!(resolve(".env.local"), resolve(".env.local"));
        assert_eq!(resolve(".somerc"), icon("_config"));
        assert_eq!(resolve("noext"), icon("_default"));
        assert_eq!(resolve("a.spec.ts"), icon(lookup(EXT, "spec.ts").unwrap()));
    }

    #[test]
    fn every_icon_has_its_glyph() {
        for (name, ch, _) in DEFS {
            assert!(svg(*ch).is_some(), "{name}");
        }
    }
}
