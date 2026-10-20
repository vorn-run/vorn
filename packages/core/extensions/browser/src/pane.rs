//! A session's browser pane: its tabs, each with the address it was asked
//! for and what its page last said about itself, and which is in front.

use serde::{Deserialize, Serialize};
use vorn_extension_sdk::{json, Value};

use crate::address::{display_host, DEFAULT_URL};

/// The artifact a tab shows, when it shows one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabArtifact {
    pub id: String,
    /// `page`, `doc` or `design`.
    pub kind: String,
    pub version: u32,
    pub title: String,
}

/// One tab: the address it was asked for, and what its page reported.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tab {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The title a design page declares for itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub design: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<TabArtifact>,
}

impl Tab {
    pub fn at(url: impl Into<String>) -> Tab {
        Tab {
            url: url.into(),
            ..Tab::default()
        }
    }

    /// Where the tab is: its page's last report, else where it was sent.
    pub fn current(&self) -> &str {
        self.live_url.as_deref().unwrap_or(&self.url)
    }

    /// What its strip calls it: the artifact's title, the design's, else
    /// the host it shows.
    pub fn label(&self) -> String {
        let named = |t: &Option<String>| t.clone().filter(|t| !t.is_empty());
        named(&self.artifact.as_ref().map(|a| a.title.clone()))
            .or_else(|| named(&self.design))
            .unwrap_or_else(|| display_host(self.current()))
    }

    /// The kit icon before its label: what kind of page it shows.
    pub fn icon(&self) -> Option<&'static str> {
        match self.artifact.as_ref().map(|a| a.kind.as_str()) {
            Some("doc") => Some("align-left"),
            Some("design") => Some("shapes"),
            Some(_) => Some("file-text"),
            None if self.design.is_some() => Some("shapes"),
            None => None,
        }
    }

    /// What the page in it reported. A new address drops the old page's
    /// title and design. False when nothing changed.
    pub fn sync(
        &mut self,
        url: Option<String>,
        title: Option<String>,
        design: Option<String>,
    ) -> bool {
        let moved_on = url.as_deref().is_some_and(|u| u != self.current());
        let live_url = url
            .filter(|u| !u.is_empty())
            .or_else(|| self.live_url.clone());
        let title = title
            .filter(|t| !t.is_empty())
            .or_else(|| (!moved_on).then(|| self.title.clone()).flatten());
        let design = design.filter(|t| !t.is_empty());
        if live_url == self.live_url && title == self.title && design == self.design {
            return false;
        }
        self.live_url = live_url;
        self.title = title;
        self.design = design;
        true
    }
}

/// A session's browser pane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pane {
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
}

impl Default for Pane {
    fn default() -> Pane {
        Pane::new(Tab::at(DEFAULT_URL))
    }
}

impl Pane {
    pub fn new(tab: Tab) -> Pane {
        Pane {
            tabs: vec![tab],
            active_tab: 0,
        }
    }

    pub fn active(&self) -> Option<&Tab> {
        self.tabs.get(self.active_tab)
    }

    pub fn active_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active_tab)
    }

    /// Only what was asked for: what a page reported is seen again once it
    /// shows, and the front tab is one that exists.
    pub fn intent(mut self) -> Pane {
        for t in &mut self.tabs {
            t.live_url = None;
            t.title = None;
            t.design = None;
        }
        self.active_tab = self.active_tab.min(self.tabs.len().saturating_sub(1));
        self
    }

    /// The front tab goes to `url`, as a page of its own.
    pub fn go(&mut self, url: String) {
        match self.active_mut() {
            Some(t) => *t = Tab::at(url),
            None => *self = Pane::new(Tab::at(url)),
        }
    }

    /// Adds a tab in front, at `url` or blank.
    pub fn add_tab(&mut self, url: Option<String>) {
        self.tabs
            .push(Tab::at(url.unwrap_or_else(|| DEFAULT_URL.to_owned())));
        self.active_tab = self.tabs.len() - 1;
    }

    /// Closes tab `index`, its neighbour coming to the front. `None` when
    /// it was the last: the pane goes with it.
    pub fn close_tab(mut self, index: usize) -> Option<Pane> {
        if index >= self.tabs.len() {
            return Some(self);
        }
        if self.tabs.len() == 1 {
            return None;
        }
        self.tabs.remove(index);
        let active = if index <= self.active_tab {
            self.active_tab.saturating_sub(1)
        } else {
            self.active_tab
        };
        self.active_tab = active.min(self.tabs.len() - 1);
        Some(self)
    }

    /// Brings tab `index` to the front. False when nothing changed.
    pub fn select_tab(&mut self, index: usize) -> bool {
        if index < self.tabs.len() && self.active_tab != index {
            self.active_tab = index;
            return true;
        }
        false
    }

    /// The artifact's tab is replaced in place, or added, and comes to the front.
    pub fn open_artifact(&mut self, url: String, artifact: TabArtifact) {
        let at = self
            .tabs
            .iter()
            .position(|t| t.artifact.as_ref().is_some_and(|a| a.id == artifact.id));
        let tab = Tab {
            url,
            artifact: Some(artifact),
            ..Tab::default()
        };
        match at {
            Some(i) => {
                self.tabs[i] = tab;
                self.active_tab = i;
            }
            None => {
                self.tabs.push(tab);
                self.active_tab = self.tabs.len() - 1;
            }
        }
    }

    /// The tabs as the agent's `list_tabs` reads them.
    pub fn tab_infos(&self) -> Vec<Value> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let mut v = json!({
                    "index": i,
                    "url": t.current(),
                    "active": i == self.active_tab,
                });
                if let Some(title) = &t.title {
                    v["title"] = title.clone().into();
                }
                v
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(urls: &[&str], active: usize) -> Pane {
        Pane {
            tabs: urls.iter().map(|u| Tab::at(*u)).collect(),
            active_tab: active,
        }
    }

    fn art(id: &str, version: u32) -> TabArtifact {
        TabArtifact {
            id: id.to_owned(),
            kind: "doc".to_owned(),
            version,
            title: format!("Doc {id}"),
        }
    }

    #[test]
    fn closing_tabs_keeps_the_neighbour_and_the_last_takes_the_pane() {
        let p = pane(&["a", "b", "c"], 2);
        let p = p.close_tab(2).expect("two left");
        assert_eq!((p.tabs.len(), p.active_tab), (2, 1));
        let p = p.close_tab(0).expect("one left");
        assert_eq!((p.tabs[0].url.as_str(), p.active_tab), ("b", 0));
        let p = p.close_tab(5).expect("a tab not there changes nothing");
        assert_eq!(p.tabs.len(), 1);
        assert_eq!(p.close_tab(0), None);
    }

    #[test]
    fn a_new_page_drops_the_old_title() {
        let mut t = Tab::at("https://a.test/");
        assert!(t.sync(None, Some("A".into()), None));
        t.sync(Some("https://a.test/".into()), None, None);
        assert_eq!(
            t.title.as_deref(),
            Some("A"),
            "the same page keeps its title"
        );
        assert!(t.sync(Some("https://b.test/".into()), None, None));
        assert_eq!((t.current(), t.title.as_deref()), ("https://b.test/", None));
        assert_eq!(t.url, "https://a.test/", "what was asked for stays");
    }

    #[test]
    fn an_artifact_reopens_in_its_own_tab() {
        let mut p = pane(&["https://a.test/"], 0);
        p.open_artifact("http://h/artifact/x/1".into(), art("x", 1));
        assert_eq!((p.tabs.len(), p.active_tab), (2, 1));
        p.select_tab(0);
        p.open_artifact("http://h/artifact/x/2".into(), art("x", 2));
        assert_eq!((p.tabs.len(), p.active_tab), (2, 1));
        assert_eq!(p.tabs[1].artifact.as_ref().map(|a| a.version), Some(2));
        assert_eq!(p.tabs[1].label(), "Doc x");
        assert_eq!(p.tabs[1].icon(), Some("align-left"));
    }

    #[test]
    fn saving_keeps_intent_only() {
        let mut p = pane(&["https://a.test/"], 4);
        p.tabs[0].sync(
            Some("https://a.test/x".into()),
            Some("X".into()),
            Some("D".into()),
        );
        let p = p.intent();
        assert_eq!(p.tabs[0], Tab::at("https://a.test/"));
        assert_eq!(p.active_tab, 0);
    }

    #[test]
    fn a_tab_is_named_by_what_it_shows() {
        let mut t = Tab::at(DEFAULT_URL);
        assert_eq!((t.label().as_str(), t.icon()), ("New tab", None));
        t.sync(
            Some("http://localhost:3000/a".into()),
            Some("App".into()),
            None,
        );
        assert_eq!(t.label(), "localhost:3000");
        t.sync(None, None, Some("Landing".into()));
        assert_eq!((t.label().as_str(), t.icon()), ("Landing", Some("shapes")));
    }
}
