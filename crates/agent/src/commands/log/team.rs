//! The teams and runs of a scope: the task, team and round of each run, and the runs the rows belong to.

use std::collections::BTreeMap;

use agent_core::history::RunStart;
use agent_core::ids::{RunId, TeamId};
use agent_runtime::history::{ReadError, Reader};

/// A team with what its history says of its runs, in one order.
#[derive(Debug, Clone)]
pub(crate) struct Team {
    /// The team.
    pub(crate) team: TeamId,
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
            runs: BTreeMap::new(),
        };
        for run in reader.runs_of_team(&team.value.team)? {
            made.runs.insert(run.start.value.run.clone(), run.start.value);
        }
        teams.push(made);
    }
    Ok(teams)
}
