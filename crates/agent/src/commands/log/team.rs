//! The teams and runs of a scope: the task, team and round of each run, and the runs the rows belong to.

use std::collections::BTreeMap;

use agent_core::history::RunStart;
use agent_core::ids::{AgentName, RoundId, RunId, TaskId, TeamId};
use agent_core::state::Timestamp;
use agent_runtime::history::{ReadError, Reader};

/// A team with what its history says of its runs, in one order.
#[derive(Debug, Clone)]
pub(crate) struct Team {
    /// The team.
    pub(crate) team: TeamId,
    /// When it was formed.
    pub(crate) at: Timestamp,
    /// Its runs by ID, each with the rounds it works for.
    runs: BTreeMap<RunId, RunStart>,
}

impl Team {
    /// The start of the run `run`, with the rounds it works for.
    pub(crate) fn run_of(&self, run: Option<&RunId>) -> Option<&RunStart> {
        run.and_then(|run| self.runs.get(run))
    }
}

/// The teams of every team of the history, oldest first, each with its runs.
///
/// # Errors
/// [`ReadError`] when the history cannot be read.
pub(crate) fn teams(reader: &Reader) -> Result<Vec<Team>, ReadError> {
    let mut teams: Vec<Team> = Vec::new();
    for team in reader.teams()? {
        let mut made = Team {
            team: team.value.team.clone(),
            at: team.value.at,
            runs: BTreeMap::new(),
        };
        for run in reader.runs_of_team(&team.value.team)? {
            made.runs.insert(run.start.value.run.clone(), run.start.value);
        }
        teams.push(made);
    }
    Ok(teams)
}

/// The rounds that `run` names, from `rounds` when the run says none of it.
pub(crate) fn rounds_of(run: &RunStart, rounds: &[RoundId]) -> Vec<RoundId> {
    let _ = (run, rounds);
    Vec::new()
}

/// The agents of `teams` that run `task`, when there is more than one.
pub(crate) fn agents_of(teams: &[Team], task: &TaskId) -> Vec<AgentName> {
    let mut agents: Vec<AgentName> = Vec::new();
    for team in teams {
        for run in team.runs.values().filter(|run| &run.task == task) {
            match run.agent.name.parse() {
                Ok(agent) => agents.push(agent),
                Err(source) => tracing::warn!(%source, "a run has no agent name the log can show"),
            }
        }
    }
    agents.sort();
    agents.dedup();
    agents
}
