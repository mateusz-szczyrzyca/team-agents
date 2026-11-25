//! `[commit_reminder]`: when a run that holds uncommitted work is reminded to commit it.

use serde::Deserialize;

use super::ConfigError;

/// The uncommitted lines at which a run is reminded, when `[commit_reminder]` says nothing. The measurements of
/// 24.11.2026 (two members that worked 40 minutes with 618 and 271 uncommitted lines) are the reason for it.
pub const DEFAULT_LINES: u32 = 300;

/// The minutes without a commit at which a run is reminded, when `[commit_reminder]` says nothing.
pub const DEFAULT_MINUTES: u32 = 30;

/// The percent of its context bound at which a session is reminded, when `[commit_reminder]` says nothing.
pub const DEFAULT_CONTEXT_PERCENT: u8 = 80;

/// The minutes a reminder to commit may be ignored before the run is cut, when `[commit_reminder]` says nothing.
pub const DEFAULT_CUT_AFTER: u32 = 0;

/// `[commit_reminder]`: the two bounds a run must reach, both, to be reminded to commit, and the share of the context
/// bound at which a session is reminded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct CommitReminder {
    /// The uncommitted lines (added plus removed) at which a run is reminded; `0` turns the reminder off.
    pub lines: u32,
    /// The minutes since the later of the last commit and the start of the run at which a run is reminded; `0` turns the
    /// reminder off.
    pub minutes: u32,
    /// The minutes after a reminder to commit at which a run that has not committed is cut and resumed with the order to
    /// commit; `0` turns the cut off.
    pub cut_after: u32,
    /// The percent of the session's context bound (`0..=100`) at which the session is reminded once to commit and to
    /// write down where it is; `0` turns the reminder off. A value above `100` is refused when the file is loaded.
    pub context_percent: u8,
}

impl Default for CommitReminder {
    fn default() -> Self {
        Self {
            lines: DEFAULT_LINES,
            minutes: DEFAULT_MINUTES,
            cut_after: DEFAULT_CUT_AFTER,
            context_percent: DEFAULT_CONTEXT_PERCENT,
        }
    }
}

/// The highest value `context_percent` may have: all of the bound.
const MAX_CONTEXT_PERCENT: u8 = 100;

impl CommitReminder {
    /// Checks the section: `context_percent` is at most 100.
    ///
    /// # Errors
    /// [`ConfigError::ContextPercentAbove`] when `context_percent` is above 100.
    pub(super) const fn check(&self) -> Result<(), ConfigError> {
        if self.context_percent > MAX_CONTEXT_PERCENT {
            return Err(ConfigError::ContextPercentAbove {
                percent: self.context_percent,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
