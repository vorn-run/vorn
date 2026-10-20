//! The browser pane without OS web views (the terminal UI): no pages, the
//! pane says why.

use std::time::Instant;

use super::{Slot, WebEvent, NO_PANE};
use crate::client::rpc::Wake;
use crate::ui::NativeWindow;

const OFF: &str = "Pages show in the Vorn window; open them in your browser.";

/// The web views of every open pane: none in this build.
pub struct Host {
}

impl Host {
    /// Nothing holds the keyboard here.
    pub fn release_keys(&self) {}

    pub fn new(_wake: Wake) -> Host {
        Host {
        }
    }

    pub fn set_profiles(&mut self, _dir: std::path::PathBuf) {}

    pub fn ensure(&mut self, _key: &str, _url: &str, _profile: Option<&str>) -> Result<(), String> {
        Err(OFF.to_owned())
    }

    pub fn forget(&mut self, _connection: &str) {}

    pub fn attach(&mut self, _window: &std::sync::Arc<NativeWindow>) {}

    pub fn unavailable(&self) -> Option<&str> {
        Some(OFF)
    }

    pub fn show(&mut self, _slots: &[Slot]) {}

    pub fn close(&mut self, _gone: impl Fn(&str) -> bool) {}

    pub fn has(&self, _key: &str) -> bool {
        false
    }

    pub fn poll(&mut self) -> Vec<WebEvent> {
        Vec::new()
    }

    pub fn generation(&self, _key: &str) -> Option<u32> {
        None
    }

    pub fn nav(&self, _key: &str) -> (bool, bool) {
        (false, false)
    }

    pub fn step(&mut self, _key: &str, _back: bool) -> Result<String, String> {
        Err(NO_PANE.to_owned())
    }

    pub fn load(&mut self, _key: &str, _url: &str) {}

    pub fn reload(&mut self, _key: &str) {}

    pub fn run(&self, _key: &str, _js: &str) {}

    pub fn eval(&mut self, _key: &str, _js: &str) -> Result<u64, String> {
        Err(NO_PANE.to_owned())
    }

    pub fn tick(&mut self) -> Option<Instant> {
        None
    }
}
