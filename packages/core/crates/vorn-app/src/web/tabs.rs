//! The browser pane in a browser: the page cannot hold another site inside
//! it, so each page opens in a tab of its own and the pane says so.

use std::collections::HashMap;
use std::sync::Arc;
use web_time::Instant;

use super::{Slot, WebEvent, NO_PANE};
use crate::client::rpc::Wake;
use crate::ui::NativeWindow;

/// What the pane says where a desktop shows the page.
pub const IN_A_TAB: &str = "In the browser, pages open in a tab of their own.";

/// The pages each pane opened, by key.
pub struct Host {
    opened: HashMap<String, String>,
}

impl Host {
    /// Nothing holds the keyboard here.
    pub fn release_keys(&self) {}

    pub fn new(_wake: Wake) -> Host {
        Host {
            opened: HashMap::new(),
        }
    }

    pub fn set_profiles(&mut self, _dir: std::path::PathBuf) {}

    /// A page the app works in unseen cannot open here.
    pub fn ensure(&mut self, _key: &str, _url: &str, _profile: Option<&str>) -> Result<(), String> {
        Err(IN_A_TAB.to_owned())
    }

    pub fn forget(&mut self, _connection: &str) {}

    pub fn attach(&mut self, _window: &Arc<NativeWindow>) {}

    pub fn unavailable(&self) -> Option<&str> {
        Some(IN_A_TAB)
    }

    /// Opens each slot's page in a tab, once per address.
    pub fn show(&mut self, slots: &[Slot]) {
        for slot in slots {
            if self.opened.get(&slot.key) != Some(&slot.url) {
                crate::page::open_tab(&slot.url);
                self.opened.insert(slot.key.clone(), slot.url.clone());
            }
        }
    }

    pub fn close(&mut self, gone: impl Fn(&str) -> bool) {
        self.opened.retain(|k, _| !gone(k));
    }

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

    /// Opens `url` in a tab, as the person asked for it again.
    pub fn load(&mut self, key: &str, url: &str) {
        crate::page::open_tab(url);
        self.opened.insert(key.to_owned(), url.to_owned());
    }

    pub fn reload(&self, _key: &str) {}

    pub fn run(&self, _key: &str, _js: &str) {}

    pub fn eval(&mut self, _key: &str, _js: &str) -> Result<u64, String> {
        Err(IN_A_TAB.to_owned())
    }

    pub fn tick(&mut self) -> Option<Instant> {
        None
    }
}
