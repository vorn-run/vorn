//! Every helper process in the core crates starts through `vorn_spawn`, so
//! none opens a console window on Windows. A spawn the user is meant to see
//! carries a `// spawn-visible: <why>` comment on the line above it.

use std::fs;
use std::path::{Path, PathBuf};

const MARKER: &str = "spawn-visible:";

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

/// `file:line` for each bare `Command::new` in `text` without the marker above it.
fn bare_spawns(file: &Path, text: &str) -> Vec<String> {
    let lines: Vec<&str> = production(text).lines().collect();
    lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains("Command::new(") && !line.trim_start().starts_with("//"))
        .filter(|(i, _)| !(*i > 0 && lines[i - 1].contains(MARKER)))
        .map(|(i, _)| format!("{}:{}", file.display(), i + 1))
        .collect()
}

#[test]
fn every_helper_process_starts_hidden() {
    let own = Path::new(env!("CARGO_MANIFEST_DIR"));
    let crates = own.parent().expect("crates/spawn sits in crates/");
    let mut files = Vec::new();
    for entry in fs::read_dir(crates).expect("crates/ lists").flatten() {
        if entry.path() != own {
            sources(&entry.path().join("src"), &mut files);
        }
    }
    assert!(
        files.len() > 50,
        "found only {} sources under {}",
        files.len(),
        crates.display()
    );
    let bare: Vec<String> = files
        .iter()
        .flat_map(|file| bare_spawns(file, &fs::read_to_string(file).expect("source reads")))
        .collect();
    assert!(
        bare.is_empty(),
        "start these through vorn_spawn::command or vorn_spawn::tokio_command, \
         or mark one the user must see with `// {MARKER} <why>`:\n{}",
        bare.join("\n")
    );
}

#[test]
fn the_guard_sees_a_bare_spawn_and_skips_marked_and_test_ones() {
    let text = "\
fn a() { std::process::Command::new(\"git\"); }
// spawn-visible: the user's own console.
fn b() { Command::new(\"vornd\"); }
// Command::new(\"in a comment\")
#[cfg(test)]
mod tests {
    fn c() { Command::new(\"true\"); }
}
";
    assert_eq!(bare_spawns(Path::new("f.rs"), text), ["f.rs:1"]);
}
