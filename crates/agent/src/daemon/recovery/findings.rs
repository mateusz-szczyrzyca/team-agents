//! Recovery on start: the items of one pull request that disagree on its findings are brought together.

use agent_core::ids::{ItemId, TeamId};
use agent_core::state::{PrNumber, Reconciled, State, reconcile};

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
pub(in crate::daemon) fn reconcile_findings(state: &mut State) -> Vec<RepairedFinding> {
    let mut repaired = Vec::new();
    let teams: Vec<TeamId> = state.teams().iter().map(|team| team.id().clone()).collect();
    for team_id in teams {
        let Some(mut team) = state.team_mut(&team_id) else {
            continue;
        };
        let mut pulls: Vec<PrNumber> = team.items().iter().filter_map(super::super::merge::pull_of).collect();
        pulls.sort_unstable();
        pulls.dedup();
        for pr in pulls {
            let items: Vec<ItemId> = super::super::merge::items_of(&team, pr)
                .into_iter()
                .filter(|id| team.item(*id).is_some_and(|item| item.findings().is_some()))
                .collect();
            let mut ledgers: Vec<_> = items
                .iter()
                .filter_map(|id| team.item(*id)?.findings().cloned())
                .collect();
            let findings = reconcile(&mut ledgers);
            if findings.is_empty() {
                continue;
            }
            for (id, ledger) in items.iter().zip(ledgers) {
                if let Some(mut item) = team.item_mut(*id)
                    && let Some(saved) = item.findings_mut()
                {
                    *saved = ledger;
                }
            }
            repaired.extend(findings.into_iter().map(|finding| RepairedFinding {
                team: team_id.clone(),
                pr,
                items: items.clone(),
                finding,
            }));
        }
    }
    repaired
}

#[cfg(test)]
mod tests;
