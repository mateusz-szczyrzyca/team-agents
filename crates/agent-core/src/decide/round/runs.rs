//! The rounds a run works for: decided when the run starts, from the role of its task and from why the run starts.
//!
//! A pure function of those and of what the state says about the rounds of the task's team ([`TaskRounds`]); the state
//! records the answer with the run (`Team::begin_run`), and a run that has started keeps its rounds.

use crate::ids::RoundId;
use crate::kinds::Role;

/// Why a run starts, as far as its rounds are concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStart<'a> {
    /// The task starts or goes on with its own work: an item is given to it, a message or the board woke it, or it is
    /// the first run of the task.
    Work,
    /// A worker is asked to serve a request of a run that works for these rounds (none, when that run works for none).
    Request {
        /// The rounds of the run that asked.
        asked_by: &'a [RoundId],
    },
    /// A reviewer is asked to review the pull request of this round — or the same again after a fix pass or when its
    /// report had no verdict.
    Review {
        /// The round of the pull request; none when no round has it.
        round: Option<&'a RoundId>,
    },
    /// The run goes on where the task was: a fix pass, the answers of the human to its questions, a resume after a
    /// stop, a hang, a limit or a restart of the daemon.
    Continuation {
        /// The rounds the task's last run worked for.
        previous: &'a [RoundId],
    },
}

/// What the state says about the rounds of the team of the task that starts a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskRounds<'a> {
    /// The round of the item the task works on — the member's or the solo agent's own; none while it has no item.
    pub item: Option<&'a RoundId>,
    /// The rounds of the team that are open, oldest first.
    pub open: &'a [RoundId],
}

/// The rounds the run of a task that has `role` works for when it starts because of `start`, oldest first:
///
/// - a member or a solo agent: the round of its item, none while it has no item;
/// - the coordinator: every round of its team that is open;
/// - a reviewer: the round of the pull request it reviews;
/// - a worker: the rounds of the run that asked for it;
/// - a fix pass, answers and a resume stay in the rounds the task works for.
#[must_use]
pub fn rounds_of_run(role: Role, start: RunStart<'_>, task: &TaskRounds<'_>) -> Vec<RoundId> {
    let _ = (role, start, task);
    Vec::new()
}

#[cfg(test)]
mod tests;
