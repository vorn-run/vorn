//! The frozen core, checked on the tree: the built-ins call only the public
//! API, and every feature left in a core or host crate is listed in
//! `freeze.toml` with its reason. A fixture that breaks each rule shows the
//! check names each break by file and line.

use std::path::Path;

use vorn_extensions::freeze;

fn workspace() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits at packages/core/crates/extensions")
}

#[test]
fn the_core_and_its_built_ins_keep_to_the_frozen_api() {
    if let Err(found) = freeze::check(workspace()) {
        let lines: Vec<String> = found.iter().map(ToString::to_string).collect();
        panic!("the frozen core is broken:\n{}", lines.join("\n"));
    }
}

#[test]
fn a_built_in_past_the_public_api_and_an_unlisted_hook_are_each_named() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/freeze/bad");
    let found = freeze::check(&root).expect_err("the fixture breaks every rule");
    let mut lines: Vec<String> = found
        .iter()
        .map(|f| format!("{}:{}", f.path.display(), f.line).replace('\\', "/"))
        .collect();
    lines.sort();
    assert_eq!(
        lines,
        [
            "Cargo.toml:5",
            "crates/host/src/lib.rs:3",
            "extensions/bad/Cargo.toml:7",
            "extensions/bad/src/lib.rs:3",
            "extensions/bad/src/lib.rs:7",
            "extensions/bad/vorn-extension.toml:4",
            "freeze.toml:2",
            "freeze.toml:8",
        ],
        "{found:#?}"
    );
    let says = |needle: &str| found.iter().any(|f| f.message.contains(needle));
    assert!(says("built for vorn:extension@0.9"), "{found:#?}");
    assert!(says(
        "depends on crates/core; a built-in builds on crates/extension-sdk alone"
    ));
    assert!(says("uses vorn_extension_sdk::bindings"));
    assert!(says(
        "names task.secretly, which is no public bridge method"
    ));
    assert!(says(
        "crates/stray is neither core, host, tooling, a built-in nor an exception"
    ));
    assert!(says("crates/core is in area `billing`"));
    assert!(says(
        "names the built-in vorn.bad from a core or host crate"
    ));
    assert!(says("crates/host/src/lib.rs#vorn.gone, which is gone"));
}
