//! Whether an agent may start a run now. An agent has at most `[limits] runs_per_agent` runs going at once over all teams
//! and solo tasks (`runs_per_worker` for an agent of `[tiers] workers`); a queued task whose agent is at its limit waits
//! for the run that holds it. The waiting task is queued, never failed: the run that holds the agent ends, and the agent
//! is free again.

use std::num::NonZeroU32;

use crate::config::Limits;
use crate::ids::{AgentName, TaskId, TeamId};
use crate::kinds::Tier;
use crate::state::Team;
use crate::status::TaskStatus;

#[cfg(test)]
mod tests;

/// How many runs an agent may have going at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunLimit {
    /// No limit: the agent may run as many tasks at once as are queued (`0` in the configuration).
    Unlimited,
    /// At most this many runs; a positive number, so that an agent with a limit can always run one.
    AtMost(NonZeroU32),
}

impl RunLimit {
    /// The limit a configured number of runs means: `0` is no limit, any other number is the most runs.
    #[must_use]
    pub const fn of(configured: u32) -> Self {
        match NonZeroU32::new(configured) {
            Some(most) => Self::AtMost(most),
            None => Self::Unlimited,
        }
    }
}

/// The limit of runs that applies to an agent of `tier`: `runs_per_worker` for a worker, `runs_per_agent` for the others.
#[must_use]
pub const fn limit_for(limits: &Limits, tier: Tier) -> RunLimit {
    let configured = match tier {
        Tier::Worker => limits.runs_per_worker,
        Tier::Normal | Tier::Investigator => limits.runs_per_agent,
    };
    RunLimit::of(configured)
}

/// A run that goes: the agent that works in it, its team and its task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Going {
    /// The agent of the run.
    pub agent: AgentName,
    /// The team of the task of the run.
    pub team: TeamId,
    /// The task of the run.
    pub task: TaskId,
}

/// The first run of `going` that `agent` holds at `limit`: the agent has no run of its own to wait for when `None`.
#[must_use]
pub fn busy_run<'a>(agent: &AgentName, limit: RunLimit, going: &'a [Going]) -> Option<&'a Going> {
    let RunLimit::AtMost(most) = limit else {
        return None;
    };
    // The runs of `agent` are counted in order: the limit is reached at the run that makes the count `most`, and the
    // first run of the agent is the one a waiting task is told about.
    let mut seen: u32 = 0;
    let mut first = None;
    for run in going.iter().filter(|run| run.agent == *agent) {
        seen = seen.saturating_add(1);
        first.get_or_insert(run);
        if seen >= most.get() {
            return first;
        }
    }
    None
}

/// The runs that go over `teams`: every member whose task runs, of the crew and of the reviewers of each team, in the
/// order of the teams and of their members.
#[must_use]
pub fn runs_going(teams: &[Team]) -> Vec<Going> {
    teams
        .iter()
        .flat_map(|team| {
            let crew = team.crew().roster().into_iter().map(|(_, member)| member);
            crew.chain(team.reviewers().iter())
                .filter(|member| member.status() == TaskStatus::Running)
                .map(|member| Going {
                    agent: member.agent().agent_name(),
                    team: team.id().clone(),
                    task: member.task().clone(),
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// A queued task that asks to start a run: its task, its team, its agent and the agent's tier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queued {
    /// The task that is queued.
    pub task: TaskId,
    /// The team of the queued task.
    pub team: TeamId,
    /// The agent of the queued task.
    pub agent: AgentName,
    /// The tier of the agent.
    pub tier: Tier,
}

/// Whether a queued task starts a run now or waits for a run of its agent that holds the limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slot {
    /// The agent is below its limit of runs: the task starts now and takes a slot.
    Start,
    /// The agent is at its limit of runs: the task stays queued and waits for this run.
    Wait(Going),
}

/// Decides, for each task of `queued` in order, whether it starts now or waits for a run of its agent, given the runs
/// `going` and `limits`. A task that starts takes a slot of its agent for the tasks queued after it.
#[must_use]
pub fn plan_starts(queued: &[Queued], _going: &[Going], _limits: &Limits) -> Vec<(TaskId, Slot)> {
    queued.iter().map(|item| (item.task.clone(), Slot::Start)).collect()
}
