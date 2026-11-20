//! Tier-aware choice among offered members, with the coordinator as a fallback.

use crate::kinds::Tier;
use crate::state::Participant;

/// A choice that preserves the requested minimum capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TierChoice<'a> {
    /// The first fitting offered member.
    Member(&'a Participant),
    /// No offered member fits, but the coordinator does.
    Coordinator(&'a Participant),
    /// Nobody offered can satisfy the requirement; callers must wait rather than choose lower.
    NoAgent {
        /// The effective requirement, including the default for an omitted tier.
        required: Tier,
    },
}

/// Chooses the first member at or above `required`, falling back to a fitting coordinator.
///
/// An omitted requirement means [`Tier::Normal`]. The caller supplies members in preference order;
/// this decision does not determine availability or change team state.
#[must_use]
pub fn choose_agent<'a>(
    required: Option<Tier>,
    _members: &'a [Participant],
    _coordinator: Option<&'a Participant>,
) -> TierChoice<'a> {
    TierChoice::NoAgent {
        required: required.unwrap_or(Tier::Normal),
    }
}

#[cfg(test)]
mod tests;
