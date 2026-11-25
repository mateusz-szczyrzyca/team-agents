//! `[commit_reminder]`: when a run that holds uncommitted work is reminded to commit it.

use serde::Deserialize;

/// The uncommitted lines at which a run is reminded, when `[commit_reminder]` says nothing. The measurements of
/// 24.11.2026 (two members that worked 40 minutes with 618 and 271 uncommitted lines) are the reason for it.
pub const DEFAULT_LINES: u32 = 300;

/// The minutes without a commit at which a run is reminded, when `[commit_reminder]` says nothing.
pub const DEFAULT_MINUTES: u32 = 30;

/// `[commit_reminder]`: the two bounds a run must reach, both, to be reminded to commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct CommitReminder {
    /// The uncommitted lines (added plus removed) at which a run is reminded; `0` turns the reminder off.
    pub lines: u32,
    /// The minutes since the later of the last commit and the start of the run at which a run is reminded; `0` turns the
    /// reminder off.
    pub minutes: u32,
}

impl Default for CommitReminder {
    fn default() -> Self {
        Self { lines: 0, minutes: 0 }
    }
}

#[cfg(test)]
mod tests;
