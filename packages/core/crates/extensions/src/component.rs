//! What the Wasm tier needs to name with or without its runtime: where a
//! component comes from, what compiles it, and the components Vorn ships.
//!
//! A component runs only as code Vorn compiled itself, kept in the extensions
//! folder's [`COMPILED`] cache under the SHA-256 of the `.wasm` it came from.
//! vornd links Wasmtime's runtime and no compiler, so a component missing
//! from the cache is compiled by the `vorn` program ([`Compiler::Program`])
//! when it is installed, linked or first started. The runtime itself is the
//! `wasm` feature's [`crate::wasm`].

use std::path::PathBuf;
use std::sync::Arc;

use crate::manifest::Manifest;

/// The folder, in the extensions folder, of what Vorn compiled.
pub const COMPILED: &str = ".compiled";

/// What fills the cache of compiled components.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Compiler {
    /// Nothing: only what the cache already holds runs.
    #[default]
    None,
    /// The `vorn` program, run as `vorn ext compile <wasm> --into <cache>`.
    Program(PathBuf),
    /// Wasmtime's compiler in this process (the `wasm-compile` feature): the
    /// vorn command and tests, never vornd.
    #[cfg(feature = "wasm-compile")]
    InProcess,
}

impl Compiler {
    /// The `vorn` program beside the running one (vornd ships beside it), when it is there.
    pub fn beside_this_program() -> Compiler {
        let name = format!("vorn{}", std::env::consts::EXE_SUFFIX);
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|d| d.join(&name)))
            .filter(|p| p.is_file())
            .map_or(Compiler::None, Compiler::Program)
    }
}

/// Where a component's `.wasm` is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Bytes compiled into the host: an extension Vorn ships.
    Bundled(&'static [u8]),
    /// A package's file read once, so what was checked of it is what runs.
    Read(std::sync::Arc<[u8]>),
}

/// An extension Vorn ships as a Wasm component: its manifest, and the
/// component compiled into vornd. It runs through the same router, gate,
/// lifecycle and crash budget as an installed package's component.
#[derive(Debug, Clone)]
pub struct Bundled {
    pub manifest: Arc<Manifest>,
    pub wasm: &'static [u8],
}
