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

use std::collections::HashMap;
use std::num::{NonZeroU16, NonZeroUsize};

use crate::ids::ItemId;
use crate::repo_path::RepoPath;
use crate::task_list::{self, MAX_TASK_LIST_BYTES, NumberProblem};

pub mod judge;

/// Where a task keeps its plan, relative to the root of its worktree: the task's working file, never part of what it
/// delivers.
pub const PLAN_FILE: &str = ".agent/plan.md";

/// The indentation of the text lines of a step written back.
const TEXT_INDENT: &str = "    ";

/// The byte order mark some editors put at the start of a file; it is not part of the text.
const BYTE_ORDER_MARK: char = '\u{feff}';

/// The text that starts the line listing the paths a step is about.
const FILES_PREFIX: &str = "Files:";

/// The text that starts the line naming the check of a step.
const CHECK_PREFIX: &str = "Check:";

/// The check that says a person judges the step done; it is never a command.
const REVIEWER_CHECK: &str = "reviewer";

/// The check that says a path is there.
const EXISTS_CHECK: &str = "exists";

/// The step's title and whatever a step line carries besides its mark and ID.
#[derive(Debug)]
struct StepLine {
    /// The state of the step.
    state: StepState,
    /// The step's ID.
    id: ItemId,
    /// The rest of the line.
    title: String,
}

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

    /// What changed from this plan to `newer`.
    ///
    /// Steps are compared by ID only: a step whose ID both versions have is the same step, and a title it took on is
    /// reported as a [`Retitle`], not as a step added and another disappeared.
    #[must_use]
    pub fn compare(&self, newer: &Self) -> PlanChanges {
        let before: HashMap<ItemId, &Step> = self.steps().map(|step| (step.id, step)).collect();
        let after: HashMap<ItemId, &Step> = newer.steps().map(|step| (step.id, step)).collect();
        let mut changes = PlanChanges::default();
        for step in newer.steps() {
            let Some(old) = before.get(&step.id) else {
                changes.added.push(step.id);
                continue;
            };
            if old.state != step.state {
                changes.state_changed.push(StateChange {
                    id: step.id,
                    from: old.state,
                    to: step.state,
                });
            }
            if old.title != step.title {
                changes.retitled.push(Retitle {
                    id: step.id,
                    was: old.title.clone(),
                    now: step.title.clone(),
                });
            }
        }
        changes.disappeared = self
            .steps()
            .map(|step| step.id)
            .filter(|id| !after.contains_key(id))
            .collect();
        changes
    }

    /// Every step, stage by stage, in the order written.
    pub fn steps(&self) -> impl Iterator<Item = &Step> {
        self.stages.iter().flat_map(Stage::steps)
    }

    /// The plan as the text of `.agent/plan.md`; [`parse_plan`] reads it back into an equal plan.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut text = String::new();
        if !self.intent.is_empty() {
            text.push_str(&self.intent);
            // An intent alone ends the text without a line end, so that a text at the limit stays at it.
            if !self.stages.is_empty() {
                text.push_str("\n\n");
            }
        }
        for stage in &self.stages {
            text.push_str(&stage.to_text());
        }
        if text.is_empty() {
            // A plan without an intent and without a stage is read back as an empty text only: a blank line.
            text.push('\n');
        }
        text
    }
}

/// What changed from one version of a plan to the next.
///
/// The versions are compared step by step by ID, never by their text: a step that keeps its ID keeps its identity,
/// whatever its title says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanChanges {
    /// The steps the newer version has and the older one did not, in the newer version's order.
    pub added: Vec<ItemId>,
    /// The steps in both versions whose state is not the same, in the newer version's order.
    pub state_changed: Vec<StateChange>,
    /// The steps of the older version that the newer one no longer has, in the older version's order.
    pub disappeared: Vec<ItemId>,
    /// The steps in both versions whose ID the newer version uses for another title, in the newer version's order.
    pub retitled: Vec<Retitle>,
}

/// A step whose state changed between two versions of a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateChange {
    /// The step.
    pub id: ItemId,
    /// Its state in the older version.
    pub from: StepState,
    /// Its state in the newer version.
    pub to: StepState,
}

/// A step whose ID the newer version of a plan uses for another title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retitle {
    /// The step.
    pub id: ItemId,
    /// Its title in the older version.
    pub was: String,
    /// Its title in the newer version.
    pub now: String,
}

impl PlanChanges {
    /// Whether the two versions are the same in every way a comparison reports.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.state_changed.is_empty()
            && self.disappeared.is_empty()
            && self.retitled.is_empty()
    }

    /// Whether a step of the older version is missing from the newer one, or its ID now names another step.
    #[must_use]
    pub fn loses_a_step(&self) -> bool {
        !self.disappeared.is_empty() || !self.retitled.is_empty()
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

    /// The stage as the lines of a plan: its line, then its steps.
    fn to_text(&self) -> String {
        let mut text = format!("{}. {}\n", self.number, self.title);
        for step in &self.steps {
            text.push_str(&step.to_text());
        }
        text
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

    /// The step as the lines of a plan: its line, then its text indented by [`TEXT_INDENT`].
    fn to_text(&self) -> String {
        let mark = match self.state {
            StepState::Todo => "[ ]",
            StepState::InHand => "[>]",
            StepState::Done => "[x]",
            StepState::GivenUp => "[-]",
        };
        let mut text = format!("{mark} {} {}\n", self.id, self.title);
        for line in self.note.lines() {
            text.push_str(TEXT_INDENT);
            text.push_str(line);
            text.push('\n');
        }
        if let Some(files) = self.files_line() {
            text.push_str(&files);
        }
        if let Some(check) = self.check.as_ref().map(Check::to_text) {
            text.push_str(&check);
        }
        text
    }

    /// The step's `Files:` line, indented, with the line's end; `None` when the step is about no path.
    fn files_line(&self) -> Option<String> {
        if self.files.is_empty() {
            return None;
        }
        let paths: Vec<&str> = self.files.iter().map(RepoPath::as_str).collect();
        Some(format!("{TEXT_INDENT}{FILES_PREFIX} {}\n", paths.join(", ")))
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

impl StepState {
    /// The mark character representing this state: `' '` for `Todo`, `'>'` for `InHand`, `'x'` for `Done`, `'-'` for `GivenUp`.
    #[must_use]
    pub const fn mark(&self) -> char {
        match self {
            Self::Todo => ' ',
            Self::InHand => '>',
            Self::Done => 'x',
            Self::GivenUp => '-',
        }
    }
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

impl Check {
    /// The check as a `Check:` line, indented, with the line's end.
    fn to_text(&self) -> String {
        match self {
            Self::Command(command) => format!("{TEXT_INDENT}{CHECK_PREFIX} {command}\n"),
            Self::Exists(path) => format!("{TEXT_INDENT}{CHECK_PREFIX} {EXISTS_CHECK} {}\n", path.as_str()),
            Self::Reviewer => format!("{TEXT_INDENT}{CHECK_PREFIX} {REVIEWER_CHECK}\n"),
        }
    }
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

/// Why a text is not a [`Plan`]. Every variant but [`Empty`](Self::Empty), [`TooLarge`](Self::TooLarge) and
/// [`TooLargeWrittenBack`](Self::TooLargeWrittenBack) carries the number of the line it is on, counted from 1.
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
    /// The text is within the limit, but the plan written back by [`Plan::to_text`] is not: it widens every
    /// indentation of a step's text to four spaces. A plan that is read always writes back into a text that is read.
    #[error("the plan written back is {len} bytes: the limit is {max}")]
    TooLargeWrittenBack {
        /// The length of the plan written back, in bytes.
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
    /// The number of the line the fault is on; `None` for [`Empty`](Self::Empty), [`TooLarge`](Self::TooLarge) and
    /// [`TooLargeWrittenBack`](Self::TooLargeWrittenBack), which are faults of the whole text.
    #[must_use]
    pub fn line(&self) -> Option<NonZeroUsize> {
        match self {
            Self::Empty | Self::TooLarge { .. } | Self::TooLargeWrittenBack { .. } => None,
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
/// A [`PlanError`]: the text is empty, it or the plan written back ([`Plan::to_text`]) is over
/// [`MAX_TASK_LIST_BYTES`], or a line is not valid where it is; all but those three carry the line.
pub fn parse_plan(text: &str) -> Result<Plan, PlanError> {
    if text.len() > MAX_TASK_LIST_BYTES {
        return Err(PlanError::TooLarge {
            len: text.len(),
            max: MAX_TASK_LIST_BYTES,
        });
    }
    let text = text.strip_prefix(BYTE_ORDER_MARK).unwrap_or(text);
    let mut parser = Parser::default();
    for (index, line) in text.lines().enumerate() {
        parser.accept(NonZeroUsize::MIN.saturating_add(index), line)?;
    }
    let plan = parser.finish()?;
    // The text written back may be longer than the one read; a plan is read only if it can be read again.
    let len = plan.to_text().len();
    if len > MAX_TASK_LIST_BYTES {
        return Err(PlanError::TooLargeWrittenBack {
            len,
            max: MAX_TASK_LIST_BYTES,
        });
    }
    Ok(plan)
}

/// What a line of a plan is, by its first characters.
enum LineKind<'a> {
    /// Nothing but whitespace.
    Blank,
    /// Starts with a space or a tab.
    Indented,
    /// `N. Title`: the text before the `.` and the title after it.
    Stage {
        /// The text before the `.`, which is all ASCII digits.
        digits: &'a str,
        /// The text after the `.`, without the spaces around it.
        title: &'a str,
    },
    /// Starts with `[`: a step line, or something that was meant to be one.
    Step,
    /// Anything else.
    Text,
}

/// Tells what kind of line `line` is.
fn classify(line: &str) -> LineKind<'_> {
    if line.trim().is_empty() {
        return LineKind::Blank;
    }
    if line.starts_with([' ', '\t']) {
        return LineKind::Indented;
    }
    if line.starts_with('[') {
        return LineKind::Step;
    }
    match line.split_once('.') {
        Some((digits, rest))
            if !digits.is_empty()
                && digits.bytes().all(|b| b.is_ascii_digit())
                && (rest.is_empty() || rest.starts_with(char::is_whitespace)) =>
        {
            LineKind::Stage {
                digits,
                title: rest.trim(),
            }
        }
        _ => LineKind::Text,
    }
}

/// Whether `line` starts with the mark of a valid step line: `[ ]`, `[>]`, `[x]`, `[X]` or `[-]`.
fn has_step_mark(line: &str) -> bool {
    ["[ ]", "[>]", "[x]", "[X]", "[-]"]
        .iter()
        .any(|mark| line.starts_with(mark))
}

/// Parses a step line, which starts with `[`.
fn parse_step_line(line: &str) -> Result<StepLine, StepProblem> {
    let after_bracket = line.strip_prefix('[').ok_or(StepProblem::BadMark)?;
    let state = match after_bracket.chars().next() {
        Some(' ') => StepState::Todo,
        Some('>') => StepState::InHand,
        Some('x' | 'X') => StepState::Done,
        Some('-') => StepState::GivenUp,
        _ => return Err(StepProblem::BadMark),
    };
    // The mark's character is one byte, so `get(1..)` is the rest.
    let after_mark = after_bracket
        .get(1..)
        .and_then(|rest| rest.strip_prefix(']'))
        .ok_or(StepProblem::BadMark)?;
    if !after_mark.starts_with(char::is_whitespace) {
        return Err(StepProblem::NoSpaceAfterMark);
    }
    let after_mark = after_mark.trim();
    let (token, title) = after_mark
        .split_once(char::is_whitespace)
        .map_or((after_mark, ""), |(token, title)| (token, title.trim()));
    let id = token.parse::<ItemId>().map_err(StepProblem::BadId)?;
    if title.is_empty() {
        return Err(StepProblem::MissingTitle);
    }
    Ok(StepLine {
        state,
        id,
        title: title.to_owned(),
    })
}

/// A step whose line has been read and whose text is being collected.
struct OpenStep<'a> {
    /// The step line's state, ID and title.
    line: StepLine,
    /// The line of the step line.
    at: NonZeroUsize,
    /// The lines that follow it.
    body: Vec<(NonZeroUsize, &'a str)>,
}

/// The state of the parse: what has been read so far and what the next line may be.
#[derive(Default)]
struct Parser<'a> {
    /// The lines before the first stage.
    intent: Vec<&'a str>,
    /// The stages read so far; the last one is the current one.
    stages: Vec<Stage>,
    /// The lines of the stages' headings, by number, to refuse a repeated number.
    stage_lines: HashMap<NonZeroU16, NonZeroUsize>,
    /// The lines of the step lines, by ID, to refuse a repeated ID.
    step_lines: HashMap<ItemId, NonZeroUsize>,
    /// The line of the step in hand, for the second one in hand.
    in_hand: Option<NonZeroUsize>,
    /// The step whose text is being read.
    step: Option<OpenStep<'a>>,
}

impl<'a> Parser<'a> {
    /// Reads the line `text` that is line number `line` of the input.
    fn accept(&mut self, line: NonZeroUsize, text: &'a str) -> Result<(), PlanError> {
        match classify(text) {
            LineKind::Blank => self.blank(line, text),
            LineKind::Indented => self.indented(line, text)?,
            LineKind::Stage { digits, title } => self.stage(line, digits, title)?,
            LineKind::Step => self.step_line(line, text)?,
            LineKind::Text if self.stages.is_empty() => self.intent.push(text),
            LineKind::Text => {
                // The open step's text is above this line: its faults come first.
                self.close_step()?;
                return Err(PlanError::UnexpectedLine { line });
            }
        }
        Ok(())
    }

    /// A blank line: part of the open step's text or of the intent.
    fn blank(&mut self, line: NonZeroUsize, text: &'a str) {
        if let Some(open) = &mut self.step {
            open.body.push((line, text));
        } else if self.stages.is_empty() {
            self.intent.push(text);
        }
    }

    /// An indented line: part of the open step's text or of the intent.
    fn indented(&mut self, line: NonZeroUsize, text: &'a str) -> Result<(), PlanError> {
        if let Some(open) = &mut self.step {
            open.body.push((line, text));
        } else if self.stages.is_empty() {
            self.intent.push(text);
        } else {
            return Err(PlanError::TextBeforeStep { line });
        }
        Ok(())
    }

    /// A stage line with the `digits` of its number and its `title`: closes the open step and opens a stage.
    fn stage(&mut self, line: NonZeroUsize, digits: &str, title: &str) -> Result<(), PlanError> {
        self.close_step()?;
        let number = task_list::parse_number(digits).map_err(|problem| PlanError::StageNumber {
            line,
            text: digits.to_owned(),
            problem,
        })?;
        if title.is_empty() {
            return Err(PlanError::StageWithoutTitle { line });
        }
        if let Some(&first_line) = self.stage_lines.get(&number) {
            return Err(PlanError::DuplicateStage {
                line,
                number,
                first_line,
            });
        }
        self.stage_lines.insert(number, line);
        self.stages.push(Stage {
            number,
            title: title.to_owned(),
            steps: Vec::new(),
        });
        Ok(())
    }

    /// A line that starts with `[`: a step line in a stage, text in the intent unless it has a step's mark.
    fn step_line(&mut self, line: NonZeroUsize, text: &'a str) -> Result<(), PlanError> {
        let Some(stage) = self.stages.last() else {
            if has_step_mark(text) {
                return Err(PlanError::StepBeforeStage { line });
            }
            self.intent.push(text);
            return Ok(());
        };
        let stage = stage.number;
        self.close_step()?;
        let step = parse_step_line(text).map_err(|problem| PlanError::MalformedStep { line, problem })?;
        if step.id.feature() != stage {
            return Err(PlanError::StepInOtherStage {
                line,
                id: step.id,
                stage,
            });
        }
        if let Some(&first_line) = self.step_lines.get(&step.id) {
            return Err(PlanError::DuplicateStep {
                line,
                id: step.id,
                first_line,
            });
        }
        if step.state == StepState::InHand {
            if let Some(first_line) = self.in_hand {
                return Err(PlanError::TwoInHand { line, first_line });
            }
            self.in_hand = Some(line);
        }
        self.step_lines.insert(step.id, line);
        self.step = Some(OpenStep {
            line: step,
            at: line,
            body: Vec::new(),
        });
        Ok(())
    }

    /// Ends the open step, if any: reads its text and adds it to the current stage.
    fn close_step(&mut self) -> Result<(), PlanError> {
        let Some(OpenStep { line, at, body }) = self.step.take() else {
            return Ok(());
        };
        let labeled = read_labels(&body, narrowest_indentation(&body))?;
        let note = task_list::join_trimmed(&labeled.note);
        if line.state == StepState::GivenUp && note.is_empty() {
            return Err(PlanError::GivenUpWithoutReason { line: at });
        }
        // A step is opened only under a stage (`step_line`), so the current stage is there.
        if let Some(stage) = self.stages.last_mut() {
            stage.steps.push(Step {
                id: line.id,
                title: line.title,
                state: line.state,
                files: labeled.files,
                check: labeled.check,
                note,
            });
        }
        Ok(())
    }

    /// Ends the text: the plan, or [`PlanError::Empty`] when it has nothing in it.
    fn finish(mut self) -> Result<Plan, PlanError> {
        self.close_step()?;
        let intent = task_list::join_trimmed(&self.intent);
        if intent.is_empty() && self.stages.is_empty() {
            return Err(PlanError::Empty);
        }
        Ok(Plan {
            intent,
            stages: self.stages,
        })
    }
}

/// The smallest indentation of the text lines of a step, which their text is measured from; 0 when there are none.
fn narrowest_indentation(body: &[(NonZeroUsize, &str)]) -> usize {
    body.iter()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(_, line)| task_list::indentation(line))
        .min()
        .unwrap_or(0)
}

/// The `Files:` line of a step and its `Check:` line, read from its text lines.
struct Labeled<'a> {
    /// The paths of the `Files:` line.
    files: Vec<RepoPath>,
    /// The check of the `Check:` line.
    check: Option<Check>,
    /// The text lines that are neither of the two lines, dedented.
    note: Vec<&'a str>,
}

/// Reads a step's text lines, dedented by `narrowest`, into its `Files:` and `Check:` lines and the rest of its text.
fn read_labels<'a>(body: &[(NonZeroUsize, &'a str)], narrowest: usize) -> Result<Labeled<'a>, PlanError> {
    let mut files: Option<Vec<RepoPath>> = None;
    let mut check: Option<Check> = None;
    let mut note: Vec<&'a str> = Vec::new();
    for &(line, text) in body {
        let part = task_list::dedent(text, narrowest);
        if let Some(paths) = part.strip_prefix(FILES_PREFIX) {
            if files.is_some() {
                return Err(PlanError::DuplicatePart {
                    line,
                    part: StepPart::Files,
                });
            }
            files = Some(read_paths(line, paths)?);
        } else if let Some(rest) = part.strip_prefix(CHECK_PREFIX) {
            if check.is_some() {
                return Err(PlanError::DuplicatePart {
                    line,
                    part: StepPart::Check,
                });
            }
            check = Some(read_check(line, rest)?);
        } else {
            note.push(part);
        }
    }
    Ok(Labeled {
        files: files.unwrap_or_default(),
        check,
        note,
    })
}

/// The paths of a `Files:` line that is on `line`, separated by commas; an empty entry is an empty path, refused.
fn read_paths(line: NonZeroUsize, paths: &str) -> Result<Vec<RepoPath>, PlanError> {
    paths
        .split(',')
        .map(str::trim)
        .map(|path| path.parse().map_err(|problem| PlanError::BadFile { line, problem }))
        .collect()
}

/// The check of a `Check:` line that is on `line`, after `Check:`.
fn read_check(line: NonZeroUsize, rest: &str) -> Result<Check, PlanError> {
    let rest = rest.trim();
    if rest.is_empty() {
        return Err(PlanError::EmptyCheck { line });
    }
    if rest == REVIEWER_CHECK {
        return Ok(Check::Reviewer);
    }
    if rest == EXISTS_CHECK {
        return Err(PlanError::ExistsWithoutPath { line });
    }
    // `rest` is trimmed, so after `exists` and its whitespace there is a path.
    let Some(path) = rest
        .strip_prefix(EXISTS_CHECK)
        .filter(|path| path.starts_with(char::is_whitespace))
    else {
        return Ok(Check::Command(rest.to_owned()));
    };
    path.trim()
        .parse()
        .map(Check::Exists)
        .map_err(|problem| PlanError::BadCheck { line, problem })
}

#[cfg(test)]
mod tests;
