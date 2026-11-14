//! A store that notes what the database refused: when it can write again, an event says which record is missing and why.
//!
//! The history never blocks the work and counts what it loses ([`HistoryHealth`](agent_core::history::HistoryHealth)); that
//! count is a number, and a story read later needs to know where the gap is. A record the database refuses is noted
//! here, with the team and the rounds it belonged to, and the note is written with the next write that succeeds. It is a
//! best effort: a record whose run was never seen by this store has no place to be noted, a batch that fails is not
//! noted (the writer tries its records one by one), and a note that cannot be written waits, a bounded number of them.

use std::collections::HashMap;

use agent_core::history::{EventRecord, HistoryEvent, Record};
use agent_core::ids::{RoundId, RunId, TeamId};
use agent_core::state::Timestamp;

use super::db::Store;
use super::error::{RecordKind, WriteError};
use super::handle::Cause;
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
    /// The rounds the run works for, each once.
    rounds: Vec<RoundId>,
    /// When the run started.
    at: Timestamp,
}

/// A store that notes what the store it wraps refused.
#[derive(Debug)]
pub struct Noting<S> {
    /// The store the records go to.
    inner: S,
    /// The team and rounds of the runs this store has seen start, to place the records of a run.
    scopes: HashMap<RunId, Scope>,
    /// The notes that wait for a write that succeeds.
    pending: Vec<EventRecord>,
}

impl<S: Store> Noting<S> {
    /// Wraps `inner`.
    #[must_use]
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            scopes: HashMap::new(),
            pending: Vec::new(),
        }
    }

    /// Remembers where the records of `run` belong.
    fn learn(&mut self, run: &agent_core::history::RunStart) {
        if self.scopes.len() >= MAX_SCOPES {
            self.scopes.clear();
        }
        self.scopes.insert(
            run.run.clone(),
            Scope {
                team: run.team.clone(),
                rounds: once_each(&run.rounds),
                at: run.at,
            },
        );
    }

    /// Notes that `record` was refused because of `error`, when it can be placed.
    fn note(&mut self, record: &Record, error: &WriteError) {
        let Some((what, team, rounds, at)) = self.placed(record) else {
            return;
        };
        if self.pending.len() >= MAX_PENDING {
            self.pending.remove(0);
        }
        self.pending.push(EventRecord {
            at,
            team,
            rounds,
            event: HistoryEvent::Lost {
                what,
                cause: Cause::Write(error.kind()).to_string(),
                detail: error_chain(error),
            },
        });
    }

    /// What `record` is, and the team, rounds and time it belongs to; `None` when that cannot be told.
    fn placed(&self, record: &Record) -> Option<(String, TeamId, Vec<RoundId>, Timestamp)> {
        let of_run = |run: &RunId, at: Option<Timestamp>, kind: RecordKind| {
            self.scopes.get(run).map(|scope| {
                (
                    kind.to_string(),
                    scope.team.clone(),
                    scope.rounds.clone(),
                    at.unwrap_or(scope.at),
                )
            })
        };
        match record {
            Record::Team(team) => Some((RecordKind::Team.to_string(), team.team.clone(), Vec::new(), team.at)),
            Record::Round(round) => Some((
                RecordKind::Round.to_string(),
                round.team.clone(),
                vec![round.round.clone()],
                round.at,
            )),
            Record::Run(run) => Some((
                RecordKind::Run.to_string(),
                run.team.clone(),
                once_each(&run.rounds),
                run.at,
            )),
            Record::Launch(launch) => of_run(&launch.run, Some(launch.at), RecordKind::Launch),
            Record::Step(step) => of_run(&step.run, Some(step.at), RecordKind::Step),
            Record::RunEnd(end) => of_run(&end.run, Some(end.at), RecordKind::RunEnd),
            Record::Session(session) => of_run(&session.run, Some(session.at), RecordKind::Session),
            Record::Changes(changes) => of_run(&changes.run, None, RecordKind::Changes),
            // A note is not noted: that would never end.
            Record::Event(event) => match event.event {
                HistoryEvent::Lost { .. } => None,
                _ => Some((
                    event.event.kind().to_string(),
                    event.team.clone(),
                    once_each(&event.rounds),
                    event.at,
                )),
            },
        }
    }

    /// Writes the notes that wait; the ones that cannot be written wait on.
    fn write_notes(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let notes: Vec<Record> = self
            .pending
            .iter()
            .map(|note| Record::Event(Box::new(note.clone())))
            .collect();
        match self.inner.write(&notes) {
            Ok(()) => self.pending.clear(),
            // The notes are about records already counted as lost: they wait for the next write, and the work goes on.
            Err(error) => {
                tracing::warn!(waiting = self.pending.len(), error = %error_chain(&error), "the notes of the lost records of the history cannot be written yet");
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
}
