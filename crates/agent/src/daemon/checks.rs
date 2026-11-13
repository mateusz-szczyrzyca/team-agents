//! The checks before a review, in the daemon: the reasons the team is given, the questions the human is asked and what
//! their answers mean.
//!
//! What the checks find is `agent_core::decide::checks`, what becomes of it is `agent_core::decide::round::Checking`; the
//! owner carries it out (`owner/rounds/checks.rs`). Nothing here reads a file or runs a process, but the texts, which come
//! from `prompts/harness/checks/`.

use std::num::NonZeroU32;

use agent_core::decide::checks::{Issue, Subject};
use agent_core::decide::round::AskReason;
use agent_core::ids::{TaskId, TeamId};
use agent_core::prompt::VariablesError;
use agent_core::questions::{ComposeError, EntryError, QuestionsFile};
use agent_core::state::PrNumber;
use agent_runtime::checks_prompts::{ChecksPrompts, RenderChecksTextError};

/// The word with which the human allows what a question names; any case.
pub(super) const ALLOW_WORD: &str = "allow";

/// The word with which the human refuses what a question names.
pub(super) const REFUSE_WORD: &str = "refuse";

/// How many times the lead is woken to put the question to the human before the human is told directly that it did not.
pub(super) const MAX_RELAYS: u32 = 2;

/// Why a text of the checks cannot be made.
#[derive(Debug, thiserror::Error)]
pub(super) enum ChecksTextError {
    /// A text cannot be rendered.
    #[error("cannot render a text of the checks")]
    Render(#[from] RenderChecksTextError),
    /// A variable name of the program is not valid.
    #[error("a variable of a text is not valid")]
    Variable(#[from] VariablesError),
    /// A question cannot be made of the texts.
    #[error("a question of the checks cannot be made")]
    Entry(#[from] EntryError),
    /// The questions file cannot be made of the questions.
    #[error("the questions file of the checks cannot be made")]
    Compose(#[from] ComposeError),
}

/// The reason of `issue`, as the team and the human are told it.
///
/// # Errors
/// [`ChecksTextError`].
pub(super) fn reason(_texts: &ChecksPrompts, _issue: &Issue) -> Result<String, ChecksTextError> {
    Ok(String::new())
}

/// What the lead is told when the checks send the round back as the `pass`-th of `passes`: every reason, and what the
/// human `refused`.
///
/// # Errors
/// [`ChecksTextError`].
pub(super) fn team_message(
    _texts: &ChecksPrompts,
    _pr: PrNumber,
    _pass: NonZeroU32,
    _passes: u32,
    _issues: &[Issue],
    _refused: &[Subject],
) -> Result<String, ChecksTextError> {
    Ok(String::new())
}

/// The questions the human is asked about the `issues` of a round, and what each is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Questions {
    /// The questions file: one question for each subject, in the order of the subjects.
    pub(super) file: QuestionsFile,
    /// What each question is about, in the order of the questions.
    pub(super) subjects: Vec<Subject>,
}

/// The questions for the `issues` of the round of the team `team`, whose lead is `task`: one for each subject. `reason` says
/// why the human is asked, and `passes` is what the team used.
///
/// # Errors
/// [`ChecksTextError`].
pub(super) fn questions(
    _texts: &ChecksPrompts,
    _team: &TeamId,
    _task: &TaskId,
    _issues: &[Issue],
    _reason: AskReason,
    _passes: u32,
) -> Result<Questions, ChecksTextError> {
    Ok(Questions {
        file: QuestionsFile::new("Questions", vec![entry_one()?])?,
        subjects: Vec::new(),
    })
}

/// A question to build a stub from.
fn entry_one() -> Result<agent_core::questions::Entry, EntryError> {
    agent_core::questions::Entry::new(NonZeroU32::MIN, "stub")
}

/// What the human decided about the subjects of a questions file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct Decisions {
    /// The subjects the human allowed.
    pub(super) allowed: Vec<Subject>,
    /// The subjects the human refused, or did not answer: only an explicit word allows.
    pub(super) refused: Vec<Subject>,
}

/// What the human wrote in `file` about `subjects`, which the questions were made for, in their order.
#[must_use]
pub(super) fn decisions(subjects: &[Subject], _file: &QuestionsFile) -> Decisions {
    Decisions {
        allowed: Vec::new(),
        refused: subjects.to_vec(),
    }
}

/// What the lead is told to put the questions of `round` in `file` to the human.
///
/// # Errors
/// [`ChecksTextError`].
pub(super) fn relay(
    _texts: &ChecksPrompts,
    _task: &TaskId,
    _round: NonZeroU32,
    _file: &std::path::Path,
) -> Result<String, ChecksTextError> {
    Ok(String::new())
}

/// What the lead is told when the human answered the questions of the checks.
///
/// # Errors
/// [`ChecksTextError`].
pub(super) fn answered(_texts: &ChecksPrompts) -> Result<String, ChecksTextError> {
    Ok(String::new())
}

#[cfg(test)]
mod tests;
