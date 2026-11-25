//! The agents their provider refused, as `agent status` shows them until each returns.

use agent_core::ids::AgentName;
use agent_core::local_time::Zone;
use agent_core::state::Timestamp;

#[cfg(test)]
mod tests;

/// The line that names each agent refused at `now` with the time it returns, in the local time `zone` gives; `None` when
/// no agent is refused. `refused` is what [`agent_core::state::State::refusals_at`] lists, in name order.
#[must_use]
pub(crate) fn refused_line(refused: &[(AgentName, Timestamp)], zone: Zone) -> Option<String> {
    if refused.is_empty() {
        return None;
    }
    let agents: Vec<String> = refused
        .iter()
        .map(|(agent, until)| format!("{agent} until {}", zone.stamp(*until)))
        .collect();
    Some(format!(
        "refused by their provider, kept out of new tasks, reviews and takeovers: {}\n",
        agents.join(", ")
    ))
}
