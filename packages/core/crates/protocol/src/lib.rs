//! Vorn's protocol types, generated from the definitions in `packages/shared`.
//!
//! `scripts/gen-store-schema.mjs` turns the records in `packages/shared` and
//! the store's own shapes into `schema/store.json`, and this crate's build
//! turns that into serde types, so the web client and Rust read one definition.
//! Field names match `packages/shared`; JSON a column holds whole is a
//! [`serde_json::Value`], and string unions are strings (see the script for
//! why).

#![allow(clippy::all, clippy::pedantic, missing_docs)]

include!(concat!(env!("OUT_DIR"), "/store.rs"));
