//! Embeds every package under `agents/` in the order `agents/shipped.toml`
//! names them, so adding an agent is adding its folder and its name there.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn files(dir: &Path, root: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.expect("a readable entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            // Recorded screens test the package; Vorn does not ship them.
            if path.file_name().is_some_and(|n| n != "fixtures") {
                files(&path, root, out);
            }
        } else if path.file_name().is_some_and(|n| n != "vorn-extension.toml") {
            out.push(path.strip_prefix(root).expect("under its root").to_path_buf());
        }
    }
}

fn list(text: &str, key: &str) -> Vec<String> {
    let line = text
        .lines()
        .find(|l| l.trim_start().starts_with(key))
        .unwrap_or_else(|| panic!("shipped.toml has no {key}"));
    let inner = &line[line.find('[').expect("a list") + 1..line.rfind(']').expect("a list")];
    inner
        .split(',')
        .map(|s| s.trim().trim_matches('"').to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo")).join("agents");
    println!("cargo:rerun-if-changed={}", root.display());
    let shipped = std::fs::read_to_string(root.join("shipped.toml")).expect("agents/shipped.toml");
    let (order, off) = (list(&shipped, "order"), list(&shipped, "off"));
    let mut code = String::from("&[\n");
    for name in &order {
        let dir = root.join(name);
        let manifest = dir.join("vorn-extension.toml");
        assert!(manifest.is_file(), "{} has no manifest", dir.display());
        let mut found = Vec::new();
        files(&dir, &dir, &mut found);
        let _ = writeln!(code, "    Shipped {{\n        name: {name:?},\n        manifest: include_str!({:?}),", manifest.display().to_string());
        let _ = writeln!(code, "        on_by_default: {},\n        files: &[", !off.contains(name));
        for f in &found {
            let published = f.to_string_lossy().replace('\\', "/");
            let _ = writeln!(code, "            ({published:?}, include_bytes!({:?})),", dir.join(f).display().to_string());
        }
        code.push_str("        ],\n    },\n");
    }
    code.push(']');
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("set by cargo")).join("shipped.rs");
    std::fs::write(out, code).expect("OUT_DIR is writable");
}
