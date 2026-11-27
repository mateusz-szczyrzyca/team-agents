//! `agent checkpoints`: list the checkpoints of a round or a task from the history database, show one as the agent is
//! told it, or print them as JSON lines. The history is read directly, as `agent log` does: no daemon is asked.

use std::io::Write;
use std::num::NonZeroUsize;
use std::path::Path;
use std::str::FromStr;

use agent_core::ids::{RoundId, TaskId};
use agent_core::local_time::Zone;
use agent_runtime::harness_prompts::{HarnessPromptsError, RenderTextError};
use agent_runtime::history::ReadError;

#[cfg(test)]
mod tests;

/// The exit status of `agent checkpoints` when the round or the task asked for is not in the history, as `agent log`.
pub(crate) const UNKNOWN_ID_EXIT: u8 = 3;

/// What `agent checkpoints` is asked about: a round or a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    /// The checkpoints of every holder of a round.
    Round(RoundId),
    /// The checkpoints of one task, over all its rounds.
    Task(TaskId),
}

/// A word that names neither a round nor a task.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{text:?} is no round or task: write round-… or task-…")]
pub(crate) struct InvalidTarget {
    /// What was written.
    pub(crate) text: String,
}

impl FromStr for Target {
    type Err = InvalidTarget;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || InvalidTarget { text: text.to_owned() };
        if text.starts_with("round-") {
            text.parse().map(Self::Round).map_err(|_| invalid())
        } else {
            text.parse().map(Self::Task).map_err(|_| invalid())
        }
    }
}

/// What is printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// One line a checkpoint.
    Listing,
    /// One checkpoint, the `N`th (1 is the oldest) or the last, as the agent is told it.
    Show(Option<NonZeroUsize>),
    /// The records, one JSON object a line.
    Json,
}

/// What `agent checkpoints` is asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckpointsRequest {
    /// Whose checkpoints.
    pub(crate) target: Target,
    /// What to print of them.
    pub(crate) mode: Mode,
}

/// Why `agent checkpoints` failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CheckpointsError {
    /// The history cannot be read.
    #[error("cannot read the history")]
    Read(#[from] ReadError),
    /// The history has no such round.
    #[error("the history has no round {round}")]
    UnknownRound {
        /// The round.
        round: RoundId,
    },
    /// The history has no such task.
    #[error("the history has no task {task}")]
    UnknownTask {
        /// The task.
        task: TaskId,
    },
    /// The checkpoint asked for with `--show` is not there.
    #[error("there is no checkpoint {number}: there are {found}")]
    NoSuchCheckpoint {
        /// The number asked for.
        number: NonZeroUsize,
        /// How many there are.
        found: usize,
    },
    /// The texts of the harness cannot be loaded.
    #[error(transparent)]
    Texts(#[from] HarnessPromptsError),
    /// The text of a checkpoint cannot be rendered.
    #[error(transparent)]
    Render(#[from] RenderTextError),
    /// The answer cannot be written.
    #[error("cannot write the checkpoints")]
    Output(#[source] std::io::Error),
}

impl CheckpointsError {
    /// The exit status of the command for this failure: its own for an ID the history does not know.
    pub(crate) const fn exit_code(&self) -> u8 {
        match self {
            Self::UnknownRound { .. } | Self::UnknownTask { .. } => UNKNOWN_ID_EXIT,
            Self::Read(_) | Self::NoSuchCheckpoint { .. } | Self::Texts(_) | Self::Render(_) | Self::Output(_) => 1,
        }
    }
}

/// Prints what `request` asks of the history in `history`, its times in `zone`; `prompts` is the directory of the
/// harness's texts, which `--show` renders a checkpoint with.
///
/// # Errors
/// [`CheckpointsError`].
pub(crate) async fn checkpoints_in(
    history: &Path,
    prompts: &Path,
    request: &CheckpointsRequest,
    zone: Zone,
    out: &mut impl Write,
) -> Result<(), CheckpointsError> {
    let _ = (history, prompts, request, zone, out);
    Ok(())
}
