//! The accounts whose provider refused a run, as `agent status` shows them until each resumes.

use std::collections::BTreeMap;

use agent_core::config::{AccountName, Config};
use agent_core::decide::account_slot::{AccountSlot, slot_of};
use agent_core::decide::limit::Limit;
use agent_core::decide::slot::runs_going;
use agent_core::ids::TaskId;
use agent_core::limit_reading::LimitWindow;
use agent_core::local_time::Zone;
use agent_core::protocol::TeamSummary;
use agent_core::state::State;
use agent_core::state::{AccountPause, KeptReading, LimitReadings, Timestamp};
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
pub(crate) fn reading_lines(readings: &LimitReadings, now: Timestamp, zone: Zone) -> Option<String> {
    let lines: Vec<String> = readings
        .live(now)
        .into_iter()
        .map(|(owner, kept)| {
            let windows: Vec<String> = kept.iter().map(|kept| window_text(kept, now, zone)).collect();
            format!("{owner}: {}\n", windows.join(", "))
        })
        .collect();
    (!lines.is_empty()).then(|| lines.concat())
}

/// One window of a reading line: `5h 62% (resets 15:30, read 3 min ago)`. The share is rounded to a whole percent.
fn window_text(kept: &KeptReading, now: Timestamp, zone: Zone) -> String {
    let reading = kept.reading();
    let label = match reading.window() {
        LimitWindow::FiveHour => "5h",
        LimitWindow::SevenDay => "7d",
    };
    let resets = reading.resets_at();
    let resets = if resets.as_utc() - now.as_utc() < SAME_DAY {
        zone.clock_minute(resets)
    } else {
        zone.stamp_minute(resets)
    };
    let used = reading.used() * 100.0;
    format!(
        "{label} {used:.0}% (resets {resets}, read {})",
        age_text(kept.read_at(), now)
    )
}

/// How long before `now` a reading was read: `just now` under a minute — and for a clock that went back —, then whole
/// minutes, hours and days.
fn age_text(read_at: Timestamp, now: Timestamp) -> String {
    let age = now.as_utc() - read_at.as_utc();
    if age < time::Duration::MINUTE {
        "just now".to_owned()
    } else if age < time::Duration::HOUR {
        format!("{} min ago", age.whole_minutes())
    } else if age < SAME_DAY {
        format!("{} h ago", age.whole_hours())
    } else {
        format!("{} d ago", age.whole_days())
    }
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
