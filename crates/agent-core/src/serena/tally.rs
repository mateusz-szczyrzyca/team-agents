//! The decision of when a run is reminded: the tally of one session.
//!
//! The run engine notes every step of a session here and asks whether the one reminder of the session is due. The tally is
//! pure. A new session counts anew; one that goes on — the one of a resumed run — keeps what it counted, so a session is
//! reminded once and never again.

use crate::cli::ActivityEvent;

use super::code_search;

#[cfg(test)]
mod counting;
#[cfg(test)]
mod tests;

/// The fewest searches of code without Serena that bring a reminder: the default of `[serena] remind_after`. Forty reads
/// of code in one session are far more than a task needs once it has found its symbols, and few enough that a run of
/// hours is still reminded early.
pub const DEFAULT_REMIND_AFTER: u64 = 40;

/// After how many searches of code without Serena a run is reminded; never zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemindAfter(u64);

impl RemindAfter {
    /// A count of `value` searches; zero is refused, since no search reaches it.
    ///
    /// # Errors
    /// [`RemindAfterError::Zero`] when `value` is zero.
    pub fn new(value: u64) -> Result<Self, RemindAfterError> {
        if value == 0 {
            return Err(RemindAfterError::Zero);
        }
        Ok(Self(value))
    }

    /// The count.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl Default for RemindAfter {
    /// The default of `[serena] remind_after`.
    fn default() -> Self {
        Self(DEFAULT_REMIND_AFTER)
    }
}

/// Why a count of searches is no point at which to remind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RemindAfterError {
    /// The count is zero: no search reaches it.
    #[error("a run is not reminded after zero searches of code")]
    Zero,
}

/// What one session of a run did without Serena, as the run engine notes it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionTally {
    /// What its steps counted so far.
    counted: Counted,
    /// Whether this session was reminded already.
    reminded: bool,
}

impl SessionTally {
    /// The tally of a session that has made `searches` searches of code without Serena and was not reminded yet: what a
    /// session that goes on after its reminder has counted when the watch that tells the reminder is gone.
    #[must_use]
    pub const fn counted_from(searches: u64) -> Self {
        Self {
            counted: Counted {
                searches,
                used_serena: false,
            },
            reminded: false,
        }
    }

    /// The tally of a session that has done nothing: its count of searches is one, for the search that reaches a bound.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            counted: Counted {
                searches: 0,
                used_serena: false,
            },
            reminded: false,
        }
    }

    /// Notes the step `event` of the session and returns the reminder it makes due now, if any.
    #[must_use]
    pub fn note(&mut self, event: &ActivityEvent, after: RemindAfter) -> Option<Reminder> {
        self.counted.count(event);
        (self.is_due(after)).then_some(Reminder {
            searches: self.counted.searches,
        })
    }

    /// Whether the reminder of the session is due, and marks the session as reminded: it is told once.
    fn is_due(&mut self, after: RemindAfter) -> bool {
        if self.reminded || self.counted.used_serena {
            return false;
        }
        self.reminded = self.counted.searches >= after.get();
        self.reminded
    }

    /// The searches of code without Serena the session made so far.
    #[must_use]
    pub const fn searches(&self) -> u64 {
        self.counted.searches
    }

    /// Whether the session was reminded already.
    #[must_use]
    pub const fn was_reminded(&self) -> bool {
        self.reminded
    }
}

/// What a session counted by the time of one of its steps: the searches of code and whether Serena was called.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counted {
    /// The searches of code without Serena.
    pub searches: u64,
    /// Whether Serena was called at least once.
    pub used_serena: bool,
}

impl Counted {
    /// Counts the step `event`: one more search of code for a search of code, and Serena once it is called.
    pub fn count(&mut self, event: &ActivityEvent) {
        match event {
            ActivityEvent::ToolCall { name, .. } if is_serena(name) => self.used_serena = true,
            _ => {
                if code_search(event).is_some() {
                    self.searches = self.searches.saturating_add(1);
                }
            }
        }
    }
}

/// Whether the tool `name` is one of Serena's.
fn is_serena(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    ["mcp__serena__", "serena.", "serena_"]
        .iter()
        .filter_map(|prefix| lower.strip_prefix(prefix))
        .any(|tool| !tool.is_empty())
}

/// The reminder that is due: the count of searches that brought it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reminder {
    /// The searches of code without Serena the session made when it was reminded.
    pub searches: u64,
}
