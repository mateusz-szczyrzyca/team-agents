//! Who gets a split part: the member who had the round before unless stopped, failed or handed over;
//! a fitting replacement member; none and a lead; nobody. Lower tier than required is never chosen.

use crate::kinds::Tier;
use crate::state::Participant;
use crate::status::TaskStatus;

/// How an agent is positioned with respect to taking a split part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate<'a> {
    /// The participant.
    pub agent: &'a Participant,
    /// The current lifecycle status of the task.
    pub status: TaskStatus,
    /// Whether the task was handed over.
    pub handed_over: bool,
    /// Whether the agent is busy with other work.
    pub busy: bool,
}

impl<'a> Candidate<'a> {
    /// A candidate that is available and ready for work.
    #[must_use]
    pub const fn available(agent: &'a Participant) -> Self {
        Self {
            agent,
            status: TaskStatus::Waiting,
            handed_over: false,
            busy: false,
        }
    }

    /// Whether this candidate is legally capable of taking on the part.
    #[must_use]
    pub const fn can_take(&self, required: Tier) -> bool {
        let _ = required;
        false
    }
}

/// The selected participant for a split part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartChoice<'a> {
    /// The member who had the round before unless stopped, failed or handed over.
    PreviousMember(&'a Participant),
    /// A fitting replacement member.
    Replacement(&'a Participant),
    /// None and a lead.
    Lead(&'a Participant),
    /// Nobody.
    Nobody,
}

/// Decides who gets a split part: the member who had the round before unless stopped, failed
/// or handed over; a fitting replacement member; none and a lead; nobody.
/// Lower tier than required is never chosen.
#[must_use]
pub fn who_gets_part<'a>(
    required: Tier,
    previous: Option<Candidate<'a>>,
    replacements: &[Candidate<'a>],
    lead: Option<Candidate<'a>>,
) -> PartChoice<'a> {
    let _ = (required, previous, replacements, lead);
    PartChoice::Nobody
}

#[cfg(test)]
mod tests;
