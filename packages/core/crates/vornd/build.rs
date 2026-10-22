//! Embeds the shipped set, the released packs `shipped.toml` pins, and the
//! pins of `pinned.toml`'s, which only the catalog offers ([`crate::shipped`]).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

fn main() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    embed_shipped(&workspace, &out);
    embed_pinned(&workspace, &out);
}

/// Writes `pinned.rs`: each `(id, version, sha256)` `pinned.toml` pins. No
/// pack is embedded; the catalog's is checked against these.
fn embed_pinned(workspace: &Path, out: &Path) {
    let lock = workspace.join("pinned.toml");
    println!("cargo:rerun-if-changed={}", lock.display());
    let text = std::fs::read_to_string(&lock).unwrap_or_else(|e| panic!("{}: {e}", lock.display()));
    let mut code = String::from("&[\n");
    for entry in extensions(&text, "pinned.toml") {
        let field = |key: &str| {
            entry
                .get(key)
                .and_then(toml::Value::as_str)
                .unwrap_or_else(|| panic!("pinned.toml: an extension has no {key}"))
        };
        let (id, version, sha256) = (field("id"), field("version"), field("sha256"));
        let _ = writeln!(code, "    ({id:?}, {version:?}, {sha256:?}),");
    }
    code.push(']');
    std::fs::write(out.join("pinned.rs"), code).unwrap_or_else(|e| panic!("pinned.rs: {e}"));
}

/// The `[[extension]]` tables of the lock `name` reads as `text`.
fn extensions(text: &str, name: &str) -> Vec<toml::Value> {
    let table: toml::Table = toml::from_str(text).unwrap_or_else(|e| panic!("{name}: {e}"));
    table
        .get("extension")
        .and_then(toml::Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// Writes `shipped.rs`: every pack `shipped.toml` pins, read from
/// `$VORN_SHIPPED_DIR` (else `shipped/`) and checked against its sha256.
/// A missing pack leaves it out with a warning, unless `VORN_SHIPPED_REQUIRED`
/// is set, as it is for CI and packaging; a pack that is not the pinned one
/// always fails the build.
fn embed_shipped(workspace: &Path, out: &Path) {
    println!("cargo:rerun-if-env-changed=VORN_SHIPPED_DIR");
    println!("cargo:rerun-if-env-changed=VORN_SHIPPED_REQUIRED");
    let lock = workspace.join("shipped.toml");
    println!("cargo:rerun-if-changed={}", lock.display());
    let dir = std::env::var_os("VORN_SHIPPED_DIR")
        .map_or_else(|| workspace.join("shipped"), PathBuf::from);
    println!("cargo:rerun-if-changed={}", dir.display());
    let required = std::env::var_os("VORN_SHIPPED_REQUIRED").is_some_and(|v| v != "0");
    let text = std::fs::read_to_string(&lock).unwrap_or_else(|e| panic!("{}: {e}", lock.display()));
    let mut code = String::from("&[\n");
    for entry in extensions(&text, "shipped.toml") {
        let field = |key: &str| {
            entry
                .get(key)
                .and_then(toml::Value::as_str)
                .unwrap_or_else(|| panic!("shipped.toml: an extension has no {key}"))
        };
        let (id, version, sha256) = (field("id"), field("version"), field("sha256"));
        let default = entry
            .get("default")
            .and_then(toml::Value::as_bool)
            .unwrap_or(true);
        let file = dir.join(format!("{id}-{version}.vorn.tgz"));
        let Ok(bytes) = std::fs::read(&file) else {
            assert!(
                !required,
                "{} is missing; run scripts/shipped.mjs",
                file.display()
            );
            println!(
                "cargo:warning={} is missing, so this build ships without {id}; run scripts/shipped.mjs",
                file.display()
            );
            continue;
        };
        let found: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            found,
            sha256,
            "{} is not the pack shipped.toml pins; run scripts/shipped.mjs",
            file.display()
        );
        let _ = writeln!(
            code,
            "    vorn_extensions::shipped::Shipped {{ id: {id:?}, version: {version:?}, sha256: {sha256:?}, default: {default}, bytes: include_bytes!({:?}) }},",
            file.display().to_string()
        );
    }
    code.push(']');
    std::fs::write(out.join("shipped.rs"), code).unwrap_or_else(|e| panic!("shipped.rs: {e}"));
}
