//! `agent log`: tell the story of a round or a team from the history database, which is read directly: no daemon is asked.

mod follow;
pub(crate) mod rows;
pub(crate) mod steps;

#[cfg(test)]
mod tests;
mod timeline;
mod views;
mod wait;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use agent_core::cli::RunUsage;
use agent_core::history::{Record, SequencedRecord};
use agent_core::ids::{RoundId, RunId, TeamId};
use agent_core::local_time::{TypedTimeError, Zone};
use agent_core::story::{Selection, StoryOptions, json_lines, render_with_usage};
use agent_runtime::history::{ReadError, Reader};

/// The exit status of `agent log` when the round or team asked for is not in the history.
pub(crate) const UNKNOWN_ID_EXIT: u8 = 3;

/// How the records are printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    /// The story, as text.
    Text(StoryOptions),
    /// The records, one JSON object a line.
    Json,
}

/// What `agent log` is asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LogRequest {
    /// What to tell.
    pub(crate) selection: Selection,
    /// How to print it.
    pub(crate) format: Format,
}

/// Why `agent log` failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum LogError {
    /// The timer for following could not be started.
    #[error("cannot start log following")]
    FollowRuntime(#[source] std::io::Error),

    /// Neither a round nor a team was named.
    #[error("name a round (--round round-...) or a team (--team team-...)")]
    NoSubject,
    /// Both a round and a team were named.
    #[error("name a round or a team, not both")]
    TwoSubjects,
    /// The history cannot be read.
    #[error("cannot read the history")]
    Read(#[from] ReadError),
    /// The history has no such round.
    #[error("the history has no round {round}")]
    UnknownRound {
        /// The round.
        round: RoundId,
    },
    /// The history has no such team.
    #[error("the history has no team {team}")]
    UnknownTeam {
        /// The team.
        team: TeamId,
    },
    /// A time given for `flag` cannot be read: it is RFC 3339 with its zone, or a date and a time of the zone printed.
    #[error("{flag} needs a time: RFC 3339 with its zone, or a date and a time such as 2026-11-21 17:29")]
    Time {
        /// The flag the time was given for.
        flag: &'static str,
        /// Why the text is not a time.
        #[source]
        source: TypedTimeError,
    },
    /// The story cannot be written.
    #[error("cannot write the story")]
    Output(#[source] std::io::Error),
}

impl LogError {
    /// The exit status of the command for this failure: its own for an ID the history does not know.
    pub(crate) const fn exit_code(&self) -> u8 {
        match self {
            Self::UnknownRound { .. } | Self::UnknownTeam { .. } => UNKNOWN_ID_EXIT,
            Self::NoSubject
            | Self::TwoSubjects
            | Self::Read(_)
            | Self::Output(_)
            | Self::FollowRuntime(_)
            | Self::Time { .. } => 1,
        }
    }
}

impl LogRequest {
    /// The request of a command line: one of `round` and `team`, and how to format it.
    ///
    /// # Errors
    /// [`LogError::NoSubject`] or [`LogError::TwoSubjects`] unless exactly one is given.
    pub(crate) fn new(round: Option<RoundId>, team: Option<TeamId>, format: Format) -> Result<Self, LogError> {
        let selection = match (round, team) {
            (Some(round), None) => Selection::Round(round),
            (None, Some(team)) => Selection::Team(team),
            (None, None) => return Err(LogError::NoSubject),
            (Some(_), Some(_)) => return Err(LogError::TwoSubjects),
        };
        Ok(Self { selection, format })
    }
}

/// Prints what `request` asks of the history in `history`, its times in the RFC 3339 form of the files.
///
/// # Errors
/// As [`log_in`].
#[cfg(test)]
pub(crate) fn log(history: &Path, request: &LogRequest, out: &mut impl Write) -> Result<(), LogError> {
    log_in(history, request, Zone::Utc, out)
}

/// Prints what `request` asks of the history in `history`, with every time in `zone`.
///
/// # Errors
/// [`LogError::Read`] when the history cannot be read, [`LogError::UnknownRound`] or [`LogError::UnknownTeam`] when it
/// has no record of what is asked for, [`LogError::Output`] when `out` refuses the text.
pub(crate) fn log_in(history: &Path, request: &LogRequest, zone: Zone, out: &mut impl Write) -> Result<(), LogError> {
    let reader = Reader::open(history)?;
    let records = match &request.selection {
        Selection::Round(round) => reader.round_records(round)?,
        Selection::Team(team) => reader.team_records(team)?,
    };
    if records.is_empty() {
        return Err(match &request.selection {
            Selection::Round(round) => LogError::UnknownRound { round: round.clone() },
            Selection::Team(team) => LogError::UnknownTeam { team: team.clone() },
        });
    }
    let text = match request.format {
        Format::Text(options) => {
            let usage = usage_of_ended(&reader, &records)?;
            render_with_usage(&records, &request.selection, options, zone, &usage)
        }
        Format::Json => json_lines(&records, &request.selection)
            .into_iter()
            .fold(String::new(), |mut text, line| {
                text.push_str(&line);
                text.push('\n');
                text
            }),
    };
    out.write_all(text.as_bytes()).map_err(LogError::Output)?;
    out.flush().map_err(LogError::Output)
}

/// The usage of every run of `records` that ended, read in one call. A history without usage has none.
///
/// # Errors
/// [`LogError::Read`] when the usage cannot be read.
fn usage_of_ended(reader: &Reader, records: &[SequencedRecord]) -> Result<BTreeMap<RunId, RunUsage>, LogError> {
    let mut runs: Vec<RunId> = Vec::new();
    for stored in records {
        if let Record::RunEnd(end) = &stored.record
            && !runs.contains(&end.run)
        {
            runs.push(end.run.clone());
        }
    }
    Ok(reader.run_usages(&runs)?)
}
