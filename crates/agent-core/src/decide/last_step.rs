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
    mode: ReplacementMode,
    role: Role,
    asked: Asked,
    held: HeldBefore,
    work: MembersWork,
) -> AfterLastStep {
    if matches!(mode, ReplacementMode::Off) || matches!(asked, Asked::Once) {
        return AfterLastStep::Fail;
    }
    let taken_over = matches!(held, HeldBefore::Others);
    let asks = match role {
        Role::Solo | Role::Member => taken_over,
        Role::Coordinator => taken_over || matches!(work, MembersWork::OfAStoppedMember),
        Role::Worker | Role::Reviewer => false,
    };
    if asks {
        AfterLastStep::AskHuman
    } else {
        AfterLastStep::Fail
    }
}

/// What the verdict of the question after the last step says; `None` when it has none. Only an answer that names the
/// option `a` starts the task once more: `ok`, `b` and any other words stop it, so that nothing more runs on a task
/// nobody asked to go on.
#[must_use]
pub fn last_step_answer(verdict: Option<&Verdict>) -> LastStepAnswer {
    match verdict {
        Some(Verdict::Own(answer)) if names_option_a(answer.as_str()) => LastStepAnswer::OnceMore,
        Some(Verdict::Accept | Verdict::Own(_)) | None => LastStepAnswer::Stop,
    }
}

/// Whether `answer` names the option `a`: `a` alone, or `a` followed by what is not a letter or a digit (`a)`, `a —`).
fn names_option_a(answer: &str) -> bool {
    let mut chars = answer.trim_start().chars();
    matches!(chars.next(), Some('a' | 'A')) && chars.next().is_none_or(|next| !next.is_alphanumeric())
}

#[cfg(test)]
mod tests;
