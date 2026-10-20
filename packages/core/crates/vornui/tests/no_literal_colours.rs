//! Every colour vornui, vorn-components and vorn-app draw comes from the
//! theme's tokens, so a theme recolours all of it. A colour that is content
//! rather than chrome (a file type's mark, a tint a person picked) sits under
//! a `// literal-colour: <why>` comment, which covers lines up to the next
//! blank one.

use std::fs;
use std::path::{Path, PathBuf};

const MARKER: &str = "literal-colour:";

/// Where colours are defined rather than drawn.
const DEFINITIONS: &[&str] = &["vornui/src/scene.rs", "vornui/src/palette.rs"];

/// What builds a colour from numbers rather than naming a token.
const CALLS: &[&str] = &[
    "Rgba::hex(",
    "Rgba::hexa(",
    "rgba_const(",
    "Rgba::white(",
    "Rgba::BLACK",
    "Rgba::WHITE",
    "Rgba([",
];

/// The `.rs` files under `dir`, leaving out test-only files and directories.
fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if !matches!(name.as_ref(), "tests" | "benches" | "examples") {
                sources(&path, out);
            }
        } else if name.ends_with(".rs") && name != "tests.rs" {
            out.push(path);
        }
    }
}

/// `text` up to its trailing test module, if it has one.
fn production(text: &str) -> &str {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut offset = 0;
    for (i, line) in lines.iter().enumerate() {
        let attr = line.trim_start();
        let test_cfg = attr.starts_with("#[cfg(test)]") || attr.starts_with("#[cfg(all(test");
        let opens_module = lines.get(i + 1).is_some_and(|next| {
            let next = next.trim();
            (next.starts_with("mod ") || next.starts_with("pub mod ")) && next.ends_with('{')
        });
        if test_cfg && opens_module {
            return &text[..offset];
        }
        offset += line.len();
    }
    text
}

/// Whether `code` holds a `0xRRGGBB` literal: exactly six hex digits.
fn has_rgb_literal(code: &str) -> bool {
    let b = code.as_bytes();
    (0..b.len().saturating_sub(1)).any(|i| {
        let starts = b[i] == b'0'
            && b[i + 1] == b'x'
            && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_'));
        let digits = b[i + 2..]
            .iter()
            .take_while(|c| c.is_ascii_hexdigit())
            .count();
        let after = b.get(i + 2 + digits);
        starts && digits == 6 && !after.is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
    })
}

/// `file:line` for each literal colour in `text` outside a marked block.
fn literal_colours(file: &str, text: &str) -> Vec<String> {
    let mut marked = false;
    let mut out = Vec::new();
    for (i, line) in production(text).lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            marked = false;
            continue;
        }
        if trimmed.starts_with("//") {
            marked |= trimmed.contains(MARKER);
            continue;
        }
        let code = line.split("//").next().unwrap_or(line);
        if !marked && (CALLS.iter().any(|c| code.contains(c)) || has_rgb_literal(code)) {
            out.push(format!("{file}:{}", i + 1));
        }
    }
    out
}

#[test]
fn every_drawn_colour_is_a_theme_token() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/vornui sits in crates/");
    let mut files = Vec::new();
    for name in ["vornui", "vorn-components", "vorn-app"] {
        sources(&crates.join(name).join("src"), &mut files);
    }
    assert!(files.len() > 100, "found only {} sources", files.len());
    let found: Vec<String> = files
        .iter()
        .filter_map(|file| {
            let rel = file
                .strip_prefix(crates)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            (!DEFINITIONS.contains(&rel.as_str())).then_some((file, rel))
        })
        .flat_map(|(file, rel)| {
            literal_colours(&rel, &fs::read_to_string(file).expect("source reads"))
        })
        .collect();
    assert!(
        found.is_empty(),
        "draw these with a token from vornui::theme::color (or theme::bright / theme::shadow), \
         or mark a content colour with `// {MARKER} <why>`:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_guard_sees_literals_and_skips_marked_comment_and_test_ones() {
    let text = "\
fn a() -> Rgba { Rgba::hex(0x123456) }
fn b() -> u32 { 0xabcdef }
fn c() -> u32 { 0xE023 + 0x1234567 }
// literal-colour: a file type's mark.
const MARK: u32 = 0x519aba;
const NEXT: u32 = 0x519aba;

fn d() -> Rgba { Rgba::white(0.5) } // Rgba::hex in a trailing comment is fine
// Rgba::hex(0x000000) in a comment
#[cfg(test)]
mod tests {
    fn e() -> Rgba { Rgba::hex(0xffffff) }
}
";
    assert_eq!(
        literal_colours("f.rs", text),
        ["f.rs:1", "f.rs:2", "f.rs:8"]
    );
}
