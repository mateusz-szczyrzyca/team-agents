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
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter)]
pub enum Obligation {
    /// The PR delivering this item must merge before its round can finish.
    Merge,
    /// A harness gate is queued, running, or scheduled for retry.
    Gate,
    /// Reviewers or the publication scheduler must advance the review.
    Review,
    /// The review needs an available independent investigator.
    FreeInvestigator,
    /// An independent investigator must answer a disputed finding.
    SecondOpinion,
    /// Only an instruction or answer from the human can advance the round.
    Human,
    /// The lead must correct its handover or answer.
    LeadCorrection,
    /// A task of the round is queued, or waits for a free agent or a limit of runs, and its run starts the round.
    QueuedRun,
}

/// Who is obliged to act on an [`Obligation`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOwner {
    /// The lead of the team.
    Lead,
    /// The human.
    Human,
    /// The reviewers and the investigators that answer for the review.
    Reviewers,
    /// The daemon itself, or GitHub through it: nobody has to be asked.
    Harness,
}

/// How whoever owes an [`Obligation`] learns what is expected of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Told {
    /// In its next prompt or a message on the bus, which also wakes it.
    ByPrompt,
    /// In a question or a notification that reaches the human.
    ByNotification,
    /// It needs no telling: the daemon does the work itself.
    NoOneToTell,
}

impl Obligation {
    /// Who is obliged to act.
    #[must_use]
    pub const fn owner(self) -> WaitOwner {
        WaitOwner::Harness
    }

    /// How the owner is told what is expected of it.
    #[must_use]
    pub const fn told(self) -> Told {
        Told::NoOneToTell
    }
}

impl WaitOwner {
    /// The name of the owner in a wait line, from the harness's prompt catalog.
    #[must_use]
    pub const fn text(self) -> &'static str {
        ""
    }
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
            Self::For(Obligation::Merge) => include_str!("../../../../prompts/harness/pr/wait-merge.md"),
            Self::For(Obligation::Gate) => include_str!("../../../../prompts/harness/pr/wait-gate.md"),
            Self::For(Obligation::Review) => include_str!("../../../../prompts/harness/pr/wait-review.md"),
            Self::For(Obligation::FreeInvestigator) => {
                include_str!("../../../../prompts/harness/pr/wait-investigator.md")
            }
            Self::For(Obligation::SecondOpinion) => include_str!("../../../../prompts/harness/pr/wait-opinion.md"),
            Self::For(Obligation::Human) => include_str!("../../../../prompts/harness/pr/wait-human.md"),
            Self::For(Obligation::LeadCorrection) => include_str!("../../../../prompts/harness/pr/wait-correction.md"),
            Self::For(Obligation::QueuedRun) => include_str!("../../../../prompts/harness/pr/wait-queued.md"),
            Self::Defect => include_str!("../../../../prompts/harness/pr/wait-nothing.md"),
        }
    }
}

impl Waiting {
    /// The wait as a line that names who is obliged and, when it is known, since when: `since` is the time as the reader
    /// is shown it. An unowned round names nobody.
    #[must_use]
    pub fn line(self, _since: Option<&str>) -> String {
        self.text().to_owned()
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
