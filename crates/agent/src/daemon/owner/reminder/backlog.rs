//! The reminders of Serena the daemon has still to put on the bus, and those it has put there.
//!
//! The engine hands a reminder over once. A reminder that cannot be rendered, has no recipient or is refused by the bus
//! stays here and is tried again at the next turn of the daemon, up to [`MAX_ATTEMPTS`] times; only a reminder the bus took
//! is remembered as delivered, so that the run that earned it is told no more than once.

use std::collections::HashSet;

use agent_core::ids::{RunId, TaskId};
use agent_runtime::run::serena::DueReminder;

#[cfg(test)]
mod tests;

/// How often a reminder is tried before the daemon leaves it to the end of the run, which reads it from the history again.
pub(crate) const MAX_ATTEMPTS: u32 = 5;

/// A reminder that is to be put on the bus now, with the attempts that failed before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Attempt {
    /// The reminder.
    pub(crate) due: DueReminder,
    /// How many times it was tried and failed.
    pub(crate) failures: u32,
}

/// What became of a reminder that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Failed {
    /// It stays and is tried at the next turn.
    Retry,
    /// It failed [`MAX_ATTEMPTS`] times and is dropped: the end of its run reads it from the history again.
    GaveUp,
}

/// The reminders waiting to be put on the bus, and the runs whose reminder the bus took.
#[derive(Debug, Default)]
pub(crate) struct Backlog {
    /// The reminders to try at the next turn, oldest first.
    waiting: Vec<Attempt>,
    /// The task and the run of each reminder the bus took: such a run is not told again.
    delivered: HashSet<(TaskId, RunId)>,
}

impl Backlog {
    /// Adds the reminder `due` unless its run was told already or it waits already.
    pub(crate) fn offer(&mut self, _due: DueReminder) {}

    /// The reminders to try now: they leave the backlog, and come back through [`Backlog::failed`] when they fail.
    pub(crate) fn take(&mut self) -> Vec<Attempt> {
        Vec::new()
    }

    /// Notes that the bus took the reminder of `attempt`.
    pub(crate) fn delivered(&mut self, _attempt: &Attempt) {}

    /// Notes that the reminder of `attempt` failed once more.
    pub(crate) fn failed(&mut self, _attempt: Attempt) -> Failed {
        Failed::GaveUp
    }
}
