//! `agent log`: tell the story of a round or a team from the history database, which is read directly: no daemon is asked.

use std::io::Write;
use std::path::Path;

use agent_core::ids::{RoundId, TeamId};
use agent_core::story::{PromptView, Selection, StepView, StoryOptions, json_lines, render};
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
    /// The story cannot be written.
    #[error("cannot write the story")]
    Output(#[source] std::io::Error),
}

impl LogError {
    /// The exit status of the command for this failure: its own for an ID the history does not know.
    pub(crate) const fn exit_code(&self) -> u8 {
        match self {
            Self::UnknownRound { .. } | Self::UnknownTeam { .. } => UNKNOWN_ID_EXIT,
            Self::NoSubject | Self::TwoSubjects | Self::Read(_) | Self::Output(_) => 1,
        }
    }
}

impl LogRequest {
    /// The request of a command line: one of `round` and `team`, and the flags.
    ///
    /// # Errors
    /// [`LogError::NoSubject`] or [`LogError::TwoSubjects`] unless exactly one is given.
    pub(crate) fn new(
        round: Option<RoundId>,
        team: Option<TeamId>,
        prompts: bool,
        no_steps: bool,
        json: bool,
    ) -> Result<Self, LogError> {
        let selection = match (round, team) {
            (Some(round), None) => Selection::Round(round),
            (None, Some(team)) => Selection::Team(team),
            (None, None) => return Err(LogError::NoSubject),
            (Some(_), Some(_)) => return Err(LogError::TwoSubjects),
        };
        let options = StoryOptions {
            prompts: if prompts {
                PromptView::Whole
            } else {
                PromptView::Preview
            },
            steps: if no_steps { StepView::LeftOut } else { StepView::Listed },
        };
        Ok(Self {
            selection,
            format: if json { Format::Json } else { Format::Text(options) },
        })
    }
}

/// Prints what `request` asks of the history in `history`.
///
/// # Errors
/// [`LogError::Read`] when the history cannot be read, [`LogError::UnknownRound`] or [`LogError::UnknownTeam`] when it
/// has no record of what is asked for, [`LogError::Output`] when `out` refuses the text.
pub(crate) fn log(history: &Path, request: &LogRequest, out: &mut impl Write) -> Result<(), LogError> {
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
        Format::Text(options) => render(&records, &request.selection, options),
        Format::Json => json_lines(&records, &request.selection)
            .iter()
            .map(|line| format!("{line}\n"))
            .collect(),
    };
    out.write_all(text.as_bytes()).map_err(LogError::Output)?;
    out.flush().map_err(LogError::Output)
}
