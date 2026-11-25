//! The obligation that can advance an idle round, independent of I/O and scheduling.

use crate::ids::{AgentName, TeamId};

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
        match self {
            Self::Merge | Self::Gate | Self::FreeInvestigator | Self::QueuedRun => WaitOwner::Harness,
            Self::Review | Self::SecondOpinion => WaitOwner::Reviewers,
            Self::Human => WaitOwner::Human,
            Self::LeadCorrection => WaitOwner::Lead,
        }
    }

    /// How the owner is told what is expected of it.
    #[must_use]
    pub const fn told(self) -> Told {
        match self.owner() {
            WaitOwner::Lead | WaitOwner::Reviewers => Told::ByPrompt,
            WaitOwner::Human => Told::ByNotification,
            WaitOwner::Harness => Told::NoOneToTell,
        }
    }
}

impl WaitOwner {
    /// The name of the owner in a wait line, from the harness's prompt catalog.
    #[must_use]
    pub const fn text(self) -> &'static str {
        match self {
            Self::Lead => include_str!("../../../../prompts/harness/pr/wait-owner-lead.md"),
            Self::Human => include_str!("../../../../prompts/harness/pr/wait-owner-human.md"),
            Self::Reviewers => include_str!("../../../../prompts/harness/pr/wait-owner-reviewers.md"),
            Self::Harness => include_str!("../../../../prompts/harness/pr/wait-owner-harness.md"),
        }
    }
}

/// What an idle open round waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waiting {
    /// An outstanding action has an owner.
    For(Obligation),
    /// A task of the round is paused because its provider refused it: only the provider's return, or another agent, can
    /// advance the round. The agent and the time are told by [`provider_line`].
    Provider,
    /// Nothing can advance the round: the harness has lost an obligation.
    Defect,
}

/// The end of a wait line of a queued run that goes after the run of `agent` in `team`: the wait line says which run.
#[must_use]
pub fn behind_run(agent: &AgentName, team: &TeamId) -> String {
    include_str!("../../../../prompts/harness/pr/wait-behind.md")
        .replace("{{agent}}", agent.as_str())
        .replace("{{team}}", team.as_str())
}

/// The wait line of a round whose task of `agent` is paused by its provider until `until`, the time as the reader is shown it.
#[must_use]
pub fn provider_line(agent: &AgentName, until: &str) -> String {
    Waiting::Provider
        .text()
        .replace("{{agent}}", agent.as_str())
        .replace("{{until}}", until)
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
            Self::Provider => include_str!("../../../../prompts/harness/pr/wait-provider.md"),
            Self::Defect => include_str!("../../../../prompts/harness/pr/wait-nothing.md"),
        }
    }
}

impl Waiting {
    /// The wait as a line that names who is obliged and, when it is known, since when: `since` is the time as the reader
    /// is shown it. An unowned round names nobody.
    #[must_use]
    pub fn line(self, since: Option<&str>) -> String {
        let Self::For(obligation) = self else {
            return self.text().to_owned();
        };
        let template = match since {
            Some(_) => include_str!("../../../../prompts/harness/pr/wait-owned.md"),
            None => include_str!("../../../../prompts/harness/pr/wait-owned-undated.md"),
        };
        template
            .replace("{{wait}}", self.text())
            .replace("{{owner}}", obligation.owner().text())
            .replace("{{since}}", since.unwrap_or_default())
    }

    /// The wait as a line that also names the active run `(agent, team)` holding a queued task of the round at its
    /// run limit. Only [`Obligation::QueuedRun`] carries a holding run; any other wait state ignores `holder`.
    #[must_use]
    pub fn line_with_holder(self, since: Option<&str>, holder: Option<(&AgentName, &TeamId)>) -> String {
        let base = self.line(since);
        match (self, holder) {
            (Self::For(Obligation::QueuedRun), Some((agent, team))) => {
                format!("{base}{}", behind_run(agent, team))
            }
            _ => base,
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
