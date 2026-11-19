//! The obligation that can advance an idle round, independent of I/O and scheduling.

/// Whether a round still needs work and whether an agent can currently do it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    /// The round has finished.
    Closed,
    /// A task is queued or running.
    Active,
    /// All tasks are waiting or unable to run.
    Idle,
}

/// An outstanding action with an owner that can advance a round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Obligation {
    /// A harness gate is queued, running, or scheduled for retry.
    Gate,
    /// Reviewers or the publication scheduler must advance the review.
    Review,
    /// An independent investigator must answer a disputed finding.
    SecondOpinion,
    /// Only an instruction or answer from the human can advance the round.
    Human,
    /// The lead must correct its handover or answer.
    LeadCorrection,
}

/// What an idle open round waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waiting {
    /// An outstanding action has an owner.
    For(Obligation),
    /// Nothing can advance the round: the harness has lost an obligation.
    Defect,
}

impl Waiting {
    /// The short status text, shared with the harness's prompt catalog.
    #[must_use]
    pub const fn text(self) -> &'static str {
        match self {
            Self::For(Obligation::Gate) => include_str!("../../../../prompts/harness/pr/wait-gate.md"),
            Self::For(Obligation::Review) => include_str!("../../../../prompts/harness/pr/wait-review.md"),
            Self::For(Obligation::SecondOpinion) => include_str!("../../../../prompts/harness/pr/wait-opinion.md"),
            Self::For(Obligation::Human) => include_str!("../../../../prompts/harness/pr/wait-human.md"),
            Self::For(Obligation::LeadCorrection) => include_str!("../../../../prompts/harness/pr/wait-correction.md"),
            Self::Defect => include_str!("../../../../prompts/harness/pr/wait-nothing.md"),
        }
    }
}

/// Reports an idle round's obligation; a closed or active round needs no wait line.
#[must_use]
pub fn round_wait(activity: Activity, obligation: Option<Obligation>) -> Option<Waiting> {
    match activity {
        Activity::Closed | Activity::Active => None,
        Activity::Idle => Some(obligation.map_or(Waiting::Defect, Waiting::For)),
    }
}

#[cfg(test)]
mod tests;
