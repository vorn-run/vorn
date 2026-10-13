//! A session's browser pane, as the renderer's store keeps it
//! (`ui-slice.ts`): tabs with the address each was asked for and what the
//! page last said about itself, the address bar's rules
//! (`shared/browser-url.ts`), and the panes saved between runs.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use url::Url;

/// What a new tab shows.
pub const DEFAULT_URL: &str = "about:blank";

/// Names that read as a scheme before a port, never as a host.
const SCHEME_NAMES: [&str; 7] = [
    "javascript",
    "file",
    "data",
    "blob",
    "vbscript",
    "about",
    "chrome",
];

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
}

/// A session's browser pane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pane {
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
}

impl Pane {
    fn new(tab: Tab) -> Pane {
        Pane {
            tabs: vec![tab],
            active_tab: 0,
        }
    }

    pub fn active(&self) -> Option<&Tab> {
        self.tabs.get(self.active_tab)
    }
}

/// Every session's pane, and the tabs of the ones closed, for reopening.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Panes {
    #[serde(default)]
    pub open: HashMap<String, Pane>,
    #[serde(default)]
    pub memory: HashMap<String, Pane>,
    /// A design's controls as last set, by the file it was loaded from.
    #[serde(default)]
    pub tweaks: HashMap<String, serde_json::Map<String, serde_json::Value>>,
}

impl Panes {
    /// The panes saved in `path`; none when it is missing or unreadable.
    pub fn load(path: &Path) -> Panes {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Panes>(&b).ok())
            .map(Panes::intent)
            .unwrap_or_default()
    }

    /// Writes the panes to `path`. As the renderer's `savePanes`, only what
    /// was asked for is kept: what a page reported is seen again next run.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let body = serde_json::to_vec(&self.clone().intent()).map_err(std::io::Error::other)?;
        std::fs::write(path, body)
    }

    fn intent(mut self) -> Panes {
        for pane in self.open.values_mut().chain(self.memory.values_mut()) {
            for t in &mut pane.tabs {
                t.live_url = None;
                t.title = None;
            }
            pane.active_tab = pane.active_tab.min(pane.tabs.len().saturating_sub(1));
        }
        self.open.retain(|_, p| !p.tabs.is_empty());
        self
    }

    pub fn get(&self, session: &str) -> Option<&Pane> {
        self.open.get(session)
    }

    /// `openBrowserPane`: with a vetted `url`, the active tab goes there;
    /// without, the pane opens as it was left, or blank. False when nothing
    /// changed.
    pub fn open(&mut self, session: &str, url: Option<String>) -> bool {
        match (url, self.open.get_mut(session)) {
            (Some(url), Some(pane)) => {
                let at = pane.active_tab;
                if let Some(t) = pane.tabs.get_mut(at) {
                    *t = Tab::at(url);
                }
            }
            (Some(url), None) => {
                self.open
                    .insert(session.to_owned(), Pane::new(Tab::at(url)));
            }
            (None, None) => {
                let pane = self
                    .memory
                    .get(session)
                    .cloned()
                    .unwrap_or_else(|| Pane::new(Tab::at(DEFAULT_URL)));
                self.open.insert(session.to_owned(), pane);
            }
            (None, Some(_)) => return false,
        }
        true
    }

    /// `closeBrowserPane`: its tabs are kept for the next open.
    pub fn close(&mut self, session: &str) -> bool {
        match self.open.remove(session) {
            Some(pane) => {
                self.memory.insert(session.to_owned(), pane);
                true
            }
            None => false,
        }
    }

    pub fn toggle(&mut self, session: &str) {
        if !self.close(session) {
            self.open(session, None);
        }
    }

    /// `addBrowserTab`, with a vetted url or a blank tab.
    pub fn add_tab(&mut self, session: &str, url: Option<String>) -> bool {
        let Some(pane) = self.open.get_mut(session) else {
            return false;
        };
        pane.tabs
            .push(Tab::at(url.unwrap_or_else(|| DEFAULT_URL.to_owned())));
        pane.active_tab = pane.tabs.len() - 1;
        true
    }

    /// `closeBrowserTab`: the last tab takes the pane with it, and leaves
    /// nothing to reopen.
    pub fn close_tab(&mut self, session: &str, index: usize) -> bool {
        let Some(pane) = self.open.get_mut(session) else {
            return false;
        };
        if index >= pane.tabs.len() {
            return false;
        }
        if pane.tabs.len() == 1 {
            self.open.remove(session);
            self.memory.remove(session);
            return true;
        }
        pane.tabs.remove(index);
        let active = if index <= pane.active_tab {
            pane.active_tab.saturating_sub(1)
        } else {
            pane.active_tab
        };
        pane.active_tab = active.min(pane.tabs.len() - 1);
        true
    }

    /// `setActiveBrowserTab`.
    pub fn select_tab(&mut self, session: &str, index: usize) -> bool {
        match self.open.get_mut(session) {
            Some(pane) if index < pane.tabs.len() && pane.active_tab != index => {
                pane.active_tab = index;
                true
            }
            _ => false,
        }
    }

    /// `syncBrowserTab`: what the page in tab `index` reported. A new
    /// address drops the old page's title.
    pub fn sync_tab(
        &mut self,
        session: &str,
        index: usize,
        url: Option<String>,
        title: Option<String>,
    ) -> bool {
        let Some(tab) = self
            .open
            .get_mut(session)
            .and_then(|p| p.tabs.get_mut(index))
        else {
            return false;
        };
        let moved_on = url.as_deref().is_some_and(|u| u != tab.current());
        let live_url = url.or_else(|| tab.live_url.clone());
        let title = title
            .filter(|t| !t.is_empty())
            .or_else(|| (!moved_on).then(|| tab.title.clone()).flatten());
        if live_url == tab.live_url && title == tab.title {
            return false;
        }
        tab.live_url = live_url.filter(|u| !u.is_empty());
        tab.title = title;
        true
    }

    /// `openArtifactTab`: the artifact's tab is replaced in place, or added.
    pub fn open_artifact(&mut self, session: &str, url: String, artifact: TabArtifact) {
        let mut pane = self
            .open
            .remove(session)
            .or_else(|| self.memory.get(session).cloned());
        let tab = Tab {
            url,
            artifact: Some(artifact),
            ..Tab::default()
        };
        let pane = match pane.as_mut() {
            None => Pane::new(tab),
            Some(p) => {
                let id = tab.artifact.as_ref().map(|a| a.id.clone());
                let at = p
                    .tabs
                    .iter()
                    .position(|t| t.artifact.as_ref().map(|a| &a.id) == id.as_ref());
                match at {
                    Some(i) => {
                        p.tabs[i] = tab;
                        p.active_tab = i;
                    }
                    None => {
                        p.tabs.push(tab);
                        p.active_tab = p.tabs.len() - 1;
                    }
                }
                p.clone()
            }
        };
        self.open.insert(session.to_owned(), pane);
    }

    /// `setArtifactTabVersion`.
    pub fn set_artifact_version(&mut self, session: &str, index: usize, url: String, version: u32) {
        let Some(tab) = self
            .open
            .get_mut(session)
            .and_then(|p| p.tabs.get_mut(index))
        else {
            return;
        };
        let Some(artifact) = tab.artifact.clone() else {
            return;
        };
        *tab = Tab {
            url,
            artifact: Some(TabArtifact {
                version,
                ..artifact
            }),
            ..Tab::default()
        };
    }

    /// The tabs as the agent's `list_tabs` reads them.
    pub fn tab_infos(&self, session: &str) -> Vec<serde_json::Value> {
        let Some(pane) = self.open.get(session) else {
            return Vec::new();
        };
        pane.tabs
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let mut v = serde_json::json!({
                    "index": i,
                    "url": t.current(),
                    "active": i == pane.active_tab,
                });
                if let Some(title) = &t.title {
                    v["title"] = title.clone().into();
                }
                v
            })
            .collect()
    }
}

/// Where the address bar sends text that is not a web address.
const SEARCH_URL: &str = "https://www.google.com/search";

/// What the address bar opens for what was typed: the web address it names,
/// or a search for words that name none. `None` for an address it will not load.
pub fn address_target(input: &str) -> Option<String> {
    let raw = input.trim();
    if raw.is_empty() {
        return None;
    }
    if looks_like_address(raw) {
        return normalize_url(raw, false);
    }
    let mut url = Url::parse(SEARCH_URL).ok()?;
    url.query_pairs_mut().append_pair("q", raw);
    Some(url.to_string())
}

/// A scheme, a port, a dotted host or `localhost`; a lone word or a phrase is a search.
fn looks_like_address(raw: &str) -> bool {
    if raw.chars().any(char::is_whitespace) {
        return false;
    }
    if host_and_port(raw) || has_scheme(raw) || raw.starts_with("//") {
        return true;
    }
    let host = raw.split(['/', '?', '#']).next().unwrap_or_default();
    host.contains('.') || host.eq_ignore_ascii_case("localhost") || host.starts_with('[')
}

/// `normalizeUrl`: what the address bar loads for what was typed, or `None`
/// for something it will not load. `file:` only with `allow_file`, with no
/// host, and without its query and hash.
pub fn normalize_url(input: &str, allow_file: bool) -> Option<String> {
    let raw = input.trim();
    if raw.is_empty() {
        return None;
    }
    if host_and_port(raw) {
        let name = raw[..raw.find(':')?].to_ascii_lowercase();
        return if SCHEME_NAMES.contains(&name.as_str()) {
            None
        } else {
            build_url(raw)
        };
    }
    if has_scheme(raw) {
        let mut parsed = Url::parse(raw).ok()?;
        return match parsed.scheme() {
            "about" => (raw == DEFAULT_URL).then(|| DEFAULT_URL.to_owned()),
            "file" => {
                if !allow_file || parsed.host_str().is_some_and(|h| !h.is_empty()) {
                    return None;
                }
                parsed.set_query(None);
                parsed.set_fragment(None);
                Some(parsed.to_string())
            }
            "http" | "https" => Some(parsed.to_string()),
            _ => None,
        };
    }
    build_url(raw.strip_prefix("//").unwrap_or(raw))
}

/// `/^[a-z0-9.-]+:\d+(\/.*)?$/i`: `localhost:3000`, which a URL parser
/// would read as the scheme `localhost`.
fn host_and_port(raw: &str) -> bool {
    let Some((name, rest)) = raw.split_once(':') else {
        return false;
    };
    let digits = rest.find('/').map_or(rest, |i| &rest[..i]);
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && !rest[digits.len()..].contains('\n')
}

/// `/^[a-z][a-z0-9+.-]*:/i`.
fn has_scheme(raw: &str) -> bool {
    let Some(i) = raw.find(':') else {
        return false;
    };
    let name = &raw.as_bytes()[..i];
    name.first().is_some_and(u8::is_ascii_alphabetic)
        && name
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || b == b'+' || b == b'.' || b == b'-')
}

fn build_url(host_and_path: &str) -> Option<String> {
    let scheme = if is_local_host(host_and_path) {
        "http://"
    } else {
        "https://"
    };
    let parsed = Url::parse(&format!("{scheme}{host_and_path}")).ok()?;
    parsed.host_str().filter(|h| !h.is_empty())?;
    Some(parsed.to_string())
}

fn is_local_host(value: &str) -> bool {
    let authority = value.split('/').next().unwrap_or_default();
    let host = if authority.starts_with('[') {
        authority[..authority.find(']').map_or(0, |i| i + 1)].to_ascii_lowercase()
    } else {
        authority
            .split(':')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase()
    };
    matches!(
        host.as_str(),
        "localhost" | "127.0.0.1" | "0.0.0.0" | "[::1]"
    ) || host.ends_with(".local")
        || host.ends_with(".localhost")
}

/// `displayHost`: what a tab is called before its page has a title.
pub fn display_host(url: &str) -> String {
    if url == DEFAULT_URL {
        return "New tab".to_owned();
    }
    let Ok(parsed) = Url::parse(url) else {
        return url.to_owned();
    };
    if parsed.scheme() == "file" {
        let Some(name) = parsed.path().split('/').rfind(|s| !s.is_empty()) else {
            return url.to_owned();
        };
        return percent_decode(name).unwrap_or_else(|| name.to_owned());
    }
    match (parsed.host_str().filter(|h| !h.is_empty()), parsed.port()) {
        (None, _) => url.to_owned(),
        (Some(h), Some(p)) => format!("{h}:{p}"),
        (Some(h), None) => h.to_owned(),
    }
}

/// `decodeURIComponent`, or `None` where it would throw.
fn percent_decode(s: &str) -> Option<String> {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `flattenPageText`: page text on one line, cut at `max` characters.
pub fn flatten_page_text(raw: &str, max: usize) -> String {
    let spaced: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let one_line = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    // JavaScript counts UTF-16 units; the cut lands on the same character
    // for text in the Basic Multilingual Plane.
    if one_line.encode_utf16().count() > max {
        let cut: String = one_line.chars().take(max).collect();
        format!("{cut}\u{2026}")
    } else {
        one_line
    }
}

/// `loadableUrl`: what the agent may load in a session whose directory is
/// `root`. A `file:` address must resolve, links followed, inside the root,
/// because an agent reads pages it did not choose.
pub fn loadable_url(input: &str, root: Option<&Path>) -> Option<String> {
    let real_root = root.and_then(|r| r.canonicalize().ok());
    let normalized = normalize_url(input, real_root.is_some())?;
    if !normalized.starts_with("file:") {
        return Some(normalized);
    }
    let parsed = Url::parse(&normalized).ok()?;
    if parsed.host_str().is_some_and(|h| !h.is_empty()) {
        return None;
    }
    let real = parsed.to_file_path().ok()?.canonicalize().ok()?;
    real.starts_with(real_root?).then_some(normalized)
}

/// `parseCursor`: where a paged read resumes; a cursor from another page
/// generation starts over.
pub fn parse_cursor(cursor: Option<&str>, generation: u32) -> usize {
    let Some((gen, at)) = cursor.and_then(|c| c.split_once(':')) else {
        return 0;
    };
    if gen.parse::<u32>().ok() != Some(generation) {
        return 0;
    }
    at.parse().unwrap_or(0)
}

/// The design file a `file:` address shows, which its tweaks are kept by.
pub fn design_path(url: &str) -> Option<std::path::PathBuf> {
    Url::parse(url)
        .ok()
        .filter(|u| u.scheme() == "file")?
        .to_file_path()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursors_resume_only_on_the_same_page() {
        assert_eq!(parse_cursor(None, 3), 0);
        assert_eq!(parse_cursor(Some("3:120"), 3), 120);
        assert_eq!(parse_cursor(Some("2:120"), 3), 0);
        assert_eq!(parse_cursor(Some("3:x"), 3), 0);
        assert_eq!(parse_cursor(Some("junk"), 3), 0);
    }

    #[test]
    fn file_addresses_stay_inside_the_session() {
        let base = std::env::temp_dir().join(format!("vorn-loadable-{}", std::process::id()));
        let root = base.join("proj");
        std::fs::create_dir_all(&root).expect("root");
        std::fs::write(root.join("a.html"), "a").expect("inside");
        std::fs::write(base.join("secret.html"), "s").expect("outside");
        let inside = Url::from_file_path(root.join("a.html").canonicalize().expect("real"))
            .expect("url")
            .to_string();
        let outside = Url::from_file_path(base.join("secret.html").canonicalize().expect("real"))
            .expect("url")
            .to_string();
        assert_eq!(loadable_url(&inside, Some(&root)), Some(inside.clone()));
        assert_eq!(loadable_url(&outside, Some(&root)), None);
        assert_eq!(loadable_url(&inside, None), None);
        assert_eq!(
            loadable_url("example.com", Some(&root)).as_deref(),
            Some("https://example.com/")
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_address_bar_reads_hosts_ports_and_schemes() {
        let n = |s: &str| normalize_url(s, false);
        assert_eq!(n("example.com").as_deref(), Some("https://example.com/"));
        assert_eq!(
            n("localhost:3000").as_deref(),
            Some("http://localhost:3000/")
        );
        assert_eq!(n("javascript:1").as_deref(), None);
        assert_eq!(n("about:blank").as_deref(), Some("about:blank"));
        assert_eq!(n("about:config").as_deref(), None);
        assert_eq!(n("file:///a/b.html"), None);
        assert_eq!(
            normalize_url("file:///a/b.html?x#y", true).as_deref(),
            Some("file:///a/b.html")
        );
        assert_eq!(n("//app.local/x").as_deref(), Some("http://app.local/x"));
        assert_eq!(n("   ").as_deref(), None);
    }

    #[test]
    fn words_become_a_search_and_addresses_load() {
        let a = |s: &str| address_target(s);
        assert_eq!(
            a("google").as_deref(),
            Some("https://www.google.com/search?q=google")
        );
        assert_eq!(
            a(" rust borrow checker ").as_deref(),
            Some("https://www.google.com/search?q=rust+borrow+checker")
        );
        assert_eq!(a("a&b").as_deref(), Some("https://www.google.com/search?q=a%26b"));
        assert_eq!(a("google.com").as_deref(), Some("https://google.com/"));
        assert_eq!(
            a("https://example.com/a?b=1").as_deref(),
            Some("https://example.com/a?b=1")
        );
        assert_eq!(a("localhost").as_deref(), Some("http://localhost/"));
        assert_eq!(
            a("localhost:3000/x").as_deref(),
            Some("http://localhost:3000/x")
        );
        assert_eq!(a("127.0.0.1:8080").as_deref(), Some("http://127.0.0.1:8080/"));
        assert_eq!(a("about:blank").as_deref(), Some("about:blank"));
        assert_eq!(a("javascript:alert(1)"), None);
        assert_eq!(a(""), None);
    }

    #[test]
    fn closing_tabs_keeps_the_neighbour_and_the_last_takes_the_pane() {
        let mut p = Panes::default();
        p.open("s", Some("https://a/".into()));
        p.add_tab("s", Some("https://b/".into()));
        p.add_tab("s", None);
        assert_eq!(p.get("s").map(|p| p.active_tab), Some(2));
        p.close_tab("s", 2);
        assert_eq!(p.get("s").map(|p| p.active_tab), Some(1));
        p.close_tab("s", 0);
        assert_eq!(p.get("s").map(|p| p.active_tab), Some(0));
        p.close("s");
        p.open("s", None);
        assert_eq!(
            p.get("s").map(|p| p.tabs[0].url.as_str()),
            Some("https://b/")
        );
        p.close_tab("s", 0);
        assert!(p.get("s").is_none());
        p.open("s", None);
        assert_eq!(
            p.get("s").map(|p| p.tabs[0].url.as_str()),
            Some(DEFAULT_URL)
        );
    }

    #[test]
    fn a_new_page_drops_the_old_title() {
        let mut p = Panes::default();
        p.open("s", Some("https://a/".into()));
        assert!(p.sync_tab("s", 0, None, Some("A".into())));
        p.sync_tab("s", 0, Some("https://a/".into()), None);
        assert_eq!(
            p.get("s").and_then(|p| p.tabs[0].title.clone()),
            Some("A".into())
        );
        p.sync_tab("s", 0, Some("https://b/".into()), None);
        assert_eq!(p.get("s").and_then(|p| p.tabs[0].title.clone()), None);
    }

    #[test]
    fn an_artifact_reopens_in_its_own_tab() {
        let mut p = Panes::default();
        let art = |v| TabArtifact {
            id: "a1".into(),
            kind: "page".into(),
            version: v,
            title: "Plan".into(),
        };
        p.open("s", Some("https://x/".into()));
        p.open_artifact("s", "http://v1".into(), art(1));
        p.select_tab("s", 0);
        p.open_artifact("s", "http://v2".into(), art(2));
        let pane = p
            .get("s")
            .cloned()
            .unwrap_or_else(|| Pane::new(Tab::default()));
        assert_eq!(pane.tabs.len(), 2);
        assert_eq!(pane.active_tab, 1);
        assert_eq!(pane.tabs[1].url, "http://v2");
    }

    #[test]
    fn saving_keeps_intent_only() {
        let dir = std::env::temp_dir().join(format!("vorn-panes-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("panes.json");
        let mut p = Panes::default();
        p.open("s", Some("https://a/".into()));
        p.sync_tab("s", 0, Some("https://a/b".into()), Some("B".into()));
        p.save(&path).ok();
        let back = Panes::load(&path);
        assert_eq!(
            back.get("s").map(|p| p.tabs[0].clone()),
            Some(Tab::at("https://a/"))
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
