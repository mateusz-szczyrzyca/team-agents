//! The decision of when a run is reminded: the tally of one session.

#[cfg(test)]
mod tests;

/// The fewest searches of code without Serena that bring a reminder.
pub const DEFAULT_REMIND_AFTER: u64 = 40;

/// After how many searches of code without Serena a run is reminded; never zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemindAfter(u64);

impl RemindAfter {
    /// A count of `value` searches.
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

/// Why a count of searches is no point at which to remind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RemindAfterError {
    /// The count is zero.
    #[error("a run is not reminded after zero searches of code")]
    Zero,
}

/// What one session of a run did without Serena.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionTally {
    /// The searches of code so far.
    searches: u64,
}

impl SessionTally {
    /// The tally of a session that has done nothing.
    #[must_use]
    pub const fn new() -> Self {
        Self { searches: 0 }
    }

    /// Notes the step `event` and returns the reminder it makes due.
    pub fn note(&mut self, _event: &crate::cli::ActivityEvent, _after: RemindAfter) -> Option<Reminder> {
        None
    }

    /// The searches of code the session made so far.
    #[must_use]
    pub const fn searches(&self) -> u64 {
        self.searches
    }
}

/// The reminder that is due.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reminder {
    /// The searches the session made when it was reminded.
    pub searches: u64,
}

impl Default for RemindAfter {
    /// The default of `[serena] remind_after`.
    fn default() -> Self {
        Self(DEFAULT_REMIND_AFTER)
    }
}
