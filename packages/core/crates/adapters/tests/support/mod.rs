//! The shipped adapters as Vorn reads them, for the tests that replay what
//! 0.8 recorded against them.

#![allow(dead_code)]

use std::path::PathBuf;

use vorn_agents::{Agent, Agents};

/// The adapter shipped as `name`, read from its manifest.
pub fn agent(name: &str) -> Agent {
    let shipped = vorn_adapters::shipped()
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no adapter ships as {name}"));
    let agent = vorn_agents::from_manifest_text(shipped.manifest)
        .unwrap_or_else(|e| panic!("{name}: {e}"))
        .unwrap_or_else(|| panic!("{name} has no [agent] section"));
    if let Some(env) = &agent.spec().env {
        vorn_agents::launch::env::add_markers(&env.strip, &env.strip_prefixes);
    }
    agent
}

/// Every shipped adapter, those switched off by default too.
pub fn all() -> Agents {
    Agents::new(vorn_adapters::shipped().iter().map(|s| agent(s.name)))
}

/// The shipped adapters on by default: the agents 0.8 knew.
pub fn on_by_default() -> Agents {
    Agents::new(
        vorn_adapters::shipped()
            .iter()
            .filter(|s| s.on_by_default)
            .map(|s| agent(s.name)),
    )
}

/// Every adapter's markers, registered as vornd registers them.
pub fn markers() {
    all();
}

/// A file under the repository's `tests/fixtures`.
pub fn repo_fixture(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../tests/fixtures")
        .join(path)
}

/// The package folder of the adapter shipped as `name`.
pub fn package(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("agents").join(name)
}
