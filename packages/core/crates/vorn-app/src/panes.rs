//! The panes beside a session's terminal (`PaneColumn`): its browser and
//! its device, what each shows between frames, and what their controls do.
//! The browser's tabs persist ([`crate::browser::Panes`]); the rest lives as
//! long as the app, as the renderer's component state does.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

use serde_json::{Map, Value};

use crate::act::{Act, Acts};
use crate::artifact::{self, Anchor, Comment, Manifest};
use crate::browser::{Panes, TabArtifact};
use crate::device::Listed;
use crate::ui::Id;

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
    Browser,
    Device,
}

/// The view keys: a session's page, its compare page, its artboards and
/// its device.
pub fn page_key(session: &str) -> String {
    session.to_owned()
}

pub fn compare_key(session: &str) -> String {
    format!("{session}#compare")
}

pub fn board_key(session: &str, board: &str) -> String {
    format!("{session}#board:{board}")
}

pub fn device_key(session: &str) -> String {
    format!("{session}#device")
}

/// The session a view key belongs to.
pub fn session_of(key: &str) -> &str {
    key.split_once('#').map_or(key, |(s, _)| s)
}

/// The custom box of the design canvas, whose size fits the artboards.
pub fn canvas_id(session: &str) -> Id {
    Id::new("pane-canvas").child(session)
}

/// The custom box of the device frame, whose size fits the screen.
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

/// The browser pane's state that is not its tabs (`BrowserCard`'s own).
#[derive(Debug, Clone, Default)]
pub struct Live {
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
    /// The `id:version` the address of each artifact tab was asked for.
    pub refreshed: HashSet<String>,
    /// The page's history, as the view reports it.
    pub nav: (bool, bool),
}

/// A rectangle in page pixels: x, y, width, height.
pub type PageRect = (f32, f32, f32, f32);

/// How long a selection holds still before the comment popover opens, as
/// the renderer's two matching 400 ms polls.
pub const SELECTION_SETTLE: std::time::Duration = std::time::Duration::from_millis(400);

/// How many console messages and requests a page keeps for the agent.
pub const LOG_LIMIT: usize = 500;

/// A simulator a session holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub udid: String,
    pub name: String,
    /// The app booted it, so releasing it shuts it down.
    pub booted_here: bool,
}

/// A session's device pane (`DeviceCard`'s own state).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DevicePane {
    pub error: Option<String>,
    pub dismissed: Option<String>,
    pub picking: bool,
    pub typing: bool,
    pub saving: bool,
    /// The frame's size against fitting the pane; `None` fits.
    pub zoom: Option<f64>,
    /// The picker under the pane's name is open.
    pub switching: bool,
}

/// The device picker over a card.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Picker {
    pub session: String,
    /// Opened from the device pane's name rather than the card's button.
    pub from_pane: bool,
    pub loading: bool,
    pub devices: Vec<Listed>,
    pub error: Option<String>,
}

/// One web view this frame may show.
#[derive(Debug, Clone, PartialEq)]
pub struct WebSlot {
    pub key: String,
    pub url: String,
    pub zoom: f64,
}

/// Every pane's state.
#[derive(Debug, Default)]
pub struct Workbench {
    pub browsers: Panes,
    pub live: HashMap<String, Live>,
    /// `artifact:get` answers, by artifact id.
    pub artifacts: HashMap<String, artifact::State>,
    pub devices: HashMap<String, DevicePane>,
    /// The simulator each session holds, pane or not.
    pub claims: HashMap<String, Claim>,
    /// Sessions whose device is booting.
    pub claiming: HashSet<String>,
    pub picker: Option<Picker>,
    /// `project:detectMobile` answers, by project path.
    pub mobile: HashMap<String, bool>,
    pub maximized: HashMap<String, PaneKind>,
    /// Why pages cannot show in this window.
    pub unavailable: Option<String>,
    /// The web views of this frame, in custom-id order.
    pub slots: Vec<WebSlot>,
    /// Each device's latest screenshot size, in pixels.
    pub frames: HashMap<String, (u32, u32)>,
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
        if self.browsers.get(session).is_some() {
            out.push(PaneKind::Browser);
        }
        if self.devices.contains_key(session) {
            out.push(PaneKind::Device);
        }
        out
    }

    /// Who holds each simulator, as `device::list` takes it.
    pub fn holders(&self) -> HashMap<String, String> {
        self.claims
            .iter()
            .map(|(s, c)| (c.udid.clone(), s.clone()))
            .collect()
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

    /// The device frame's scale and its screen in points: the zoom chosen,
    /// else what fits `area`.
    pub fn device_scale(&self, session: &str, area: Option<(f32, f32)>) -> (f64, (f64, f64)) {
        let points = self.frames.get(session).map_or((0.0, 0.0), |&(w, h)| {
            let k = crate::device::point_scale(w, h);
            (f64::from(w) / k, f64::from(h) / k)
        });
        let chosen = self.devices.get(session).and_then(|d| d.zoom);
        let scale = chosen.unwrap_or_else(|| {
            area.map_or(1.0, |(w, h)| {
                crate::device::fit_scale(points, (f64::from(w), f64::from(h)))
            })
        });
        (scale, points)
    }

    /// An overlay the web views would cover is open.
    pub fn covered(&self) -> bool {
        self.picker.is_some() || self.live.values().any(|l| l.versions)
    }

    /// The artifact the session's active tab shows, and what vornd said of
    /// it, once asked.
    pub fn artifact(&self, session: &str) -> Option<(&TabArtifact, Option<&artifact::State>)> {
        let art = self.browsers.get(session)?.active()?.artifact.as_ref()?;
        Some((art, self.artifacts.get(&art.id)))
    }

    /// The artifact's unsent comments and the last batch sent
    /// (`useQuoteComments`), as the rail and the page's marks show them.
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
    /// The card's globe: opens or closes the browser.
    ToggleBrowser(String),
    /// The card's phone: the picker, or closes the device.
    DeviceButton(String),
    /// The device pane's name: the picker, to switch simulators.
    SwitchDevice(String),
    PickDevice {
        session: String,
        udid: String,
    },
    ClosePicker,
    Close(String, PaneKind),
    Maximize(String, PaneKind),
    AddTab(String),
    CloseTab(String, usize),
    SelectTab(String, usize),
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
    DismissDeviceError(String),
    /// A device control: `home`, `lock`, `save`, `keyboard`, `rotate`,
    /// `zoom-in`, `zoom-out`, `fit`, `actual`, `pick`, `draw`.
    Device(String, &'static str),
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
            device_key("s1"),
        ] {
            assert_eq!(session_of(&key), "s1");
        }
        assert_eq!(web_index(WEB_ID_BASE + 3), Some(3));
        assert_eq!(web_index(1_000), None);
    }
}
