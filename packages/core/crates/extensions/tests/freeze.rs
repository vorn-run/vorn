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

/// The shipped set, in the extensions repository's checkout that
/// `VORN_EXTENSIONS_DIR` names (CI's, built for this lock); skipped without one.
#[test]
fn the_shipped_set_keeps_to_the_frozen_api_where_it_moved() {
    let Some(checkout) = std::env::var_os("VORN_EXTENSIONS_DIR") else {
        eprintln!("skipped: VORN_EXTENSIONS_DIR names no extensions checkout");
        return;
    };
    if let Err(found) = freeze::check_extensions(workspace(), Path::new(&checkout)) {
        let lines: Vec<String> = found.iter().map(ToString::to_string).collect();
        panic!("the shipped set is past the frozen API:\n{}", lines.join("\n"));
    }
}

#[test]
fn a_shipped_extension_past_the_public_api_is_named() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/freeze");
    let found = freeze::check_extensions(&fixtures.join("shipped"), &fixtures.join("elsewhere"))
        .expect_err("the fixture breaks every rule");
    let mut lines: Vec<String> = found
        .iter()
        .map(|f| format!("{}:{}", f.path.display(), f.line).replace('\\', "/"))
        .collect();
    lines.sort();
    assert_eq!(
        lines,
        [
            "bad/Cargo.toml:8",
            "bad/src/lib.rs:3",
            "bad/src/lib.rs:5",
            "bad/vorn-extension.toml:1",
        ],
        "{found:#?}"
    );
    let says = |needle: &str| found.iter().any(|f| f.message.contains(needle));
    assert!(says("is vorn.bad@1.0.0, and shipped.toml pins vorn.bad@2.0.0"));
    assert!(says("depends on vorn-extensions; an extension builds on vorn-extension-sdk alone"));
    assert!(says("uses vorn_extension_sdk::guest"));
    assert!(says("names task.secretly, which is no public bridge method"));
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
