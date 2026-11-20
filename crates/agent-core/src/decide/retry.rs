//! How long a step of the harness that failed for a reason that may pass waits before it is tried again: 30 seconds
//! at the first failure, doubling with every further one, at most 10 minutes.

use std::num::NonZeroU32;

use time::Duration;

#[cfg(test)]
mod tests;

/// The pause before a failed step is tried again for the first time.
pub const FIRST_PAUSE: Duration = Duration::seconds(30);

/// The longest pause between two tries of a step that keeps failing.
pub const LONGEST_PAUSE: Duration = Duration::minutes(10);

/// The pause before the try that follows `tries` failures.
///
/// The first pause is [`FIRST_PAUSE`]; every further one doubles the pause before it, up to [`LONGEST_PAUSE`], and it
/// stays there. The count saturates: a number of tries no run reaches gives the longest pause, never an overflow.
#[must_use]
pub fn pause(tries: NonZeroU32) -> Duration {
    let doublings = tries.get().saturating_sub(1);
    let mut pause = FIRST_PAUSE;
    for _ in 0..doublings {
        if pause >= LONGEST_PAUSE {
            return LONGEST_PAUSE;
        }
        pause = pause.saturating_mul(2);
    }
    pause.min(LONGEST_PAUSE)
}
