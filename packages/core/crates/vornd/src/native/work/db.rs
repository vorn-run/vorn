//! The work model's own connection to `vorn.db`, on a thread of its own.
//!
//! Every read and write the work model makes goes through one queue to one
//! thread holding one store, so a run's writes land in the order they were
//! made and a read made after a write sees it, without a lock or a blocking
//! call on an async task.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use tokio::sync::oneshot;
use tracing::warn;
use vorn_store::Store;

type Job = Box<dyn FnOnce(&mut Store) + Send>;

/// The queue to the store's thread.
#[derive(Clone, Debug)]
pub struct Db {
    jobs: mpsc::Sender<Job>,
    path: PathBuf,
}

impl Db {
    /// Opens the store beside `db` on a thread of its own. `None` when it
    /// cannot be opened.
    pub fn open(db: &Path) -> Option<Db> {
        let store = match Store::open_beside(db) {
            Ok(Some(store)) => store,
            Ok(None) => return None,
            Err(err) => {
                warn!(%err, "the work model could not open the database");
                return None;
            }
        };
        Some(Db::with(store, db.to_path_buf()))
    }

    /// A queue to `store`, which `path` names.
    pub fn with(mut store: Store, path: PathBuf) -> Db {
        let (jobs, queue) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("vornd-work-db".into())
            .spawn(move || {
                for job in queue {
                    // A job that panics loses its answer, not the thread.
                    let _ =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&mut store)));
                }
            })
            .expect("a thread for the work model's store can be started");
        Db { jobs, path }
    }

    /// The database file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Runs `f` on the store after everything queued before it. `None` when
    /// the thread is gone or `f` panicked.
    pub async fn run<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Store) -> T + Send + 'static,
    ) -> Option<T> {
        let (tx, rx) = oneshot::channel();
        let job: Job = Box::new(move |store| {
            let _ = tx.send(f(store));
        });
        self.jobs.send(job).ok()?;
        rx.await.ok()
    }

    /// Queues `f` without waiting for it.
    pub fn fire(&self, f: impl FnOnce(&mut Store) + Send + 'static) {
        let _ = self.jobs.send(Box::new(f));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn runs_jobs_in_order_and_survives_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vorn.db");
        let store = super::super::test_store(&path);
        let db = Db::with(store, path);
        db.fire(|s| {
            s.call("addScheduleLogEntry", json!([{ "workflowId": "w", "workflowName": "W", "executedAt": "2030-01-01T00:00:00.000Z", "status": "success", "sessionsLaunched": 1 }])).unwrap();
        });
        let none: Option<()> = db.run(|_| panic!("a job that fails")).await;
        assert!(none.is_none());
        let log = db
            .run(|s| s.call("getScheduleLogEntries", json!(["w"])).unwrap())
            .await
            .unwrap();
        assert_eq!(log.as_array().unwrap().len(), 1);
    }
}
