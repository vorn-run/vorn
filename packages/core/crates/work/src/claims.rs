//! Which run may carry out a workflow trigger (`workflowRun:claim`).
//!
//! One claim is granted per workflow and trigger fingerprint inside its
//! window; a second identical trigger is told the run already holding it.
//! Claims lapse on their own, so a run that disappears never wedges its
//! workflow, and only the holder may release one early.

use std::collections::HashMap;

/// Long enough to swallow one tick seen twice, short enough that a
/// deliberate re-run moments later still goes through.
pub const DEFAULT_WINDOW_MS: i64 = 10_000;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Held {
    run_id: String,
    claimed_at: i64,
    window_ms: i64,
}

/// What a claim came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Claim {
    /// False when another run already holds this trigger.
    pub granted: bool,
    /// The holder's run: the new one when granted.
    pub run_id: String,
}

/// The claims held now, by workflow and fingerprint.
#[derive(Debug, Default)]
pub struct Claims {
    held: HashMap<(String, String), Held>,
}

/// An absent or empty fingerprint is a manual run's.
fn fingerprint(params: Option<&str>) -> String {
    params
        .filter(|p| !p.is_empty())
        .unwrap_or("manual")
        .to_owned()
}

impl Claims {
    /// Claims `workflow_id` with `params` at `now_ms`, for `new_run_id` when
    /// granted.
    pub fn claim(
        &mut self,
        workflow_id: &str,
        params: Option<&str>,
        window_ms: Option<i64>,
        now_ms: i64,
        new_run_id: impl FnOnce() -> String,
    ) -> Claim {
        self.held.retain(|_, h| now_ms - h.claimed_at < h.window_ms);
        let key = (workflow_id.to_owned(), fingerprint(params));
        if let Some(held) = self.held.get(&key) {
            return Claim {
                granted: false,
                run_id: held.run_id.clone(),
            };
        }
        let run_id = new_run_id();
        self.held.insert(
            key,
            Held {
                run_id: run_id.clone(),
                claimed_at: now_ms,
                window_ms: window_ms.unwrap_or(DEFAULT_WINDOW_MS),
            },
        );
        Claim {
            granted: true,
            run_id,
        }
    }

    /// Gives a claim up early; only its holder may.
    pub fn release(&mut self, workflow_id: &str, params: Option<&str>, run_id: &str) {
        let key = (workflow_id.to_owned(), fingerprint(params));
        if self.held.get(&key).is_some_and(|h| h.run_id == run_id) {
            self.held.remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_claim_per_trigger_inside_the_window() {
        let mut claims = Claims::default();
        let first = claims.claim("w", Some("task:1"), None, 0, || "r1".into());
        assert!(first.granted);
        let again = claims.claim("w", Some("task:1"), None, 5_000, || "r2".into());
        assert_eq!(
            again,
            Claim {
                granted: false,
                run_id: "r1".into()
            }
        );
        assert!(
            claims
                .claim("w", Some("task:2"), None, 5_000, || "r3".into())
                .granted
        );
        // Lapsed.
        assert!(
            claims
                .claim("w", Some("task:1"), None, 10_000, || "r4".into())
                .granted
        );
    }

    #[test]
    fn only_the_holder_releases_and_empty_is_manual() {
        let mut claims = Claims::default();
        claims.claim("w", None, Some(60_000), 0, || "r1".into());
        claims.release("w", Some(""), "other");
        assert!(
            !claims
                .claim("w", Some("manual"), None, 1, || "x".into())
                .granted
        );
        claims.release("w", None, "r1");
        assert!(claims.claim("w", None, None, 2, || "r2".into()).granted);
    }
}
