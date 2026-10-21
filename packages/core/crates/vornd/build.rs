//! Builds the extensions Vorn still carries as built-ins as Wasm components
//! ([`crate::native::extensions::builtin`]): each one's crate is built for
//! wasm32-wasip2 and its `.wasm` copied into `OUT_DIR`. Then embeds the
//! shipped set, the released packs `shipped.toml` pins ([`crate::shipped`]).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// `(workspace package, its folder, the name it is kept under)`.
const SHIPPED: [(&str, &str, &str); 8] = [
    ("vorn-ext-widget", "extensions/widget", "widget.wasm"),
    ("vorn-ext-tasks", "extensions/tasks", "tasks.wasm"),
    (
        "vorn-ext-workflows",
        "extensions/workflows",
        "workflows.wasm",
    ),
    ("vorn-ext-blocks", "extensions/blocks", "blocks.wasm"),
    ("vorn-ext-files", "extensions/files", "files.wasm"),
    ("vorn-ext-browser", "extensions/browser", "browser.wasm"),
    ("vorn-ext-device", "extensions/device", "device.wasm"),
    ("vorn-ext-ssh", "extensions/ssh", "ssh.wasm"),
];

fn main() {
    let workspace = vorn_guest_build::workspace();
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let sdk = workspace.join("crates/extension-sdk");
    let wit = workspace.join("crates/extensions/wit");
    let tree = workspace.join("crates/tree");
    // Tasks words its MCP tools with the server's toolkit.
    let mcp = workspace.join("crates/mcp");
    for (package, folder, name) in SHIPPED {
        let folder = workspace.join(folder);
        vorn_guest_build::rerun_if_changed(&[&folder, &sdk, &wit, &tree, &mcp]);
        let built = vorn_guest_build::component(package).unwrap_or_else(|e| panic!("{e}"));
        std::fs::copy(&built, out.join(name))
            .unwrap_or_else(|e| panic!("{}: {e}", built.display()));
    }
    embed_shipped(&workspace, &out);
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
    let table: toml::Table = toml::from_str(&text).unwrap_or_else(|e| panic!("shipped.toml: {e}"));
    let mut code = String::from("&[\n");
    for entry in table
        .get("extension")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
    {
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
            assert!(!required, "{} is missing; run scripts/shipped.mjs", file.display());
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
