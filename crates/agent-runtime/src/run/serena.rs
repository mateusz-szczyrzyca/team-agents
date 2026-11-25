//! The count of one session's searches of code without Serena, as the run engine sees them.
//!
//! The engine gives every step of a run to a [`SessionWatch`]: it holds the session's tally (the rule of
//! [`agent_core::serena`]) and says `Due` once, at the step that brings the reminder. A watch of a session that goes on
//! after its reminder counts from there and says nothing more.

use std::path::Path;

use agent_core::cli::ActivityEvent;
use agent_core::history::{HistoryEvent, Record};
use agent_core::ids::{RunId, TaskId};
use agent_core::kinds::CliKind;
use agent_core::serena::{RemindAfter, SessionTally};

use crate::history::{ReadError, Reader};

#[cfg(test)]
#[path = "serena_tests.rs"]
mod tests;

/// A reminder that came due in a run that goes on: who is to be told, and what it counted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueReminder {
    /// The task whose run made the searches.
    pub task: TaskId,
    /// The run that goes on and is to be told.
    pub run: RunId,
    /// The model CLI of the run.
    pub cli: CliKind,
    /// The searches of code without Serena the session had made when the reminder came due.
    pub searches: u64,
}

/// What a [`SessionWatch`] says about one step of the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Watched {
    /// The step counted nothing for the reminder.
    Quiet,
    /// The step made the reminder of the session due: the session has made `searches` searches of code without it.
    Due {
        /// The searches of code without Serena when the step was given.
        searches: u64,
    },
}

/// The count of one run's session, and what it needs to say the reminder once.
#[derive(Debug, Clone)]
pub struct SessionWatch {
    /// The session's tally, when the run has Serena and the reminder is not off.
    tally: Option<SessionTally>,
    /// Whether the session was reminded already: a session is told once.
    reminded: bool,
}

impl SessionWatch {
    /// A watch of a session that has done nothing; `None` when the run has no Serena or the reminder is off, and no
    /// step is counted then.
    #[must_use]
    pub fn new(after: Option<RemindAfter>) -> Option<Self> {
        after.map(|_| Self::counting())
    }

    /// A watch of a session that has done nothing.
    const fn counting() -> Self {
        Self {
            tally: Some(SessionTally::new()),
            reminded: false,
        }
    }

    /// The watch of the session `session` of `task`, as the history at `path` holds it: the steps of every run of the
    /// session counted again in order, and the session taken as reminded when the history holds a reminder of one of its
    /// runs, whatever the bound was then. A resumed session of a new engine, after a restart of the daemon, goes on from
    /// it.
    ///
    /// # Errors
    /// [`ReadError`] when the history cannot be read: the caller decides what a session without its count does.
    pub fn from_history(path: &Path, task: &TaskId, session: &str) -> Result<Self, ReadError> {
        let reader = Reader::open(path)?;
        let mut watch = Self::counting();
        let mut teams = Vec::new();
        let mut runs = Vec::new();
        for row in reader.runs_of_task(task)? {
            let start = &row.start.value;
            let of_session = start.session.as_ref().is_some_and(|known| known.as_str() == session)
                || reader
                    .sessions(&start.run)?
                    .iter()
                    .any(|seen| seen.session.as_str() == session);
            if !of_session {
                continue;
            }
            for step in reader.steps(&start.run)? {
                watch.replay(&step.value.event);
            }
            if !teams.contains(&start.team) {
                teams.push(start.team.clone());
            }
            runs.push(start.run.clone());
        }
        for team in &teams {
            for stored in reader.team_records(team)? {
                if let Record::Event(event) = &stored.record
                    && let HistoryEvent::SerenaReminded { run, .. } = &event.event
                    && runs.contains(run)
                {
                    watch.told_before();
                }
            }
        }
        Ok(watch)
    }

    /// Counts the step `event` of a run that was carried out before. It brings no reminder: a session was reminded when
    /// the history holds the record of it, not when its count passes today's bound.
    fn replay(&mut self, event: &ActivityEvent) {
        if let Some(tally) = self.tally.as_mut() {
            tally.count(event);
        }
    }

    /// Takes the session as reminded: it counts on and is told no more.
    fn told_before(&mut self) {
        let searches = self.tally.map_or(0, |tally| tally.searches());
        *self = Self::after_reminder(searches);
    }

    /// The watch of a session that goes on after the reminder its `searches` searches brought: it counts on from
    /// there and is not reminded again. A session that goes on is a resumed one; a new session begins with
    /// [`SessionWatch::new`].
    #[must_use]
    pub fn after_reminder(searches: u64) -> Self {
        Self {
            tally: Some(SessionTally::counted_from(searches)),
            reminded: true,
        }
    }

    /// The watch of a session that goes on with the tally `tally` it had: a resumed session keeps the searches it made
    /// and is reminded only if it was not yet.
    #[must_use]
    pub fn resumed(tally: SessionTally) -> Self {
        Self {
            tally: Some(tally),
            reminded: tally.was_reminded(),
        }
    }

    /// The tally of the session as the watch has counted it so far, for a session that goes on in a later run.
    #[must_use]
    pub fn tally(&self) -> Option<SessionTally> {
        self.tally
    }

    /// Gives the step `event` to the count.
    pub fn note(&mut self, event: &ActivityEvent, after: RemindAfter) -> Watched {
        let Some(tally) = self.tally.as_mut() else {
            return Watched::Quiet;
        };
        let searches = match tally.note(event, after) {
            Some(reminder) if !self.reminded => {
                self.reminded = true;
                reminder.searches
            }
            Some(_) | None => return Watched::Quiet,
        };
        Watched::Due { searches }
    }
}
