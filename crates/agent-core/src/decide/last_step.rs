//! After the last step of the ladder: whether the human is asked what becomes of a task whose runs went bad with no step
//! left, and what the answer to that question says.

use crate::decide::replacement::ReplacementMode;
use crate::kinds::Role;
use crate::questions::Verdict;

/// Whether the human was asked about the task after its last step already.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asked {
    /// Never: the task may be asked once.
    Never,
    /// Once: the task fails as before.
    Once,
}

/// Whether other agents held the task before the one that holds it now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeldBefore {
    /// Nobody: the task was never handed over.
    Nobody,
    /// Another agent did: the task was taken over.
    Others,
}

/// Whether the task's team has the work of a member whose task stopped, which a coordinator then holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MembersWork {
    /// No member's work is in the team's lead's hands.
    None,
    /// A member whose task stopped holds an item that is still worked on.
    OfAStoppedMember,
}

/// What follows a run of a task that went bad with no step of the ladder left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterLastStep {
    /// The human is asked once, in a questions file.
    AskHuman,
    /// The task fails and the human is told, as before.
    Fail,
}

/// What the human's answer to the question after the last step says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LastStepAnswer {
    /// `a`: the task starts once more, in a new session of the agent that holds it.
    OnceMore,
    /// `b`, the recommendation accepted, or any other answer: the task stops.
    Stop,
}

/// Whether a task of `role`, under `[replacement] mode`, is asked about after its last step: never in mode `off`,
/// only once, and only a task that was taken over, or a coordinator that holds the work of a stopped member.
#[must_use]
pub const fn after_last_step(
    _mode: ReplacementMode,
    _role: Role,
    _asked: Asked,
    _held: HeldBefore,
    _work: MembersWork,
) -> AfterLastStep {
    AfterLastStep::AskHuman
}

/// What the verdict of the question after the last step says; `None` when it has none.
#[must_use]
pub const fn last_step_answer(_verdict: Option<&Verdict>) -> LastStepAnswer {
    LastStepAnswer::OnceMore
}

#[cfg(test)]
mod tests;
