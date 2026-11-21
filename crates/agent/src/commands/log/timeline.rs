//! One text in the order things happened: rows and steps together, each once, and the newest lines of both.
//!
//! A scope tells its daemon rows and the steps of the runs it covers as one story: a step is lined up against the row
//! recorded before it, so the text keeps the order of the day. A run is told once, before the first step of it that the
//! text shows, so that each run's first line names its task, its team and its round; then every row and step in the
//! order they happened, each once, and the newest lines of both at the end.

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use agent_core::ids::AgentName;
use agent_core::log::LogEntry;
use agent_core::log::render::{record_line, steps::step_line};
use agent_core::state::Timestamp;

use super::steps::{Place, TimedStep, places};

/// What one thing of the text is, for the newest line and for following.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A run's first line, with which its steps are introduced.
    Run(usize),
    /// A row of the scope.
    Row(usize),
    /// A step of the scope.
    Step(usize),
}

/// The text of the rows of a scope and of the steps of its runs, in the order they happened.
#[derive(Debug)]
pub(crate) struct Timeline<'a> {
    /// The rows of the scope, oldest first.
    rows: Vec<&'a LogEntry>,
    /// The steps of the runs of the scope, oldest first.
    steps: Vec<&'a TimedStep>,
    /// Where each step stands against the rows.
    places: Vec<Place>,
    /// The agents whose runs the text covers: a line names its agent when there is more than one.
    followed: Vec<AgentName>,
    /// Every thing of the text, in the order it happened.
    order: Vec<Kind>,
}

impl<'a> Timeline<'a> {
    /// The text of `rows` and of `steps`, the steps of the agents `followed`.
    pub(crate) fn new(rows: Vec<&'a LogEntry>, steps: Vec<&'a TimedStep>, followed: Vec<AgentName>) -> Self {
        let places = places(&rows, &steps);
        let order = order_of(&rows, &steps, &places);
        Self {
            rows,
            steps,
            places,
            followed,
            order,
        }
    }

    /// The newest `limit` things of the text, oldest first: each once, with the run of every step it shows told
    /// before the first of its steps.
    pub(crate) fn newest(&self, limit: u32) -> String {
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let shown = self
            .order
            .iter()
            .skip(self.order.len().saturating_sub(limit))
            .collect::<Vec<_>>();
        // A run whose steps are shown is told first, with the first of them the text shows; it is not shown again.
        let mut runs: BTreeSet<usize> = BTreeSet::new();
        for kind in &shown {
            if let Kind::Run(step) = kind {
                runs.insert(*step);
            }
        }
        let mut text = String::new();
        for step in runs {
            text.push_str(&self.run_line(step));
        }
        for kind in shown {
            if !matches!(kind, Kind::Run(_)) {
                text.push_str(&self.line(*kind));
            }
        }
        text
    }

    /// The row's line: what happened, with the task, the agent, the run and the fields.
    fn row_line(&self, row: &LogEntry) -> String {
        let run = self
            .steps
            .iter()
            .find(|timed| Some(&timed.step.run) == row.run())
            .map(|timed| &timed.run);
        format!("{}\n", record_line(row, run))
    }

    /// A run's first line: the agent, the task, the team and the round it works for.
    fn run_line(&self, step: usize) -> String {
        match self.steps.get(step) {
            Some(timed) => format!("{}\n", run_start_line(&timed.run, timed.at)),
            None => String::new(),
        }
    }

    /// The line of one thing of the text.
    fn line(&self, kind: Kind) -> String {
        match kind {
            Kind::Run(step) => self.run_line(step),
            Kind::Row(row) => match self.rows.get(row) {
                Some(row) => self.row_line(row),
                None => String::new(),
            },
            Kind::Step(step) => match self.steps.get(step) {
                Some(timed) => format!("{}\n", step_line(&timed.step, Some(&timed.run), &self.followed)),
                None => String::new(),
            },
        }
    }
}

/// The first line of a run: when it started, the agent, the task, the team and the round it works for.
fn run_start_line(run: &agent_core::history::RunStart, at: Timestamp) -> String {
    let rounds: Vec<&str> = run.rounds.iter().map(|round| round.as_str()).collect();
    let mut line = format!("{at} run {} started by {}", run.run, run.agent.name);
    line.push_str(&format!(" on task {} of team {} ({})", run.task, run.team, run.role));
    if !rounds.is_empty() {
        line.push_str(" in round ");
        line.push_str(&rounds.join(","));
    }
    line
}

/// Every thing of the text, in the order it happened: a run before its first step, then each row with the steps that
/// follow it, and the steps before every row first.
fn order_of(rows: &[&LogEntry], steps: &[&TimedStep], places: &[Place]) -> Vec<Kind> {
    let index_of: BTreeMap<i64, usize> = rows.iter().enumerate().map(|(index, row)| (row.id(), index)).collect();
    let mut after: Vec<Vec<usize>> = vec![Vec::new(); rows.len()];
    let mut before: Vec<usize> = Vec::new();
    for (index, place) in places.iter().enumerate() {
        match place {
            Place::First => before.push(index),
            Place::After(id) => match index_of.get(id).and_then(|at| after.get_mut(*at)) {
                Some(list) => list.push(index),
                None => before.push(index),
            },
        }
    }
    let mut order: Vec<Kind> = Vec::new();
    let mut told: BTreeSet<String> = BTreeSet::new();
    let mut push_steps = |order: &mut Vec<Kind>, told: &mut BTreeSet<String>, step: usize| {
        if let Some(timed) = steps.get(step)
            && told.insert(timed.run.run.to_string())
        {
            order.push(Kind::Run(step));
        }
        order.push(Kind::Step(step));
    };
    for step in before {
        push_steps(&mut order, &mut told, step);
    }
    for (row, following) in after.iter().enumerate() {
        order.push(Kind::Row(row));
        for step in following {
            push_steps(&mut order, &mut told, *step);
        }
    }
    order
}
