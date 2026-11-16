//! A gate for work that is expensive in memory.
//!
//! Reading a file for preview is bounded by size, but not by *count*: ten clicks on ten large
//! files used to mean ten of them in flight at once, each held until its JSON crossed to the
//! WebView. On a 200 MB file that is not a slow preview, it is a machine that stops responding.
//!
//! Deliberately not a queue: a preview the user asked for ten clicks ago is no longer wanted, so
//! the eleventh caller is told to wait rather than being served after the first nine finish.

use std::sync::atomic::{AtomicUsize, Ordering};

/// How many previews may be read at the same time.
pub const DEFAULT_PREVIEW_LIMIT: usize = 3;

pub struct Gate {
    limit: usize,
    in_flight: AtomicUsize,
}

impl Gate {
    pub fn new(limit: usize) -> Self {
        Self {
            limit: limit.max(1),
            in_flight: AtomicUsize::new(0),
        }
    }

    /// Take a permit, or fail immediately when the limit is reached.
    ///
    /// Returns `None` rather than blocking: the caller is a Tauri command thread, and blocking
    /// there turns "too many previews" into "the window stopped responding".
    pub fn try_enter(&self) -> Option<Permit<'_>> {
        let mut current = self.in_flight.load(Ordering::Acquire);
        loop {
            if current >= self.limit {
                return None;
            }
            match self.in_flight.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(Permit { gate: self }),
                // Another caller got in between; re-read and try again.
                Err(actual) => current = actual,
            }
        }
    }

    /// How many are in flight right now, for diagnostics.
    pub fn in_flight(&self) -> usize {
        self.in_flight.load(Ordering::Acquire)
    }
}

/// Releases its slot when dropped — including on an early return or a panic.
pub struct Permit<'a> {
    gate: &'a Gate,
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        self.gate.in_flight.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limit_is_enforced_and_released_on_drop() {
        let gate = Gate::new(2);
        let first = gate.try_enter().expect("first");
        let second = gate.try_enter().expect("second");
        assert!(
            gate.try_enter().is_none(),
            "a third must be refused, not queued"
        );

        drop(first);
        assert_eq!(gate.in_flight(), 1);
        let third = gate.try_enter().expect("a slot came back");
        drop(second);
        drop(third);
        assert_eq!(gate.in_flight(), 0);
    }

    #[test]
    fn a_panic_still_releases_the_slot() {
        // Otherwise one failed preview would permanently shrink the pool.
        let gate = Gate::new(1);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _permit = gate.try_enter().expect("permit");
            panic!("preview blew up");
        }));
        assert!(result.is_err());
        assert_eq!(gate.in_flight(), 0);
        assert!(gate.try_enter().is_some(), "the slot is usable again");
    }
}
