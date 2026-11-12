//! What a run's end does to its task: the task's next status, the notice, the pause and its wake-up time.
//!
//! Pure functions: the daemon feeds them what the engine observed and carries out what they say.

use agent_core::config::{Limits, Minutes};
use agent_core::decide::outcome::{ClassifyError, MAX_PAUSE, Pause, RESET_MARGIN};
use agent_core::kinds::Role;
use agent_core::state::{RunEnd, Timestamp};
use agent_core::status::{RunOutcome, TaskStatus};

use super::notice::Notice;

/// How many characters of an agent's final message a notice keeps: its end, where the agent says why.
pub(super) const MAX_MESSAGE_CHARS: usize = 2000;

/// What a run's end does to its task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Settled {
    /// The task goes to this status.
    Status(TaskStatus),
    /// The task failed: it goes to `failed` and the human is told.
    Failed(Notice),
    /// The agent asked questions: the file of the round decides what follows.
    Questions,
    /// The provider's limit pauses the task.
    Paused(Pause),
    /// The watchdog killed the run: whether it is resumed is [`agent_core::decide::outcome::after_hang`]'s decision.
    Hung,
}

/// What the end `end` of a run of the member of role `role` does to its task; `pause` is what the classifier made
/// of a limit, `final_message` what the agent said last.
pub(super) fn settle(role: Role, end: &RunEnd, pause: Option<&Pause>, final_message: Option<&str>) -> Settled {
    match end {
        RunEnd::Reported { outcome } => match outcome {
            RunOutcome::Done => Settled::Status(after_done(role)),
            RunOutcome::Questions => Settled::Questions,
            RunOutcome::Failed => Settled::Failed(Notice::Failed {
                message: end_of(final_message.unwrap_or_default()),
            }),
            RunOutcome::Sleeping => Settled::Status(TaskStatus::Waiting),
            // A member the coordinator excluded acknowledges it and stops for good.
            RunOutcome::Excluded => Settled::Status(TaskStatus::Done),
        },
        RunEnd::NoStatus { exit_code } => Settled::Failed(Notice::NoStatus { exit: *exit_code }),
        RunEnd::QuotaExhausted { .. } | RunEnd::RateLimited => pause.map_or(
            // The classifier gives every limit a pause, so this is not reached; a limit that cannot be waited for is
            // a run that failed.
            Settled::Failed(Notice::NoStatus { exit: None }),
            |pause| Settled::Paused(pause.clone()),
        ),
        RunEnd::Hung => Settled::Hung,
        RunEnd::Stopped => Settled::Status(TaskStatus::Stopped),
    }
}

/// Where a task goes when its agent reports `DONE`: a solo task and a review are over; a member of a team waits for
/// its next assignment, a coordinator for the work of the others, a worker for its next request.
const fn after_done(role: Role) -> TaskStatus {
    match role {
        Role::Solo | Role::Reviewer => TaskStatus::Done,
        Role::Coordinator | Role::Member | Role::Worker => TaskStatus::Waiting,
    }
}

/// The last [`MAX_MESSAGE_CHARS`] characters of `message`.
fn end_of(message: &str) -> String {
    let skip = message.chars().count().saturating_sub(MAX_MESSAGE_CHARS);
    message.chars().skip(skip).collect()
}

/// When a task paused by the end `end` of a run that ended at `ended_at` is queued again; `None` for an end that
/// pauses nothing.
///
/// This is the time the classifier gave the pause, worked out again from what the state keeps, so that a pause survives
/// a restart of the daemon.
///
/// # Errors
/// [`ClassifyError`] when the pause would end beyond the years a timestamp can hold, or the configured default pause
/// cannot be a duration.
pub(super) fn wake_time(
    end: &RunEnd,
    ended_at: Timestamp,
    limits: &Limits,
) -> Result<Option<Timestamp>, ClassifyError> {
    let default_pause = |minutes: Minutes| {
        time::Duration::try_from(minutes.as_duration()).map_err(|source| ClassifyError::DefaultPauseTooLong { source })
    };
    let ended = ended_at.as_utc();
    let wait = match end {
        // A reset time that is not in the future is no reset time, as for the classifier.
        RunEnd::QuotaExhausted { resets_at: Some(reset) } if reset.as_utc() > ended => {
            (reset.as_utc() - ended).min(MAX_PAUSE)
        }
        RunEnd::QuotaExhausted { .. } => default_pause(limits.quota_pause_minutes)?,
        RunEnd::RateLimited => default_pause(limits.rate_limit_pause_minutes)?,
        RunEnd::Reported { .. } | RunEnd::NoStatus { .. } | RunEnd::Hung | RunEnd::Stopped => return Ok(None),
    };
    let until = ended
        .checked_add(wait)
        .and_then(|time| time.checked_add(RESET_MARGIN))
        .ok_or(ClassifyError::PauseOverflow { now: ended_at })?;
    let until =
        Timestamp::from_utc(until).map_err(|source| ClassifyError::PauseOutOfRange { now: ended_at, source })?;
    Ok(Some(until))
}

#[cfg(test)]
mod tests;
