//! Extensions: the one kind of package that extends Vorn, and the bridge
//! that runs them.
//!
//! A package is a folder with a manifest ([`manifest`]), installed from git
//! or linked in place ([`install`], [`pack`]). Its program speaks the
//! extension API, defined in `wit/extension.wit` and generated into [`api`],
//! or, under a `[connector]` section, the connector protocol that API
//! version includes ([`connector`], [`host`]). A service may be a Wasm
//! component in place of a program ([`component`], and the `wasm` feature's
//! runtime), sandboxed in the host. Every program
//! runs under one [`lifecycle`]: started when first needed, stopped when
//! idle, and held to a cap. The [`router`] carries calls and events to them
//! and answers what they ask through the [`gate`].
//!
//! Nothing here knows how sessions are kept or how a client is reached; the
//! daemon that hosts extensions hands those in.

pub mod activation;
pub mod api;
pub mod blobs;
pub mod bridge;
pub mod child;
pub mod component;
pub mod connector;
pub mod events;
pub mod footer;
pub mod gate;
pub mod grants;
pub mod host;
pub mod install;
pub mod js;
pub mod lifecycle;
pub mod links;
pub mod manifest;
pub mod netproxy;
pub mod pack;
pub mod page;
pub mod polls;
pub mod router;
pub mod sandbox;
pub mod signin;
pub mod token;
pub mod usage;
pub mod points;
#[cfg(feature = "wasm")]
pub mod wasm;

#[cfg(test)]
pub(crate) mod testing {
    //! Agents this crate's tests name, read as an adapter's are, so their ids are known.

    /// Reads adapters for `alpha` and `beta`.
    pub fn agents() {
        for id in ["alpha", "beta"] {
            let text = format!(
                "[agent]\nid = \"{id}\"\nname = \"{id}\"\n[agent.launch]\ncommand = \"{id}\"\n"
            );
            vorn_agents::from_manifest_text(&text).expect("a test adapter reads");
        }
    }
}
