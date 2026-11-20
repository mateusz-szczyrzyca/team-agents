//! Which agents of a team that starts work in another open team too: `agent run` names them, since the agent then has
//! a session in each team and a message must reach the right one.

use agent_core::protocol::SharedAgent;
use agent_core::state::Team;

/// The agents of the crew of `team` that are in the crew of an open team of `others` too, with that team and their
/// task there, in the order of `team`'s roster and then of `others`.
pub(in crate::daemon) fn shared_agents<'a>(
    _team: &Team,
    others: impl IntoIterator<Item = &'a Team> + Clone,
) -> Vec<SharedAgent> {
    others
        .into_iter()
        .flat_map(|other| {
            other.crew().roster().into_iter().filter_map(|(_, member)| {
                Some(SharedAgent {
                    agent: member.agent().name().parse().ok()?,
                    team: other.id().clone(),
                    task: member.task().clone(),
                })
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;
