//! The video primitive's side of the app: the simulators vornd asks after
//! for the Device extension (`video:*`), what a part asks of the client (to
//! start its device, list the simulators for its picker, save a
//! screenshot), each done off the UI thread and told back as a page event,
//! and each shown pane's frames, taken on a thread of their own and drawn
//! here as pictures. Frames never pass
//! through the extension: it holds which simulator a session has and what
//! its pane shows, and this side only takes the screens.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use web_time::{Duration, Instant};

use serde_json::{json, Value};

use super::{App, Call};
use crate::client::rpc::Wake;
use crate::panes::{
    device_key, stage_id, video_part, PaneKind, DEVICE, DEVICE_OPEN, DEVICE_PANE, VIDEO_ID_BASE,
    WEB_ID_BASE,
};
use crate::points::kind;
use crate::ui::{Laid, Ui};
use crate::video::{self, Frame};

/// How often at most a shown simulator's screen is taken.
const FRAME_EVERY: Duration = Duration::from_millis(250);

/// What a simulator thread finished.
enum Done {
    /// A new screen of `udid`, for `session`'s pane.
    Frame {
        session: String,
        udid: String,
        frame: Frame,
    },
    /// What a part asked of the client, done: the page event to tell its node.
    Told {
        key: String,
        node: String,
        what: Value,
    },
}

/// What a part asks of the client, each at most once while it asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Job {
    /// Start its device.
    Boot,
    /// List the simulators for its picker.
    List,
    /// Save a screenshot of its device.
    Save,
}

/// A pane's frame thread, stopped by its flag.
struct Poller {
    udid: String,
    stop: Arc<AtomicBool>,
}

/// The simulator threads and what they bring.
pub struct Video {
    tx: Sender<Done>,
    rx: Receiver<Done>,
    /// One per session whose pane shows a started simulator.
    pollers: HashMap<String, Poller>,
    /// What each session's part asked and is being done, until its tree
    /// stops asking: `(session, job, udid)`, the udid empty for a list.
    doing: HashSet<(String, Job, String)>,
    /// The Device was told which sessions are left, and has not answered with fewer.
    pruning: bool,
    wake: Wake,
}

impl Video {
    pub fn new(wake: Wake) -> Video {
        let (tx, rx) = channel();
        Video {
            tx,
            rx,
            pollers: HashMap::new(),
            doing: HashSet::new(),
            pruning: false,
            wake,
        }
    }

    /// Runs `work` on a thread of its own and hands what it made back.
    fn spawn(&self, name: &str, work: impl FnOnce() -> Done + Send + 'static) {
        let tx = self.tx.clone();
        let wake = Arc::clone(&self.wake);
        if cfg!(target_arch = "wasm32") {
            let _ = tx.send(work());
            return wake();
        }
        let spawned = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let _ = tx.send(work());
                wake();
            });
        if let Err(e) = spawned {
            eprintln!("{name} did not start: {e}");
        }
    }

    /// Takes `udid`'s screen every [`FRAME_EVERY`] until stopped, handing
    /// back each one that differs from the last.
    fn poll(&self, session: &str, udid: &str) -> Option<Poller> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let (tx, wake) = (self.tx.clone(), Arc::clone(&self.wake));
        let (s, u) = (session.to_owned(), udid.to_owned());
        let spawned = std::thread::Builder::new()
            .name("vorn-video-frames".into())
            .spawn(move || {
                let mut last = Vec::new();
                while !flag.load(Ordering::Relaxed) {
                    let started = Instant::now();
                    // An unchanged screen is neither decoded nor drawn again.
                    if let Some(png) = video::screenshot(&u).ok().filter(|png| *png != last) {
                        if let Ok(frame) = video::frame(&png) {
                            let done = Done::Frame {
                                session: s.clone(),
                                udid: u.clone(),
                                frame,
                            };
                            if tx.send(done).is_err() {
                                return;
                            }
                            wake();
                        }
                        last = png;
                    }
                    // A screenshot takes longer than the period; only what is
                    // left of it is waited, so frames come as fast as simctl gives them.
                    std::thread::sleep(FRAME_EVERY.saturating_sub(started.elapsed()));
                }
            });
        match spawned {
            Ok(_) => Some(Poller {
                udid: udid.to_owned(),
                stop,
            }),
            Err(e) => {
                eprintln!("simulator frames did not start: {e}");
                None
            }
        }
    }
}

/// Does `job` for a part, on a thread of its own: the page event that says how it went.
fn run(job: Job, udid: &str, title: &str) -> Value {
    match job {
        Job::Boot => match video::boot(udid) {
            Ok((_, here)) => json!({ "type": "booted", "bootedHere": here }),
            Err(message) => json!({ "type": "failed", "message": message }),
        },
        Job::List => match video::list() {
            Ok(devices) => json!({ "type": "listed", "devices": devices }),
            Err(error) => json!({ "type": "listed", "error": error }),
        },
        Job::Save => match video::save(udid, title) {
            Ok(path) => json!({ "type": "saved", "path": path.to_string_lossy() }),
            Err(error) => json!({ "type": "saved", "error": error }),
        },
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        for p in self.pollers.values() {
            p.stop.store(true, Ordering::Relaxed);
        }
    }
}

impl App {
    /// Answers `video:{op}`, the Device asking after the simulators on
    /// behalf of the session `params.view` names. Each runs on a thread,
    /// since `simctl` takes its time.
    pub(super) fn video_bridge(&mut self, id: Value, op: &str, params: &Value) {
        let session = params["view"].as_str().unwrap_or_default();
        if self.store.session(session).is_none() {
            let error = format!("No session {session} in this app.");
            return self.rpc.respond(id, Err(error));
        }
        let arg = |k: &str| params[k].as_str().unwrap_or_default().to_owned();
        let udid = arg("udid");
        let work: Box<dyn FnOnce() -> Result<Value, String> + Send> = match op {
            "list" => Box::new(|| video::list().map(|d| json!(d))),
            "boot" => Box::new(move || {
                video::boot(&udid).map(|(name, here)| json!({ "name": name, "bootedHere": here }))
            }),
            "shutdown" => Box::new(move || video::shutdown(&udid).map(|()| json!({ "ok": true }))),
            "launch" | "terminate" | "openUrl" | "install" => {
                let (op, bundle, url, path) =
                    (op.to_owned(), arg("bundleId"), arg("url"), arg("path"));
                Box::new(move || {
                    match op.as_str() {
                        "launch" => video::launch(&udid, &bundle),
                        "terminate" => video::terminate(&udid, &bundle),
                        "openUrl" => video::open_url(&udid, &url),
                        _ => video::install(&udid, &path),
                    }
                    .map(|()| json!({ "ok": true }))
                })
            }
            "screenshot" => Box::new(move || {
                let png = video::screenshot(&udid)?;
                let (width, height) = video::png_size(&png).unwrap_or_default();
                Ok(json!({ "data": crate::client::base64(&png), "width": width, "height": height }))
            }),
            "save" => {
                let name = arg("name");
                Box::new(move || {
                    video::save(&udid, &name).map(|p| json!({ "path": p.to_string_lossy() }))
                })
            }
            other => {
                let error = format!("video:{other} is not available in the native app yet.");
                return self.rpc.respond(id, Err(error));
            }
        };
        let responder = self.rpc.responder();
        // A page has one thread; its simulator work fails at once.
        if cfg!(target_arch = "wasm32") {
            return responder.respond(id, work());
        }
        let spawned = std::thread::Builder::new()
            .name("vorn-video".into())
            .spawn(move || responder.respond(id, work()));
        if let Err(e) = spawned {
            eprintln!("simulator thread did not start: {e}");
        }
    }

    /// Asks the Device to do `action` for `session`'s pane.
    pub(super) fn device_action(&mut self, action: &str, session: &str) {
        self.call(
            Call::Ignore,
            "extension:call",
            json!({ "id": DEVICE, "action": action, "args": { "session": session } }),
        );
    }

    /// The trees the Device draws now: each open pane's, and each card's
    /// whose button shows its picker or the simulator picked there starting.
    pub(super) fn device_trees(&self) -> Vec<String> {
        let bench = &self.st.bench;
        bench
            .devices
            .iter()
            .chain(&bench.device_cards)
            .filter_map(|s| device_key(&self.st.points, s))
            .collect()
    }

    /// Before a frame: the panes the Device lists, the simulator each
    /// asks to start or shows, and what is left of the dead sessions' panes.
    pub(super) fn video_tick(&mut self) {
        let open = (DEVICE.to_owned(), DEVICE_OPEN.to_owned());
        if !self.st.points.wanted.contains(&open) {
            self.st.points.wanted.insert(open);
        }
        let on = self
            .st
            .points
            .find(DEVICE, kind::PANES, DEVICE_PANE)
            .is_some();
        let live: HashSet<&str> = self
            .store
            .sessions
            .iter()
            .map(|s| s.info.id.as_str())
            .collect();
        let rows: Vec<(&str, bool)> = if on {
            self.st
                .points
                .rows_of(DEVICE)
                .get(DEVICE_OPEN)
                .into_iter()
                .flatten()
                .map(|r| {
                    let card = r.fields.get("card").and_then(Value::as_bool);
                    (r.id.as_str(), card.unwrap_or(false))
                })
                .collect()
        } else {
            Vec::new()
        };
        let gone = self.store.sessions_known && rows.iter().any(|(id, _)| !live.contains(id));
        let (mut devices, mut cards) = (HashSet::new(), HashSet::new());
        for (id, card) in rows.into_iter().filter(|(id, _)| live.contains(id)) {
            if card { &mut cards } else { &mut devices }.insert(id.to_owned());
        }
        let live: Vec<String> = live.into_iter().map(str::to_owned).collect();

        // In the cards' order, so a screen keeps its custom id between frames.
        let mut videos = Vec::new();
        let mut picker = None;
        let mut shown = HashMap::new();
        let mut asks = Vec::new();
        for s in &self.store.sessions {
            let session = s.info.id.as_str();
            let card = cards.contains(session);
            if !card && !devices.contains(session) {
                continue;
            }
            let Some(key) = device_key(&self.st.points, session) else {
                continue;
            };
            let Some((node, v)) = self
                .st
                .points
                .tree(&key)
                .and_then(|t| video_part(t, session))
            else {
                continue;
            };
            let mut ask = |job: Job, udid: &str, title: &str| {
                let what = (session.to_owned(), job, udid.to_owned());
                asks.push((key.clone(), node.to_owned(), what, title.to_owned()));
            };
            if v.picker.as_ref().is_some_and(|p| p.loading) {
                ask(Job::List, "", "");
            }
            if let Some(udid) = &v.device {
                if v.boot {
                    ask(Job::Boot, udid, "");
                } else if v.save {
                    ask(Job::Save, udid, &v.title);
                }
            }
            if v.picker.is_some() {
                picker = Some(session.to_owned());
            }
            // Only a pane shows the screen, and only once it is up.
            if let Some(udid) = v.device.as_ref().filter(|_| !card && !v.busy) {
                shown.insert(session.to_owned(), udid.clone());
                videos.push(session.to_owned());
            }
        }
        self.video
            .doing
            .retain(|d| asks.iter().any(|(_, _, what, _)| what == d));
        for (key, node, what, title) in asks {
            if !self.video.doing.insert(what.clone()) {
                continue;
            }
            let (_, job, udid) = what;
            self.video.spawn("vorn-video-job", move || Done::Told {
                key,
                node,
                what: run(job, &udid, &title),
            });
        }

        let bench = &mut self.st.bench;
        for s in bench.devices.difference(&devices) {
            if bench.maximized.get(s) == Some(&PaneKind::Device) {
                bench.maximized.remove(s);
            }
        }
        for s in bench.devices.union(&bench.device_cards) {
            // A closed pane's last tree goes, so opening it again never
            // shows what it held before the Device draws it afresh.
            if devices.contains(s) || cards.contains(s) {
                continue;
            }
            if let Some(key) = device_key(&self.st.points, s) {
                self.st.points.trees.remove(&key);
            }
        }
        bench.devices = devices;
        bench.device_cards = cards;
        bench.videos = videos;
        bench.video_picker = picker;
        bench.frames.retain(|s, _| shown.contains_key(s));
        self.video.pollers.retain(|s, p| {
            let keep = shown.get(s) == Some(&p.udid);
            if !keep {
                p.stop.store(true, Ordering::Relaxed);
                bench.frames.remove(s);
            }
            keep
        });
        for (session, udid) in shown {
            if !self.video.pollers.contains_key(&session) {
                if let Some(p) = self.video.poll(&session, &udid) {
                    self.video.pollers.insert(session, p);
                }
            }
        }

        if gone && !self.video.pruning {
            self.video.pruning = true;
            self.call(
                Call::Ignore,
                "extension:call",
                json!({ "id": DEVICE, "action": "prune", "args": { "live": live } }),
            );
        } else if !gone {
            self.video.pruning = false;
        }
    }

    /// Folds in what the simulator threads brought.
    pub(super) fn pump_video(&mut self) {
        while let Ok(done) = self.video.rx.try_recv() {
            match done {
                Done::Frame {
                    session,
                    udid,
                    frame,
                } => {
                    // A frame of a simulator the pane no longer shows is dropped.
                    let current = self.video.pollers.get(&session).map(|p| &p.udid);
                    if current == Some(&udid) {
                        self.st.bench.frames.insert(session, frame);
                    }
                }
                Done::Told { key, node, what } => {
                    let event =
                        vorn_tree::Event::new(node, vorn_tree::EventKind::Page, what.to_string());
                    self.send_told(&key, &event);
                }
            }
        }
    }

    /// Draws each shown simulator's latest screen into the box its pane left.
    pub(super) fn paint_video(&self, ui: &mut Ui, laid: &Laid) {
        if ui.is_cells() {
            return;
        }
        let bench = &self.st.bench;
        for (id, r) in &laid.customs {
            if !(VIDEO_ID_BASE..WEB_ID_BASE).contains(id) {
                continue;
            }
            let Some((session, frame)) = usize::try_from(id - VIDEO_ID_BASE)
                .ok()
                .and_then(|i| bench.videos.get(i))
                .and_then(|s| Some((s.as_str(), bench.frames.get(s)?)))
            else {
                continue;
            };
            let (w, h) = frame.size;
            let src = [0.0, 0.0, w as f32, h as f32];
            // Cut to the screen's corners, as its frame is drawn round them.
            let area = ui
                .rect_of(stage_id(session))
                .map(|a| (f64::from(a.w), f64::from(a.h)));
            let drawn = (f64::from(r.w), f64::from(r.h));
            let (_, _, radius) = video::frame_metrics(drawn, area, frame.points);
            ui.picture_rounded(frame.key, frame.size, &frame.rgba, src, *r, radius as f32);
        }
    }
}
