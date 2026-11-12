//! What a run's end does to its task: the task's next status, the notice, the pause and its wake-up time.
//!
//! Pure functions: the daemon feeds them what the engine observed and carries out what they say.

use agent_core::config::Limits;
use agent_core::decide::outcome::{ClassifyError, Pause};
use agent_core::kinds::Role;
use agent_core::state::{RunEnd, Timestamp};
use agent_core::status::TaskStatus;

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
    let _ = (role, end, pause, final_message);
    Settled::Questions
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
pub(super) fn wake_time(end: &RunEnd, ended_at: Timestamp, limits: &Limits) -> Result<Option<Timestamp>, ClassifyError> {
    let _ = (end, ended_at, limits);
    Ok(None)
}

#[cfg(test)]
mod tests;
