//! The accounts whose provider refused a run, as `agent status` shows them until each resumes.

use agent_core::config::AccountName;
use agent_core::decide::limit::Limit;
use agent_core::local_time::Zone;
use agent_core::state::{AccountPause, Timestamp};

#[cfg(test)]
mod tests;

/// A pause further away than this shows its date: a time of day alone would not say which day it is.
const SAME_DAY: time::Duration = time::Duration::hours(24);

/// The lines that name each account paused at `now` with the time its pause ends in the local time `zone` gives and the
/// limit that caused it, in the order of `paused`; `None` when no account is paused.
#[must_use]
pub(crate) fn account_lines<'a>(
    paused: impl Iterator<Item = (&'a AccountName, &'a AccountPause)>,
    now: Timestamp,
    zone: Zone,
) -> Option<String> {
    let lines: String = paused
        .filter(|(_, pause)| now < pause.until)
        .map(|(account, pause)| {
            let until = if pause.until.as_utc() - now.as_utc() < SAME_DAY {
                zone.clock_minute(pause.until)
            } else {
                zone.stamp_minute(pause.until)
            };
            let reason = match pause.reason {
                Limit::Quota { .. } => "quota",
                Limit::RateLimit => "rate limit",
            };
            format!("account {account} paused until {until} ({reason})\n")
        })
        .collect();
    (!lines.is_empty()).then_some(lines)
}
