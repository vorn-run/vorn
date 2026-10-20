//! A vornd restart under a running app: the link it ended opens again. A
//! thread dials the data directory's vornd, backing off while none answers,
//! and hands the app a new link for the one that ended: the grid's, or the
//! RPC one with its models, or both. Everything taken from the endpoint (the
//! web address, the port) and from the models (tasks, layout, columns) is
//! read again. The cards attach again on the next paint, showing their last
//! screen until the first frame comes. A vornd that refuses the app (its
//! key, or the grid protocol) is not dialed again.

use std::path::Path;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{App, Call};
use crate::client::rpc::Wake;
use crate::client::{Endpoint, Event, Rpc, RpcError, Store};
use crate::grid::Grid;

/// The first wait between tries; it doubles up to [`REDIAL_MAX`].
const REDIAL: Duration = Duration::from_millis(250);
/// The longest wait between tries.
const REDIAL_MAX: Duration = Duration::from_secs(5);
/// A link that lasted this long ended on its own: the next dial starts at
/// once. One that ended sooner (a vornd that admits, then closes) waits.
const STEADY: Duration = Duration::from_secs(10);

/// The relinking state the app keeps.
pub(super) struct Relinking {
    dialing: Option<Receiver<Dialed>>,
    /// Why vornd refused this app; nothing is dialed again.
    refused: Option<String>,
    /// The wait before the next dial, grown while links keep ending soon.
    wait: Duration,
    /// When the RPC link and the grid link last opened.
    linked: (Instant, Instant),
}

impl Default for Relinking {
    fn default() -> Relinking {
        Relinking {
            dialing: None,
            refused: None,
            wait: Duration::ZERO,
            linked: (Instant::now(), Instant::now()),
        }
    }
}

/// What the dialing thread hands back.
enum Dialed {
    Linked(Relinked),
    Refused(String),
}

/// The new links, each only for the one that ended.
struct Relinked {
    ep: Endpoint,
    rpc: Option<(Rpc, Receiver<Event>, Store)>,
    grid: Option<Arc<Grid>>,
}

impl App {
    /// Takes new links in once the dialing thread has them; starts the
    /// thread when the grid or the RPC link has ended and none is dialing.
    pub(super) fn relink(&mut self) {
        if self.relinking.refused.is_some() {
            return;
        }
        if let Some(rx) = &self.relinking.dialing {
            match rx.try_recv() {
                Ok(Dialed::Linked(r)) => {
                    self.relinking.dialing = None;
                    let (now, linked) = (Instant::now(), &mut self.relinking.linked);
                    if r.rpc.is_some() {
                        linked.0 = now;
                    }
                    if r.grid.is_some() {
                        linked.1 = now;
                    }
                    self.relinked(r);
                }
                Ok(Dialed::Refused(why)) => self.refused(why),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.relinking.dialing = None,
            }
            return;
        }
        let Some(grid) = self.grid.as_ref() else {
            return;
        };
        if self.rpc.rejected() {
            return self.refused(RpcError::Rejected.to_string());
        }
        if let Some(why) = grid.refused() {
            return self.refused(why);
        }
        let (rpc, relink_grid) = (self.closed, grid.closed());
        if !rpc && !relink_grid {
            return;
        }
        let Some(dir) = self.data_dir.clone() else {
            return;
        };
        let r = &mut self.relinking;
        let soon = |at: Instant| at.elapsed() < STEADY;
        r.wait = if (rpc && soon(r.linked.0)) || (relink_grid && soon(r.linked.1)) {
            (r.wait * 2).clamp(REDIAL, REDIAL_MAX)
        } else {
            Duration::ZERO
        };
        let first = r.wait;
        // The app (and with it the grid) gone, the thread stops.
        let alive = Arc::downgrade(grid);
        let (tx, rx) = mpsc::channel();
        let wake = Arc::clone(&self.wake);
        let dialing = std::thread::Builder::new()
            .name("vornd-relink".into())
            .spawn(move || {
                std::thread::sleep(first);
                if let Some(d) = dial(&dir, rpc, relink_grid, &wake, &alive) {
                    if tx.send(d).is_ok() {
                        wake();
                    }
                }
            });
        if dialing.is_ok() {
            self.relinking.dialing = Some(rx);
        }
    }

    fn refused(&mut self, why: String) {
        self.relinking.dialing = None;
        self.grid_error = Some(why.clone());
        self.relinking.refused = Some(why);
    }

    fn relinked(&mut self, r: Relinked) {
        self.web_url = r.ep.web_url();
        self.port = r.ep.port;
        if let Some(grid) = r.grid {
            self.grid = Some(grid);
            self.grid_error = None;
        }
        if let Some((rpc, events, store)) = r.rpc {
            // The old link's calls were failed as it closed.
            self.rpc = rpc;
            self.events = events;
            let before = self.store.workspace().to_owned();
            self.store = store;
            self.closed = false;
            if self.store.workspace() != before {
                self.workspace_switched();
            }
            self.reloaded();
        }
    }

    /// What the app keeps from the models, read again from a new store.
    fn reloaded(&mut self) {
        self.st.layout = super::LayoutMode::from_config(&self.store.config);
        self.st.grids =
            crate::flex::grids_from_json(self.store.config.pointer("/defaults/gridLayout"));
        self.load_columns();
        self.see_projects();
        self.st.tasks.reloading = false;
        self.reload_tasks();
        self.call(Call::Sessions, "terminal:listActive", Value::Null);
        self.call(Call::Restored, "sessions:restored", Value::Null);
        self.call(Call::Headless, "headless:list", Value::Null);
        self.call(Call::Installed, "agent:locate", Value::Null);
    }
}

/// Tries the vornd `dir` names until one answers, while `alive` holds,
/// waiting longer after each miss.
fn dial(dir: &Path, rpc: bool, grid: bool, wake: &Wake, alive: &Weak<Grid>) -> Option<Dialed> {
    let mut wait = REDIAL;
    while alive.strong_count() > 0 {
        if let Ok(ep) = Endpoint::find(dir) {
            match open(ep, rpc, grid, wake) {
                Ok(r) => return Some(Dialed::Linked(r)),
                Err(RpcError::Rejected) => {
                    return Some(Dialed::Refused(RpcError::Rejected.to_string()))
                }
                Err(_) => {}
            }
        }
        std::thread::sleep(wait);
        wait = (wait * 2).min(REDIAL_MAX);
    }
    None
}

fn open(ep: Endpoint, rpc: bool, grid: bool, wake: &Wake) -> Result<Relinked, RpcError> {
    let rpc = if rpc {
        let (rpc, events) = Rpc::connect(&ep, Arc::clone(wake))?;
        let store = Store::load(&rpc)?;
        Some((rpc, events, store))
    } else {
        None
    };
    let grid = if grid {
        let g = Grid::connect(&ep.grid, &ep.token, Arc::clone(wake))
            .map_err(|e| RpcError::Connect(e.to_string()))?;
        Some(g)
    } else {
        None
    };
    Ok(Relinked { ep, rpc, grid })
}
