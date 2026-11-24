//! What `agent status` says of the pool of workers from `[tiers] workers`: each worker, whom it serves (team or task, step) or that it is free.

use agent_core::ids::AgentName;
use agent_core::kinds::Role;
use agent_core::protocol::TeamSummary;
use agent_core::status::TaskStatus;
use agent_runtime::agentbus::{DelegationStatus, HelperEntry};

#[cfg(test)]
mod tests;

/// An unsettled workspace observation for a pool worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnsettledWorker {
    pub(crate) worker: Option<AgentName>,
    pub(crate) from_task: String,
    pub(crate) step: Option<String>,
    pub(crate) damaged: bool,
}

/// The lines describing the pool workers of `configured`: each worker, whom it serves (team or task, step) or that it is free.
///
/// Returns `None` if `configured` is empty.
#[cfg(test)]
pub(crate) fn render_pool(
    configured: &[AgentName],
    teams: &[TeamSummary],
    helpers: Option<&[HelperEntry]>,
) -> Option<String> {
    render_pool_with_unsettled(configured, teams, helpers, None)
}

/// The lines describing the pool workers of `configured`, also taking unsettled workspace observations into account.
///
/// Returns `None` if `configured` is empty.
pub(crate) fn render_pool_with_unsettled(
    configured: &[AgentName],
    teams: &[TeamSummary],
    helpers: Option<&[HelperEntry]>,
    unsettled: Option<&[UnsettledWorker]>,
) -> Option<String> {
    if configured.is_empty() {
        return None;
    }
    let mut lines = vec!["pool:".to_owned()];
    for worker in configured {
        let served = find_serving(worker, teams, helpers, unsettled);
        lines.push(format!("  {}  {}", worker, served.unwrap_or_else(|| "free".to_owned())));
    }
    let mut text = lines.join("\n");
    text.push('\n');
    Some(text)
}

fn find_serving(
    worker: &AgentName,
    teams: &[TeamSummary],
    helpers: Option<&[HelperEntry]>,
    unsettled: Option<&[UnsettledWorker]>,
) -> Option<String> {
    if let Some(helpers) = helpers {
        for helper in helpers {
            if !matches!(helper.status, DelegationStatus::Running | DelegationStatus::Starting) {
                continue;
            }
            if helper.worker.as_ref() != Some(worker) {
                continue;
            }
            let from_task = helper.from_task.as_str();
            let team_id = teams
                .iter()
                .find(|t| t.members.iter().any(|m| m.task.as_str() == from_task))
                .map_or(from_task, |t| t.id.as_str());
            if let Some(ref step) = helper.step
                && !step.is_empty()
            {
                return Some(format!("{team_id} (step {step})"));
            }
            if !team_id.is_empty() {
                return Some(team_id.to_owned());
            }
        }
    }
    if let Some(unsettled) = unsettled {
        for entry in unsettled {
            if entry.worker.as_ref() != Some(worker) {
                continue;
            }
            if entry.damaged {
                return Some("unsettled".to_owned());
            }
            let from_task = entry.from_task.as_str();
            let team_id = teams
                .iter()
                .find(|t| t.members.iter().any(|m| m.task.as_str() == from_task))
                .map_or(from_task, |t| t.id.as_str());
            if let Some(ref step) = entry.step
                && !step.is_empty()
            {
                return Some(format!("{team_id} (step {step})"));
            }
            if !team_id.is_empty() {
                return Some(team_id.to_owned());
            }
        }
        if unsettled.iter().any(|u| u.damaged && u.worker.is_none()) {
            return Some("unsettled".to_owned());
        }
    }
    for team in teams {
        if team_is_stopped(team) {
            continue;
        }
        if let Some(m) = team.members.iter().find(|m| m.agent.name() == worker.as_str())
            && (m.status == TaskStatus::Running
                || (m.role == Role::Worker
                    && !matches!(m.status, TaskStatus::Done | TaskStatus::Failed | TaskStatus::Stopped)))
        {
            return Some(team.id.to_string());
        }
    }
    None
}

fn team_is_stopped(team: &TeamSummary) -> bool {
    !team.members.is_empty()
        && team
            .members
            .iter()
            .all(|m| matches!(m.status, TaskStatus::Stopped | TaskStatus::Failed | TaskStatus::Done))
}
