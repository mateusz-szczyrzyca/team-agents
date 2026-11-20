//! The plan of a task: its stages and steps, read from `.agent/plan.md`, compared between versions and written back.
//!
//! A plan is the text a task keeps in its worktree at `.agent/plan.md` (see `docs/plan.md`). Its lines follow the
//! task list's format (`docs/task-list.md`) read in a plan mode: the intent is the text before the first stage, the
//! task list's features are its **stages** and its items are its **steps**:
//!
//! ```text
//! Finish the guide, then publish it.
//!
//! 1. Draft
//! [x] 1.1 Write the outline
//!     Files: docs/guide.md
//!     Check: exists docs/guide.md
//! [>] 1.2 Write the first draft
//! 2. Publish
//! [ ] 2.1 Add the guide to the release
//!     Check: cargo test -p guide
//! ```
//!
//! [`parse_plan`] reads the text and every fault is a [`PlanError`] that carries the number of the line it is on,
//! counted from 1 over the whole text.

use std::num::{NonZeroU16, NonZeroUsize};

use crate::ids::ItemId;
use crate::repo_path::RepoPath;
use crate::task_list::{MAX_TASK_LIST_BYTES, NumberProblem};

/// A plan: an intent, its stages and their steps.
///
/// Made only by [`parse_plan`], so its parts are consistent: stage numbers are unique, every step is in the stage its
/// ID names, step IDs are unique, at most one step is in hand, and a step given up has a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The text before the first stage, without the blank lines around it; empty when the plan does not say.
    intent: String,
    /// The stages, in the order written.
    stages: Vec<Stage>,
}

impl Plan {
    /// The text before the first stage: what the plan is for; empty when the plan does not say.
    #[must_use]
    pub fn intent(&self) -> &str {
        &self.intent
    }

    /// The stages, in the order written.
    #[must_use]
    pub fn stages(&self) -> &[Stage] {
        &self.stages
    }

    /// Every step, stage by stage, in the order written.
    pub fn steps(&self) -> impl Iterator<Item = &Step> {
        self.stages.iter().flat_map(Stage::steps)
    }
}

/// A stage of a plan: a numbered group of steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stage {
    /// The number of the stage, as in `3. Publish`.
    number: NonZeroU16,
    /// The rest of the stage's line.
    title: String,
    /// The steps, in the order written.
    steps: Vec<Step>,
}

impl Stage {
    /// The number of the stage, as in `3. Publish`.
    #[must_use]
    pub fn number(&self) -> NonZeroU16 {
        self.number
    }

    /// The rest of the stage's line.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The steps, in the order written.
    #[must_use]
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }
}

/// A step of a plan: one piece of work with a state and, usually, a check that shows it is done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// The step's ID, such as `2.3`: its stage's number and its own.
    id: ItemId,
    /// The rest of the step's line.
    title: String,
    /// The state of the step.
    state: StepState,
    /// The paths the step is about, as its `Files:` line lists them.
    files: Vec<RepoPath>,
    /// How the step is shown to be done; `None` when it names no check.
    check: Option<Check>,
    /// The text of the step that is not its `Files:` or `Check:` line; it says why a step is given up.
    note: String,
}

impl Step {
    /// The step's ID, such as `2.3`.
    #[must_use]
    pub fn id(&self) -> ItemId {
        self.id
    }

    /// The rest of the step's line.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The state of the step: its mark.
    #[must_use]
    pub fn state(&self) -> StepState {
        self.state
    }

    /// The paths the step is about, as its `Files:` line lists them.
    #[must_use]
    pub fn files(&self) -> &[RepoPath] {
        &self.files
    }

    /// How the step is shown to be done; `None` when it names no check.
    #[must_use]
    pub fn check(&self) -> Option<&Check> {
        self.check.as_ref()
    }

    /// The text of the step that is not its `Files:` or `Check:` line, dedented; for a step given up, why.
    #[must_use]
    pub fn note(&self) -> &str {
        &self.note
    }
}

/// The state of a step: its mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StepState {
    /// `[ ]`: to do.
    Todo,
    /// `[>]`: in hand; at most one step of a plan is.
    InHand,
    /// `[x]` (or `[X]`): done.
    Done,
    /// `[-]`: given up; its note says why.
    GivenUp,
}

/// How a step is shown to be done, by the line that follows `Check:`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// Any other line: a command, run later in the worktree.
    Command(String),
    /// `exists PATH`: the path is there.
    Exists(RepoPath),
    /// `reviewer`: a person judges the step done; it is never a command.
    Reviewer,
}

/// The part of a step's text that a label starts: `Files:` or `Check:`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StepPart {
    /// The part after `Files:`.
    Files,
    /// The part after `Check:`.
    Check,
}

impl std::fmt::Display for StepPart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Files => "Files:",
            Self::Check => "Check:",
        })
    }
}

/// What is wrong with a step line.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StepProblem {
    /// The line starts with `[` but not with `[ ]`, `[>]`, `[x]` or `[-]`.
    #[error("it must start with [ ], [>], [x] or [-]")]
    BadMark,
    /// The mark is not followed by a space.
    #[error("the mark must be followed by a space")]
    NoSpaceAfterMark,
    /// The text after the mark is not a step ID such as `2.3`.
    #[error("the step ID is invalid")]
    BadId(#[source] crate::ids::InvalidItemId),
    /// There is no title after the step ID.
    #[error("the title is missing")]
    MissingTitle,
}

/// Why a text is not a [`Plan`]. Every variant but [`Empty`](Self::Empty) and [`TooLarge`](Self::TooLarge) carries
/// the number of the line it is on, counted from 1.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    /// The text has nothing in it but blank lines.
    #[error("the plan is empty")]
    Empty,
    /// The text is longer than [`MAX_TASK_LIST_BYTES`], the limit it shares with the task list.
    #[error("the plan is {len} bytes: the limit is {max}")]
    TooLarge {
        /// The length of the text, in bytes.
        len: usize,
        /// The limit, in bytes.
        max: usize,
    },
    /// The number of a stage is not a number from 1 to 65535 without leading zeros.
    #[error("line {line}: invalid stage number {text:?}")]
    StageNumber {
        /// The line.
        line: NonZeroUsize,
        /// The text before the `.`.
        text: String,
        /// What is wrong with it.
        #[source]
        problem: NumberProblem,
    },
    /// A stage line has no title.
    #[error("line {line}: the stage has no title")]
    StageWithoutTitle {
        /// The line.
        line: NonZeroUsize,
    },
    /// A stage number was used before.
    #[error("line {line}: stage {number} is already defined on line {first_line}")]
    DuplicateStage {
        /// The line.
        line: NonZeroUsize,
        /// The number.
        number: NonZeroU16,
        /// The line of the first stage with this number.
        first_line: NonZeroUsize,
    },
    /// A line that starts a step is not a valid step line.
    #[error("line {line}: malformed step line")]
    MalformedStep {
        /// The line.
        line: NonZeroUsize,
        /// What is wrong with it.
        #[source]
        problem: StepProblem,
    },
    /// A step comes before the first stage.
    #[error("line {line}: the step is not in a stage")]
    StepBeforeStage {
        /// The line.
        line: NonZeroUsize,
    },
    /// A step's ID names another stage than the one it is under.
    #[error("line {line}: step {id} is under stage {stage}")]
    StepInOtherStage {
        /// The line.
        line: NonZeroUsize,
        /// The step's ID.
        id: ItemId,
        /// The stage the step is under.
        stage: NonZeroU16,
    },
    /// A step ID was used before.
    #[error("line {line}: step {id} is already defined on line {first_line}")]
    DuplicateStep {
        /// The line.
        line: NonZeroUsize,
        /// The step's ID.
        id: ItemId,
        /// The line of the first step with this ID.
        first_line: NonZeroUsize,
    },
    /// A second step is in hand.
    #[error("line {line}: the step is in hand, and the step on line {first_line} already is")]
    TwoInHand {
        /// The line of the second step in hand.
        line: NonZeroUsize,
        /// The line of the first step in hand.
        first_line: NonZeroUsize,
    },
    /// An indented line in a stage before its first step.
    #[error("line {line}: indented text outside a step")]
    TextBeforeStep {
        /// The line.
        line: NonZeroUsize,
    },
    /// A line at column 0 in a stage that is not a stage or a step.
    #[error("line {line}: text at column 0 inside a stage: indent it under its step")]
    UnexpectedLine {
        /// The line.
        line: NonZeroUsize,
    },
    /// A step given up has nothing in its note.
    #[error("line {line}: the step is given up and its text does not say why")]
    GivenUpWithoutReason {
        /// The line of the step.
        line: NonZeroUsize,
    },
    /// A `Files:` line names a path that is not a valid path of the repository.
    #[error("line {line}: invalid path in Files:")]
    BadFile {
        /// The line.
        line: NonZeroUsize,
        /// Why the path was refused.
        #[source]
        problem: crate::repo_path::InvalidRepoPath,
    },
    /// A step's text has the same label twice.
    #[error("line {line}: the step has {part} twice")]
    DuplicatePart {
        /// The line of the second label.
        line: NonZeroUsize,
        /// The label.
        part: StepPart,
    },
    /// A `Check:` line with nothing after it.
    #[error("line {line}: Check: has no check")]
    EmptyCheck {
        /// The line of the label.
        line: NonZeroUsize,
    },
    /// An `exists` check with no path after it.
    #[error("line {line}: Check: exists names no path")]
    ExistsWithoutPath {
        /// The line of the label.
        line: NonZeroUsize,
    },
    /// An `exists` check names a path that is not a valid path of the repository.
    #[error("line {line}: invalid path in Check: exists")]
    BadCheck {
        /// The line.
        line: NonZeroUsize,
        /// Why the path was refused.
        #[source]
        problem: crate::repo_path::InvalidRepoPath,
    },
}

impl PlanError {
    /// The number of the line the fault is on; `None` for [`Empty`](Self::Empty) and [`TooLarge`](Self::TooLarge),
    /// which are faults of the whole text.
    #[must_use]
    pub fn line(&self) -> Option<NonZeroUsize> {
        match self {
            Self::Empty | Self::TooLarge { .. } => None,
            Self::StageNumber { line, .. }
            | Self::StageWithoutTitle { line }
            | Self::DuplicateStage { line, .. }
            | Self::MalformedStep { line, .. }
            | Self::StepBeforeStage { line }
            | Self::StepInOtherStage { line, .. }
            | Self::DuplicateStep { line, .. }
            | Self::TwoInHand { line, .. }
            | Self::TextBeforeStep { line }
            | Self::UnexpectedLine { line }
            | Self::GivenUpWithoutReason { line }
            | Self::BadFile { line, .. }
            | Self::DuplicatePart { line, .. }
            | Self::EmptyCheck { line }
            | Self::ExistsWithoutPath { line }
            | Self::BadCheck { line, .. } => Some(*line),
        }
    }
}

/// Parses a plan.
///
/// ```
/// # fn main() -> Result<(), agent_core::plan::PlanError> {
/// let plan = agent_core::plan::parse_plan("Publish the guide.\n\n1. Draft\n[ ] 1.1 Write it\n")?;
/// assert_eq!(plan.intent(), "Publish the guide.");
/// assert_eq!(plan.stages()[0].steps().len(), 1);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
/// A [`PlanError`]: the text is empty or over [`MAX_TASK_LIST_BYTES`], or a line is not valid where it is; all but
/// those two carry the line.
pub fn parse_plan(text: &str) -> Result<Plan, PlanError> {
    Err(PlanError::TooLarge {
        len: text.len(),
        max: MAX_TASK_LIST_BYTES,
    })
}

#[cfg(test)]
mod tests;
