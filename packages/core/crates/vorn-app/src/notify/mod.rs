//! System notifications: which vornd events are worth interrupting someone for,
//! under the `defaults.notifications` preferences, and handing them to the OS.
//!
//! Only while the window is not focused (unless they are shown always), at
//! most one per source every ten seconds. Extensions' notices arrive as
//! `extension:notify`, already let through by their switches in vornd.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use web_time::{Duration, Instant};

use serde_json::Value;

use crate::runs::s;

#[cfg(target_os = "macos")]
mod mac;

/// `COOLDOWN_MS`: one notification per source in this long.
pub const COOLDOWN: Duration = Duration::from_secs(10);
/// `NOTICE_RECEIPTS_KEPT`: effect ids remembered, to show each once.
const RECEIPTS_KEPT: usize = 2048;

/// One notification to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
    /// The source the cooldown counts against; `None` skips the cooldown.
    pub key: Option<String>,
    /// The sound it plays: `None` is silent, `Some("default")` the system's.
    pub sound: Option<String>,
}

/// When notices are shown: `defaults.notifications.show`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Show {
    /// Only while another app has focus.
    #[default]
    Background,
    Always,
}

impl Show {
    /// The stored value.
    pub fn key(self) -> &'static str {
        match self {
            Show::Background => "background",
            Show::Always => "always",
        }
    }
}

/// The `defaults.notifications` preferences.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefs {
    pub enabled: bool,
    pub on_waiting: bool,
    pub on_bell: bool,
    pub show: Show,
    /// `"default"`, `"none"` or one of the system's sound names.
    pub sound: String,
}

impl Prefs {
    /// Read from the config; on, in the background, with the default sound,
    /// until changed.
    pub fn of(config: &Value) -> Prefs {
        let p = config.pointer("/defaults/notifications");
        let flag = |k: &str| p.and_then(|p| p.get(k)).and_then(Value::as_bool);
        let text = |k: &str| p.and_then(|p| p.get(k)).and_then(Value::as_str);
        Prefs {
            enabled: flag("enabled") != Some(false),
            on_waiting: flag("onWaiting") != Some(false),
            on_bell: flag("onBell") != Some(false),
            show: match text("show") {
                Some("always") => Show::Always,
                _ => Show::Background,
            },
            sound: text("sound")
                .filter(|s| !s.is_empty())
                .unwrap_or("default")
                .to_owned(),
        }
    }

    fn bell(&self) -> bool {
        self.enabled && self.on_bell
    }

    fn sound(&self) -> Option<String> {
        (self.sound != "none").then(|| self.sound.clone())
    }
}

/// What the session that rang is called: its agent's name and its title.
pub struct Who<'a> {
    pub agent: &'a str,
    pub name: String,
}

/// What has been announced, so each thing is announced once.
#[derive(Debug, Default)]
pub struct Notifier {
    last: HashMap<String, Instant>,
    effects: HashMap<String, Instant>,
}

impl Notifier {
    /// The notices a vornd notification raises. `who` names a session.
    pub fn heard<'a>(
        &mut self,
        method: &str,
        params: &Value,
        config: &Value,
        who: impl Fn(&str) -> Option<Who<'a>>,
        now: Instant,
    ) -> Vec<Notice> {
        let prefs = Prefs::of(config);
        match method {
            "terminal:bell" if prefs.bell() => s(params, "id")
                .and_then(|id| Some((id, who(id)?)))
                .map(|(id, w)| Notice {
                    title: format!("{} notification", w.agent),
                    body: format!("{} is requesting your attention", w.name),
                    key: Some(id.to_owned()),
                    sound: prefs.sound(),
                })
                .into_iter()
                .collect(),
            "terminal:notify" if prefs.bell() => {
                if !self.first_effect(params, now) {
                    return Vec::new();
                }
                let Some((id, w)) = s(params, "id").and_then(|id| Some((id, who(id)?))) else {
                    return Vec::new();
                };
                let title = s(params, "title").filter(|t| !t.is_empty());
                vec![Notice {
                    title: title.map_or(w.name, str::to_owned),
                    body: s(params, "body").unwrap_or("").to_owned(),
                    key: Some(format!("notify:{id}")),
                    sound: prefs.sound(),
                }]
            }
            // vornd let it through every switch but the window's focus.
            "extension:notify" if prefs.enabled => {
                let (Some(ext), Some(kind), Some(title)) = (
                    s(params, "extensionId"),
                    s(params, "kind"),
                    s(params, "title"),
                ) else {
                    return Vec::new();
                };
                vec![Notice {
                    title: title.to_owned(),
                    body: s(params, "body").unwrap_or("").to_owned(),
                    key: Some(format!("ext:{ext}:{kind}")),
                    sound: prefs.sound(),
                }]
            }
            _ => Vec::new(),
        }
    }

    /// `NoticeReceipts.shows`: a payload whose effect was shown already is
    /// not shown again.
    fn first_effect(&mut self, params: &Value, now: Instant) -> bool {
        let Some(effect) = s(params, "effectId").filter(|e| !e.is_empty()) else {
            return true;
        };
        if self.effects.contains_key(effect) {
            return false;
        }
        if self.effects.len() >= RECEIPTS_KEPT {
            let oldest = self
                .effects
                .iter()
                .min_by_key(|(_, at)| **at)
                .map(|(k, _)| k.clone());
            if let Some(k) = oldest {
                self.effects.remove(&k);
            }
        }
        self.effects.insert(effect.to_owned(), now);
        true
    }

    /// Whether `notice` is shown now: not while the window has focus unless
    /// `show` is [`Show::Always`], and once per source in [`COOLDOWN`].
    pub fn admit(&mut self, notice: &Notice, focused: bool, show: Show, now: Instant) -> bool {
        if focused && show == Show::Background {
            return false;
        }
        let Some(key) = &notice.key else {
            return true;
        };
        if self
            .last
            .get(key)
            .is_some_and(|at| now.duration_since(*at) < COOLDOWN)
        {
            return false;
        }
        self.last.insert(key.clone(), now);
        true
    }
}

/// Where admitted notices go. The app shows them through [`Os`]; tests and
/// offscreen frames swap in [`Recorded`], so they never reach the OS.
pub trait Sink: Send + Sync {
    fn show(&self, notice: Notice);
}

/// The OS's notifier.
#[derive(Debug, Default, Clone, Copy)]
pub struct Os;

/// Notices handed to the OS by this process.
static OS_SHOWN: AtomicUsize = AtomicUsize::new(0);

/// How many notices this process has handed to the OS's notifier.
pub fn os_shown() -> usize {
    OS_SHOWN.load(Ordering::Relaxed)
}

/// The preference that turns all of one extension's notifications on or off.
pub const ALL_KEY: &str = "notify:all";

/// An installed extension that declares notifications.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source<'a> {
    pub id: &'a str,
    pub name: &'a str,
    /// Its [`ALL_KEY`] switch.
    pub on: bool,
}

/// The extensions in `lists` (built-ins and packs) that declare
/// notifications, each once, by the switch vornd lists for them.
pub fn sources<'a>(lists: &[&'a [Value]]) -> Vec<Source<'a>> {
    let mut out: Vec<Source<'a>> = Vec::new();
    for e in lists.iter().copied().flatten() {
        let Some(id) = s(e, "id") else { continue };
        let all = e
            .get("preferences")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|p| s(p, "id") == Some(ALL_KEY));
        let Some(all) = all else { continue };
        if out.iter().any(|o| o.id == id) {
            continue;
        }
        let name = s(e, "name")
            .or_else(|| e.pointer("/manifest/name").and_then(Value::as_str))
            .unwrap_or(id);
        let on = all.get("value").and_then(Value::as_bool) != Some(false);
        out.push(Source { id, name, on });
    }
    out
}

/// The sounds the system offers by name, read once: on macOS, those in
/// `/System/Library/Sounds`; elsewhere none beyond Default and None.
pub fn system_sounds() -> &'static [String] {
    static SOUNDS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    SOUNDS.get_or_init(|| {
        if !cfg!(target_os = "macos") {
            return Vec::new();
        }
        let mut names: Vec<String> = std::fs::read_dir("/System/Library/Sounds")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                Some(path.file_stem()?.to_str()?.to_owned())
            })
            .collect();
        names.sort();
        names
    })
}

/// Whether the OS refused Vorn's notifications, as last heard.
static BLOCKED: AtomicBool = AtomicBool::new(false);

/// Whether the OS refuses Vorn's notifications, so Settings can say so.
pub fn blocked() -> bool {
    BLOCKED.load(Ordering::Relaxed)
}

/// Says the OS refuses, for an offscreen frame of that state.
pub fn show_blocked() {
    BLOCKED.store(true, Ordering::Relaxed);
}

/// Asks the OS again whether Vorn may notify; [`blocked`] follows.
pub fn refresh() {
    #[cfg(target_os = "macos")]
    mac::refresh();
}

/// Opens the system's settings for Vorn's notifications.
pub fn open_system_settings() {
    #[cfg(target_os = "macos")]
    {
        let id = mac::bundle_id().unwrap_or_default();
        let url =
            format!("x-apple.systempreferences:com.apple.Notifications-Settings.extension?id={id}");
        let _ = vorn_spawn::command("open").arg(url).spawn();
    }
}

impl Sink for Os {
    /// Hands `notice` to the OS on a thread of its own, so a slow helper
    /// never holds up a frame. Failing to show one is not worth reporting.
    fn show(&self, notice: Notice) {
        OS_SHOWN.fetch_add(1, Ordering::Relaxed);
        #[cfg(target_os = "macos")]
        if mac::available() {
            mac::show(notice.title, notice.body, notice.sound);
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::spawn(move || {
            let _ = command(&notice.title, &notice.body, notice.sound.as_deref()).status();
        });
        #[cfg(target_arch = "wasm32")]
        crate::page::notify(&notice.title, &notice.body);
    }
}

/// Keeps the notices it is given, for a test to read; clones share them.
#[derive(Debug, Default, Clone)]
pub struct Recorded(Arc<Mutex<Vec<Notice>>>);

impl Recorded {
    /// The notices shown so far, oldest first.
    pub fn notices(&self) -> Vec<Notice> {
        self.0.lock().map(|n| n.clone()).unwrap_or_default()
    }
}

impl Sink for Recorded {
    fn show(&self, notice: Notice) {
        if let Ok(mut n) = self.0.lock() {
            n.push(notice);
        }
    }
}

/// The OS's own notifier, given the text and sound as arguments or
/// environment so nothing in it is ever read as script.
#[cfg(not(target_arch = "wasm32"))]
fn command(title: &str, body: &str, sound: Option<&str>) -> std::process::Command {
    use std::process::Stdio;
    let mut c = if cfg!(target_os = "macos") {
        let mut c = vorn_spawn::command("osascript");
        let show = if sound.is_some() {
            "display notification (item 2 of argv) with title (item 1 of argv) sound name (item 3 of argv)"
        } else {
            "display notification (item 2 of argv) with title (item 1 of argv)"
        };
        c.args([
            "-e",
            "on run argv",
            "-e",
            show,
            "-e",
            "end run",
            title,
            body,
        ]);
        if let Some(name) = sound {
            c.arg(name);
        }
        c
    } else if cfg!(windows) {
        let mut c = vorn_spawn::command("powershell");
        c.args(["-NoProfile", "-NonInteractive", "-Command", WINDOWS_TOAST])
            .env("VORN_NOTICE_TITLE", title)
            .env("VORN_NOTICE_BODY", body);
        if sound.is_none() {
            c.env("VORN_NOTICE_SILENT", "1");
        }
        c
    } else {
        let mut c = vorn_spawn::command("notify-send");
        c.arg("--app-name=Vorn");
        c.arg(match sound {
            Some(_) => "--hint=string:sound-name:message-new-instant",
            None => "--hint=boolean:suppress-sound:true",
        });
        c.args([title, body]);
        c
    };
    c.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    c
}

#[cfg(not(target_arch = "wasm32"))]
/// A toast under PowerShell's registered app id, which needs no install step.
const WINDOWS_TOAST: &str = r"
[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null
$t = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02)
$x = $t.GetElementsByTagName('text')
$x.Item(0).AppendChild($t.CreateTextNode($env:VORN_NOTICE_TITLE)) > $null
$x.Item(1).AppendChild($t.CreateTextNode($env:VORN_NOTICE_BODY)) > $null
if ($env:VORN_NOTICE_SILENT) { $a = $t.CreateElement('audio'); $a.SetAttribute('silent', 'true'); $t.DocumentElement.AppendChild($a) > $null }
$id = '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe'
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($id).Show([Windows.UI.Notifications.ToastNotification]::new($t))
";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn on() -> Value {
        json!({ "defaults": { "notifications": { "enabled": true } } })
    }

    fn who(id: &str) -> Option<Who<'static>> {
        (id == "s1").then(|| Who {
            agent: "Acme Code",
            name: "api".into(),
        })
    }

    fn notice(key: Option<&str>) -> Notice {
        Notice {
            title: "t".into(),
            body: "b".into(),
            key: key.map(str::to_owned),
            sound: None,
        }
    }

    #[test]
    fn notifications_are_on_by_default_in_the_background_with_the_default_sound() {
        let p = Prefs::of(&json!({}));
        assert!(p.enabled && p.on_waiting && p.on_bell);
        assert_eq!(p.show, Show::Background);
        assert_eq!(p.sound(), Some("default".into()));
        let quiet =
            json!({ "defaults": { "notifications": { "sound": "none", "show": "always" } } });
        let p = Prefs::of(&quiet);
        assert_eq!((p.show, p.sound()), (Show::Always, None));
    }

    #[test]
    fn a_bell_needs_notifications_on_and_a_known_session() {
        let mut n = Notifier::default();
        let now = Instant::now();
        let bell = json!({ "id": "s1" });
        let off = json!({ "defaults": { "notifications": { "enabled": false } } });
        assert!(n.heard("terminal:bell", &bell, &off, who, now).is_empty());
        let muted = json!({ "defaults": { "notifications": { "onBell": false } } });
        assert!(n.heard("terminal:bell", &bell, &muted, who, now).is_empty());
        let stranger = json!({ "id": "x" });
        assert!(n
            .heard("terminal:bell", &stranger, &on(), who, now)
            .is_empty());
        let got = n.heard("terminal:bell", &bell, &on(), who, now);
        assert_eq!(got[0].title, "Acme Code notification");
        assert_eq!(got[0].body, "api is requesting your attention");
        assert_eq!(got[0].sound.as_deref(), Some("default"));
    }

    #[test]
    fn a_program_notice_shows_once_per_effect_and_falls_back_to_the_name() {
        let mut n = Notifier::default();
        let now = Instant::now();
        let p = json!({ "id": "s1", "title": "", "body": "done", "effectId": "e1" });
        let got = n.heard("terminal:notify", &p, &on(), who, now);
        assert_eq!(
            (got[0].title.as_str(), got[0].body.as_str()),
            ("api", "done")
        );
        assert!(n.heard("terminal:notify", &p, &on(), who, now).is_empty());
    }

    #[test]
    fn an_extension_notice_is_shown_as_sent_unless_notifications_are_off() {
        let mut n = Notifier::default();
        let now = Instant::now();
        let p = json!({ "extensionId": "vorn.workflows", "extension": "Workflows",
                        "kind": "finished", "title": "Vorn", "body": "Workflow \"Ship\" completed" });
        let got = n.heard("extension:notify", &p, &on(), who, now);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].title, "Vorn");
        assert_eq!(got[0].body, "Workflow \"Ship\" completed");
        assert_eq!(got[0].key.as_deref(), Some("ext:vorn.workflows:finished"));
        let off = json!({ "defaults": { "notifications": { "enabled": false } } });
        assert!(n.heard("extension:notify", &p, &off, who, now).is_empty());
        let unnamed = json!({ "extensionId": "x", "kind": "k" });
        assert!(n
            .heard("extension:notify", &unnamed, &on(), who, now)
            .is_empty());
    }

    #[test]
    fn focus_holds_a_notice_back_only_when_shown_in_the_background() {
        let mut n = Notifier::default();
        let now = Instant::now();
        let key = notice(Some("s1"));
        assert!(!n.admit(&key, true, Show::Background, now));
        assert!(n.admit(&key, true, Show::Always, now));
        assert!(!n.admit(&notice(None), true, Show::Background, now));
    }

    #[test]
    fn the_cooldown_holds_back_a_second_notice_from_one_source() {
        let mut n = Notifier::default();
        let now = Instant::now();
        let key = notice(Some("s1"));
        assert!(n.admit(&key, false, Show::Background, now));
        assert!(!n.admit(&key, false, Show::Background, now + Duration::from_secs(9)));
        assert!(n.admit(&key, false, Show::Background, now + COOLDOWN));
        assert!(n.admit(&notice(None), false, Show::Background, now));
    }

    #[test]
    fn a_recorded_sink_keeps_notices_and_reaches_no_os() {
        let sink = Recorded::default();
        let shown = os_shown();
        let notice = notice(None);
        sink.clone().show(notice.clone());
        assert_eq!(sink.notices(), [notice]);
        assert_eq!(os_shown(), shown);
    }

    #[test]
    fn sources_are_the_extensions_with_a_switch_for_all_their_notices() {
        let builtins = vec![
            json!({ "id": "vorn.workflows", "name": "Workflows",
                    "preferences": [{ "id": "top_bar", "value": true },
                                    { "id": "notify:all", "value": false, "notifications": "all" }] }),
            json!({ "id": "vorn.files", "name": "Files", "preferences": [] }),
        ];
        let packs = vec![
            json!({ "id": "acme.chat", "manifest": { "name": "Chat" },
                    "preferences": [{ "id": "notify:all", "value": true }] }),
            json!({ "id": "vorn.workflows", "preferences": [{ "id": "notify:all", "value": true }] }),
        ];
        let got = sources(&[&builtins, &packs]);
        assert_eq!(
            got,
            [
                Source {
                    id: "vorn.workflows",
                    name: "Workflows",
                    on: false
                },
                Source {
                    id: "acme.chat",
                    name: "Chat",
                    on: true
                },
            ]
        );
        assert!(sources(&[&[]]).is_empty(), "no such extension, no row");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_sound_is_an_argument_and_none_plays_nothing() {
        let args = |sound| {
            command("t", "b", sound)
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        let loud = args(Some("Glass"));
        assert_eq!(loud.last().map(String::as_str), Some("Glass"));
        assert!(loud
            .iter()
            .any(|a| a.contains("sound name (item 3 of argv)")));
        assert!(!args(None).iter().any(|a| a.contains("sound name")));
    }
}
