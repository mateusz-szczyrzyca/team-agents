//! `agent checkpoints`: list the checkpoints of a round or a task from the history database, show one as the agent is
//! told it, or print them as JSON lines. The history is read directly, as `agent log` does: no daemon is asked.

use std::io::Write;
use std::num::NonZeroUsize;
use std::path::Path;
use std::str::FromStr;

use agent_core::config::Language;
use agent_core::history::{CheckpointCause, CheckpointContent, HistoryEvent, Record, SequencedRecord};
use agent_core::ids::{InvalidId, RoundId, TaskId};
use agent_core::prompt::{HarnessPrompt, Variables, VariablesError};
use agent_core::state::{CommitId, PrNumber, Timestamp};
use agent_core::story::{Selection, json_lines};
use agent_runtime::harness_prompts::{HarnessPrompts, HarnessPromptsError, RenderTextError};
use agent_runtime::history::{ReadError, Reader};

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

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Round(round) => write!(f, "{round}"),
            Self::Task(task) => write!(f, "{task}"),
        }
    }
}

/// A word that names neither a round nor a task.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{text:?} is no round or task: write round-… or task-…")]
pub(crate) struct InvalidTarget {
    /// What was written.
    pub(crate) text: String,
    /// Why the id was refused.
    #[source]
    pub(crate) source: InvalidId,
}

impl FromStr for Target {
    type Err = InvalidTarget;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = |source| InvalidTarget {
            text: text.to_owned(),
            source,
        };
        if text.starts_with("round-") {
            text.parse().map(Self::Round).map_err(invalid)
        } else {
            text.parse().map(Self::Task).map_err(invalid)
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
    /// A variable of the text of a checkpoint cannot be set.
    #[error(transparent)]
    Variable(#[from] VariablesError),
    /// The answer cannot be written.
    #[error("cannot write the checkpoints")]
    Output(#[source] std::io::Error),
}

impl CheckpointsError {
    /// The exit status of the command for this failure: its own for an ID the history does not know.
    pub(crate) const fn exit_code(&self) -> u8 {
        match self {
            Self::UnknownRound { .. } | Self::UnknownTask { .. } => UNKNOWN_ID_EXIT,
            Self::Read(_)
            | Self::NoSuchCheckpoint { .. }
            | Self::Texts(_)
            | Self::Render(_)
            | Self::Variable(_)
            | Self::Output(_) => 1,
        }
    }
}

/// What a checkpoint says that `agent checkpoints` prints.
struct Row {
    /// When it was taken.
    at: Timestamp,
    /// The step it follows.
    cause: CheckpointCause,
    /// The pull request of the round then, when there was one.
    pull_request: Option<PrNumber>,
    /// Whether that pull request was a draft.
    draft: bool,
    /// The tip of the holder's branch.
    head: Option<CommitId>,
    /// The review pass.
    review_pass: Option<u32>,
    /// The reference of the snapshot of the worktree.
    snapshot: Option<String>,
    /// What is meant, done and left.
    content: Option<CheckpointContent>,
}

impl Row {
    /// The checkpoint of `record` when it is one of `target`.
    fn of(record: &SequencedRecord, target: &Target) -> Option<Self> {
        let Record::Event(event) = &record.record else {
            return None;
        };
        let HistoryEvent::Checkpoint {
            round,
            task,
            cause,
            pull_request,
            head,
            draft,
            review_pass,
            snapshot,
            content,
            ..
        } = &event.event
        else {
            return None;
        };
        let wanted = match target {
            Target::Round(wanted) => wanted == round,
            Target::Task(wanted) => wanted == task,
        };
        wanted.then(|| Self {
            at: event.at,
            cause: *cause,
            pull_request: *pull_request,
            draft: *draft,
            head: head.clone(),
            review_pass: *review_pass,
            snapshot: snapshot.clone(),
            content: content.clone(),
        })
    }

    /// The line of the listing.
    fn line(&self) -> String {
        let pull = self.pull_request.map_or_else(
            || "none".to_owned(),
            |number| format!("#{} {}", number.get(), if self.draft { "draft" } else { "ready" }),
        );
        let (done, left) = match self.content.as_ref().and_then(CheckpointContent::counts) {
            Some(counts) => (
                format!("{} commits, {} steps", counts.commits, counts.steps_done),
                format!("{} steps, {} findings", counts.steps_left, counts.findings_open),
            ),
            None => ("n/a".to_owned(), "n/a".to_owned()),
        };
        format!(
            "{}  {}  PR {pull}  head {}  pass {}  done {done}  left {left}  snapshot {}",
            self.at,
            self.cause,
            self.head.as_ref().map_or_else(
                || "none".to_owned(),
                |head| head.as_str().chars().take(SHORT_HASH).collect()
            ),
            self.review_pass.unwrap_or_default(),
            self.snapshot.as_deref().unwrap_or("none"),
        )
    }

    /// The checkpoint as the agent is told it, rendered with `checkpoint-state.md`.
    fn shown(&self, texts: &HarnessPrompts) -> Result<String, CheckpointsError> {
        let kept = |part: fn(&CheckpointContent) -> &str| self.content.as_ref().map_or(NO_CONTENT, part).to_owned();
        let values = Variables::new()
            .with("meant", kept(|content| &content.meant))?
            .with("done", kept(|content| &content.done))?
            .with("left", kept(|content| &content.left))?
            .with("cause", self.cause.to_string())?
            .with("time", self.at.to_string())?
            .with(
                "head",
                self.head
                    .as_ref()
                    .map_or_else(|| "none".to_owned(), ToString::to_string),
            )?;
        Ok(texts.render(HarnessPrompt::CheckpointState, &values)?)
    }
}

/// What `--show` says of a checkpoint that kept no account of the work: one taken before checkpoints had a content.
const NO_CONTENT: &str = "(this checkpoint kept no account of the work)";

/// How many characters of a commit's hash the listing shows.
const SHORT_HASH: usize = 7;

/// The records of the history that belong to `target`, and the selection that tells them in JSON.
///
/// # Errors
/// [`CheckpointsError::UnknownRound`] or [`CheckpointsError::UnknownTask`] when the history has no record of the target;
/// [`CheckpointsError::Read`].
fn records_of(reader: &Reader, target: &Target) -> Result<(Vec<SequencedRecord>, Selection), CheckpointsError> {
    match target {
        Target::Round(round) => {
            let records = reader.round_records(round)?;
            if records.is_empty() {
                return Err(CheckpointsError::UnknownRound { round: round.clone() });
            }
            Ok((records, Selection::Round(round.clone())))
        }
        Target::Task(task) => {
            let runs = reader.runs_of_task(task)?;
            let Some(first) = runs.first() else {
                return Err(CheckpointsError::UnknownTask { task: task.clone() });
            };
            let team = first.start.value.team.clone();
            Ok((reader.team_records(&team)?, Selection::Team(team)))
        }
    }
}

/// Prints what `request` asks of the history in `history`, with its times as the files hold them; `prompts` is the
/// directory of the harness's texts, which `--show` renders a checkpoint with.
///
/// # Errors
/// [`CheckpointsError`].
pub(crate) async fn checkpoints_in(
    history: &Path,
    prompts: &Path,
    request: &CheckpointsRequest,
    out: &mut impl Write,
) -> Result<(), CheckpointsError> {
    let reader = Reader::open(history)?;
    let (records, selection) = records_of(&reader, &request.target)?;
    let taken: Vec<(SequencedRecord, Row)> = records
        .into_iter()
        .filter_map(|record| Row::of(&record, &request.target).map(|row| (record, row)))
        .collect();
    let text = match request.mode {
        Mode::Json => {
            let records: Vec<SequencedRecord> = taken.iter().map(|(record, _)| record.clone()).collect();
            json_lines(&records, &selection)
                .into_iter()
                .fold(String::new(), |mut text, line| {
                    text.push_str(&line);
                    text.push('\n');
                    text
                })
        }
        _ if taken.is_empty() => format!("No checkpoints for {}.\n", request.target),
        Mode::Listing => taken.iter().fold(String::new(), |mut text, (_, row)| {
            text.push_str(&row.line());
            text.push('\n');
            text
        }),
        Mode::Show(number) => {
            let index = number.map_or(taken.len(), NonZeroUsize::get);
            let Some((_, row)) = index.checked_sub(1).and_then(|index| taken.get(index)) else {
                // `number` is `Some` here: the last checkpoint always exists when the list is not empty.
                return Err(CheckpointsError::NoSuchCheckpoint {
                    number: number.unwrap_or(NonZeroUsize::MIN),
                    found: taken.len(),
                });
            };
            let texts = HarnessPrompts::load(prompts, &Language::default()).await?;
            row.shown(&texts)?
        }
    };
    out.write_all(text.as_bytes()).map_err(CheckpointsError::Output)?;
    out.flush().map_err(CheckpointsError::Output)
}
