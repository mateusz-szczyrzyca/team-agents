//! Whether a completed fix pass has supplied the commit its answers require.

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
pub fn fix_step(_progress: Progress, _budget: Budget) -> Step {
    Step::Verify
}

#[cfg(test)]
mod tests;
