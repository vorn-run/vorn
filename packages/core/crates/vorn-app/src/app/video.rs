//! The video primitive's side of the app: the simulators vornd asks after
//! for the Device extension (`video:*`), the device a pane asks to start,
//! started off the UI thread, and each shown pane's frames, taken on a
//! thread of their own and drawn here as pictures. Frames never pass
//! through the extension: it holds which simulator a session has and what
//! its pane shows, and this side only takes the screens.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use web_time::Duration;

use serde_json::{json, Value};

use super::{App, Call};
use crate::client::rpc::Wake;
use crate::panes::{
    device_key, video_part, PaneKind, DEVICE, DEVICE_OPEN, DEVICE_PANE, VIDEO_ID_BASE,
    WEB_ID_BASE,
};
use crate::points::kind;
use crate::ui::{Laid, Ui};
use crate::video::{self, Frame};

/// How often a shown simulator's screen is taken.
const FRAME_EVERY: Duration = Duration::from_millis(250);

/// What a simulator thread finished.
enum Done {
    /// A new screen of `udid`, for `session`'s pane.
    Frame {
        session: String,
        udid: String,
        frame: Frame,
    },
    /// `udid` started for `session`'s pane, or could not.
    Booted {
        key: String,
        node: String,
        result: Result<bool, String>,
    },
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
    /// `(session, udid)` being started for a pane, until its tree stops asking.
    booting: HashSet<(String, String)>,
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
            booting: HashSet::new(),
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
                    std::thread::sleep(FRAME_EVERY);
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
                let (op, bundle, url, path) = (op.to_owned(), arg("bundleId"), arg("url"), arg("path"));
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

    /// The trees the Device draws now: each open pane's.
    pub(super) fn device_trees(&self) -> Vec<String> {
        self.st
            .bench
            .devices
            .iter()
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
        let rows: Vec<&str> = if on {
            self.st
                .points
                .rows_of(DEVICE)
                .get(DEVICE_OPEN)
                .into_iter()
                .flatten()
                .map(|r| r.id.as_str())
                .collect()
        } else {
            Vec::new()
        };
        let gone = self.store.sessions_known && rows.iter().any(|id| !live.contains(id));
        let devices: HashSet<String> = rows
            .iter()
            .filter(|id| live.contains(*id))
            .map(|id| (*id).to_owned())
            .collect();
        let live: Vec<String> = live.into_iter().map(str::to_owned).collect();

        // In the cards' order, so a screen keeps its custom id between frames.
        let mut videos = Vec::new();
        let mut picker = None;
        let mut shown = HashMap::new();
        let mut boots = Vec::new();
        for s in &self.store.sessions {
            let session = s.info.id.as_str();
            if !devices.contains(session) {
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
            if v.picker.is_some() {
                picker = Some(session.to_owned());
            }
            match (&v.device, v.boot) {
                (Some(udid), true) => boots.push((key.clone(), node.to_owned(), session, udid)),
                (Some(udid), false) => {
                    shown.insert(session.to_owned(), udid.clone());
                    videos.push(session.to_owned());
                }
                (None, _) => {}
            }
        }
        let asked: HashSet<(String, String)> = boots
            .iter()
            .map(|(_, _, s, u)| ((*s).to_owned(), (*u).clone()))
            .collect();
        self.video.booting.retain(|b| asked.contains(b));
        for (key, node, session, udid) in boots {
            let pair = (session.to_owned(), udid.clone());
            if self.video.booting.insert(pair) {
                let udid = udid.clone();
                self.video.spawn("vorn-video-boot", move || Done::Booted {
                    key,
                    node,
                    result: video::boot(&udid).map(|(_, here)| here),
                });
            }
        }

        let bench = &mut self.st.bench;
        for s in bench.devices.difference(&devices) {
            if bench.maximized.get(s) == Some(&PaneKind::Device) {
                bench.maximized.remove(s);
            }
        }
        bench.devices = devices;
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
                Done::Booted { key, node, result } => {
                    let what = match result {
                        Ok(here) => json!({ "type": "booted", "bootedHere": here }),
                        Err(message) => json!({ "type": "failed", "message": message }),
                    };
                    let event = vorn_tree::Event::new(
                        node,
                        vorn_tree::EventKind::Page,
                        what.to_string(),
                    );
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
            let Some(frame) = usize::try_from(id - VIDEO_ID_BASE)
                .ok()
                .and_then(|i| bench.videos.get(i))
                .and_then(|s| bench.frames.get(s))
            else {
                continue;
            };
            let (w, h) = frame.size;
            let src = [0.0, 0.0, w as f32, h as f32];
            ui.picture(frame.key, frame.size, &frame.rgba, src, *r);
        }
    }
}
