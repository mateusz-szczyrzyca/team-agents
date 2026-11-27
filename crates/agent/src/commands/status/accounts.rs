//! The accounts whose provider refused a run, as `agent status` shows them until each resumes.

use agent_core::config::AccountName;
use agent_core::local_time::Zone;
use agent_core::state::{AccountPause, Timestamp};

#[cfg(test)]
mod tests;

/// The lines that name each account paused at `now` with the time its pause ends in the local time `zone` gives and the
/// limit that caused it, in the order of `paused`; `None` when no account is paused.
#[must_use]
pub(crate) fn account_lines<'a>(
    paused: impl Iterator<Item = (&'a AccountName, &'a AccountPause)>,
    now: Timestamp,
    zone: Zone,
) -> Option<String> {
    let _ = (paused, now, zone);
    None
}
