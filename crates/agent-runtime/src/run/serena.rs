//! The count of one session's searches of code without Serena, as the run engine sees them.
//!
//! The engine gives every step of a run to a [`SessionWatch`]: it holds the session's tally (the rule of
//! [`agent_core::serena`]) and says `Due` once, at the step that brings the reminder. A watch of a session that goes on
//! after its reminder counts from there and says nothing more.

use agent_core::cli::ActivityEvent;
use agent_core::serena::{RemindAfter, SessionTally};

#[cfg(test)]
#[path = "serena_tests.rs"]
mod tests;

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
        after.map(|_| Self {
            tally: Some(SessionTally::new()),
            reminded: false,
        })
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
