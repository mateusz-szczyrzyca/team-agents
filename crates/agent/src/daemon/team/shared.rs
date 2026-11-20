//! Which agents of a team that starts work in another open team too: `agent run` names them, since the agent then has
//! a session in each team and a message must reach the right one.

use agent_core::protocol::SharedAgent;
use agent_core::state::Team;
use agent_core::status::TaskStatus;

use super::is_finished;

/// The agents of the crew of `team` that are in the crew of an open team of `others` too, with that team and their
/// task there, in the order of `team`'s roster and then of `others`. A team is open until it is finished, and an agent
/// is in it until its task there is done: until then the bus may wake its session there.
pub(in crate::daemon) fn shared_agents<'a>(
    team: &Team,
    others: impl IntoIterator<Item = &'a Team>,
) -> Vec<SharedAgent> {
    let open: Vec<&Team> = others.into_iter().filter(|other| !is_finished(other)).collect();
    let mut shared = Vec::new();
    for (_, member) in team.crew().roster() {
        let agent = member.agent().agent_name();
        for other in &open {
            for (_, theirs) in other.crew().roster() {
                if theirs.agent().agent_name() == agent && theirs.status() != TaskStatus::Done {
                    shared.push(SharedAgent {
                        agent: agent.clone(),
                        team: other.id().clone(),
                        task: theirs.task().clone(),
                    });
                }
            }
        }
    }
    shared
}

#[cfg(test)]
mod tests;
