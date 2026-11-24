//! What `agent status` says of the pool of workers from `[tiers] workers`: each worker, whom it serves (team or task, step) or that it is free.

use agent_core::ids::AgentName;
use agent_core::protocol::TeamSummary;
use agent_core::status::TaskStatus;
use agent_runtime::store::Home;

#[cfg(test)]
mod tests;

/// The lines describing the pool workers of `configured`: each worker, whom it serves (team or task, step) or that it is free.
///
/// Returns `None` if `configured` is empty.
pub(crate) fn render_pool(
    configured: &[AgentName],
    teams: &[TeamSummary],
    helpers: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Option<String> {
    if configured.is_empty() {
        return None;
    }
    let mut lines = vec!["pool:".to_owned()];
    for worker in configured {
        let served = find_serving(worker, teams, helpers);
        lines.push(format!("  {}  {}", worker, served.unwrap_or_else(|| "free".to_owned())));
    }
    let mut text = lines.join("\n");
    text.push('\n');
    Some(text)
}

/// Reads the helper requests from the bus state file of `home`, if present.
pub(crate) fn read_helpers(home: &Home) -> Option<serde_json::Map<String, serde_json::Value>> {
    let state_file = home.root().join("bus/state.json");
    let text = std::fs::read_to_string(state_file).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.get("helpers").and_then(serde_json::Value::as_object).cloned()
}

fn find_serving(
    worker: &AgentName,
    teams: &[TeamSummary],
    helpers: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Option<String> {
    if let Some(helpers) = helpers {
        for helper in helpers.values() {
            let status = helper
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if status != "running" && status != "starting" {
                continue;
            }
            let assigned_worker = helper
                .get("worker")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if assigned_worker != worker.as_str() {
                continue;
            }
            let from_task = helper
                .get("from_task")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let team_id = teams
                .iter()
                .find(|t| t.members.iter().any(|m| m.task.as_str() == from_task))
                .map_or(from_task, |t| t.id.as_str());
            let step = helper
                .get("step")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if !step.is_empty() {
                return Some(format!("{team_id} (step {step})"));
            }
            if !team_id.is_empty() {
                return Some(team_id.to_owned());
            }
        }
    }
    for team in teams {
        if team
            .members
            .iter()
            .any(|m| m.agent.name() == worker.as_str() && m.status == TaskStatus::Running)
        {
            return Some(team.id.to_string());
        }
    }
    None
}
