//! The one lifecycle every extension program runs under, whichever protocol
//! it speaks: started on its first call or event, stopped after it sits idle,
//! and held to a cap on how many run at once, the least recently used
//! stopping first.
//!
//! Each service is its own child with its own reader tasks, and a call holds
//! no lock while it waits, so a slow or stalled service never holds up
//! another's call; starts are serialised per service only. A service that
//! ends on its own counts as a crash and starts again on its next use, unless
//! it has crashed [`Policy::max_crashes`] times within [`Policy::crash_window`].

use std::any::Any;
use std::collections::HashMap;
use std::future::Future;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::Duration;

use tokio::time::Instant;
use tracing::{info, warn};

/// Something the lifecycle keeps running: a child or a component, however it is spoken to.
pub trait Running: Send + Sync + 'static {
    fn name(&self) -> &str;
    /// Whether it ended; one that ended on its own counts as a crash.
    fn exited(&self) -> bool;
    fn close(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

/// [`Running`] for a type that is one child.
#[macro_export]
macro_rules! running_child {
    ($ty:ty, $field:tt) => {
        impl $crate::lifecycle::Running for $ty {
            fn name(&self) -> &str {
                self.$field.name()
            }
            fn exited(&self) -> bool {
                self.$field.exited()
            }
            fn close(
                &self,
            ) -> ::std::pin::Pin<Box<dyn ::std::future::Future<Output = ()> + Send + '_>> {
                Box::pin(self.$field.close())
            }
        }
    };
}

/// Which program, for what: a project, a connection, or nothing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ServiceKey {
    pub extension: String,
    pub scope: String,
}

impl ServiceKey {
    pub fn new(extension: impl Into<String>, scope: impl Into<String>) -> ServiceKey {
        ServiceKey {
            extension: extension.into(),
            scope: scope.into(),
        }
    }
}

/// How long services live and how many run at once.
///
/// The defaults come from the 20-extension load test
/// (docs/extension-load-test.md). Twenty busy extensions peak at 19 running
/// services with Vorn's own, so a cap of 24 never evicts. One idle minute
/// frees every program's memory, and the next call costs at most ~160 ms (Node).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Stopped after this long with no call in flight.
    pub idle: Duration,
    /// At most this many run at once; starting one more stops the least recently used.
    pub cap: usize,
    pub max_crashes: usize,
    pub crash_window: Duration,
}

impl Default for Policy {
    fn default() -> Policy {
        Policy {
            idle: Duration::from_secs(60),
            cap: 24,
            max_crashes: 5,
            crash_window: Duration::from_secs(60),
        }
    }
}

impl Policy {
    /// This policy with the idle time and cap a bench gives vornd
    /// (`VORN_EXTENSION_IDLE_MS`, `VORN_EXTENSION_CAP`); a value that is not
    /// a positive number leaves the policy's own.
    #[must_use]
    pub fn overridden(self, idle_ms: Option<&str>, cap: Option<&str>) -> Policy {
        let positive = |v: Option<&str>| {
            v.and_then(|v| v.trim().parse::<u64>().ok())
                .filter(|&n| n > 0)
        };
        Policy {
            idle: positive(idle_ms).map_or(self.idle, Duration::from_millis),
            cap: positive(cap)
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(self.cap),
            ..self
        }
    }
}

#[derive(Clone)]
struct Held {
    any: Arc<dyn Any + Send + Sync>,
    run: Arc<dyn Running>,
}

struct SlotState {
    held: Option<Held>,
    last_used: Instant,
    in_flight: usize,
    crashes: Vec<Instant>,
}

struct Slot {
    /// Held while starting, so callers that arrive meanwhile share one start.
    gate: tokio::sync::Mutex<()>,
    state: Mutex<SlotState>,
}

impl Slot {
    fn new() -> Slot {
        Slot {
            gate: tokio::sync::Mutex::new(()),
            state: Mutex::new(SlotState {
                held: None,
                last_used: Instant::now(),
                in_flight: 0,
                crashes: Vec::new(),
            }),
        }
    }
}

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A service in use. While any lease is held it is neither idle nor
/// evicted; dropping the last one starts its idle time.
pub struct Lease<T: ?Sized> {
    service: Arc<T>,
    slot: Arc<Slot>,
}

impl<T: ?Sized> Deref for Lease<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.service
    }
}

impl<T: ?Sized> Lease<T> {
    /// The service itself, to hold beyond the lease.
    pub fn shared(&self) -> Arc<T> {
        Arc::clone(&self.service)
    }
}

impl<T: ?Sized> Drop for Lease<T> {
    fn drop(&mut self) {
        let mut st = guard(&self.slot.state);
        st.in_flight = st.in_flight.saturating_sub(1);
        st.last_used = Instant::now();
    }
}

impl<T: ?Sized> std::fmt::Debug for Lease<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Lease")
    }
}

/// Every service, by key.
pub struct Lifecycle {
    policy: Mutex<Policy>,
    slots: Mutex<HashMap<ServiceKey, Arc<Slot>>>,
    reaper: OnceLock<()>,
    me: Weak<Lifecycle>,
}

impl std::fmt::Debug for Lifecycle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lifecycle")
            .field("policy", &*guard(&self.policy))
            .field("running", &self.running().len())
            .finish()
    }
}

impl Lifecycle {
    pub fn new(policy: Policy) -> Arc<Lifecycle> {
        Arc::new_cyclic(|me| Lifecycle {
            policy: Mutex::new(policy),
            slots: Mutex::new(HashMap::new()),
            reaper: OnceLock::new(),
            me: me.clone(),
        })
    }

    pub fn policy(&self) -> Policy {
        *guard(&self.policy)
    }

    /// Changes the policy; a lower cap takes effect at the next start.
    pub fn set_policy(&self, policy: Policy) {
        *guard(&self.policy) = policy;
    }

    fn slot(&self, key: &ServiceKey) -> Arc<Slot> {
        Arc::clone(
            guard(&self.slots)
                .entry(key.clone())
                .or_insert_with(|| Arc::new(Slot::new())),
        )
    }

    /// The running service `key`, started by `start` if it is not.
    pub async fn get_or_start<T, F, Fut>(
        &self,
        key: &ServiceKey,
        start: F,
    ) -> Result<Lease<T>, String>
    where
        T: Running,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, String>>,
    {
        self.spawn_reaper();
        let slot = self.slot(key);
        let _starting = slot.gate.lock().await;
        if let Some(lease) = lease_of::<T>(&slot, key)? {
            return Ok(lease);
        }
        self.crash_budget(&slot, key)?;
        self.make_room(key);
        let service = Arc::new(start().await?);
        let still_wanted = guard(&self.slots)
            .get(key)
            .is_some_and(|s| Arc::ptr_eq(s, &slot));
        if !still_wanted {
            // Stopped while it started.
            service.close().await;
            return Err(format!("{} was stopped before it started", service.name()));
        }
        let mut st = guard(&slot.state);
        st.held = Some(Held {
            any: Arc::clone(&service) as Arc<dyn Any + Send + Sync>,
            run: Arc::clone(&service) as Arc<dyn Running>,
        });
        st.in_flight += 1;
        st.last_used = Instant::now();
        drop(st);
        Ok(Lease {
            service,
            slot: Arc::clone(&slot),
        })
    }

    /// The running service `key`, without starting one.
    pub fn get<T: Running>(&self, key: &ServiceKey) -> Option<Lease<T>> {
        let slot = guard(&self.slots).get(key).cloned()?;
        lease_of::<T>(&slot, key).ok().flatten()
    }

    /// Refuses a start while the service has crashed too often too recently.
    fn crash_budget(&self, slot: &Slot, key: &ServiceKey) -> Result<(), String> {
        let policy = self.policy();
        let mut st = guard(&slot.state);
        let now = Instant::now();
        st.crashes
            .retain(|at| now.duration_since(*at) < policy.crash_window);
        if st.crashes.len() >= policy.max_crashes {
            let oldest = st.crashes.first().copied().unwrap_or(now);
            let wait = policy
                .crash_window
                .saturating_sub(now.duration_since(oldest));
            return Err(format!(
                "{} crashed {} times in {} s; it starts again in {} s",
                key.extension,
                st.crashes.len(),
                policy.crash_window.as_secs(),
                wait.as_secs().max(1)
            ));
        }
        Ok(())
    }

    /// Stops the least recently used idle services until one more fits the cap.
    fn make_room(&self, starting: &ServiceKey) {
        let cap = self.policy().cap.max(1);
        let mut stopping = Vec::new();
        {
            let slots = guard(&self.slots);
            let mut idle: Vec<(Instant, &ServiceKey, &Arc<Slot>)> = Vec::new();
            let mut running: usize = 0;
            for (key, slot) in slots.iter().filter(|(k, _)| *k != starting) {
                let st = guard(&slot.state);
                if st.held.as_ref().is_some_and(|h| !h.run.exited()) {
                    running += 1;
                    if st.in_flight == 0 {
                        idle.push((st.last_used, key, slot));
                    }
                }
            }
            idle.sort_by_key(|(at, _, _)| *at);
            for (_, key, slot) in idle.into_iter().take((running + 1).saturating_sub(cap)) {
                if let Some(held) = guard(&slot.state).held.take() {
                    info!(
                        "[extensions] {} stops for the cap of {cap} running",
                        key.extension
                    );
                    stopping.push(held);
                }
            }
        }
        for held in stopping {
            tokio::spawn(async move { held.run.close().await });
        }
    }

    /// Every running service, by key.
    pub fn running(&self) -> Vec<ServiceKey> {
        let slots = guard(&self.slots);
        let mut keys: Vec<ServiceKey> = slots
            .iter()
            .filter(|(_, s)| {
                guard(&s.state)
                    .held
                    .as_ref()
                    .is_some_and(|h| !h.run.exited())
            })
            .map(|(k, _)| k.clone())
            .collect();
        keys.sort();
        keys
    }

    /// Every running service held as a `T`.
    pub fn running_as<T: Running>(&self) -> Vec<(ServiceKey, Arc<T>)> {
        let slots = guard(&self.slots);
        slots
            .iter()
            .filter_map(|(k, s)| {
                let held = guard(&s.state).held.clone()?;
                if held.run.exited() {
                    return None;
                }
                let service = held.any.downcast::<T>().ok()?;
                Some((k.clone(), service))
            })
            .collect()
    }

    /// Stops every service `matches` picks; they start again only when asked.
    pub async fn stop_where(&self, matches: impl Fn(&ServiceKey) -> bool) {
        let removed: Vec<Arc<Slot>> = {
            let mut slots = guard(&self.slots);
            let keys: Vec<ServiceKey> = slots.keys().filter(|k| matches(k)).cloned().collect();
            keys.iter().filter_map(|k| slots.remove(k)).collect()
        };
        let closing: Vec<_> = removed
            .iter()
            .filter_map(|slot| guard(&slot.state).held.take())
            .map(|held| tokio::spawn(async move { held.run.close().await }))
            .collect();
        for handle in closing {
            let _ = handle.await;
        }
    }

    pub async fn stop(&self, key: &ServiceKey) {
        self.stop_where(|k| k == key).await;
    }

    pub async fn stop_extension(&self, extension: &str) {
        self.stop_where(|k| k.extension == extension).await;
    }

    pub async fn stop_all(&self) {
        self.stop_where(|_| true).await;
    }

    /// Stops the services idle past the policy's idle time.
    pub fn reap(&self) {
        let idle_for = self.policy().idle;
        let now = Instant::now();
        let mut stopping = Vec::new();
        for (key, slot) in guard(&self.slots).iter() {
            let mut st = guard(&slot.state);
            let idle = st.in_flight == 0 && now.duration_since(st.last_used) >= idle_for;
            if idle && st.held.as_ref().is_some_and(|h| !h.run.exited()) {
                if let Some(held) = st.held.take() {
                    info!("[extensions] {} stops after sitting idle", key.extension);
                    stopping.push(held);
                }
            }
        }
        for held in stopping {
            tokio::spawn(async move { held.run.close().await });
        }
    }

    /// Looks for idle services a few times per idle period, while the lifecycle lives.
    fn spawn_reaper(&self) {
        if self.reaper.set(()).is_err() {
            return;
        }
        let me = self.me.clone();
        tokio::spawn(async move {
            loop {
                let Some(every) = me.upgrade().map(|l| {
                    (l.policy().idle / 4).clamp(Duration::from_millis(20), Duration::from_secs(15))
                }) else {
                    return;
                };
                tokio::time::sleep(every).await;
                match me.upgrade() {
                    Some(lifecycle) => lifecycle.reap(),
                    None => return,
                }
            }
        });
    }
}

/// A lease on the service `slot` holds, if it runs; one that ended on its
/// own is counted as a crash and let go.
fn lease_of<T: Running>(slot: &Arc<Slot>, key: &ServiceKey) -> Result<Option<Lease<T>>, String> {
    let mut st = guard(&slot.state);
    let Some(held) = st.held.clone() else {
        return Ok(None);
    };
    if held.run.exited() {
        st.held = None;
        st.crashes.push(Instant::now());
        warn!("[extensions] {} ended; it starts again", key.extension);
        return Ok(None);
    }
    let service = held
        .any
        .downcast::<T>()
        .map_err(|_| format!("{} runs as another kind of service", key.extension))?;
    st.in_flight += 1;
    st.last_used = Instant::now();
    Ok(Some(Lease {
        service,
        slot: Arc::clone(slot),
    }))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::child::{Child, Launch};
    use std::path::PathBuf;

    struct Sleeper(Child);

    crate::running_child!(Sleeper, 0);

    impl Sleeper {
        fn child(&self) -> &Child {
            &self.0
        }
    }

    fn sleeper(name: &str) -> Result<Sleeper, String> {
        let launch = Launch {
            program: PathBuf::from("sh"),
            // `exec`: a shell that forked `cat` would leave it holding the pipes once the shell is killed.
            args: vec!["-c".into(), "exec cat >/dev/null".into()],
            cwd: std::env::temp_dir(),
            env: std::env::vars().filter(|(k, _)| k == "PATH").collect(),
        };
        Child::start(&launch, name.to_owned()).map(Sleeper)
    }

    fn policy(idle_ms: u64, cap: usize) -> Policy {
        Policy {
            idle: Duration::from_millis(idle_ms),
            cap,
            ..Policy::default()
        }
    }

    #[tokio::test]
    async fn starts_once_for_callers_that_share_a_start() {
        let l = Lifecycle::new(policy(60_000, 4));
        let key = ServiceKey::new("a", "");
        let (x, y) = tokio::join!(
            l.get_or_start(&key, || async { sleeper("a") }),
            l.get_or_start(&key, || async { sleeper("a") })
        );
        assert_eq!(x.unwrap().child().pid(), y.unwrap().child().pid());
        assert_eq!(l.running(), [key]);
        l.stop_all().await;
        assert!(l.running().is_empty());
    }

    #[tokio::test]
    async fn stops_the_least_recently_used_past_the_cap() {
        let l = Lifecycle::new(policy(60_000, 2));
        for id in ["a", "b"] {
            drop(
                l.get_or_start(&ServiceKey::new(id, ""), || async { sleeper(id) })
                    .await
                    .unwrap(),
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        // `a` used again, so `b` is now the least recently used.
        drop(l.get(&ServiceKey::new("a", "")).map(|l: Lease<Sleeper>| l));
        let _c = l
            .get_or_start(&ServiceKey::new("c", ""), || async { sleeper("c") })
            .await
            .unwrap();
        let names: Vec<String> = l.running().into_iter().map(|k| k.extension).collect();
        assert_eq!(names, ["a", "c"]);
        l.stop_all().await;
    }

    #[tokio::test]
    async fn never_evicts_a_service_in_use() {
        let l = Lifecycle::new(policy(60_000, 1));
        let busy = l
            .get_or_start(&ServiceKey::new("a", ""), || async { sleeper("a") })
            .await
            .unwrap();
        let _b = l
            .get_or_start(&ServiceKey::new("b", ""), || async { sleeper("b") })
            .await
            .unwrap();
        assert_eq!(l.running().len(), 2);
        drop(busy);
        l.stop_all().await;
    }

    #[tokio::test]
    async fn stops_an_idle_service_and_starts_it_again_when_asked() {
        let l = Lifecycle::new(policy(50, 4));
        let key = ServiceKey::new("a", "");
        let first = l
            .get_or_start(&key, || async { sleeper("a") })
            .await
            .unwrap()
            .child()
            .pid();
        // The reaper stops it once idle; the bound only keeps a hang from running on.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        while !l.running().is_empty() {
            assert!(tokio::time::Instant::now() < deadline, "idle past its time");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let again = l
            .get_or_start(&key, || async { sleeper("a") })
            .await
            .unwrap()
            .child()
            .pid();
        assert_ne!(first, again);
        l.stop_all().await;
    }

    #[tokio::test]
    async fn counts_a_crash_and_holds_off_after_too_many() {
        let l = Lifecycle::new(Policy {
            max_crashes: 2,
            ..policy(60_000, 4)
        });
        let key = ServiceKey::new("a", "");
        for _ in 0..2 {
            let lease = l
                .get_or_start(&key, || async { sleeper("a") })
                .await
                .unwrap();
            let service = lease.shared();
            let pid = service.child().pid().unwrap();
            drop(lease);
            // SAFETY: kill(2) on a child this test started and still holds.
            unsafe { libc::kill(pid as i32, libc::SIGKILL) };
            // The crash counts once its exit is seen; the bound only keeps a hang from running on.
            tokio::time::timeout(Duration::from_secs(30), service.child().wait())
                .await
                .expect("the killed child is seen to end");
        }
        assert!(l.get::<Sleeper>(&key).is_none());
        let err = l
            .get_or_start(&key, || async { sleeper("a") })
            .await
            .unwrap_err();
        assert!(err.starts_with("a crashed 2 times in 60 s"), "{err}");
        l.stop_all().await;
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;

    #[test]
    fn a_bench_overrides_the_idle_time_and_cap_with_positive_numbers_only() {
        let base = Policy::default();
        let both = base.overridden(Some("2500"), Some(" 3 "));
        assert_eq!(
            (both.idle, both.cap, both.max_crashes),
            (Duration::from_millis(2500), 3, base.max_crashes)
        );
        for bad in [None, Some(""), Some("0"), Some("-4"), Some("soon")] {
            assert_eq!(base.overridden(bad, bad), base, "{bad:?}");
        }
    }
}
