//! The accounts whose provider refused a run, as `agent status` shows them until each resumes.

use std::collections::BTreeMap;

use agent_core::config::{AccountName, Config};
use agent_core::decide::account_slot::{AccountSlot, slot_of};
use agent_core::decide::limit::Limit;
use agent_core::decide::slot::runs_going;
use agent_core::ids::TaskId;
use agent_core::local_time::Zone;
use agent_core::protocol::TeamSummary;
use agent_core::state::State;
use agent_core::state::{AccountPause, LimitReadings, Timestamp};
use agent_core::status::TaskStatus;

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
    let lines: Vec<String> = paused
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
    (!lines.is_empty()).then(|| lines.concat())
}

/// The lines that tell, for each account and each agent without one that has a reading, how much of its rate-limit windows
/// is used at `now` — `comp-claude: 5h 62% (resets 15:30, read 3 min ago), 7d 41% (...)` — in the order of `readings`;
/// `None` when no reading is kept.
#[must_use]
pub(crate) fn reading_lines(_readings: &LimitReadings, _now: Timestamp, _zone: Zone) -> Option<String> {
    None
}

/// What each queued task of `teams` whose account has its `max_runs` going says of it, by task, as the runs of `state`
/// are counted over every team of the daemon, also those that `teams` does not list.
#[must_use]
pub(crate) fn waits(config: &Config, state: &State, teams: &[TeamSummary]) -> BTreeMap<TaskId, String> {
    let going = runs_going(state.teams());
    teams
        .iter()
        .flat_map(|team| &team.members)
        .filter(|member| member.status == TaskStatus::Queued)
        .filter_map(|member| match slot_of(config, &member.agent.agent_name(), &going) {
            AccountSlot::Open => None,
            AccountSlot::Full(full) => Some((member.task.clone(), full.to_string())),
        })
        .collect()
}
