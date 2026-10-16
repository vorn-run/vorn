//! System notifications: which vornd events are worth interrupting someone for,
//! under the `defaults.notifications` preferences, and handing them to the OS.
//!
//! Only while the window is not focused, at most one per source every ten
//! seconds. A run that ends is told regardless.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use web_time::{Duration, Instant};

use serde_json::Value;

use crate::runs::{run_status, s, states};

/// `COOLDOWN_MS`: one notification per source in this long.
pub const COOLDOWN: Duration = Duration::from_secs(10);
/// `NOTICE_RECEIPTS_KEPT`: effect ids remembered, to show each once.
const RECEIPTS_KEPT: usize = 2048;

/// One notification to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
    /// The source the cooldown counts against; `None` skips the cooldown
    /// and the focus check (a run that ended).
    pub key: Option<String>,
}

/// The `defaults.notifications` preferences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Prefs {
    enabled: bool,
    on_waiting: bool,
    on_bell: bool,
}

impl Prefs {
    fn of(config: &Value) -> Prefs {
        let p = config.pointer("/defaults/notifications");
        let flag = |k: &str| p.and_then(|p| p.get(k)).and_then(Value::as_bool);
        Prefs {
            enabled: flag("enabled") == Some(true),
            on_waiting: flag("onWaiting") != Some(false),
            on_bell: flag("onBell") != Some(false),
        }
    }

    fn bell(self) -> bool {
        self.enabled && self.on_bell
    }

    fn waiting(self) -> bool {
        self.enabled && self.on_waiting
    }
}

/// What the session that rang is called: its agent's name and its title.
pub struct Who<'a> {
    pub agent: &'a str,
    pub name: String,
}

/// A running run's waiting gates as last seen: announcing a gate once
/// needs what came before.
type Seen = BTreeSet<String>;

/// What has been announced, so each thing is announced once.
#[derive(Debug, Default)]
pub struct Notifier {
    last: HashMap<String, Instant>,
    runs: HashMap<String, Seen>,
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
                }]
            }
            "workflow:runUpdated" => self.run(params, config, prefs),
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

    /// Gates that just started waiting, and a run that just ended.
    fn run(&mut self, run: &Value, config: &Value, prefs: Prefs) -> Vec<Notice> {
        let key = crate::runs::key(run);
        let previous = self.runs.remove(&key);
        let waiting: Seen = states(run)
            .filter(|ns| s(ns, "status") == Some("waiting"))
            .filter_map(|ns| s(ns, "nodeId").map(str::to_owned))
            .collect();
        let workflow = s(run, "workflowId").and_then(|id| {
            config
                .get("workflows")?
                .as_array()?
                .iter()
                .find(|w| s(w, "id") == Some(id))
        });
        let mut out = Vec::new();
        if let Some(wf) = workflow {
            if prefs.waiting() {
                let fresh = waiting
                    .iter()
                    .filter(|id| !previous.as_ref().is_some_and(|p| p.contains(*id)));
                out.extend(fresh.map(|id| gate(wf, run, id)));
            }
            let status = s(run, "status").unwrap_or("");
            if matches!(status, "success" | "error") && previous.is_some() {
                out.push(ended(wf, run));
            }
        }
        // A run that stopped gets no further update, so nothing is kept for it.
        if s(run, "status") == Some("running") {
            self.runs.insert(key, waiting);
        }
        out
    }

    /// Whether `notice` is shown now: not while the window has focus, and
    /// once per source in [`COOLDOWN`].
    pub fn admit(&mut self, notice: &Notice, focused: bool, now: Instant) -> bool {
        let Some(key) = &notice.key else {
            return true;
        };
        if focused
            || self
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

/// The notice for a run waiting on a gate or a sign-in.
fn gate(wf: &Value, run: &Value, id: &str) -> Notice {
    let name = s(wf, "name").unwrap_or("");
    let wf_id = s(wf, "id").unwrap_or("");
    let node = wf.get("nodes").and_then(|n| crate::runs::node(n, id));
    let label = node.and_then(|n| s(n, "label"));
    let state = states(run).find(|ns| s(ns, "nodeId") == Some(id));
    let said = |m: Option<&str>| m.filter(|m| !m.is_empty()).map(str::to_owned);
    if state.and_then(|st| s(st, "waitingFor")) == Some("signIn") {
        let fallback = format!(
            "{} is waiting for you to sign in again",
            label.unwrap_or("A step")
        );
        return Notice {
            title: format!("{name} \u{b7} awaiting sign-in"),
            body: said(state.and_then(|st| s(st, "error"))).unwrap_or(fallback),
            key: Some(format!("workflow-sign-in:{wf_id}:{id}")),
        };
    }
    let message = state
        .and_then(|st| s(st, "message"))
        .or_else(|| node?.pointer("/config/message")?.as_str());
    let fallback = format!(
        "{} is waiting for your approval",
        label.unwrap_or("Approval")
    );
    Notice {
        title: format!("{name} \u{b7} awaiting approval"),
        body: said(message).unwrap_or(fallback),
        key: Some(format!("workflow-gate:{wf_id}:{id}")),
    }
}

/// The run-finished notice, with how many steps ran.
fn ended(wf: &Value, run: &Value) -> Notice {
    let steps = states(run)
        .filter(|ns| s(ns, "status") != Some("pending"))
        .count();
    let outcome = match (s(run, "status"), run_status(run)) {
        (Some("success"), _) => "completed",
        (_, "cancelled") => "was rejected",
        _ => "failed",
    };
    let plural = if steps == 1 { "" } else { "s" };
    Notice {
        title: "Vorn".to_owned(),
        body: format!(
            "Workflow \"{}\" {outcome} \u{2014} {steps} step{plural}",
            s(wf, "name").unwrap_or("")
        ),
        key: None,
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

impl Sink for Os {
    /// Hands `notice` to the OS on a thread of its own, so a slow helper
    /// never holds up a frame. Failing to show one is not worth reporting.
    fn show(&self, notice: Notice) {
        OS_SHOWN.fetch_add(1, Ordering::Relaxed);
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::spawn(move || {
            let _ = command(&notice.title, &notice.body).status();
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

/// The OS's own notifier, given the text as arguments or environment so
/// nothing in it is ever read as script.
#[cfg(not(target_arch = "wasm32"))]
fn command(title: &str, body: &str) -> std::process::Command {
    use std::process::Stdio;
    let mut c = if cfg!(target_os = "macos") {
        let mut c = vorn_spawn::command("osascript");
        c.args([
            "-e",
            "on run argv",
            "-e",
            "display notification (item 2 of argv) with title (item 1 of argv)",
            "-e",
            "end run",
            title,
            body,
        ]);
        c
    } else if cfg!(windows) {
        let mut c = vorn_spawn::command("powershell");
        c.args(["-NoProfile", "-NonInteractive", "-Command", WINDOWS_TOAST])
            .env("VORN_NOTICE_TITLE", title)
            .env("VORN_NOTICE_BODY", body);
        c
    } else {
        let mut c = vorn_spawn::command("notify-send");
        c.args(["--app-name=Vorn", title, body]);
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
$id = '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe'
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($id).Show([Windows.UI.Notifications.ToastNotification]::new($t))
";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn on() -> Value {
        json!({
            "defaults": { "notifications": { "enabled": true } },
            "workflows": [{
                "id": "w", "name": "Ship",
                "nodes": [{ "id": "g", "label": "Review", "type": "approval",
                            "config": { "message": "Look at it" } }]
            }]
        })
    }

    fn who(id: &str) -> Option<Who<'static>> {
        (id == "s1").then(|| Who {
            agent: "Acme Code",
            name: "api".into(),
        })
    }

    fn run(status: &str, gate: &str) -> Value {
        json!({ "runId": "r", "workflowId": "w", "status": status,
                "nodeStates": [{ "nodeId": "g", "status": gate }] })
    }

    #[test]
    fn a_bell_needs_notifications_on_and_a_known_session() {
        let mut n = Notifier::default();
        let now = Instant::now();
        let bell = json!({ "id": "s1" });
        let off = json!({ "defaults": { "notifications": { "enabled": false } } });
        assert!(n.heard("terminal:bell", &bell, &off, who, now).is_empty());
        let muted =
            json!({ "defaults": { "notifications": { "enabled": true, "onBell": false } } });
        assert!(n.heard("terminal:bell", &bell, &muted, who, now).is_empty());
        let stranger = json!({ "id": "x" });
        assert!(n
            .heard("terminal:bell", &stranger, &on(), who, now)
            .is_empty());
        let got = n.heard("terminal:bell", &bell, &on(), who, now);
        assert_eq!(got[0].title, "Acme Code notification");
        assert_eq!(got[0].body, "api is requesting your attention");
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
    fn a_gate_is_announced_when_it_starts_waiting_and_the_end_once() {
        let mut n = Notifier::default();
        let now = Instant::now();
        let mut hear = |r: Value| n.heard("workflow:runUpdated", &r, &on(), who, now);
        let first = hear(run("running", "waiting"));
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].title, "Ship \u{b7} awaiting approval");
        assert_eq!(first[0].body, "Look at it");
        assert!(hear(run("running", "waiting")).is_empty(), "told once");
        let end = hear(run("success", "success"));
        assert_eq!(end[0].body, "Workflow \"Ship\" completed \u{2014} 1 step");
        assert_eq!(end[0].key, None);
        assert!(
            hear(run("success", "success")).is_empty(),
            "an end not seen running"
        );
    }

    #[test]
    fn a_rejected_run_reads_as_rejected() {
        let mut n = Notifier::default();
        let now = Instant::now();
        n.heard(
            "workflow:runUpdated",
            &run("running", "running"),
            &on(),
            who,
            now,
        );
        let rejected = json!({ "runId": "r", "workflowId": "w", "status": "error",
            "nodeStates": [{ "nodeId": "g", "status": "error", "rejectedAt": "t" }] });
        let end = n.heard("workflow:runUpdated", &rejected, &on(), who, now);
        assert_eq!(
            end[0].body,
            "Workflow \"Ship\" was rejected \u{2014} 1 step"
        );
    }

    #[test]
    fn focus_and_the_cooldown_hold_a_notice_back() {
        let mut n = Notifier::default();
        let now = Instant::now();
        let notice = Notice {
            title: "t".into(),
            body: "b".into(),
            key: Some("s1".into()),
        };
        assert!(!n.admit(&notice, true, now));
        assert!(n.admit(&notice, false, now));
        assert!(!n.admit(&notice, false, now + Duration::from_secs(9)));
        assert!(n.admit(&notice, false, now + COOLDOWN));
        let ended = Notice {
            key: None,
            ..notice
        };
        assert!(n.admit(&ended, true, now), "a run's end is told regardless");
    }

    #[test]
    fn a_recorded_sink_keeps_notices_and_reaches_no_os() {
        let sink = Recorded::default();
        let shown = os_shown();
        let notice = Notice {
            title: "Vorn".into(),
            body: "done".into(),
            key: None,
        };
        sink.clone().show(notice.clone());
        assert_eq!(sink.notices(), [notice]);
        assert_eq!(os_shown(), shown);
    }
}
