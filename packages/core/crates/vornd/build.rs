//! Builds the extensions Vorn ships as Wasm components, which vornd carries
//! as bytes ([`crate::native::extensions::builtin`]): each one's crate is
//! built for wasm32-wasip2 and its `.wasm` copied into `OUT_DIR`.

use std::path::PathBuf;

/// `(workspace package, its folder, the name it is kept under)`.
const SHIPPED: [(&str, &str, &str); 7] = [
    (
        "vorn-ext-worktrees",
        "extensions/worktrees",
        "worktrees.wasm",
    ),
    ("vorn-ext-widget", "extensions/widget", "widget.wasm"),
    ("vorn-ext-tasks", "extensions/tasks", "tasks.wasm"),
    (
        "vorn-ext-workflow-editor",
        "extensions/workflow-editor",
        "workflow-editor.wasm",
    ),
    ("vorn-ext-blocks", "extensions/blocks", "blocks.wasm"),
    ("vorn-ext-files", "extensions/files", "files.wasm"),
    ("vorn-ext-browser", "extensions/browser", "browser.wasm"),
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
}
