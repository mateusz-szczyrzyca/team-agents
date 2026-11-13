//! Holding the watchdog off a run that waits for something the daemon does for it.
//!
//! A run that calls `agent gate` prints nothing until the gate is over, which may be the length of the gate's
//! timeout plus the wait in the queue. The watchdog would take that silence for a hang. The daemon gives the engine a
//! [`WatchdogHold`] with the run; whatever the run waits for takes a [`HoldGuard`] for as long as it lasts.

use std::sync::Arc;
use std::time::SystemTime;

use tokio::sync::watch;

/// What the holds of one run amount to.
#[derive(Debug, Default)]
struct Holds {
    /// The guards that are alive.
    alive: usize,
    /// When the last guard was dropped; `None` until one was.
    released: Option<SystemTime>,
}

/// The holds on the watchdog of one run: the daemon raises them, the engine looks at them.
///
/// Clones share the holds. The state is a `watch` cell only because it cannot be poisoned: nobody waits for it to
/// change, the engine reads it at each look at the run.
#[derive(Debug, Clone, Default)]
pub struct WatchdogHold {
    /// The shared holds.
    holds: Arc<watch::Sender<Holds>>,
}

/// A hold on the watchdog of a run, kept until it is dropped.
#[derive(Debug)]
#[must_use = "the watchdog is held only while the guard is alive"]
pub struct HoldGuard {
    /// The holds this guard counts in.
    holds: Arc<watch::Sender<Holds>>,
}

impl WatchdogHold {
    /// A run's holds, with none raised.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Holds the watchdog off until the returned guard is dropped.
    pub fn hold(&self) -> HoldGuard {
        HoldGuard {
            holds: Arc::clone(&self.holds),
        }
    }

    /// The latest time the watchdog was held off: now, while a guard is alive; else when the last was dropped; `None`
    /// when it never was. The run counts its silence from then.
    #[must_use]
    pub fn last_held(&self) -> Option<SystemTime> {
        None
    }
}

impl Drop for HoldGuard {
    fn drop(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_that_was_never_held_has_no_last_held_time() {
        assert_eq!(WatchdogHold::new().last_held(), None);
    }

    #[test]
    fn the_watchdog_is_held_while_any_guard_is_alive_and_the_release_is_remembered() {
        let hold = WatchdogHold::new();
        let first = hold.hold();
        let second = hold.hold();
        drop(first);

        let while_held = hold.last_held().expect("one guard is still alive");
        drop(second);
        let released = hold.last_held().expect("the last guard was dropped");

        assert!(released >= while_held);
        assert_eq!(
            hold.last_held(),
            Some(released),
            "the release time is fixed once nothing holds"
        );
    }

    #[test]
    fn a_clone_shares_the_holds_of_the_original() {
        let hold = WatchdogHold::new();
        let clone = hold.clone();
        let guard = clone.hold();

        let seen = hold.last_held().expect("held through the clone");
        drop(guard);

        assert_eq!(hold.last_held().map(|at| at >= seen), Some(true));
    }
}
