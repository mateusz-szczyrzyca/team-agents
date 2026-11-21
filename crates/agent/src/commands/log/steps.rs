//! The steps of a scope: the steps of the runs a scope covers, and the run that each of them belongs to.
//!
//! A step is the same thing that `agent log --round` tells as the story's numbered steps: what one CLI recorded of a
//! run. For a scope that is not a round — a task, an agent, a run — the steps are read directly from the history and
//! lined up with the daemon's own rows by the time they were recorded. A log row keeps only its database `id`, which
//! orders it among the rows and not among the steps, so the time of a row gives its place in the day: a step recorded
//! between two rows prints between them.

#[cfg(test)]
mod tests;

use agent_core::history::{RunStart, Step};
use agent_core::state::Timestamp;
use agent_runtime::history::{LogFilter, ReadError, Reader, RunRow};

/// A step of a covered run, at its place in the day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TimedStep {
    /// The step.
    pub(crate) step: Step,
    /// The start of its run.
    pub(crate) run: RunStart,
    /// When the step was recorded.
    pub(crate) at: Timestamp,
}

/// The steps of `run`, oldest first.
///
/// # Errors
/// [`ReadError`] when the history cannot be read.
pub(crate) fn run_steps(reader: &Reader, run: &RunStart) -> Result<Vec<TimedStep>, ReadError> {
    reader
        .steps(&run.run)?
        .iter()
        .map(|stored| {
            // The reader returns the steps of exactly this run, under the start it read first.
            Ok(TimedStep {
                step: stored.value.clone(),
                run: run.clone(),
                at: stored.value.at,
            })
        })
        .collect()
}

/// The runs a scope covers: those of the most specific of its run, task, agent, round and team, that also match
/// everything else the scope names. A scope that names none of them covers no run.
///
/// # Errors
/// [`ReadError`] when the history cannot be read.
pub(crate) fn covered_runs(reader: &Reader, filter: &LogFilter) -> Result<Vec<RunRow>, ReadError> {
    let runs = if let Some(run) = &filter.run {
        reader.run(run)?.into_iter().collect()
    } else if let Some(task) = &filter.task {
        reader.runs_of_task(task)?
    } else if let Some(agent) = &filter.agent {
        reader.runs_of_agent(agent)?
    } else if let Some(round) = &filter.round {
        reader.runs_of_round(round)?
    } else if let Some(team) = &filter.team {
        reader.runs_of_team(team)?
    } else {
        Vec::new()
    };
    Ok(runs
        .into_iter()
        .filter(|row| {
            let start = &row.start.value;
            filter.run.as_ref().is_none_or(|run| *run == start.run)
                && filter.task.as_ref().is_none_or(|task| *task == start.task)
                && filter
                    .agent
                    .as_ref()
                    .is_none_or(|agent| agent.as_str() == start.agent.name)
                && filter.round.as_ref().is_none_or(|round| start.rounds.contains(round))
                && filter.team.as_ref().is_none_or(|team| *team == start.team)
        })
        .collect())
}
