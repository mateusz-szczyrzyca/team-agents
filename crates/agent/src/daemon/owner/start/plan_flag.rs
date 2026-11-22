//! The `agentbus serve` this daemon starts for a run: its tier and role on the bus, and the plan of a run that keeps one.
//!
//! A run of a role in `[plan] roles` keeps a plan; its bus is given the plan file (`--plan`), so a request to a worker
//! must name a step of it. A run of another role — a worker or a reviewer — has no plan, and its bus keeps the old form
//! of a request.

use std::path::Path;

use agent_core::kinds::{Role, Tier};
use agent_core::plan::PLAN_FILE;

use super::keeps_plan;

/// The words after `agentbus` of the bus server of a run: its tier and role on the bus, and, for a run of a role in
/// `roles` and of the normal or the worker tier, the plan file of its worktree. An investigator is no role of a plan:
/// one in a team works as a coordinator or a member and keeps that role's plan.
pub(super) fn bus_words(role: Role, roles: &[Role], tier: Tier, worktree: &Path) -> Vec<String> {
    let bus_tier = match tier {
        Tier::Worker => Tier::Worker,
        Tier::Normal | Tier::Investigator => Tier::Normal,
    };
    let bus_role = match (tier, role) {
        (Tier::Investigator, _) | (_, Role::Reviewer) => "investigator",
        (Tier::Normal | Tier::Worker, Role::Solo | Role::Member | Role::Coordinator | Role::Worker) => "worker",
    };
    let mut words: Vec<String> = ["serve", "--tier", bus_tier.as_str(), "--role", bus_role]
        .iter()
        .map(|word| (*word).to_owned())
        .collect();
    if keeps_plan(roles, role) {
        words.push("--plan".to_owned());
        words.push(worktree.join(PLAN_FILE).display().to_string());
    }
    words
}

#[cfg(test)]
mod tests;
