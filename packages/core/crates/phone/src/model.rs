//! The phone's connection to one vornd: its sessions, the one terminal
//! shown, and the extensions' settings pages, kept current from its
//! notifications. Connecting runs off the UI thread.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use serde_json::{json, Value};
use vorn_app::act::{Act, Acts};
use vorn_app::client::link::Socket;
use vorn_app::client::rpc::{Event as RpcEvent, Rpc, Wake};
use vorn_app::client::store::{Store, TOPICS};
use vorn_app::grid::Grid;
use vorn_app::points::{self, kind, Point, PointAct, Points, Press};
use vorn_app::view::PaneView;
use vornui::{Id, Ui};

/// Where the phone is pointed: `host:port` and a device token.
#[derive(Clone)]
pub struct Target {
    pub host: String,
    pub token: String,
}

/// How the connection stands, for the header's computer pill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    Unpaired,
    Connecting,
    Online,
    Offline(String),
}

struct Connected {
    rpc: Rpc,
    events: Receiver<RpcEvent>,
    store: Store,
    grid: Arc<Grid>,
}

/// One vornd, as the phone's screens read it.
pub struct Model {
    pub target: Option<Target>,
    pub link: Link,
    rpc: Option<Rpc>,
    events: Option<Receiver<RpcEvent>>,
    pending: Option<Receiver<Result<Connected, String>>>,
    pub store: Store,
    pub grid: Option<Arc<Grid>>,
    pub views: HashMap<String, PaneView>,
    /// Counts the terminal updates folded in, for keystroke latency.
    pub grid_updates: u64,
    pub points: Points,
    /// What each template control does, rebound every frame it is drawn.
    pub acts: Acts,
    wake: Wake,
}

impl Model {
    pub fn new(wake: Wake) -> Model {
        Model {
            target: None,
            link: Link::Unpaired,
            rpc: None,
            events: None,
            pending: None,
            store: Store::default(),
            grid: None,
            views: HashMap::new(),
            grid_updates: 0,
            points: Points::default(),
            acts: Acts::default(),
            wake,
        }
    }

    /// Connects to `t` on a thread of its own; [`Model::pump`] picks it up.
    pub fn connect(&mut self, t: Target) {
        self.target = Some(t.clone());
        self.link = Link::Connecting;
        let (tx, rx) = mpsc::channel();
        let wake = self.wake.clone();
        self.pending = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(open(&t, wake.clone()));
            wake();
        });
    }

    /// The settings pages extensions contribute.
    pub fn settings(&self) -> impl Iterator<Item = &Point> {
        self.points.at(kind::SETTINGS)
    }

    /// Folds in what arrived since the last frame; answers whether anything changed.
    pub fn pump(&mut self) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.pending {
            if let Ok(done) = rx.try_recv() {
                self.pending = None;
                changed = true;
                match done {
                    Ok(c) => {
                        self.store = c.store;
                        self.rpc = Some(c.rpc);
                        self.events = Some(c.events);
                        self.grid = Some(c.grid);
                        self.link = Link::Online;
                        self.reload_points();
                    }
                    Err(e) => self.link = Link::Offline(e),
                }
            }
        }
        let mut stale = false;
        if let Some(events) = &self.events {
            while let Ok(e) = events.try_recv() {
                let RpcEvent::Notification { method, params } = e else {
                    continue;
                };
                changed = true;
                match method.as_str() {
                    "extension:rowsChanged" => {
                        let (Some(ext), Some(coll)) = (
                            params["extensionId"].as_str(),
                            params["collection"].as_str(),
                        ) else {
                            continue;
                        };
                        let ops = params["ops"].as_array().cloned().unwrap_or_default();
                        self.points.changed(ext, coll, &ops);
                    }
                    "extension:contributionsChanged" => stale = true,
                    m => {
                        self.store.apply(m, params);
                    }
                }
            }
        }
        if stale {
            self.reload_points();
        }
        if let Some(g) = &self.grid {
            for id in g.take_dirty() {
                if let Some(v) = g.view(&id) {
                    self.views.insert(id, v);
                    self.grid_updates += 1;
                    changed = true;
                }
            }
            if g.closed() && self.link == Link::Online {
                self.link = Link::Offline("The connection closed".into());
                changed = true;
            }
        }
        changed
    }

    fn reload_points(&mut self) {
        let Some(rpc) = &self.rpc else { return };
        let list = rpc
            .call("extension:contributions", Value::Null)
            .ok()
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        self.points.listed(&list);
        for (ext, coll) in self.points.to_ask() {
            let answer = rpc.call("extension:rows", json!({ "id": ext, "collection": coll }));
            let rows = match &answer {
                Ok(Value::Array(rows)) => Some(rows.as_slice()),
                _ => None,
            };
            self.points.fill(&ext, &coll, rows);
        }
    }

    /// A template control was pressed.
    pub fn click(&mut self, id: Id) {
        let Some(Act::Point(a)) = self.acts.clicked(id) else {
            return;
        };
        match a {
            PointAct::Press {
                extension, press, ..
            } => {
                self.points.asking = None;
                self.points.more = None;
                self.press(&extension, &press);
            }
            PointAct::Ask(id) => self.points.asking = id,
            PointAct::More(id) => self.points.more = id,
            _ => {}
        }
    }

    fn press(&self, extension: &str, press: &Press) {
        let Some(rpc) = &self.rpc else { return };
        if let Some(params) = points::call_params(extension, press) {
            let _ = rpc.request("extension:call", params);
        }
    }

    /// A template field's Return: its call with what the field holds.
    pub fn submit(&self, ui: &Ui, id: Id) {
        let (
            Some(rpc),
            Some(Act::Point(PointAct::Field {
                extension,
                call,
                row,
            })),
        ) = (&self.rpc, self.acts.text_of(id))
        else {
            return;
        };
        let mut args = json!({ "value": ui.edit_text(id) });
        if let Some(r) = row {
            args["id"] = json!(r);
        }
        let _ = rpc.request(
            "extension:call",
            json!({ "id": extension, "action": call, "args": args }),
        );
    }

    /// Sends `text` to `session` as typed.
    pub fn type_text(&self, session: &str, text: &str) {
        if let Some(g) = &self.grid {
            g.text(session, text);
        }
    }

    /// Sends key `code` (a DOM code) with `mods` to `session`.
    pub fn key(&self, session: &str, code: &str, mods: u16, text: Option<&str>) {
        if let Some(g) = &self.grid {
            g.key(session, code, mods, text);
        }
    }
}

fn open(t: &Target, wake: Wake) -> Result<Connected, String> {
    let socket = |path: &str| Socket {
        url: format!("ws://{}/{path}", t.host),
        token: t.token.clone(),
    };
    let (rpc, events) =
        Rpc::over(Box::new(socket("ws")), wake.clone()).map_err(|e| e.to_string())?;
    let mut store = Store::load(&rpc).map_err(|e| e.to_string())?;
    let mut topics: Vec<&str> = TOPICS.to_vec();
    topics.push("extension:*");
    rpc.call(
        "subscribe:set",
        json!({ "topics": topics, "terminalBytes": false }),
    )
    .map_err(|e| e.to_string())?;
    let list = rpc
        .call("terminal:listActive", Value::Null)
        .map_err(|e| e.to_string())?;
    store.take_sessions(list);
    let grid = Grid::over(Box::new(socket("grid")), wake).map_err(|e| e.to_string())?;
    Ok(Connected {
        rpc,
        events,
        store,
        grid,
    })
}
