//! The system clock, the [`Clock`] the daemon gives to everything that makes an ID or a timestamp.

use std::fmt;
use std::sync::Arc;

use agent_core::ids::Clock;
use time::UtcDateTime;

/// The time of the machine, in UTC, in whole seconds: the state, the activity and the notifications show times a human
/// reads, and the IDs of the harness count seconds too.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> UtcDateTime {
        let now = UtcDateTime::now();
        // Taking the fraction away from a time that has one cannot leave the range of times.
        now - time::Duration::nanoseconds(i64::from(now.nanosecond()))
    }
}

/// A clock many tasks share: the system's in the daemon, a controlled one in a test.
#[derive(Clone)]
pub struct SharedClock(Arc<dyn Clock + Send + Sync>);

impl SharedClock {
    /// Shares `clock`.
    #[must_use]
    pub fn new(clock: impl Clock + Send + Sync + 'static) -> Self {
        Self(Arc::new(clock))
    }
}

impl fmt::Debug for SharedClock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SharedClock")
    }
}

impl Clock for SharedClock {
    fn now(&self) -> UtcDateTime {
        self.0.now()
    }
}
