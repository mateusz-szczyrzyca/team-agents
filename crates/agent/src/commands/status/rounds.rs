//! What `agent status` says of a round after its line: where the work of a round that goes on is, when the round started
//! and, once it is closed, how long it took.
//!
//! Where the work is comes from two places: the daemon shows the round (its item, its pull request, its reviews and the
//! runs of its reviewers), and the history says whether a draft pull request is open, which the state does not keep.

use std::collections::HashMap;

use agent_core::history::Record;
use agent_core::ids::RoundId;
use agent_core::protocol::TeamSummary;
use agent_core::round_work::{DraftPull, RoundWork, draft_pull, round_work};
use agent_core::state::{Round, RoundLife};
use agent_runtime::history::Reader;
use agent_runtime::process::error_chain;
use agent_runtime::store::Home;
use time::UtcOffset;

use super::{Activity, StatusError, round_detail};
use crate::client::Connection;

/// The seconds of a minute.
const SECONDS_PER_MINUTE: u64 = 60;
/// The minutes of an hour.
const MINUTES_PER_HOUR: u64 = 60;
/// The hours of a day.
const HOURS_PER_DAY: u64 = 24;
/// What the status says when the history cannot tell a draft from local work, before the cause.
const UNREAD: &str = "the history cannot say whether a draft pull request is open";

#[cfg(test)]
mod tests;

/// What the status says of the rounds of one answer of the daemon.
#[derive(Debug)]
pub(super) struct Told {
    /// What can advance each idle open round.
    waiting: HashMap<RoundId, String>,
    /// Where the work of each round that goes on is.
    works: HashMap<RoundId, RoundWork>,
    /// The findings waiting for independent investigators.
    opinions: HashMap<RoundId, String>,
    /// The machine's offset from UTC: the times are local.
    offset: UtcOffset,
    /// Why the history could not say whether a draft is open, when it could not.
    unread: Option<String>,
}

impl Told {
    /// What follows the line of `round`.
    pub(super) fn of(&self, round: &Round) -> String {
        let mut suffix = suffix(round, self.works.get(round.id()).copied(), self.offset);
        if let Some(waiting) = self.waiting.get(round.id()) {
            suffix.push_str(", waits for ");
            suffix.push_str(waiting);
        }
        match self.opinions.get(round.id()) {
            Some(opinions) => format!("{suffix}{opinions}"),
            None => suffix,
        }
    }

    /// The line that says why a draft cannot be told from local work, or nothing when the history was read.
    pub(super) fn unread(&self) -> String {
        self.unread.clone().unwrap_or_default()
    }
}

/// The history of a home, asked whether the rounds have a draft open.
#[derive(Debug)]
enum Drafts {
    /// The history was opened.
    Read(Reader),
    /// The history could not be opened: the cause, with its chain.
    Unread(String),
}

impl Drafts {
    /// The history of `home`.
    fn open(home: &Home) -> Self {
        match Reader::open(&home.history_file()) {
            Ok(reader) => Self::Read(reader),
            Err(error) => Self::Unread(error_chain(&error)),
        }
    }

    /// Whether `round` has a draft open, or why that cannot be told.
    fn of(&self, round: &RoundId) -> Result<DraftPull, String> {
        let reader = match self {
            Self::Read(reader) => reader,
            Self::Unread(cause) => return Err(cause.clone()),
        };
        let records = reader.round_records(round).map_err(|error| error_chain(&error))?;
        Ok(draft_pull(records.iter().filter_map(|record| match &record.record {
            Record::Event(event) => Some(&event.event),
            Record::Team(_)
            | Record::Round(_)
            | Record::Run(_)
            | Record::Launch(_)
            | Record::Step(_)
            | Record::RunEnd(_)
            | Record::Changes(_)
            | Record::Snapshot(_)
            | Record::Session(_) => None,
        })))
    }
}

/// Asks the daemon about every round of `teams` that goes on, and the history of the home of `activity` about their
/// drafts. A history that cannot be read stops nothing: the rounds whose draft it would tell say that they cannot tell,
/// and [`Told::unread`] says why.
///
/// # Errors
/// [`StatusError`] when the daemon cannot be asked or answers with something else than the round.
pub(super) async fn told(
    connection: &mut Connection,
    activity: &Activity,
    teams: &[TeamSummary],
) -> Result<Told, StatusError> {
    let mut told = Told {
        waiting: HashMap::new(),
        works: HashMap::new(),
        opinions: HashMap::new(),
        offset: activity.offset,
        unread: None,
    };
    let going = teams
        .iter()
        .flat_map(|team| &team.rounds)
        .filter(|round| *round.life() == RoundLife::Open);
    let mut drafts: Option<Drafts> = None;
    for round in going {
        let detail = round_detail(connection, round.id().clone()).await?;
        if let Some(waiting) = &detail.waiting {
            told.waiting.insert(round.id().clone(), waiting.clone());
        }
        let draft = drafts
            .get_or_insert_with(|| Drafts::open(&activity.home))
            .of(round.id())
            .unwrap_or_else(|cause| {
                told.unread.get_or_insert(format!("{UNREAD}: {cause}\n"));
                DraftPull::Unknown
            });
        if let Some(work) = round_work(&detail, draft) {
            told.works.insert(round.id().clone(), work);
            told.opinions
                .insert(round.id().clone(), opinion_waits(&detail, activity.offset));
        }
    }
    Ok(told)
}

/// What follows the line of `round`: where its work is, when it started in the local time `offset` gives and, for a
/// round that is closed, how long it took.
fn suffix(round: &Round, work: Option<RoundWork>, offset: UtcOffset) -> String {
    let work = work.map_or_else(String::new, |work| format!(", {work}"));
    let opened = round.opened_at().as_utc();
    let local = opened.to_offset(offset);
    let started = format!(
        "started {:02}-{:02}-{:04} {:02}:{:02}",
        local.day(),
        u8::from(local.month()),
        local.year(),
        local.hour(),
        local.minute()
    );
    let length = match round.life() {
        RoundLife::Open => String::new(),
        RoundLife::Closed { at, .. } => {
            // A round is never closed before it was opened: the state refuses it when it is read.
            let seconds = u64::try_from((at.as_utc() - opened).whole_seconds()).unwrap_or_default();
            format!(", took {}", took(seconds))
        }
    };
    format!("{work}  {started}{length}")
}

/// `seconds` as a length of time for a reader: days, hours and minutes, or seconds under a minute.
fn took(seconds: u64) -> String {
    let minutes = seconds / SECONDS_PER_MINUTE;
    let hours = minutes / MINUTES_PER_HOUR;
    let days = hours / HOURS_PER_DAY;
    let (hour, minute) = (hours % HOURS_PER_DAY, minutes % MINUTES_PER_HOUR);
    if days > 0 {
        format!("{days}d {hour:02}h {minute:02}m")
    } else if hours > 0 {
        format!("{hours}h {minute:02}m")
    } else if minutes > 0 {
        format!("{minutes}m")
    } else {
        format!("{seconds}s")
    }
}

/// Describes each unanswered opinion independently of the round's general work label.
fn opinion_waits(detail: &agent_core::protocol::RoundDetail, offset: UtcOffset) -> String {
    use agent_core::state::OpinionStage;
    let mut text = String::new();
    for (number, opinion) in &detail.opinions {
        let description = match &opinion.stage {
            OpinionStage::Waiting => format!("finding {number} waits for a free independent investigator"),
            OpinionStage::Assigned { investigator, .. } => {
                let since = detail
                    .opinion_attempts
                    .get(number)
                    .and_then(|attempts| attempts.last())
                    .and_then(|attempt| attempt.asked_at);
                let time = match since {
                    Some(at) => {
                        let at = at.as_utc().to_offset(offset);
                        format!(
                            "since {:02}-{:02}-{:04} {:02}:{:02}",
                            at.day(),
                            u8::from(at.month()),
                            at.year(),
                            at.hour(),
                            at.minute()
                        )
                    }
                    None => "(assignment time unavailable)".to_owned(),
                };
                format!("finding {number} waits for {investigator}’s second opinion {time}")
            }
            OpinionStage::Reported { .. } => continue,
        };
        text.push_str("; ");
        text.push_str(&description);
    }
    text
}
