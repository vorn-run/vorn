//! What a person arranges by dragging and switching: the toolbar's items,
//! the sidebar's tabs and a card's panes, each an order and the ones hidden.
//! They outlive a launch in `desktop/arrange.json`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One list's order and hidden items, by id. An item the order does not
/// name yet (an extension added since) goes last, shown.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Arrangement {
    pub order: Vec<String>,
    pub hidden: BTreeSet<String>,
}

impl Arrangement {
    /// Every item of `available` in the person's order, hidden ones too.
    pub fn all<'a>(&self, available: &[&'a str]) -> Vec<&'a str> {
        let mut out: Vec<&'a str> = self
            .order
            .iter()
            .filter_map(|id| available.iter().copied().find(|a| *a == id))
            .collect();
        let rest: Vec<&'a str> = available
            .iter()
            .copied()
            .filter(|a| !out.contains(a))
            .collect();
        out.extend(rest);
        out
    }

    /// The items of `available` that show, in order.
    pub fn shown<'a>(&self, available: &[&'a str]) -> Vec<&'a str> {
        let mut out = self.all(available);
        out.retain(|a| !self.hidden.contains(*a));
        out
    }

    /// What Customize lists: the shown items in order, then the hidden ones.
    pub fn display<'a>(&self, available: &[&'a str]) -> Vec<&'a str> {
        let (mut shown, hidden): (Vec<&'a str>, Vec<&'a str>) = self
            .all(available)
            .into_iter()
            .partition(|a| !self.hidden.contains(*a));
        shown.extend(hidden);
        shown
    }

    /// Moves `id` to where the item at place `to` of [`Arrangement::display`] is.
    pub fn move_shown(&mut self, available: &[&str], id: &str, to: usize) {
        let list = self.display(available);
        // A shown item moves among the shown, never into the hidden below them.
        let room = if self.is_shown(id) {
            list.iter().filter(|a| self.is_shown(a)).count()
        } else {
            list.len()
        };
        let Some(target) = list.get(to.min(room.saturating_sub(1))).copied() else {
            return;
        };
        if target == id {
            return;
        }
        if let Some(at) = self.all(available).iter().position(|a| *a == target) {
            self.move_to(available, id, at);
        }
    }

    pub fn is_shown(&self, id: &str) -> bool {
        !self.hidden.contains(id)
    }

    /// Moves `id` to place `to` among `available`; saved ids not there now
    /// (an extension switched off) are kept after them for when they return.
    pub fn move_to(&mut self, available: &[&str], id: &str, to: usize) {
        let mut all: Vec<String> = self.all(available).into_iter().map(str::to_owned).collect();
        let Some(from) = all.iter().position(|a| a == id) else {
            return;
        };
        let item = all.remove(from);
        all.insert(to.min(all.len()), item);
        let away: Vec<String> = (self.order.iter())
            .filter(|o| !available.contains(&o.as_str()))
            .cloned()
            .collect();
        all.extend(away);
        self.order = all;
    }

    /// Shows or hides `id`.
    pub fn set_shown(&mut self, id: &str, shown: bool) {
        if shown {
            self.hidden.remove(id);
        } else {
            self.hidden.insert(id.to_owned());
        }
    }
}

/// Which list an arrangement is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum List {
    #[default]
    Toolbar,
    Tabs,
    /// A card's buttons.
    Card,
}

/// The arrangements together, as they are saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub toolbar: Arrangement,
    pub tabs: Arrangement,
    pub card: Arrangement,
    /// How many of a card's buttons its header shows; the rest go into More.
    pub card_shown: usize,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs {
            // Split waits in the customize sheet until someone wants it in the bar.
            toolbar: Arrangement {
                order: Vec::new(),
                hidden: [crate::toolbar::SPLIT.to_owned()].into(),
            },
            tabs: Arrangement::default(),
            card: Arrangement::default(),
            card_shown: crate::screen::card_buttons::DEFAULT_SHOWN,
        }
    }
}

impl Prefs {
    pub fn get(&self, list: List) -> &Arrangement {
        match list {
            List::Toolbar => &self.toolbar,
            List::Tabs => &self.tabs,
            List::Card => &self.card,
        }
    }

    pub fn get_mut(&mut self, list: List) -> &mut Arrangement {
        match list {
            List::Toolbar => &mut self.toolbar,
            List::Tabs => &mut self.tabs,
            List::Card => &mut self.card,
        }
    }

    /// The arrangements saved in `file`, else the defaults; a top bar page kept under its old `views` key is read as a page.
    pub fn load(file: &Path) -> Prefs {
        let mut prefs: Prefs = crate::keep::read(file)
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let page = |k: &String| k.replacen("/views/", "/pages/", 1);
        prefs.tabs.order = prefs.tabs.order.iter().map(page).collect();
        prefs.tabs.hidden = prefs.tabs.hidden.iter().map(page).collect();
        prefs
    }

    /// Writes them down; a failure costs only their survival past this launch.
    pub fn save(&self, file: &Path) {
        if let Ok(s) = serde_json::to_string(self) {
            let _ = crate::keep::write(file, &s);
        }
    }
}

/// Where the arrangements live beside `data_dir`'s vornd.
pub fn file(data_dir: &Path) -> PathBuf {
    data_dir.join("desktop").join("arrange.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_saved_order_leads_and_new_items_join_at_the_end() {
        let a = Arrangement {
            order: vec!["new".into(), "gone".into(), "view".into()],
            hidden: BTreeSet::new(),
        };
        assert_eq!(a.all(&["view", "recent", "new"]), ["new", "view", "recent"]);
    }

    #[test]
    fn moving_and_hiding_keep_the_rest_in_place() {
        let mut a = Arrangement::default();
        let avail = ["view", "recent", "git", "new"];
        a.move_to(&avail, "new", 0);
        assert_eq!(a.all(&avail), ["new", "view", "recent", "git"]);
        a.move_to(&avail, "view", 9);
        assert_eq!(a.all(&avail), ["new", "recent", "git", "view"]);
        a.set_shown("git", false);
        assert_eq!(a.shown(&avail), ["new", "recent", "view"]);
        assert_eq!(a.all(&avail).len(), 4, "a hidden item keeps its place");
        a.set_shown("git", true);
        assert_eq!(a.shown(&avail), ["new", "recent", "git", "view"]);
        a.move_to(&avail, "missing", 0);
        assert_eq!(a.all(&avail), ["new", "recent", "git", "view"]);
    }

    #[test]
    fn customize_lists_the_shown_first_and_moves_by_their_places() {
        let mut a = Arrangement::default();
        let avail = ["view", "recent", "split", "new"];
        a.set_shown("recent", false);
        assert_eq!(a.display(&avail), ["view", "split", "new", "recent"]);
        a.move_shown(&avail, "new", 0);
        assert_eq!(a.display(&avail), ["new", "view", "split", "recent"]);
        a.move_shown(&avail, "view", 2);
        assert_eq!(a.display(&avail), ["new", "split", "view", "recent"]);
        a.move_shown(&avail, "view", 99);
        assert_eq!(
            a.shown(&avail),
            ["new", "split", "view"],
            "past the end stays shown"
        );
    }

    #[test]
    fn ids_that_come_and_go_keep_their_place_and_new_ones_join_shown() {
        let mut a = Arrangement::default();
        let with = ["terminals", "acme/panes/notes", "vorn.files/panes/files"];
        a.move_to(&with, "vorn.files/panes/files", 0);
        a.set_shown("acme/panes/notes", false);
        // The extension goes: its id is skipped, not forgotten.
        let without = ["terminals", "vorn.files/panes/files"];
        assert_eq!(a.shown(&without), ["vorn.files/panes/files", "terminals"]);
        a.move_to(&without, "terminals", 0);
        assert!(
            a.order.iter().any(|o| o == "acme/panes/notes"),
            "{:?}",
            a.order
        );
        // It comes back where it was left, still hidden; a new one joins at the end, shown.
        let back = [
            "terminals",
            "acme/panes/notes",
            "vorn.files/panes/files",
            "x/panes/new",
        ];
        assert_eq!(
            a.all(&back),
            [
                "terminals",
                "vorn.files/panes/files",
                "acme/panes/notes",
                "x/panes/new"
            ]
        );
        assert_eq!(
            a.shown(&back),
            ["terminals", "vorn.files/panes/files", "x/panes/new"]
        );
    }

    #[test]
    fn the_arrangements_round_trip_through_their_file() {
        let dir = std::env::temp_dir().join(format!("vorn-arrange-{}", std::process::id()));
        let f = file(&dir);
        assert_eq!(Prefs::load(&f), Prefs::default());
        assert!(!Prefs::default().toolbar.is_shown(crate::toolbar::SPLIT));
        let mut p = Prefs::default();
        p.toolbar.move_to(&["view", "new"], "new", 0);
        p.tabs.set_shown("vorn.tasks/pages/tasks", false);
        p.card.move_to(
            &["terminals", "vorn.browser/panes/browser"],
            "vorn.browser/panes/browser",
            0,
        );
        p.save(&f);
        assert_eq!(Prefs::load(&f), p);
        let mut old = p.clone();
        old.tabs.set_shown("vorn.tasks/pages/tasks", true);
        old.tabs.set_shown("vorn.tasks/views/tasks", false);
        old.save(&f);
        assert_eq!(
            Prefs::load(&f),
            p,
            "a page kept under its old key is still hidden"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
