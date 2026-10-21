//! The extension-tiers bench: three extensions Vorn moved out of its core
//! (the Widget, Worktree cleanup and the RSS connector), each built from one
//! source as a Wasm component and as a program on the same manifest, run
//! through the router vornd runs, and measured side by side.
//!
//! [`program::serve`] is the program tier's whole host; [`rig::Rig`] runs
//! either tier against [`world::Bench`]; [`fixtures`] serves the feeds.

pub mod fixtures;
pub mod program;
pub mod rig;
pub mod stats;
pub mod sys;
pub mod world;
