//! The panes beside a session's terminal: the page the Browser extension
//! shows and the review drawn around it, the simulator the Device extension
//! shows, what each shows between frames, and what their controls do. The Browser's tabs live in
//! the extension; what is here lives as long as the app.

use std::collections::{HashMap, HashSet, VecDeque};
use web_time::Instant;

use serde_json::{Map, Value};

use crate::act::{Act, Acts};
use crate::artifact::{self, Anchor, Comment, Manifest};
use crate::points::{kind, scoped, Points};
use crate::ui::Id;

/// The Browser, the built-in that keeps a session's tabs and draws them
/// over a page this app shows.
pub const BROWSER: &str = "vorn.browser";
/// Its pane, and its collection of the sessions whose pane is open.
pub const BROWSER_PANE: &str = "browser";
pub const BROWSER_OPEN: &str = "open";

/// The Browser's pane tree for `session`, while the Browser is on.
pub fn browser_key(points: &Points, session: &str) -> Option<String> {
    let p = points.find(BROWSER, kind::PANES, BROWSER_PANE)?;
    Some(scoped(&p.key(), session))
}

/// The Device, the built-in that keeps which simulator a session holds and
/// draws its pane over the screens this app takes.
pub const DEVICE: &str = "vorn.device";
/// Its pane, and its collection of the sessions whose pane is open.
pub const DEVICE_PANE: &str = "device";
pub const DEVICE_OPEN: &str = "open";

/// The Device's pane tree for `session`, while the Device is on.
pub fn device_key(points: &Points, session: &str) -> Option<String> {
    let p = points.find(DEVICE, kind::PANES, DEVICE_PANE)?;
    Some(scoped(&p.key(), session))
}

/// The video part of a Device tree that shows `session`'s simulator, and
/// its node's id, which its events are told on.
pub fn video_part<'a>(
    tree: &'a vorn_tree::Node,
    session: &str,
) -> Option<(&'a str, &'a vorn_tree::Video)> {
    let mut stack = vec![tree];
    while let Some(n) = stack.pop() {
        if let vorn_tree::Kind::Video(v) = &n.kind {
            if v.view == session {
                return Some((n.id.as_deref().unwrap_or_default(), v));
            }
        }
        stack.extend(&n.children);
    }
    None
}

/// The artifact an address shows, `(id, version)`: vornd serves each
/// version at `/artifact/<id>/<version>?t=<token>` on its own host.
pub fn artifact_address(url: &str) -> Option<(String, u32)> {
    let u = url::Url::parse(url).ok()?;
    let local = matches!(u.host_str(), Some("127.0.0.1" | "localhost"));
    if !local || !u.query_pairs().any(|(k, v)| k == "t" && !v.is_empty()) {
        return None;
    }
    let mut parts = u.path_segments()?;
    let (Some("artifact"), Some(id), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    let version = version.parse().ok().filter(|v| *v > 0)?;
    (!id.is_empty()).then(|| (id.to_owned(), version))
}

/// The artifact a session's page shows, once vornd has said what it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageArtifact {
    pub id: String,
    /// `page`, `doc` or `design`.
    pub kind: String,
    pub version: u32,
    pub title: String,
}

/// Custom ids from this up to [`WEB_ID_BASE`] are simulator screens:
/// `VIDEO_ID_BASE + i` shows `Workbench::videos[i]`.
pub const VIDEO_ID_BASE: u64 = 700_000;

/// Custom ids at and above this are web views: `WEB_ID_BASE + i` shows
/// `Workbench::slots[i]`.
pub const WEB_ID_BASE: u64 = 900_000;

/// The slot a custom box shows, if it is a web view.
pub fn web_index(id: u64) -> Option<usize> {
    id.checked_sub(WEB_ID_BASE)
        .and_then(|i| usize::try_from(i).ok())
}

/// Which pane of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaneKind {
    /// The Browser extension's pane: its tabs over the page.
    Browser,
    /// The Device extension's pane: a simulator's screen and its controls.
    Device,
    /// The pane an extension opened.
    Extension,
}

/// The view keys: a session's page, its compare page and its artboards.
pub fn page_key(session: &str) -> String {
    session.to_owned()
}

pub fn compare_key(session: &str) -> String {
    format!("{session}#compare")
}

pub fn board_key(session: &str, board: &str) -> String {
    format!("{session}#board:{board}")
}

/// The session a view key belongs to.
pub fn session_of(key: &str) -> &str {
    key.split_once('#').map_or(key, |(s, _)| s)
}

/// The custom box of the design canvas, whose size fits the artboards.
pub fn canvas_id(session: &str) -> Id {
    Id::new("pane-canvas").child(session)
}

/// The stage a simulator's screen is fitted to.
pub fn stage_id(session: &str) -> Id {
    Id::new("pane-stage").child(session)
}

/// The rail's note field and comment-edit field.
pub fn note_id(session: &str) -> Id {
    Acts::id("rail-note", &Act::Pane(PaneAct::Note(session.to_owned())))
}

pub fn rail_text_id(session: &str) -> Id {
    Acts::id(
        "rail-edit",
        &Act::Pane(PaneAct::RailText(session.to_owned())),
    )
}

/// A comment about to be written: where it points, how the popover quotes
/// it, and where the popover sits in the page.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingComment {
    pub anchor: Anchor,
    pub label: String,
    pub at: (f32, f32),
}

/// A doc open for editing: the source it started from, and the words now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocDraft {
    pub original: String,
    pub edited: String,
}

/// What a session's page shows and does, which the Browser's tabs do not keep.
#[derive(Debug, Clone, Default)]
pub struct Live {
    /// Where the page is and what it is called, as the view last reported.
    pub url: Option<String>,
    pub title: Option<String>,
    /// The address bar while it is being typed in.
    pub draft: Option<String>,
    pub failed: Option<String>,
    pub loading: bool,
    pub picking: bool,
    pub manifest: Option<Manifest>,
    pub values: Map<String, Value>,
    pub board: Option<String>,
    /// The canvas zoom; `None` fits every artboard.
    pub zoom: Option<f64>,
    pub commenting: bool,
    pub sending: bool,
    pub comparing: bool,
    pub compare_url: Option<String>,
    /// The `id:version` whose banner was dismissed.
    pub dismissed: Option<String>,
    pub editing: Option<DocDraft>,
    pub saving: bool,
    pub pending: Option<PendingComment>,
    /// Which comments found their words on the page.
    pub found: HashMap<String, bool>,
    pub focus: Option<String>,
    /// The page's selection while commenting, and since when it held still:
    /// one that holds for [`SELECTION_SETTLE`] opens the popover.
    pub selection: Option<(Anchor, PageRect, Instant)>,
    /// The page's console and requests, newest last, for the agent.
    pub console: VecDeque<Value>,
    pub network: VecDeque<Value>,
    /// The versions menu is open.
    pub versions: bool,
    /// The rail's comment being edited, and its words.
    pub rail_edit: Option<String>,
    /// The `id:version` whose address was asked for again this run.
    pub refreshed: HashSet<String>,
    /// The page's history, as the view reports it.
    pub nav: (bool, bool),
}

/// A rectangle in page pixels: x, y, width, height.
pub type PageRect = (f32, f32, f32, f32);

/// How long a selection holds still before the comment popover opens.
pub const SELECTION_SETTLE: web_time::Duration = web_time::Duration::from_millis(400);

/// How many console messages and requests a page keeps for the agent.
pub const LOG_LIMIT: usize = 500;

/// One web view this frame may show.
#[derive(Debug, Clone, PartialEq)]
pub struct WebSlot {
    pub key: String,
    pub url: String,
    pub zoom: f64,
    /// The connection whose profile the view keeps cookies in.
    pub profile: Option<String>,
}

/// Every pane's state.
#[derive(Debug, Default)]
pub struct Workbench {
    /// The sessions whose Browser pane is open, each with the address its
    /// page part asks for: empty until the pane's tree is drawn.
    pub pages: HashMap<String, String>,
    pub live: HashMap<String, Live>,
    /// `artifact:get` answers, by artifact id.
    pub artifacts: HashMap<String, artifact::State>,
    /// The sessions whose Device pane is open.
    pub devices: HashSet<String>,
    /// The session whose simulator picker is open, over the web views.
    pub video_picker: Option<String>,
    /// `project:detectMobile` answers, by project path.
    pub mobile: HashMap<String, bool>,
    pub maximized: HashMap<String, PaneKind>,
    /// Why pages cannot show in this window.
    pub unavailable: Option<String>,
    /// The web views of this frame, in custom-id order.
    pub slots: Vec<WebSlot>,
    /// Each open simulator's latest screen, by the session showing it.
    pub frames: HashMap<String, crate::video::Frame>,
    /// The sessions whose screen this frame draws, in custom-id order.
    pub videos: Vec<String>,
}

impl Workbench {
    pub fn live(&self, session: &str) -> Option<&Live> {
        self.live.get(session)
    }

    pub fn live_mut(&mut self, session: &str) -> &mut Live {
        self.live.entry(session.to_owned()).or_default()
    }

    /// The panes a session's card shows, in the column's order.
    pub fn kinds(&self, session: &str) -> Vec<PaneKind> {
        let mut out = Vec::new();
        if self.pages.contains_key(session) {
            out.push(PaneKind::Browser);
        }
        if self.devices.contains(session) {
            out.push(PaneKind::Device);
        }
        out
    }

    /// The custom id of the web view `key`, when it is in this frame.
    pub fn slot_id(&self, key: &str) -> Option<u64> {
        self.slots
            .iter()
            .position(|s| s.key == key)
            .map(|i| WEB_ID_BASE + i as u64)
    }

    /// The design manifest with artboards the session's page declared.
    pub fn boards(&self, session: &str) -> Option<&[artifact::Artboard]> {
        self.live(session)
            .and_then(|l| l.manifest.as_ref())
            .filter(|m| m.kind == "design" && !m.artboards.is_empty())
            .map(|m| m.artboards.as_slice())
    }

    /// The artboard selected on the session's canvas.
    pub fn board(&self, session: &str) -> Option<&artifact::Artboard> {
        let boards = self.boards(session)?;
        let chosen = self.live(session).and_then(|l| l.board.as_deref());
        boards
            .iter()
            .find(|b| Some(b.id.as_str()) == chosen)
            .or(boards.first())
    }

    /// The canvas zoom: the one chosen, else what fits `area`.
    pub fn canvas_zoom(&self, session: &str, area: Option<(f32, f32)>) -> f64 {
        let chosen = self.live(session).and_then(|l| l.zoom);
        match (chosen, self.boards(session), area) {
            (Some(z), _, _) => z,
            (None, Some(boards), Some((w, h))) => {
                artifact::fit_zoom(boards, (f64::from(w), f64::from(h)))
            }
            _ => 0.5,
        }
    }

    /// A simulator screen's scale and its size in points: the zoom
    /// chosen, else what fits `area`.
    pub fn video_scale(
        &self,
        session: &str,
        zoom: Option<f64>,
        area: Option<(f32, f32)>,
    ) -> (f64, (f64, f64)) {
        let points = self.frames.get(session).map_or((0.0, 0.0), |f| f.points);
        let scale = zoom.unwrap_or_else(|| {
            area.map_or(1.0, |(w, h)| {
                crate::video::fit_scale(points, (f64::from(w), f64::from(h)))
            })
        });
        (scale, points)
    }

    /// The custom id of the screen `session`'s pane draws, when it is in this frame.
    pub fn video_id(&self, session: &str) -> Option<u64> {
        self.videos
            .iter()
            .position(|s| s == session)
            .map(|i| VIDEO_ID_BASE + i as u64)
    }

    /// An overlay the web views would cover is open.
    pub fn covered(&self) -> bool {
        self.video_picker.is_some() || self.live.values().any(|l| l.versions)
    }

    /// The artifact the session's page shows, and what vornd said of it:
    /// a page is a review only once vornd knows its artifact.
    pub fn artifact(&self, session: &str) -> Option<(PageArtifact, Option<&artifact::State>)> {
        let (id, version) = artifact_address(self.pages.get(session)?)?;
        let state = self.artifacts.get(&id)?;
        let art = PageArtifact {
            kind: state.artifact.kind.clone(),
            title: state.artifact.title.clone(),
            id,
            version,
        };
        Some((art, Some(state)))
    }

    /// The artifact's unsent comments and the last batch sent, as the rail
    /// and the page's marks show them.
    pub fn comments(&self, session: &str) -> (Vec<&Comment>, Vec<&Comment>) {
        match self.artifact(session) {
            Some((_, Some(state))) => (
                state.comments.iter().filter(|c| c.is_draft()).collect(),
                artifact::latest_sent_batch(&state.comments),
            ),
            _ => (Vec::new(), Vec::new()),
        }
    }
}

/// What a pane's control does.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PaneAct {
    /// The card's globe: asks the Browser to open or close the pane.
    ToggleBrowser(String),
    /// The card's phone: asks the Device to open or close the pane.
    DeviceButton(String),
    Close(String, PaneKind),
    Maximize(String, PaneKind),
    Back(String),
    Forward(String),
    ReloadOrStop(String),
    /// The address bar's field.
    Address(String),
    Pick(String),
    /// Drawing over the page for the agent.
    Ink(String),
    Versions(String),
    Version(String, u32),
    Comments(String),
    Send(String),
    Edit(String),
    Discard(String),
    SaveDoc(String, bool),
    /// The doc editor's field.
    DocText(String),
    OpenLatest(String),
    Compare(String),
    Dismiss(String),
    /// A boolean or a choice turned to `value`.
    Tweak {
        session: String,
        key: String,
        value: String,
    },
    /// A number's field.
    TweakText(String, String),
    Board(String, String),
    /// Zoom out (-1), in (1) or fit (0).
    Zoom(String, i8),
    RailEdit(String, String),
    RailSave(String),
    RailCancel(String),
    RailDelete(String, String),
    RailReveal(String, String),
    /// The rail's note field.
    Note(String),
    AddNote(String),
    /// The rail's comment-edit field.
    RailText(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_keys_name_their_session() {
        for key in [
            page_key("s1"),
            compare_key("s1"),
            board_key("s1", "m"),
        ] {
            assert_eq!(session_of(&key), "s1");
        }
        assert_eq!(web_index(WEB_ID_BASE + 3), Some(3));
        assert_eq!(web_index(1_000), None);
    }

    #[test]
    fn an_artifact_is_read_from_its_address_on_vornds_host() {
        let id = |u: &str| artifact_address(u);
        assert_eq!(
            id("http://127.0.0.1:4310/artifact/a1/3?t=k"),
            Some(("a1".to_owned(), 3))
        );
        assert_eq!(id("http://127.0.0.1:4310/artifact/a1/3"), None, "no token");
        assert_eq!(id("https://example.com/artifact/a1/3?t=k"), None);
        assert_eq!(id("http://127.0.0.1:4310/artifact/a1/0?t=k"), None);
        assert_eq!(id("http://127.0.0.1:4310/artifact/a1/3/x?t=k"), None);
        assert_eq!(id("about:blank"), None);
    }
}
