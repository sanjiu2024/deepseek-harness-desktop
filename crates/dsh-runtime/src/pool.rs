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

/// Kernels by route, started lazily.
#[derive(Clone)]
pub struct KernelPool {
    /// How many idle kernels to keep. A running session counts as in use.
    capacity: usize,
    slots: Arc<Mutex<HashMap<Route, Slot>>>,
}

impl KernelPool {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
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
        assert_eq!(route("deepseek-v4.1-flash").to_string(), "opencode-go/deepseek-v4.1-flash");
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
