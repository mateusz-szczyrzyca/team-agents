//! One text in the order things happened: rows and steps together, each once, and the newest lines of both.
//!
//! The rows are the daemon's, in the order they were written; a step is lined up against the row recorded before it, so
//! that the text keeps the order of the day. A run is told once, before the first of its steps that the text shows,
//! with its task, its team and its round.

use std::collections::{BTreeMap, BTreeSet};

use agent_core::history::RunStart;
use agent_core::ids::{AgentName, RunId, TeamId};
use agent_core::log::LogEntry;
use agent_core::log::render::{record_line, steps::step_line};
use agent_core::state::Timestamp;

use super::steps::TimedStep;

/// What one thing of the text is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A run, told by its first line.
    Run(usize),
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
}

impl<'a> Timeline<'a> {
    /// The text of `rows` and of `steps`, the steps of the agents `followed`.
    pub(crate) fn new(rows: Vec<&'a LogEntry>, steps: Vec<TimedStep>, followed: Vec<AgentName>) -> Self {
        Self {
            rows,
            steps,
            followed,
            teams: Vec::new(),
        }
    }

    /// The newest `limit` things of the text, oldest first: each once, and a run told before the first of its steps.
    pub(crate) fn newest(&self, limit: u32) -> String {
        let order = self.order();
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let shown = &order[order.len().saturating_sub(limit)..];
        let mut runs: BTreeMap<String, usize> = BTreeMap::new();
        for kind in shown {
            if let Kind::Step(step) = kind
                && let Some(timed) = self.steps.get(*step)
            {
                runs.entry(timed.run.run.to_string()).or_insert(*step);
            }
        }
        let mut text = String::new();
        for step in runs.values() {
            text.push_str(&self.run_line(*step));
        }
        for kind in shown {
            if !matches!(kind, Kind::Run(_)) {
                text.push_str(&self.line(*kind));
            }
        }
        text
    }

    /// Every thing of the text, in the order it happened: the steps before every row first, then each row with the
    /// steps recorded before the next row's time.
    fn order(&self) -> Vec<Kind> {
        let mut order: Vec<Kind> = Vec::new();
        let mut told: BTreeSet<String> = BTreeSet::new();
        let push = |order: &mut Vec<Kind>, told: &mut BTreeSet<String>, step: usize| {
            if let Some(timed) = self.steps.get(step)
                && told.insert(timed.run.run.to_string())
            {
                order.push(Kind::Run(step));
            }
            order.push(Kind::Step(step));
        };
        // The steps recorded before every row come first; the others stand after the row recorded before them.
        let mut opening: Vec<usize> = Vec::new();
        let mut rest: Vec<usize> = Vec::new();
        for step in 0..self.steps.len() {
            match self.steps.get(step) {
                Some(timed) if self.rows.first().is_none_or(|first| timed.at < first.at()) => opening.push(step),
                Some(_) => rest.push(step),
                None => {}
            }
        }
        for step in opening {
            push(&mut order, &mut told, step);
        }
        let mut steps = rest.into_iter().peekable();
        for row in 0..self.rows.len() {
            let Some(known) = self.rows.get(row) else {
                continue;
            };
            let limit = self
                .rows
                .get(row.saturating_add(1))
                .map_or_else(|| known.at(), |next| next.at());
            loop {
                let Some(step) = steps.peek().copied() else {
                    break;
                };
                let before = self.steps.get(step).is_some_and(|timed| timed.at < limit);
                if !before {
                    break;
                }
                steps.next();
                push(&mut order, &mut told, step);
            }
            order.push(Kind::Row(row));
        }
        for step in steps {
            push(&mut order, &mut told, step);
        }
        order
    }

    /// The row's line: what happened, with the task, the agent, the run and the fields.
    fn row_line(&self, row: &LogEntry) -> String {
        let run = row
            .team()
            .and_then(|team| self.teams.iter().find(|made| &made.team == team))
            .and_then(|team| team.run_of(row.run()));
        format!("{}\n", record_line(row, run))
    }

    /// A run's first line: when, the agent, the task, the team and the round it works for.
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
pub(crate) fn run_start_line(run: &RunStart, at: Timestamp) -> String {
    let rounds: Vec<&str> = run.rounds.iter().map(agent_core::ids::RoundId::as_str).collect();
    let mut line = format!(
        "{at} run {} started by {} on task {} of team {} ({})",
        run.run, run.agent.name, run.task, run.team, run.role
    );
    if !rounds.is_empty() {
        line.push_str(" in round ");
        line.push_str(&rounds.join(","));
    }
    line
}
