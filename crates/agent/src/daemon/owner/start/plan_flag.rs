//! The `agentbus serve` this daemon starts for a run: its tier and role on the bus, and the plan of a run that keeps one.
//!
//! A run of a role in `[plan] roles` keeps a plan; its bus is given the plan file (`--plan`), so a request to a worker
//! must name a step of it. A run of another role — a worker or a reviewer — has no plan, and its bus keeps the old form
//! of a request.

use std::path::Path;

use agent_core::kinds::{Role, Tier};
use agent_core::plan::PLAN_FILE;

use super::keeps_plan;

/// The value of the `--role` of `agentbus serve` for a run of an investigator.
const BUS_ROLE_INVESTIGATOR: &str = "investigator";

/// The value of the `--role` of `agentbus serve` for every other run.
const BUS_ROLE_WORKER: &str = "worker";

/// The `agent` the bus of a run that keeps a plan runs to check a request against the plan (`agent
/// _delegation-check`). A bare name on purpose: the bus is started by the run's CLI with the run's `PATH`, whose first
/// directory is the daemon's own (`agent_path_directories`), so it is the daemon's own binary.
const AGENT_COMMAND: &str = "agent";

/// The flags of the bus server of a run that depend on the run, each followed by its value: its tier and role on the
/// bus, which come from the tier alone, investigation access from the task role, and, for a run of a role in
/// `roles`, the plan file of its worktree and the agent command that checks requests against it. An investigator that coordinates or is a member keeps that role's
/// plan; one that answers an investigation request has no role of a plan.
///
/// The plan file's path is written as `worktree.display()` writes it: the caller gives a UTF-8 worktree.
pub(super) fn bus_words(role: Role, roles: &[Role], tier: Tier, worktree: &Path) -> Vec<String> {
    let (bus_tier, bus_role) = match tier {
        Tier::Normal => (Tier::Normal, BUS_ROLE_WORKER),
        Tier::Worker => (Tier::Worker, BUS_ROLE_WORKER),
        Tier::Investigator => (Tier::Normal, BUS_ROLE_INVESTIGATOR),
    };
    let investigations = match role {
        Role::Coordinator | Role::Member => "on",
        Role::Solo | Role::Worker | Role::Reviewer | Role::Investigator => "off",
    };
    let mut words: Vec<String> = [
        "--tier",
        bus_tier.as_str(),
        "--role",
        bus_role,
        "--investigations",
        investigations,
    ]
    .iter()
    .map(|word| (*word).to_owned())
    .collect();
    if keeps_plan(roles, role) {
        words.extend([
            "--plan".to_owned(),
            worktree.join(PLAN_FILE).display().to_string(),
            "--agent-cmd".to_owned(),
            AGENT_COMMAND.to_owned(),
        ]);
    }
    words
}

#[cfg(test)]
mod tests;
