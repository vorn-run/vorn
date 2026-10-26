//! Extensions' screenshots: fetched or read off the frame path, one thread
//! per picture, then registered as images on the next frame.

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};

use vornui::{decode_png, Ui};

use crate::client::rpc::Wake;
use crate::plugins::Listing;

/// The largest picture read, so a bad URL cannot fill memory.
#[cfg(not(target_arch = "wasm32"))]
const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// Where a picture stands.
#[derive(Debug)]
enum Shot {
    Loading,
    Ready(u32),
    Failed,
}

/// The screenshots asked for so far, by [`resolve`]'s key.
pub struct Shots {
    shots: HashMap<String, Shot>,
    tx: Sender<(String, Option<Vec<u8>>)>,
    rx: Receiver<(String, Option<Vec<u8>>)>,
}

impl Default for Shots {
    fn default() -> Self {
        let (tx, rx) = channel();
        Shots {
            shots: HashMap::new(),
            tx,
            rx,
        }
    }
}

impl std::fmt::Debug for Shots {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shots").field("shots", &self.shots).finish()
    }
}

impl Shots {
    /// Starts reading each `(key, location)` not asked for before: an
    /// `https` URL, or a file; `wake` asks for the frame that shows it.
    pub fn want(&mut self, wanted: impl IntoIterator<Item = (String, String)>, wake: &Wake) {
        for (key, at) in wanted {
            if self.shots.contains_key(&key) {
                continue;
            }
            self.shots.insert(key.clone(), Shot::Loading);
            spawn(key, at, self.tx.clone(), wake.clone());
        }
    }

    /// Registers the pictures that arrived since the last frame.
    pub fn poll(&mut self, ui: &mut Ui) {
        while let Ok((key, bytes)) = self.rx.try_recv() {
            let image = bytes
                .and_then(|b| decode_png(&b).ok())
                .and_then(|(w, h, rgba)| ui.image(w, h, &rgba));
            self.shots
                .insert(key, image.map_or(Shot::Failed, Shot::Ready));
        }
    }

    /// The image of screenshot `key`, once it has arrived.
    pub fn get(&self, key: &str) -> Option<u32> {
        match self.shots.get(key)? {
            Shot::Ready(id) => Some(*id),
            Shot::Loading | Shot::Failed => None,
        }
    }
}

/// Screenshot `shot` of `l` as (cache key, where it is read from); the key
/// names the extension, so two packs' `shots/a.png` stay apart.
pub fn resolve(l: &Listing, shot: &str) -> Option<(String, String)> {
    let root = l.pack.as_ref().and_then(|p| p.get("path")?.as_str());
    let at = location(shot, root)?;
    Some((format!("{}\n{at}", l.id), at))
}

/// Where a manifest's screenshot is read from: a URL as given, or a file
/// inside the pack at `root`; `None` for plain http or a path leaving the pack.
pub fn location(shot: &str, root: Option<&str>) -> Option<String> {
    if shot.starts_with("https://") {
        return Some(shot.to_owned());
    }
    if shot.contains("://") {
        return None;
    }
    let rel = std::path::Path::new(shot);
    let inside = rel
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)));
    let root = root.filter(|r| !r.is_empty())?;
    inside.then(|| {
        std::path::Path::new(root)
            .join(rel)
            .to_string_lossy()
            .into_owned()
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn(key: String, at: String, tx: Sender<(String, Option<Vec<u8>>)>, wake: Wake) {
    let run = move || {
        let _ = tx.send((key, read(&at)));
        wake();
    };
    if let Err(e) = std::thread::Builder::new()
        .name("vorn-screenshot".into())
        .spawn(run)
    {
        eprintln!("vorn-screenshot did not start: {e}");
    }
}

#[cfg(target_arch = "wasm32")]
fn spawn(key: String, _at: String, tx: Sender<(String, Option<Vec<u8>>)>, _wake: Wake) {
    // The browser client draws the placeholder.
    let _ = tx.send((key, None));
}

#[cfg(not(target_arch = "wasm32"))]
fn read(at: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    if !at.starts_with("https://") {
        let meta = std::fs::metadata(at).ok()?;
        return (meta.len() <= MAX_BYTES)
            .then(|| std::fs::read(at).ok())
            .flatten();
    }
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .user_agent(concat!("Vorn/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent();
    let mut response = agent.get(at).call().ok()?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::{location, resolve};
    use crate::plugins::Listing;
    use serde_json::json;

    #[test]
    fn a_screenshot_is_a_url_or_a_file_inside_its_pack() {
        assert_eq!(
            location("https://x.dev/a.png", None).as_deref(),
            Some("https://x.dev/a.png")
        );
        assert_eq!(
            location("shots/a.png", Some("/p")).as_deref(),
            Some("/p/shots/a.png")
        );
        assert_eq!(location("../a.png", Some("/p")), None);
        assert_eq!(location("/etc/a.png", Some("/p")), None);
        assert_eq!(location("http://x.dev/a.png", Some("/p")), None);
        assert_eq!(location("a.png", None), None);
    }

    #[test]
    fn two_packs_with_the_same_screenshot_path_keep_their_own() {
        let pack = |id: &str| -> Listing {
            let mut p = crate::plugins::Plugins {
                packs: vec![
                    json!({ "id": id, "name": id, "version": "1.0.0", "path": format!("/packs/{id}") }),
                ],
                ..Default::default()
            };
            p.rebuild();
            p.listings.remove(0)
        };
        let (a, at_a) = resolve(&pack("a"), "shots/board.png").unwrap();
        let (b, at_b) = resolve(&pack("b"), "shots/board.png").unwrap();
        assert_ne!(a, b);
        assert_eq!(at_a, "/packs/a/shots/board.png");
        assert_eq!(at_b, "/packs/b/shots/board.png");
        let (c, _) = resolve(&pack("a"), "https://x.dev/a.png").unwrap();
        let (d, _) = resolve(&pack("b"), "https://x.dev/a.png").unwrap();
        assert_ne!(c, d);
    }
}
