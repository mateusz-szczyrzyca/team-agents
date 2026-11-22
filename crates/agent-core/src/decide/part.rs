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
    pub fn can_take(&self, required: Tier) -> bool {
        !self.busy
            && !self.handed_over
            && matches!(self.status, TaskStatus::Queued | TaskStatus::Waiting)
            && self.agent.tier() >= required
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

impl PartChoice<'_> {
    /// The name of this choice kind.
    #[must_use]
    pub const fn choice_name(&self) -> &'static str {
        match self {
            Self::PreviousMember(_) => "previous_member",
            Self::Replacement(_) => "replacement",
            Self::Lead(_) => "lead",
            Self::Nobody => "nobody",
        }
    }

    /// The human- and machine-readable explanation of why this choice was made.
    #[must_use]
    pub fn explanation(&self, prev_candidate: Option<&Candidate<'_>>) -> String {
        let prev_status_desc = prev_candidate.map(|prev| {
            if prev.handed_over {
                format!("previous member {} was handed over", prev.agent.name())
            } else if prev.busy {
                format!("previous member {} is busy", prev.agent.name())
            } else {
                match prev.status {
                    TaskStatus::Stopped => format!("previous member {} is stopped", prev.agent.name()),
                    TaskStatus::Failed => format!("previous member {} failed", prev.agent.name()),
                    _ => format!("previous member {}", prev.agent.name()),
                }
            }
        });

        match self {
            Self::PreviousMember(agent) => {
                format!("previous member {} is available and qualified", agent.name())
            }
            Self::Replacement(agent) => {
                if let Some(desc) = prev_status_desc {
                    format!("{desc}; replacement {} takes the part", agent.name())
                } else {
                    format!("replacement {} takes the part", agent.name())
                }
            }
            Self::Lead(agent) => {
                if let Some(desc) = prev_status_desc {
                    format!("{desc}; lead {} takes over", agent.name())
                } else {
                    format!("lead {} takes over", agent.name())
                }
            }
            Self::Nobody => {
                if let Some(desc) = prev_status_desc {
                    format!("{desc}; no qualified candidate available")
                } else {
                    "no qualified candidate available".to_owned()
                }
            }
        }
    }
}

/// Decides who gets a split part under a specific replacement mode.
#[must_use]
pub fn who_gets_part_in_mode<'a>(
    required: Tier,
    previous: Option<Candidate<'a>>,
    replacements: &[Candidate<'a>],
    lead: Option<Candidate<'a>>,
    mode: crate::decide::replacement::ReplacementMode,
) -> PartChoice<'a> {
    if let Some(prev) = previous.filter(|cand| cand.can_take(required)) {
        return PartChoice::PreviousMember(prev.agent);
    }
    match mode {
        crate::decide::replacement::ReplacementMode::Off => PartChoice::Nobody,
        crate::decide::replacement::ReplacementMode::Lead => {
            if let Some(coord) = lead.filter(|cand| cand.can_take(required)) {
                return PartChoice::Lead(coord.agent);
            }
            PartChoice::Nobody
        }
        crate::decide::replacement::ReplacementMode::Auto => {
            let min_tier = previous.map_or(required, |prev| prev.agent.tier().max(required));
            if let Some(replacement) = replacements.iter().find(|cand| cand.can_take(min_tier)) {
                return PartChoice::Replacement(replacement.agent);
            }
            if let Some(coord) = lead.filter(|cand| cand.can_take(required)) {
                return PartChoice::Lead(coord.agent);
            }
            PartChoice::Nobody
        }
    }
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
    who_gets_part_in_mode(
        required,
        previous,
        replacements,
        lead,
        crate::decide::replacement::ReplacementMode::Auto,
    )
}

#[cfg(test)]
mod tests;
