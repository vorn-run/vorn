//! A vornd restart under a running app: the links it ended open again. A
//! thread dials the data directory's vornd until one answers, then hands the
//! app a new grid link (and a new RPC link, models loaded, when that one
//! ended too). The cards attach again on the next paint, showing their last
//! screen until the first frame comes.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Weak};
use std::time::Duration;

use super::App;
use crate::client::rpc::Wake;
use crate::client::{Endpoint, Event, Rpc, Store};
use crate::grid::Grid;

/// How long the dialing thread waits between tries.
const REDIAL: Duration = Duration::from_millis(250);

/// What the dialing thread hands back.
pub(super) struct Relinked {
    /// A new RPC link with its models, when the old one had ended.
    rpc: Option<(Rpc, Receiver<Event>, Store)>,
    grid: Arc<Grid>,
}

impl App {
    /// Takes a new link in once the dialing thread has one; starts the
    /// thread when the grid or the RPC link has ended and none is dialing.
    pub(super) fn relink(&mut self) {
        if let Some(rx) = &self.relinking {
            match rx.try_recv() {
                Ok(r) => {
                    self.relinking = None;
                    self.relinked(r);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.relinking = None,
            }
            return;
        }
        let grid = self.grid.as_ref().filter(|g| g.closed());
        if grid.is_none() && !self.closed {
            return;
        }
        let Some(dir) = self.data_dir.clone() else {
            return;
        };
        // The app (and with it the old link) gone, the thread stops.
        let alive = match grid.or(self.grid.as_ref()) {
            Some(g) => Arc::downgrade(g),
            None => return,
        };
        let (tx, rx) = mpsc::channel();
        let (rpc, wake) = (self.closed, Arc::clone(&self.wake));
        let dialing = std::thread::Builder::new()
            .name("vornd-relink".into())
            .spawn(move || {
                if let Some(r) = dial(&dir, rpc, &wake, &alive) {
                    if tx.send(r).is_ok() {
                        wake();
                    }
                }
            });
        if dialing.is_ok() {
            self.relinking = Some(rx);
        }
    }

    fn relinked(&mut self, r: Relinked) {
        if let Some((rpc, events, store)) = r.rpc {
            self.rpc = rpc;
            self.events = events;
            self.store = store;
            self.closed = false;
        }
        self.grid = Some(r.grid);
        self.grid_error = None;
    }
}

/// Tries the vornd `dir` names until one answers, while `alive` holds.
fn dial(dir: &PathBuf, rpc: bool, wake: &Wake, alive: &Weak<Grid>) -> Option<Relinked> {
    while alive.strong_count() > 0 {
        if let Some(r) = Endpoint::find(dir).ok().and_then(|ep| open(&ep, rpc, wake)) {
            return Some(r);
        }
        std::thread::sleep(REDIAL);
    }
    None
}

fn open(ep: &Endpoint, rpc: bool, wake: &Wake) -> Option<Relinked> {
    let rpc = if rpc {
        let (rpc, events) = Rpc::connect(ep, Arc::clone(wake)).ok()?;
        let store = Store::load(&rpc).ok()?;
        Some((rpc, events, store))
    } else {
        None
    };
    let grid = Grid::connect(&ep.grid, &ep.token, Arc::clone(wake)).ok()?;
    Some(Relinked { rpc, grid })
}
