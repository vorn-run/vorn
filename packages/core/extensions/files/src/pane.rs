//! A session's Files pane as it is kept: its tabs, the one in front, the
//! preview a single click opened, and whether the tree shows; and the files
//! popped out of a session into cards of their own.

use serde::{Deserialize, Serialize};

/// A session's Files pane.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pane {
    #[serde(default)]
    pub tabs: Vec<String>,
    #[serde(default)]
    pub active: Option<String>,
    /// The tab a single click opened, replaced by the next one until it is
    /// kept or typed in.
    #[serde(default)]
    pub preview: Option<String>,
    #[serde(default = "yes")]
    pub tree_visible: bool,
}

fn yes() -> bool {
    true
}

impl Default for Pane {
    fn default() -> Pane {
        Pane {
            tabs: Vec::new(),
            active: None,
            preview: None,
            tree_visible: true,
        }
    }
}

impl Pane {
    /// A single click previews, replacing the last preview unless that one
    /// has edits; `pin` keeps the tab.
    pub fn open(&mut self, path: &str, pin: bool, preview_dirty: bool) {
        if self.tabs.iter().any(|t| t == path) {
            if pin && self.preview.as_deref() == Some(path) {
                self.preview = None;
            }
        } else if pin {
            self.tabs.push(path.to_owned());
        } else {
            match self.preview.clone().filter(|_| !preview_dirty) {
                Some(old) => {
                    for t in &mut self.tabs {
                        if *t == old {
                            *t = path.to_owned();
                        }
                    }
                }
                None => self.tabs.push(path.to_owned()),
            }
            self.preview = Some(path.to_owned());
        }
        self.active = Some(path.to_owned());
    }

    /// Keeps a preview tab.
    pub fn pin(&mut self, path: &str) {
        if self.preview.as_deref() == Some(path) {
            self.preview = None;
        }
    }

    /// Brings a tab to the front.
    pub fn show(&mut self, path: &str) {
        if self.tabs.iter().any(|t| t == path) {
            self.active = Some(path.to_owned());
        }
    }

    /// Closes a tab; the next one along takes the front.
    pub fn close(&mut self, path: &str) {
        let Some(i) = self.tabs.iter().position(|t| t == path) else {
            return;
        };
        self.tabs.remove(i);
        if self.active.as_deref() == Some(path) {
            self.active = self
                .tabs
                .get(i)
                .or_else(|| i.checked_sub(1).and_then(|j| self.tabs.get(j)))
                .cloned();
        }
        if self.preview.as_deref() == Some(path) {
            self.preview = None;
        }
    }

    /// Only tabs with edits stay.
    pub fn close_saved(&mut self, dirty: impl Fn(&str) -> bool) {
        self.tabs.retain(|t| dirty(t));
        if self.active.as_ref().is_some_and(|a| !self.tabs.contains(a)) {
            self.active = self.tabs.first().cloned();
        }
        if self
            .preview
            .as_ref()
            .is_some_and(|a| !self.tabs.contains(a))
        {
            self.preview = None;
        }
    }

    /// Whether the tree shows: always while no file is open.
    pub fn tree_shown(&self) -> bool {
        self.tabs.is_empty() || self.tree_visible
    }
}

/// A file popped out of a session into a card of its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub id: String,
    pub session: String,
    pub path: String,
}

/// Every card, and the number the next one takes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cards {
    #[serde(default)]
    pub all: Vec<Card>,
    #[serde(default)]
    pub next: u32,
}

impl Cards {
    /// One card per file per session: an existing one is answered rather than a second made.
    pub fn promote(&mut self, session: &str, path: &str) -> String {
        if let Some(c) = self
            .all
            .iter()
            .find(|c| c.session == session && c.path == path)
        {
            return c.id.clone();
        }
        self.next += 1;
        let id = format!("card:{session}:{}", self.next);
        self.all.push(Card {
            id: id.clone(),
            session: session.to_owned(),
            path: path.to_owned(),
        });
        id
    }

    pub fn get(&self, id: &str) -> Option<&Card> {
        self.all.iter().find(|c| c.id == id)
    }

    pub fn remove(&mut self, id: &str) -> Option<Card> {
        let i = self.all.iter().position(|c| c.id == id)?;
        Some(self.all.remove(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preview_tab_is_replaced_until_pinned_or_edited() {
        let mut p = Pane::default();
        p.open("/a", false, false);
        p.open("/b", false, false);
        assert_eq!(p.tabs, ["/b"]);
        p.open("/c", false, true);
        assert_eq!(p.tabs, ["/b", "/c"]);
        p.pin("/c");
        p.open("/d", true, false);
        assert_eq!(p.tabs, ["/b", "/c", "/d"]);
        assert_eq!(p.active.as_deref(), Some("/d"));
        p.close("/d");
        assert_eq!(p.active.as_deref(), Some("/c"));
        p.close_saved(|t| t == "/b");
        assert_eq!(p.tabs, ["/b"]);
        assert_eq!(p.active.as_deref(), Some("/b"));
    }

    #[test]
    fn the_tree_shows_while_no_file_is_open() {
        let mut p = Pane {
            tree_visible: false,
            ..Pane::default()
        };
        assert!(p.tree_shown());
        p.open("/a", true, false);
        assert!(!p.tree_shown());
    }

    #[test]
    fn a_file_has_one_card_per_session() {
        let mut c = Cards::default();
        let a = c.promote("s", "/x");
        assert_eq!(c.promote("s", "/x"), a);
        let b = c.promote("s", "/y");
        assert_ne!(a, b);
        assert_ne!(c.promote("t", "/x"), a);
        assert_eq!(c.remove(&a).map(|c| c.path), Some("/x".into()));
        assert!(c.get(&a).is_none());
    }
}
