//! Whether a completed fix pass has supplied the commit its answers require, and whether a lead's run can be its response.

use crate::state::Timestamp;

/// The relationship between the handed-over tip and the reviewed commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// A new commit has a passing gate at its tip.
    Committed,
    /// The harness has queued or is running the gate.
    Gating,
    /// No gate remains due, and its record belongs to another commit.
    StaleGate,
    /// The tip is unchanged; accepted answers describe the remaining work.
    Unchanged(Answers),
}

/// Whether accepted answers require a code change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answers {
    /// Every blocker is explained or closed by an independent opinion.
    Accounted,
    /// At least one line claims FIXED, including mixed responses.
    Fixed,
    /// A legacy open finding has no response lines.
    Missing,
}

/// Remaining automatic corrections of the lead's handover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Budget {
    /// Another automatic correction is permitted.
    Available,
    /// The human must intervene.
    Exhausted,
}

/// Why a completed fix pass cannot enter verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hold {
    /// A claimed or legacy fix needs a new commit.
    NoCommit,
    /// The gate record does not cover the handed-over tip.
    StaleGate,
}

/// The next action after the lead hands over a fix pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Ask for verification, even at the same commit when everything is accounted for.
    Verify,
    /// Let the harness's gate finish.
    WaitGate,
    /// Return the handover to the lead.
    Correct(Hold),
    /// Notify the human once and wait for their instruction.
    AskHuman(Hold),
}

/// Decides whether the handover can proceed within its correction budget.
#[must_use]
pub fn fix_step(progress: Progress, budget: Budget) -> Step {
    let hold = match progress {
        Progress::Committed | Progress::Unchanged(Answers::Accounted) => return Step::Verify,
        Progress::Gating => return Step::WaitGate,
        Progress::StaleGate => Hold::StaleGate,
        Progress::Unchanged(Answers::Fixed | Answers::Missing) => Hold::NoCommit,
    };
    match budget {
        Budget::Available => Step::Correct(hold),
        Budget::Exhausted => Step::AskHuman(hold),
    }
}

/// The lead's last completed run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeadRun {
    /// When the run started: what decides whether it can answer the findings of a pass.
    pub started: Timestamp,
}

impl LeadRun {
    /// A run that started at `started`; that is all that decides whether it can answer a pass.
    #[must_use]
    pub const fn started_at(started: Timestamp) -> Self {
        Self { started }
    }
}

/// Whether a lead's run can be the response to the findings of a fix pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Response {
    /// The lead has no completed run to judge.
    None,
    /// The run started before the reviewers' verdict, so it cannot answer findings it never had: whatever its final
    /// message says, also when the run ended after the verdict, is not judged.
    BeforeVerdict,
    /// The run started with the findings, or after them: its final message is the response.
    AfterVerdict,
}

/// Which of the lead's runs is judged as the response: only one that started at or after the verdict of the pass.
#[must_use]
pub fn response_of(run: Option<LeadRun>, verdict_at: Option<Timestamp>) -> Response {
    match (run, verdict_at) {
        (None, _) => Response::None,
        (Some(run), Some(verdict)) if run.started < verdict => Response::BeforeVerdict,
        (Some(_), Some(_) | None) => Response::AfterVerdict,
    }
}

#[cfg(test)]
mod tests;
