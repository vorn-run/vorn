//! Builds a crate of this workspace as a Wasm component, for vornd's build
//! script (the extensions Vorn ships as Wasm) and for the tests that load one.
//!
//! The build is a cargo run of its own for `wasm32-wasip2`, in a target
//! folder of its own (`target/wasm-guests`), so it never waits on the lock of
//! the build that asked for it. It runs with the cargo and toolchain of that
//! build and without its flags, profile or features, which are for the host.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The target every component is built for.
pub const TARGET: &str = "wasm32-wasip2";
/// The workspace profile components build with: small, and abort on panic.
pub const PROFILE: &str = "wasm";

/// The workspace this crate is in: `packages/core`.
pub fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Where component builds go: `$CARGO_TARGET_DIR/wasm-guests`, else `target/wasm-guests`.
fn target_dir() -> PathBuf {
    let base = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map_or_else(
            || workspace().join("target"),
            |d| {
                if d.is_absolute() {
                    d
                } else {
                    workspace().join(d)
                }
            },
        );
    base.join("wasm-guests")
}

/// Whether the build that runs us set `name` for itself rather than for a cargo it starts.
fn host_only(name: &str) -> bool {
    const KEPT: [&str; 4] = [
        "CARGO_HOME",
        "CARGO_NET_OFFLINE",
        "CARGO_MAKEFLAGS",
        "CARGO_HTTP_TIMEOUT",
    ];
    if KEPT.contains(&name) || name.starts_with("CARGO_REGISTRIES_") {
        return false;
    }
    name.starts_with("CARGO_")
        || name.starts_with("__CARGO")
        || matches!(
            name,
            "RUSTFLAGS" | "RUSTDOCFLAGS" | "RUSTC_WRAPPER" | "RUSTC_WORKSPACE_WRAPPER" | "RUSTC"
        )
}

/// Builds workspace package `package` (a `cdylib`) as a component and answers its `.wasm`.
pub fn component(package: &str) -> Result<PathBuf, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let target_dir = target_dir();
    let mut command = Command::new(cargo);
    for (name, _) in std::env::vars_os() {
        if name.to_str().is_some_and(host_only) {
            command.env_remove(&name);
        }
    }
    command
        .current_dir(workspace())
        .args(["build", "--locked", "--target", TARGET, "--profile", PROFILE, "-p"])
        .arg(package)
        .arg("--target-dir")
        .arg(&target_dir);
    let output = command
        .output()
        .map_err(|e| format!("cargo could not run to build {package}: {e}"))?;
    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr);
        let hint = if said.contains("target may not be installed") {
            format!("\n(run `rustup target add {TARGET}`)")
        } else {
            String::new()
        };
        return Err(format!(
            "{package} did not build for {TARGET}:\n{}{hint}",
            tail(&said, 40)
        ));
    }
    let wasm = target_dir
        .join(TARGET)
        .join(PROFILE)
        .join(format!("{}.wasm", package.replace('-', "_")));
    if wasm.is_file() {
        Ok(wasm)
    } else {
        Err(format!(
            "cargo built {package} but {} is missing",
            wasm.display()
        ))
    }
}

/// The folders a build script reruns for when a component built from them changes.
pub fn rerun_if_changed(folders: &[&Path]) {
    for folder in folders {
        println!("cargo:rerun-if-changed={}", folder.display());
    }
    println!("cargo:rerun-if-changed={}", workspace().join("Cargo.lock").display());
}

fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_what_a_nested_cargo_needs_and_drops_the_hosts_build_settings() {
        for kept in ["CARGO_HOME", "CARGO_NET_OFFLINE", "CARGO_REGISTRIES_X_INDEX", "PATH", "RUSTUP_TOOLCHAIN"] {
            assert!(!host_only(kept), "{kept}");
        }
        for dropped in [
            "CARGO_TARGET_DIR",
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_PROFILE_RELEASE_LTO",
            "CARGO_FEATURE_WASM",
            "CARGO_MANIFEST_DIR",
            "RUSTFLAGS",
            "RUSTC_WORKSPACE_WRAPPER",
            "__CARGO_FIX_PLZ",
        ] {
            assert!(host_only(dropped), "{dropped}");
        }
    }
}
