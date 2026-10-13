//! Vorn's desktop app for Windows and Linux (it runs on macOS too, for
//! development): today's main screen on vornui, over a running vornd.
//!
//! - [`client`] speaks JSON-RPC to vornd and keeps its models current.
//! - [`grid`] holds the terminals' screens over the grid endpoint.
//! - [`screen`] turns that state into elements; [`paint`] draws terminals.
//! - [`browser`], [`artifact`] and [`device`] model the side pane; [`web`]
//!   seats the OS web views it shows.
//! - [`app`] ties them together and routes keys; `main` gives it a window
//!   or an offscreen target.
//!
//! The UI layer is reached only through [`ui`], so it can be repointed.

pub mod act;
pub mod app;
pub mod artifact;
pub mod blocks;
pub mod browser;
pub mod canvas;
pub mod client;
pub mod crash;
pub mod device;
pub mod editor;
pub mod extensions;
pub mod file_icons;
pub mod gates;
pub mod git;
pub mod graph;
pub mod grid;
pub mod highlight;
pub mod hotkeys;
pub mod intent;
pub mod launch;
pub mod layout;
pub mod library;
pub mod look;
pub mod markdown;
pub mod models;
pub mod network;
pub mod notify;
pub mod onboarding;
pub mod paint;
pub mod palette;
pub mod panes;
pub mod plugins;
pub mod runs;
pub mod screen;
pub mod scroll;
pub mod select;
pub mod settings;
pub mod side;
pub mod sidebar_view;
pub mod signin;
pub mod state;
pub mod system;
pub mod tasks;
pub mod templates;
pub mod ui;
pub mod update;
pub mod vars;
pub mod view;
pub mod web;
pub mod widget;
pub mod workflows;

/// Vorn's composed components, built on [`ui`].
pub use vorn_components as components;

pub use app::App;

/// The UI config at `scale`: the app's fonts and terminal size.
pub fn ui_config(scale: f32) -> ui::UiConfig {
    ui::UiConfig::system(scale, look::TERM_FONT)
}
