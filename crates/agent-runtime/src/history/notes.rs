//! A store that notes what the database refused: when it can write again, an event says which record is missing and why.
//!
//! The history never blocks the work and counts what it loses ([`HistoryHealth`](agent_core::history::HistoryHealth)); that
//! count is a number, and a story read later needs to know where the gap is. A record the database refuses is noted
//! here, with the team and the rounds it belonged to, and the note is written with the next write that succeeds. It is a
//! best effort: a record whose run was never seen by this store has no place to be noted, a batch that fails is not
//! noted (the writer tries its records one by one), and a note that cannot be written waits, a bounded number of them.

use std::collections::{HashMap, VecDeque};

use agent_core::history::{EventRecord, HistoryEvent, LostKind, Record};
use agent_core::ids::{RoundId, RunId, TaskId, TeamId};
use agent_core::state::Timestamp;

use super::db::Store;
use super::error::{RecordKind, WriteError};
use super::redact::Redactor;
use crate::process::error_chain;

/// How many runs the store remembers the team and rounds of.
const MAX_SCOPES: usize = 4096;

/// How many notes wait for a write that succeeds; the oldest are dropped beyond it.
const MAX_PENDING: usize = 256;

/// Where the records of a run belong.
#[derive(Debug, Clone)]
struct Scope {
    /// The team of the run.
    team: TeamId,
    /// The task of the run.
    task: TaskId,
    /// The rounds the run works for, each once.
    rounds: Vec<RoundId>,
    /// When the run started.
    at: Timestamp,
    /// The order in which this run was learned, to break ties when two runs of a task share a start time.
    order: u64,
}

/// A store that notes what the store it wraps refused.
#[derive(Debug)]
pub struct Noting<S> {
    /// The store the records go to.
    inner: S,
    /// Configured credentials to redact from error details in loss notes.
    redactor: Redactor,
    /// The team and rounds of the runs this store has seen start, to place the records of a run.
    scopes: HashMap<RunId, Scope>,
    /// How many runs this store has learned, to order runs of the same task.
    learned: u64,
    /// The notes that wait for a write that succeeds, oldest first.
    pending: VecDeque<EventRecord>,
    /// How many loss notes were permanently refused after the loss they describe had already been counted.
    dropped_notes: u64,
}

impl<S: Store> Noting<S> {
    /// Wraps `inner`.
    #[must_use]
    pub fn new(inner: S) -> Self {
        Self::with_secrets(inner, agent_core::config::Secrets::default())
    }

    /// Wraps `inner` and redacts `secrets` from loss notes.
    #[must_use]
    pub fn with_secrets(inner: S, secrets: agent_core::config::Secrets) -> Self {
        Self {
            inner,
            redactor: Redactor::new(secrets),
            scopes: HashMap::new(),
            learned: 0,
            pending: VecDeque::new(),
            dropped_notes: 0,
        }
    }

    /// Remembers where the records of `run` belong.
    fn learn(&mut self, run: &agent_core::history::RunStart) {
        if self.scopes.len() >= MAX_SCOPES {
            tracing::warn!(
                dropped = self.scopes.len(),
                limit = MAX_SCOPES,
                "the history note writer reached its run-scope limit and cleared remembered run scopes"
            );
            self.scopes.clear();
        }
        self.learned = self.learned.saturating_add(1);
        let scope = Scope {
            team: run.team.clone(),
            task: run.task.clone(),
            rounds: once_each(&run.rounds),
            at: run.at,
            order: self.learned,
        };
        self.scopes.insert(run.run.clone(), scope);
    }

    /// The rounds of the latest run of `task` in `team` that this store has seen start.
    fn rounds_of_task(&self, team: &TeamId, task: &TaskId) -> Vec<RoundId> {
        self.scopes
            .values()
            .filter(|scope| &scope.team == team && &scope.task == task)
            .max_by_key(|scope| (scope.at, scope.order))
            .map(|scope| scope.rounds.clone())
            .unwrap_or_default()
    }

    /// Notes that `record` was refused because of `error`, when it can be placed.
    fn note(&mut self, record: &Record, error: &WriteError) {
        let Some((what, team, rounds, at)) = self.placed(record) else {
            return;
        };
        if self.pending.len() >= MAX_PENDING
            && let Some(dropped) = self.pending.pop_front()
        {
            tracing::warn!(team = %dropped.team, limit = MAX_PENDING, "the history note writer reached its pending-note limit and evicted the oldest loss note");
        }
        let event = HistoryEvent::Lost {
            task: None,
            what,
            cause: error.kind().lost_cause(),
            detail: self.redactor.text(&error_chain(error)),
        };
        self.pending.push_back(EventRecord {
            at,
            team,
            rounds,
            event,
        });
    }

    /// What `record` is, and the team, rounds and time it belongs to; `None` when that cannot be told.
    fn placed(&self, record: &Record) -> Option<(LostKind, TeamId, Vec<RoundId>, Timestamp)> {
        let of_run = |run: &RunId, at: Option<Timestamp>, kind: RecordKind| {
            self.scopes.get(run).map(|s| {
                (
                    LostKind::Record(kind),
                    s.team.clone(),
                    s.rounds.clone(),
                    at.unwrap_or(s.at),
                )
            })
        };
        match record {
            Record::Team(t) => Some((LostKind::Record(RecordKind::Team), t.team.clone(), Vec::new(), t.at)),
            Record::Round(r) => Some((
                LostKind::Record(RecordKind::Round),
                r.team.clone(),
                vec![r.round.clone()],
                r.at,
            )),
            Record::Run(r) => Some((
                LostKind::Record(RecordKind::Run),
                r.team.clone(),
                once_each(&r.rounds),
                r.at,
            )),
            Record::Launch(launch) => of_run(&launch.run, Some(launch.at), RecordKind::Launch),
            Record::Step(step) => of_run(&step.run, Some(step.at), RecordKind::Step),
            Record::RunEnd(end) => of_run(&end.run, Some(end.at), RecordKind::RunEnd),
            Record::Session(session) => of_run(&session.run, Some(session.at), RecordKind::Session),
            Record::Changes(changes) => of_run(&changes.run, None, RecordKind::Changes),
            // A snapshot is not in the ledger, so there is no note of it.
            Record::Snapshot(_) => None,
            // A note is not noted: that would never end.
            Record::Event(e) => match &e.event {
                HistoryEvent::Lost { .. } => None,
                HistoryEvent::Gate { task, .. } if e.rounds.is_empty() => Some((
                    LostKind::Event(e.event.kind()),
                    e.team.clone(),
                    self.rounds_of_task(&e.team, task),
                    e.at,
                )),
                HistoryEvent::Gate { .. }
                | HistoryEvent::Checked { .. }
                | HistoryEvent::PullOpened { .. }
                | HistoryEvent::PullPushed { .. }
                | HistoryEvent::PullReady { .. }
                | HistoryEvent::ReviewAsked { .. }
                | HistoryEvent::ReviewSubmitted { .. }
                | HistoryEvent::FindingExplained { .. }
                | HistoryEvent::FindingAccepted { .. }
                | HistoryEvent::FindingRejected { .. }
                | HistoryEvent::FindingRaised { .. }
                | HistoryEvent::SecondOpinionAsked { .. }
                | HistoryEvent::SecondOpinionEnded { .. }
                | HistoryEvent::FindingConfirmed { .. }
                | HistoryEvent::FindingNotConfirmed { .. }
                | HistoryEvent::FixPassStarted { .. }
                | HistoryEvent::PullApproved { .. }
                | HistoryEvent::PullMerged { .. }
                | HistoryEvent::PullHandedOver { .. }
                | HistoryEvent::PullClosed { .. }
                | HistoryEvent::QuestionAsked { .. }
                | HistoryEvent::QuestionAnswered { .. }
                | HistoryEvent::Controlled { .. }
                | HistoryEvent::Notified { .. }
                | HistoryEvent::RoundClosed { .. }
                | HistoryEvent::TeamEnded {} => Some((
                    LostKind::Event(e.event.kind()),
                    e.team.clone(),
                    once_each(&e.rounds),
                    e.at,
                )),
            },
        }
    }

    /// Writes each waiting note separately; a permanently refused note cannot hold back the notes behind it.
    fn write_notes(&mut self) {
        while let Some(note) = self.pending.pop_front() {
            let record = Record::Event(Box::new(note.clone()));
            match self.inner.write(std::slice::from_ref(&record)) {
                Ok(()) => {}
                Err(error) if matches!(&error, WriteError::Rule { .. }) => {
                    self.dropped_notes = self.dropped_notes.saturating_add(1);
                    tracing::warn!(
                        team = %note.team,
                        dropped_notes = self.dropped_notes,
                        error = %error_chain(&error),
                        "a loss note is refused permanently and dropped; the loss it describes was already counted"
                    );
                }
                Err(error) => {
                    self.pending.push_front(note);
                    // These notes describe records already counted as lost; a transient failure can wait for the next write.
                    tracing::warn!(waiting = self.pending.len(), error = %error_chain(&error), "the notes of the lost records of the history cannot be written yet");
                    break;
                }
            }
        }
    }
}

/// `rounds` with each round once, in order: a record that names a round twice is the one the database refuses.
fn once_each(rounds: &[RoundId]) -> Vec<RoundId> {
    let mut once: Vec<RoundId> = Vec::new();
    for round in rounds {
        if !once.contains(round) {
            once.push(round.clone());
        }
    }
    once
}

impl<S: Store> Store for Noting<S> {
    fn retain(&mut self, request: &super::Retention) -> Result<(), super::RetainError> {
        self.inner.retain(request)
    }

    fn write(&mut self, records: &[Record]) -> Result<(), WriteError> {
        for record in records {
            if let Record::Run(run) = record {
                self.learn(run);
            }
        }
        match self.inner.write(records) {
            Ok(()) => {
                self.write_notes();
                Ok(())
            }
            Err(error) => {
                // A batch that fails is tried again record by record by the writer: only a record that fails alone is noted.
                if let [record] = records {
                    self.note(record, &error);
                }
                Err(error)
            }
        }
    }

    fn write_logs(&mut self, records: &[agent_core::log::LogRecord]) -> Result<(), WriteError> {
        self.inner.write_logs(records)
    }
}
