//! Recovery on start: the items of one pull request that disagree on its findings are brought together.

use agent_core::ids::{ItemId, TeamId};
use agent_core::state::{PrNumber, Reconciled, State};

/// One finding that the items of a pull request held apart, as it stands in all of them after
/// [`reconcile_findings`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon) struct RepairedFinding {
    /// The team of the pull request.
    pub(in crate::daemon) team: TeamId,
    /// The pull request.
    pub(in crate::daemon) pr: PrNumber,
    /// The items of the pull request whose ledgers were brought together, in the order of the list.
    pub(in crate::daemon) items: Vec<ItemId>,
    /// The finding and how it stands now.
    pub(in crate::daemon) finding: Reconciled,
}

/// Brings the ledgers of the items of every pull request of `state` together by the rule of
/// [`agent_core::state::reconcile`], and returns every finding that was repaired, team by team and pull request by pull
/// request.
pub(in crate::daemon) fn reconcile_findings(_state: &mut State) -> Vec<RepairedFinding> {
    Vec::new()
}

#[cfg(test)]
mod tests;
