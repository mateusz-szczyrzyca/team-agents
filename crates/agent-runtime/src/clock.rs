//! The system clock, the [`Clock`] the daemon gives to everything that makes an ID or a timestamp.

use std::fmt;
use std::sync::Arc;

use agent_core::ids::Clock;
use time::UtcDateTime;

/// The time of the machine, in UTC.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> UtcDateTime {
        UtcDateTime::now()
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
