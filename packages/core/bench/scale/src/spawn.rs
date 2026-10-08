//! Starting many sessions with a window of requests in flight, as a server
//! restoring a workspace would, timing each and the whole.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::error::Error;
use crate::stats::Rate;

/// How long one start may take before the bench calls it stuck.
pub const SPAWN_WITHIN: Duration = Duration::from_secs(120);

/// Something that starts sessions by request number.
pub(crate) trait Spawner {
    /// Asks for session number `i`, tagged `req`.
    async fn request(&mut self, req: u64, i: usize) -> Result<(), Error>;
    /// The next answer: the request it is for, and the session's id or why
    /// it was not started.
    async fn answer(&mut self) -> Result<(u64, Result<String, String>), Error>;
}

/// The sessions started, and the first refusal, after which no more were
/// asked for.
#[derive(Debug)]
pub struct Spawned {
    pub ids: Vec<String>,
    pub rate: Rate,
    pub refused: Option<String>,
}

/// Starts `n` sessions, `window` at a time. Request numbers start at
/// `first_req`, so several batches on one connection never share one.
pub(crate) async fn spawn_many(
    s: &mut impl Spawner,
    n: usize,
    window: usize,
    first_req: u64,
) -> Result<Spawned, Error> {
    let start = Instant::now();
    let mut asked: HashMap<u64, Instant> = HashMap::with_capacity(window);
    let mut took = Vec::with_capacity(n);
    let mut ids = Vec::with_capacity(n);
    let mut refused = None;
    let mut next = 0;
    while next < n || !asked.is_empty() {
        while refused.is_none() && next < n && asked.len() < window.max(1) {
            let req = first_req + next as u64;
            s.request(req, next).await?;
            asked.insert(req, Instant::now());
            next += 1;
        }
        if asked.is_empty() {
            break;
        }
        let (req, done) = tokio::time::timeout(SPAWN_WITHIN, s.answer())
            .await
            .map_err(|_| Error::Timeout(format!("a session to start, {} started", ids.len())))??;
        let Some(at) = asked.remove(&req) else {
            continue;
        };
        match done {
            Ok(id) => {
                took.push(at.elapsed());
                ids.push(id);
            }
            Err(why) => {
                refused.get_or_insert(why);
            }
        }
    }
    Ok(Spawned {
        rate: Rate::of(start.elapsed(), &mut took),
        ids,
        refused,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    /// Answers in the order asked, refusing from `fail_from` on, and
    /// records the most requests it ever had open.
    struct Fake {
        open: VecDeque<(u64, usize)>,
        fail_from: usize,
        most: usize,
    }

    impl Spawner for Fake {
        async fn request(&mut self, req: u64, i: usize) -> Result<(), Error> {
            self.open.push_back((req, i));
            self.most = self.most.max(self.open.len());
            Ok(())
        }
        async fn answer(&mut self) -> Result<(u64, Result<String, String>), Error> {
            let (req, i) = self.open.pop_front().ok_or(Error::Closed("fake"))?;
            Ok((
                req,
                if i < self.fail_from {
                    Ok(format!("s{i}"))
                } else {
                    Err("full".into())
                },
            ))
        }
    }

    fn fake(fail_from: usize) -> Fake {
        Fake {
            open: VecDeque::new(),
            fail_from,
            most: 0,
        }
    }

    #[tokio::test]
    async fn starts_every_session_within_the_window() {
        let mut f = fake(usize::MAX);
        let s = spawn_many(&mut f, 10, 3, 100).await.unwrap();
        assert_eq!(s.ids.len(), 10);
        assert_eq!(s.ids[9], "s9");
        assert_eq!(s.rate.count, 10);
        assert!(s.refused.is_none());
        assert_eq!(f.most, 3);
    }

    #[tokio::test]
    async fn stops_asking_after_the_first_refusal() {
        let mut f = fake(4);
        let s = spawn_many(&mut f, 100, 2, 0).await.unwrap();
        // The refusal of 4 arrives with 5 already asked for.
        assert_eq!(s.ids.len(), 4);
        assert_eq!(s.refused.as_deref(), Some("full"));
        assert!(f.open.is_empty());
    }
}
