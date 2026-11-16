//! Several kernels at once, one per route.
//!
//! A kernel process pins its provider and model at `initialize`: there is no call that
//! switches route afterwards. So "let the user change model without restarting the app"
//! *is* "run more than one kernel" — hence this pool.
//!
//! Two rules keep it from becoming a process leak:
//!
//! * **One kernel per route, reused.** Asking for the same provider+model twice returns the
//!   same client, so switching back and forth costs nothing.
//! * **A bounded number of idle kernels.** Starting a ninth route closes the one used least
//!   recently, never the route a live session is on (the caller holds that).
//!
//! Sessions are registered against the route that created them, because a session id only
//! means something inside the kernel that minted it.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use crate::process::KernelClient;

/// Which kernel a request should go to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Route {
    pub provider: String,
    pub model: String,
}

impl Route {
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
        }
    }
}

impl std::fmt::Display for Route {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}/{}", self.provider, self.model)
    }
}

/// One live kernel plus when it was last asked for.
struct Slot {
    client: Arc<KernelClient>,
    last_used: Instant,
    /// Sessions this kernel owns. A session id is meaningless to another kernel.
    sessions: Vec<String>,
}

/// How often one route may be restarted before the pool stops trying.
///
/// A kernel that dies on start-up dies again immediately: without a ceiling the client would
/// respawn it on every message and the log would fill with identical crashes. The window makes
/// "crashed five times in two minutes" a diagnosis instead of a loop.
#[derive(Debug, Clone, Copy)]
pub struct RestartPolicy {
    /// Restarts allowed inside [`RestartPolicy::window`].
    pub max_restarts: usize,
    pub window: Duration,
    /// First retry delay; doubles per attempt, capped at [`RestartPolicy::max_delay`].
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            max_restarts: 5,
            window: Duration::from_secs(120),
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(8),
        }
    }
}

/// Why a restart was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RestartError {
    #[error(
        "the kernel for {route} crashed {attempts} times in {window:?}; giving up until the cause is fixed"
    )]
    GivingUp {
        route: String,
        attempts: usize,
        window: Duration,
    },
}

/// Kernels by route, started lazily.
#[derive(Clone)]
pub struct KernelPool {
    /// How many idle kernels to keep. A running session counts as in use.
    capacity: usize,
    slots: Arc<Mutex<HashMap<Route, Slot>>>,
    /// Recent restart attempts, per route. Pruned to the policy's window on each check.
    restarts: Arc<Mutex<HashMap<Route, Vec<Instant>>>>,
    policy: RestartPolicy,
}

impl KernelPool {
    pub fn new(capacity: usize) -> Self {
        Self::with_policy(capacity, RestartPolicy::default())
    }

    /// The same pool with an explicit restart policy — tests need a short window.
    pub fn with_policy(capacity: usize, policy: RestartPolicy) -> Self {
        Self {
            capacity: capacity.max(1),
            policy,
            restarts: Arc::new(Mutex::new(HashMap::new())),
            slots: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// An existing kernel for this route, untouched — used by commands that must not start
    /// a process as a side effect (answering a frame, for instance).
    pub async fn existing(&self, route: &Route) -> Option<Arc<KernelClient>> {
        let mut slots = self.slots.lock().await;
        let slot = slots.get_mut(route)?;
        slot.last_used = Instant::now();
        Some(Arc::clone(&slot.client))
    }

    /// The kernel for this route, starting it when absent.
    ///
    /// `spawn` runs while the pool lock is held on purpose: two concurrent requests for the
    /// same new route would otherwise both start a kernel and one would be dropped with its
    /// process still running.
    pub async fn acquire<F, Fut, E>(&self, route: &Route, spawn: F) -> Result<Arc<KernelClient>, E>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<KernelClient, E>>,
    {
        let mut slots = self.slots.lock().await;
        if let Some(slot) = slots.get_mut(route) {
            slot.last_used = Instant::now();
            return Ok(Arc::clone(&slot.client));
        }

        self.evict_for(&mut slots, route);
        let client = Arc::new(spawn().await?);
        slots.insert(
            route.clone(),
            Slot {
                client: Arc::clone(&client),
                last_used: Instant::now(),
                sessions: Vec::new(),
            },
        );

        // A dead kernel must not stay in the pool. Without this, `existing()` keeps handing
        // out an `Arc` whose process is gone: every call goes into a closed stdin and the
        // user sees a spinner that never resolves, not an error.
        self.watch_exit(route.clone(), Arc::clone(&client));
        Ok(client)
    }

    /// Remove a route when its kernel exits.
    fn watch_exit(&self, route: Route, client: Arc<KernelClient>) {
        let pool = self.clone();
        let mut exited = client.subscribe_exit();
        tokio::spawn(async move {
            // A receive error means the sender was dropped, which also means the kernel is
            // gone; both paths end the same way.
            let _ = exited.recv().await;
            tracing::debug!(target: "dsh_runtime", %route, "kernel exited; forgetting it");
            let mut slots = pool.slots.lock().await;
            slots.remove(&route);
        });
    }

    /// Decide whether this route may be started again, and what to wait first.
    ///
    /// Returns the delay to observe before spawning. The first restart after a quiet period is
    /// immediate — a crashed kernel that nothing has touched for a while is not a loop, and making
    /// the user wait half a second for no reason is its own bug.
    pub async fn restart_delay(&self, route: &Route) -> Result<Duration, RestartError> {
        let mut restarts = self.restarts.lock().await;
        let history = restarts.entry(route.clone()).or_default();
        let now = Instant::now();
        history.retain(|at| now.duration_since(*at) <= self.policy.window);

        if history.len() >= self.policy.max_restarts {
            return Err(RestartError::GivingUp {
                route: route.to_string(),
                attempts: history.len(),
                window: self.policy.window,
            });
        }

        let delay = if history.is_empty() {
            Duration::ZERO
        } else {
            let doublings = (history.len() - 1).min(16) as u32;
            self.policy
                .base_delay
                .saturating_mul(1u32 << doublings)
                .min(self.policy.max_delay)
        };
        history.push(now);
        Ok(delay)
    }

    /// Forget the restart history of a route that has been healthy for long enough.
    ///
    /// Called after a successful handshake: a kernel that answered is not part of a crash loop,
    /// so the next crash starts counting from zero.
    pub async fn restarted_successfully(&self, route: &Route) {
        self.restarts.lock().await.remove(route);
    }

    /// How many restarts this route has had inside the window.
    pub async fn restart_count(&self, route: &Route) -> usize {
        let mut restarts = self.restarts.lock().await;
        let now = Instant::now();
        let history = restarts.entry(route.clone()).or_default();
        history.retain(|at| now.duration_since(*at) <= self.policy.window);
        history.len()
    }

    /// Drop a route without shutting anything down. Used by the exit watcher.
    pub async fn forget(&self, route: &Route) -> bool {
        self.slots.lock().await.remove(route).is_some()
    }

    /// Record that a session lives on this route, so later prompts find the right kernel.
    pub async fn bind_session(&self, route: &Route, session_id: &str) {
        let mut slots = self.slots.lock().await;
        if let Some(slot) = slots.get_mut(route)
            && !slot.sessions.iter().any(|known| known == session_id)
        {
            slot.sessions.push(session_id.to_owned());
        }
    }

    /// The route a session was started on, if this pool knows it.
    pub async fn route_of(&self, session_id: &str) -> Option<Route> {
        let slots = self.slots.lock().await;
        slots
            .iter()
            .find(|(_, slot)| slot.sessions.iter().any(|known| known == session_id))
            .map(|(route, _)| route.clone())
    }

    /// Every route currently running, oldest use first.
    pub async fn routes(&self) -> Vec<Route> {
        let slots = self.slots.lock().await;
        let mut listed: Vec<(Route, Instant)> = slots
            .iter()
            .map(|(route, slot)| (route.clone(), slot.last_used))
            .collect();
        listed.sort_by_key(|(_, used)| *used);
        listed.into_iter().map(|(route, _)| route).collect()
    }

    /// Close one kernel and forget its sessions.
    pub async fn release(&self, route: &Route, grace: Duration) -> bool {
        let slot = {
            let mut slots = self.slots.lock().await;
            slots.remove(route)
        };
        match slot {
            Some(slot) => {
                slot.client.shutdown(grace).await;
                true
            }
            None => false,
        }
    }

    /// Close everything. Called on exit; also the honest way to test the pool.
    pub async fn shutdown_all(&self, grace: Duration) {
        let drained: Vec<Slot> = {
            let mut slots = self.slots.lock().await;
            slots.drain().map(|(_, slot)| slot).collect()
        };
        for slot in drained {
            slot.client.shutdown(grace).await;
        }
    }

    /// Make room for one more route.
    ///
    /// A kernel with bound sessions is never evicted: closing it would end a conversation the
    /// user can still see in the sidebar. That is why eviction can find nothing to do, and why
    /// the pool may briefly exceed its capacity rather than silently kill a session.
    fn evict_for(&self, slots: &mut HashMap<Route, Slot>, incoming: &Route) {
        while slots.len() >= self.capacity {
            let victim = slots
                .iter()
                .filter(|(route, slot)| *route != incoming && slot.sessions.is_empty())
                .min_by_key(|(_, slot)| slot.last_used)
                .map(|(route, _)| route.clone());

            let Some(victim) = victim else {
                tracing::debug!(
                    target: "dsh_runtime",
                    capacity = self.capacity,
                    "no idle kernel to evict; growing past capacity"
                );
                return;
            };
            slots.remove(&victim);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(model: &str) -> Route {
        Route::new("opencode-go", model)
    }

    #[test]
    fn a_route_is_identified_by_provider_and_model_together() {
        assert_eq!(route("a"), route("a"));
        assert_ne!(route("a"), route("b"));
        assert_ne!(Route::new("p1", "m"), Route::new("p2", "m"));
    }

    #[test]
    fn display_is_what_the_header_shows() {
        assert_eq!(
            route("deepseek-v4.1-flash").to_string(),
            "opencode-go/deepseek-v4.1-flash"
        );
    }

    // The pool's mutation paths need real clients, which need a kernel; they are exercised
    // through `KernelPool`'s bookkeeping-only surface here and end to end in `tests/pool.rs`.
    #[tokio::test]
    async fn an_empty_pool_answers_nothing_rather_than_starting_a_process() {
        let pool = KernelPool::new(2);
        assert!(pool.existing(&route("a")).await.is_none());
        assert!(pool.route_of("unknown").await.is_none());
        assert!(pool.routes().await.is_empty());
    }

    #[tokio::test]
    async fn releasing_something_that_was_never_started_is_not_an_error() {
        let pool = KernelPool::new(1);
        assert!(!pool.release(&route("a"), Duration::from_millis(1)).await);
    }
}

#[cfg(test)]
mod restart_tests {
    use super::*;

    fn route() -> Route {
        Route::new("p", "m")
    }

    #[tokio::test]
    async fn the_first_restart_after_a_quiet_period_is_immediate() {
        // A kernel that crashed a while ago is not a crash loop; making the user wait would be a
        // bug of its own.
        let pool = KernelPool::with_policy(2, RestartPolicy::default());
        assert_eq!(
            pool.restart_delay(&route()).await.expect("allowed"),
            Duration::ZERO
        );
        assert_eq!(pool.restart_count(&route()).await, 1);
    }

    #[tokio::test]
    async fn delays_double_up_to_the_cap() {
        let policy = RestartPolicy {
            max_restarts: 10,
            window: Duration::from_secs(600),
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(400),
        };
        let pool = KernelPool::with_policy(2, policy);
        let route = route();

        let mut delays = Vec::new();
        for _ in 0..5 {
            delays.push(pool.restart_delay(&route).await.expect("allowed"));
        }

        assert_eq!(
            delays,
            vec![
                Duration::ZERO,             // first: immediate
                Duration::from_millis(100), // 2^0
                Duration::from_millis(200), // 2^1
                Duration::from_millis(400), // 2^2, at the cap
                Duration::from_millis(400), // stays at the cap
            ]
        );
    }

    #[tokio::test]
    async fn a_crash_loop_is_refused_instead_of_retried_forever() {
        let policy = RestartPolicy {
            max_restarts: 3,
            window: Duration::from_secs(60),
            ..RestartPolicy::default()
        };
        let pool = KernelPool::with_policy(2, policy);
        let route = route();

        for _ in 0..3 {
            pool.restart_delay(&route).await.expect("allowed");
        }
        let refused = pool.restart_delay(&route).await.expect_err("refused");

        match refused {
            RestartError::GivingUp { attempts, .. } => assert_eq!(attempts, 3),
        }
        // The message has to be actionable: it names the route and the window.
        let message = refused.to_string();
        assert!(message.contains("p/m"), "{message}");
        assert!(message.contains("crashed 3 times"), "{message}");
    }

    #[tokio::test]
    async fn a_successful_handshake_clears_the_history() {
        let policy = RestartPolicy {
            max_restarts: 2,
            window: Duration::from_secs(60),
            ..RestartPolicy::default()
        };
        let pool = KernelPool::with_policy(2, policy);
        let route = route();
        pool.restart_delay(&route).await.expect("allowed");
        pool.restart_delay(&route).await.expect("allowed");
        assert!(pool.restart_delay(&route).await.is_err());

        // The kernel answered this time: whatever was wrong is no longer worth counting.
        pool.restarted_successfully(&route).await;
        assert_eq!(pool.restart_count(&route).await, 0);
        pool.restart_delay(&route).await.expect("allowed again");
    }

    #[tokio::test]
    async fn the_window_forgets_old_crashes() {
        let policy = RestartPolicy {
            max_restarts: 2,
            window: Duration::from_millis(120),
            base_delay: Duration::from_millis(10),
            max_delay: Duration::from_millis(10),
        };
        let pool = KernelPool::with_policy(2, policy);
        let route = route();
        pool.restart_delay(&route).await.expect("allowed");
        pool.restart_delay(&route).await.expect("allowed");
        assert!(pool.restart_delay(&route).await.is_err(), "at the ceiling");

        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(pool.restart_count(&route).await, 0, "the window has passed");
        pool.restart_delay(&route).await.expect("allowed again");
    }
}
