//! The feed of `agent log --follow`: the newest lines of what a scope covers, then every row and every step recorded
//! after them, each once.
//!
//! The rows of the daemon are followed by their database ID. The steps of a run are followed by their number in the
//! run, which the writer allocates in order, so a step is never printed twice and none is skipped, whatever the order
//! of the times of rows and steps. A run is told once, before the first of its steps that is printed.

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use agent_core::history::RunStart;
use agent_core::ids::{AgentName, RoundId, RunId};
use agent_core::log::{LogEntry, LogEvent};
use agent_core::story::StepView;
use agent_runtime::history::{LogFilter, LogWindow, Reader, RunRow};

use super::LogError;
use super::steps::{TimedStep, covered_runs, run_steps};
use super::timeline::{Team, Timeline};

/// The most rows one page of a follow reads; the next page reads the rest.
const PAGE_ROWS: u32 = 1000;

/// What a follow has printed of its scope.
#[derive(Debug)]
pub(crate) struct Feed {
    /// What the follow covers.
    filter: LogFilter,
    /// Whether the steps of the runs are printed.
    steps: StepView,
    /// The ID of the last row of the daemon's log that was read.
    row_cursor: i64,
    /// For each run, the number of the last step that was read.
    step_cursors: BTreeMap<RunId, u32>,
    /// The runs that ended and were read to their last step: they are not read again.
    drained: BTreeSet<RunId>,
    /// The runs whose first line was printed.
    told: BTreeSet<String>,
    /// The runs that rows and steps refer to, by ID.
    runs: BTreeMap<RunId, RunStart>,
    /// The agents of the runs the follow covers: a line names its agent when there are more than one.
    agents: Vec<AgentName>,
}

impl Feed {
    /// Opens the feed of `filter`: the text of its newest `limit` lines, rows and steps together, and the feed that
    /// goes on after them.
    ///
    /// # Errors
    /// [`LogError::Read`] when the history cannot be read.
    pub(crate) fn open(
        reader: &Reader,
        filter: LogFilter,
        steps: StepView,
        limit: u32,
    ) -> Result<(Self, String), LogError> {
        let row_cursor = reader.log_cursor()?;
        let rows = reader.log_rows(&filter, LogWindow::Newest(limit))?;
        let mut feed = Self {
            filter,
            steps,
            row_cursor,
            step_cursors: BTreeMap::new(),
            drained: BTreeSet::new(),
            told: BTreeSet::new(),
            runs: BTreeMap::new(),
            agents: Vec::new(),
        };
        feed.row_cursor = rows.iter().map(LogEntry::id).fold(row_cursor, i64::max);
        let covered = feed.covered(reader)?;
        let timed = feed.new_steps(reader, &covered)?;
        let timeline = feed.timeline(reader, &rows, timed)?;
        let text = timeline.newest(limit);
        feed.told = timeline.newest_runs(limit);
        Ok((feed, text))
    }

    /// The text of what was recorded since the last page; `waits` tells what a round waits for after a run of it ended.
    ///
    /// # Errors
    /// [`LogError::Read`] when the history cannot be read.
    pub(crate) fn poll(
        &mut self,
        reader: &Reader,
        waits: &mut dyn FnMut(&RoundId) -> String,
    ) -> Result<String, LogError> {
        let rows = reader.log_rows(
            &self.filter,
            LogWindow::After {
                id: self.row_cursor,
                limit: PAGE_ROWS,
            },
        )?;
        self.row_cursor = rows.iter().map(LogEntry::id).fold(self.row_cursor, i64::max);
        let covered = self.covered(reader)?;
        let timed = self.new_steps(reader, &covered)?;
        let mut asked: BTreeMap<RoundId, String> = BTreeMap::new();
        for row in rows.iter().filter(|row| row.event() == LogEvent::RunEnded) {
            for round in row.rounds() {
                asked.entry(round.clone()).or_insert_with(|| waits(round));
            }
        }
        let mut timeline = self.timeline(reader, &rows, timed)?;
        timeline.waits = asked;
        let mut told = std::mem::take(&mut self.told);
        let text = timeline.page(&mut told);
        self.told = told;
        Ok(text)
    }

    /// The runs the follow covers, noted as known; none when it prints no steps.
    fn covered(&mut self, reader: &Reader) -> Result<Vec<RunRow>, LogError> {
        if self.steps == StepView::LeftOut {
            return Ok(Vec::new());
        }
        let covered = covered_runs(reader, &self.filter)?;
        for row in &covered {
            self.runs.insert(row.start.value.run.clone(), row.start.value.clone());
        }
        let mut names: Vec<AgentName> = self.agents.clone();
        names.extend(self.filter.agent.clone());
        for row in &covered {
            let name = &row.start.value.agent.name;
            match name.parse() {
                Ok(agent) => names.push(agent),
                Err(source) => {
                    // The history validated the name of a run when it wrote it, so a name that does not parse cannot
                    // happen; its steps print without the name.
                    tracing::warn!(%source, %name, "a run of the scope has no agent name a log can show");
                }
            }
        }
        names.sort();
        names.dedup();
        self.agents = names;
        Ok(covered)
    }

    /// The steps of the runs in `covered` that were not read yet, oldest first.
    fn new_steps(&mut self, reader: &Reader, covered: &[RunRow]) -> Result<Vec<TimedStep>, LogError> {
        let mut timed = Vec::new();
        for row in covered {
            let start = &row.start.value;
            if self.drained.contains(&start.run) {
                continue;
            }
            let after = self.step_cursors.get(&start.run).copied().unwrap_or(0);
            for step in run_steps(reader, start)? {
                let number = step.step.number.get();
                if number > after {
                    self.step_cursors.insert(start.run.clone(), number);
                    timed.push(step);
                }
            }
            // The end was read before the steps, so a run that had ended has no step left that was not read.
            if row.end.is_some() {
                self.drained.insert(start.run.clone());
            }
        }
        timed.sort_by_key(|timed| (timed.at, timed.step.run.clone(), timed.step.number));
        Ok(timed)
    }

    /// The text of `rows` and of `steps`, with the start of the run each row refers to.
    fn timeline<'a>(
        &mut self,
        reader: &Reader,
        rows: &'a [LogEntry],
        steps: Vec<TimedStep>,
    ) -> Result<Timeline<'a>, LogError> {
        for row in rows {
            if let Some(run) = row.run()
                && !self.runs.contains_key(run)
                && let Some(found) = reader.run(run)?
            {
                self.runs.insert(run.clone(), found.start.value);
            }
        }
        let mut timeline = Timeline::new(rows.iter().collect(), steps, self.agents.clone());
        let mut teams: BTreeMap<_, BTreeMap<RunId, RunStart>> = BTreeMap::new();
        for (id, start) in &self.runs {
            teams
                .entry(start.team.clone())
                .or_default()
                .insert(id.clone(), start.clone());
        }
        timeline
            .teams
            .extend(teams.into_iter().map(|(team, runs)| Team::new(team, runs)));
        Ok(timeline)
    }
}
