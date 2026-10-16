//! The phone's link to one vornd: sessions, the open terminal and extension settings, kept current.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use serde_json::{json, Value};
use vorn_app::act::{Act, Acts};
use vorn_app::client::link::Socket;
use vorn_app::client::rpc::{Event as RpcEvent, Pending, Rpc, Wake};
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

/// What a call's answer is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum For {
    /// A read kept by its key until asked again; run lists fold into the store's runs.
    Fetch(String),
    /// A new session, opened once it exists.
    Launch,
    /// A manual run of a workflow, by id; `true` goes back once it starts.
    Run(String, bool),
    /// A gate's answer, by `run|node`; `true` goes back once it is taken.
    Gate(String, bool),
    /// An answer nothing waits on; a failure shows as the screen's error.
    Quiet,
}

/// A read's answer so far.
#[derive(Debug, Clone, PartialEq)]
pub enum Fetched {
    Loading,
    Ready(Value),
    Failed(String),
}

struct Connected {
    rpc: Rpc,
    events: Receiver<RpcEvent>,
    store: Store,
    grid: Arc<Grid>,
    permissions: Vec<Value>,
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
    /// Permission requests agents hold open, oldest first.
    pub permissions: Vec<Value>,
    /// What went wrong, by what it was about: `launch`, `run:<workflow>`, `gate:<run|node>`, `quiet`.
    pub errors: HashMap<String, String>,
    /// Calls whose button waits, by the same keys.
    pub busy: HashSet<String>,
    /// Where the host goes next: a route, `replace:<route>` or `back`.
    pub go: Option<String>,
    calls: RefCell<Vec<(For, Pending)>>,
    fetched: RefCell<HashMap<String, Fetched>>,
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
            permissions: Vec::new(),
            errors: HashMap::new(),
            busy: HashSet::new(),
            go: None,
            calls: RefCell::new(Vec::new()),
            fetched: RefCell::new(HashMap::new()),
            wake,
        }
    }

    /// Calls `method`; its answer is folded in by [`Model::pump`] for `what`.
    pub fn call(&self, what: For, method: &str, params: Value) {
        let Some(rpc) = &self.rpc else { return };
        self.calls
            .borrow_mut()
            .push((what, rpc.request(method, params)));
    }

    /// `method`'s answer, asked once and kept until [`Model::refetch`].
    pub fn fetch(&self, method: &str, params: Value) -> Fetched {
        let key = format!("{method} {params}");
        if let Some(f) = self.fetched.borrow().get(&key) {
            return f.clone();
        }
        if self.rpc.is_none() {
            return Fetched::Loading;
        }
        self.fetched
            .borrow_mut()
            .insert(key.clone(), Fetched::Loading);
        self.call(For::Fetch(key), method, params);
        Fetched::Loading
    }

    /// Asks every read again, keeping what each said until it answers.
    pub fn refetch(&self) {
        let keys: Vec<String> = self.fetched.borrow().keys().cloned().collect();
        for key in keys {
            let Some((method, params)) = key.split_once(' ') else {
                continue;
            };
            let params = serde_json::from_str(params).unwrap_or(Value::Null);
            self.call(For::Fetch(key.clone()), method, params);
        }
    }

    fn answered(&mut self, what: For, answer: Result<Value, String>) {
        match (what, answer) {
            (For::Fetch(key), Ok(v)) => {
                if key.starts_with("workflowRun:") {
                    for run in v.as_array().into_iter().flatten() {
                        self.store.apply("workflow:runUpdated", run.clone());
                    }
                }
                self.fetched.get_mut().insert(key, Fetched::Ready(v));
            }
            (For::Fetch(key), Err(e)) => {
                let keep = matches!(self.fetched.get_mut().get(&key), Some(Fetched::Ready(_)));
                if !keep {
                    self.fetched.get_mut().insert(key, Fetched::Failed(e));
                }
            }
            (For::Launch, Ok(v)) => {
                self.busy.remove("launch");
                if let Ok(info) = serde_json::from_value(v.clone()) {
                    self.store.upsert(info);
                }
                if let Some(id) = v["id"].as_str() {
                    self.go = Some(format!("replace:session:{id}"));
                }
            }
            (For::Run(wf, back), Ok(_)) => {
                self.busy.remove(&format!("run:{wf}"));
                if back {
                    self.go = Some("back".into());
                }
            }
            (For::Gate(key, back), Ok(_)) => {
                self.busy.remove(&format!("gate:{key}"));
                if back {
                    self.go = Some("back".into());
                }
            }
            (For::Quiet, Ok(_)) => {}
            (what, Err(e)) => {
                let key = match what {
                    For::Launch => "launch".to_owned(),
                    For::Run(wf, _) => format!("run:{wf}"),
                    For::Gate(key, _) => format!("gate:{key}"),
                    _ => "quiet".to_owned(),
                };
                self.busy.remove(&key);
                self.errors.insert(key, e);
            }
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
                        self.permissions = c.permissions;
                        self.link = Link::Online;
                        self.reload_points();
                    }
                    Err(e) => self.link = Link::Offline(e),
                }
            }
        }
        let answers: Vec<(For, Result<Value, String>)> = {
            let mut calls = self.calls.borrow_mut();
            let mut done = Vec::new();
            calls.retain(|(what, p)| match p.poll() {
                Some(a) => {
                    done.push((what.clone(), a.map_err(|e| e.to_string())));
                    false
                }
                None => true,
            });
            done
        };
        for (what, a) in answers {
            changed = true;
            self.answered(what, a);
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
                    "permission:requested" => self.permissions.push(params),
                    "permission:closed" => {
                        let id = &params["requestId"];
                        self.permissions.retain(|p| &p["requestId"] != id);
                    }
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
    topics.push("permission:*");
    rpc.call(
        "subscribe:set",
        json!({ "topics": topics, "terminalBytes": false }),
    )
    .map_err(|e| e.to_string())?;
    let list = rpc
        .call("terminal:listActive", Value::Null)
        .map_err(|e| e.to_string())?;
    store.take_sessions(list);
    // A vornd from before `permission:list` lists none; its sessions still say they wait.
    let permissions = match rpc.call("permission:list", Value::Null) {
        Ok(Value::Array(open)) => open,
        _ => Vec::new(),
    };
    let grid = Grid::over(Box::new(socket("grid")), wake).map_err(|e| e.to_string())?;
    Ok(Connected {
        rpc,
        events,
        store,
        grid,
        permissions,
    })
}
