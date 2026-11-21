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
use agent_core::ids::{AgentName, TaskId};
use agent_core::log::LogEntry;
use agent_core::state::Timestamp;
use agent_runtime::history::{ReadError, Reader};

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

/// What a step is lined up against in the log: the `id` of the row it follows, or `None` for a step that comes before
/// every row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Place {
    /// Before the first row of the log.
    First,
    /// After the row with this `id`.
    After(i64),
}

/// The steps of `task` in its runs, oldest first.
///
/// # Errors
/// [`ReadError`] when the history cannot be read.
pub(crate) fn task_steps(reader: &Reader, task: &TaskId) -> Result<Vec<TimedStep>, ReadError> {
    let mut steps = Vec::new();
    for row in reader.runs_of_task(task)? {
        steps.extend(run_steps(reader, &row.start.value)?);
    }
    steps.sort_by_key(|timed| (timed.at, timed.step.run.clone(), timed.step.number));
    Ok(steps)
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

/// The steps of every run of `agent`, wherever its team, oldest first.
///
/// # Errors
/// [`agent_runtime::history::ReadError`] when the history cannot be read.
pub(crate) fn agent_steps(reader: &Reader, agent: &AgentName) -> Result<Vec<TimedStep>, ReadError> {
    let mut steps = Vec::new();
    for row in reader.runs_of_agent(agent)? {
        steps.extend(run_steps(reader, &row.start.value)?);
    }
    steps.sort_by_key(|timed| (timed.at, timed.step.run.clone(), timed.step.number));
    Ok(steps)
}

/// Where each step of `steps`, oldest first, stands against `rows`, oldest first: after the last row recorded no later
/// than its time, which keeps a step between the rows of its time.
pub(crate) fn places(rows: &[LogEntry], steps: &[TimedStep]) -> Vec<Place> {
    let mut places: Vec<Place> = Vec::with_capacity(steps.len());
    let mut next = 0_usize;
    for timed in steps {
        while let Some(row) = rows.get(next)
            && row.at() <= timed.at
        {
            next = next.saturating_add(1);
        }
        match next {
            0 | 1 => places.push(Place::First),
            _ => match rows.get(next.saturating_sub(1)) {
                Some(row) => places.push(Place::After(row.id())),
                None => places.push(Place::First),
            },
        }
    }
    places
}
