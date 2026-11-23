//! One text in the order things happened: rows and steps together, each once, and the newest lines of both.
//!
//! The rows are the daemon's, in the order they were written; rows and steps are merged by the time they were recorded,
//! a row first when the times are equal. A run is told once, before the first of its steps that the text shows, with its
//! task, its team and its round; that line takes no place of the newest lines asked for.

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use agent_core::cli::RunUsage;
use agent_core::history::RunStart;
use agent_core::ids::{AgentName, RoundId, RunId, TeamId};
use agent_core::local_time::Zone;
use agent_core::log::render::{record_line_in, steps::step_line_in};
use agent_core::log::{LogEntry, LogEvent};
use agent_core::story::usage_line;

use super::steps::TimedStep;

/// What one thing of the text is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A row of the scope.
    Row(usize),
    /// A step of the scope.
    Step(usize),
}

/// A team with its runs.
#[derive(Debug, Clone)]
pub(crate) struct Team {
    /// The team.
    pub(crate) team: TeamId,
    /// Its runs by ID, each with the rounds it works for.
    runs: BTreeMap<RunId, RunStart>,
}

impl Team {
    /// The team `team` with `runs`.
    pub(crate) fn new(team: TeamId, runs: BTreeMap<RunId, RunStart>) -> Self {
        Self { team, runs }
    }

    /// The start of `run`, when this team has it.
    fn run_of(&self, run: Option<&RunId>) -> Option<&RunStart> {
        run.and_then(|run| self.runs.get(run))
    }
}

/// The text of the rows of a scope and of the steps the scope read, in the order they happened.
#[derive(Debug)]
pub(crate) struct Timeline<'a> {
    /// The rows of the scope, oldest first.
    rows: Vec<&'a LogEntry>,
    /// The steps of the scope, oldest first.
    steps: Vec<TimedStep>,
    /// The agents whose runs the text covers: a line names its agent when there is more than one.
    followed: Vec<AgentName>,
    /// The teams the rows belong to, with their runs.
    pub(crate) teams: Vec<Team>,
    /// What each round waits for, told after the row of a run of it that ended.
    pub(crate) waits: BTreeMap<RoundId, String>,
    /// The usage of the runs that have a row, told after the row of the run's end.
    pub(crate) usage: BTreeMap<RunId, RunUsage>,
    /// The zone the times of the text are written in.
    zone: Zone,
}

impl<'a> Timeline<'a> {
    /// The text of `rows` and of `steps`, the steps of the agents `followed`, with the times in the RFC 3339 form.
    #[cfg(test)]
    pub(crate) fn new(rows: Vec<&'a LogEntry>, steps: Vec<TimedStep>, followed: Vec<AgentName>) -> Self {
        Self::new_in(rows, steps, followed, Zone::Utc)
    }

    /// The text of `rows` and of `steps` as `Timeline::new` makes it, with every time in `zone`.
    pub(crate) fn new_in(rows: Vec<&'a LogEntry>, steps: Vec<TimedStep>, followed: Vec<AgentName>, zone: Zone) -> Self {
        Self {
            rows,
            steps,
            followed,
            teams: Vec::new(),
            waits: BTreeMap::new(),
            usage: BTreeMap::new(),
            zone,
        }
    }

    /// The newest `limit` rows and steps of the text, oldest first, each once; a run is told before the first of its
    /// steps that is shown, and its line takes no place of the `limit`.
    pub(crate) fn newest(&self, limit: u32) -> String {
        self.render(&self.shown(limit), &mut BTreeSet::new())
    }

    /// The runs [`Self::newest`] with the same `limit` tells: a later page does not tell them again.
    pub(crate) fn newest_runs(&self, limit: u32) -> BTreeSet<String> {
        let mut told = BTreeSet::new();
        self.render(&self.shown(limit), &mut told);
        told
    }

    /// The runs whose end row the newest `limit` lines of the text show.
    pub(crate) fn newest_ended_runs(&self, limit: u32) -> BTreeSet<RunId> {
        self.shown(limit)
            .into_iter()
            .filter_map(|kind| match kind {
                Kind::Row(row) => self.rows.get(row).copied(),
                Kind::Step(_) => None,
            })
            .filter(|row| row.event() == LogEvent::RunEnded)
            .filter_map(|row| row.run().cloned())
            .collect()
    }

    /// Every row and step of the text, oldest first, as one page of a follow: a run in `told` is not told again, and
    /// the runs this page tells are added to it.
    pub(crate) fn page(&self, told: &mut BTreeSet<String>) -> String {
        self.render(&self.merged(), told)
    }

    /// The newest `limit` rows and steps, oldest first.
    fn shown(&self, limit: u32) -> Vec<Kind> {
        let order = self.merged();
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        order
            .get(order.len().saturating_sub(limit)..)
            .unwrap_or_default()
            .to_vec()
    }

    /// The lines of `kinds`, each step of a run that is not in `told` yet preceded by the line of its run.
    fn render(&self, kinds: &[Kind], told: &mut BTreeSet<String>) -> String {
        let mut text = String::new();
        for kind in kinds {
            if let Kind::Step(step) = kind
                && let Some(timed) = self.steps.get(*step)
                && told.insert(timed.run.run.to_string())
            {
                text.push_str(&run_start_line(&timed.run, self.zone));
                text.push('\n');
            }
            text.push_str(&self.line(*kind));
        }
        text
    }

    /// The rows and the steps in the order of their times; a row stands before a step of the same time. The rows keep
    /// the order they were written in, and so do the steps of a run.
    fn merged(&self) -> Vec<Kind> {
        let mut order: Vec<Kind> = Vec::with_capacity(self.rows.len().saturating_add(self.steps.len()));
        let (mut row, mut step) = (0, 0);
        while row < self.rows.len() || step < self.steps.len() {
            let row_at = self.rows.get(row).map(|row| row.at());
            let step_at = self.steps.get(step).map(|timed| timed.at);
            let row_first = match (row_at, step_at) {
                (Some(row), Some(step)) => row <= step,
                (Some(_), None) => true,
                (None, _) => false,
            };
            if row_first {
                order.push(Kind::Row(row));
                row += 1;
            } else {
                order.push(Kind::Step(step));
                step += 1;
            }
        }
        order
    }

    /// The row's line: what happened, with the task, the agent, the run and the fields.
    fn row_line(&self, row: &LogEntry) -> String {
        let run = row
            .team()
            .and_then(|team| self.teams.iter().find(|made| &made.team == team))
            .and_then(|team| team.run_of(row.run()));
        let mut line = format!("{}\n", record_line_in(row, run, &self.zone));
        if row.event() == LogEvent::RunEnded {
            if let Some(run) = row.run()
                && let Some(usage) = self.usage.get(run)
            {
                line.push_str(&usage_line(run, Some(usage)));
                line.push('\n');
            }
            for round in row.rounds() {
                if let Some(wait) = self.waits.get(round) {
                    line.push_str(wait);
                    line.push('\n');
                }
            }
        }
        line
    }

    /// The line of one thing of the text.
    fn line(&self, kind: Kind) -> String {
        match kind {
            Kind::Row(row) => match self.rows.get(row) {
                Some(row) => self.row_line(row),
                None => String::new(),
            },
            Kind::Step(step) => match self.steps.get(step) {
                Some(timed) => format!(
                    "{}\n",
                    step_line_in(&timed.step, Some(&timed.run), &self.followed, &self.zone)
                ),
                None => String::new(),
            },
        }
    }
}

/// The first line of a run: when it started, the agent, the task, the team and the round it works for.
pub(crate) fn run_start_line(run: &RunStart, zone: Zone) -> String {
    let rounds: Vec<&str> = run.rounds.iter().map(agent_core::ids::RoundId::as_str).collect();
    let mut line = format!(
        "{} run {} started by {} on task {} of team {} ({})",
        zone.stamp(run.at),
        run.run,
        run.agent.name,
        run.task,
        run.team,
        run.role
    );
    if !rounds.is_empty() {
        line.push_str(" in round ");
        line.push_str(&rounds.join(","));
    }
    line
}
