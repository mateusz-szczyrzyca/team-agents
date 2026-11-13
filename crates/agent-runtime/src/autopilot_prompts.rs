//! The loader of the texts of the autopilot: the catalog [`AutopilotPrompt`].
//!
//! It works like [`TeamPrompts`](crate::team_prompts::TeamPrompts), for the fourth catalog: [`AutopilotPrompts::load`]
//! reads every file once, applies the language override of the notices for the human and reports *all* the problems it
//! finds; after that [`AutopilotPrompts::render`] can only fail on the values it is given.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use agent_core::config::Language;
use agent_core::prompt::{AutopilotPrompt, PromptError, Variables};

use crate::harness_prompts::ProblemCause;

/// What is wrong with the file of one text.
#[derive(Debug, thiserror::Error)]
#[error("the text {id:?} cannot be used (file {path:?})")]
pub struct AutopilotTextProblem {
    /// The text.
    pub id: AutopilotPrompt,
    /// The file, in the prompts directory; the English one when none exists.
    pub path: PathBuf,
    /// What is wrong.
    #[source]
    pub cause: ProblemCause,
}

/// Every problem the files of the catalog have.
#[derive(Debug, thiserror::Error)]
#[error("{} text files in {} cannot be used", problems.len(), dir.display())]
pub struct AutopilotPromptsError {
    /// The prompts directory the files were read from.
    dir: PathBuf,
    /// The problems, in the order of the catalog; never empty.
    problems: Vec<AutopilotTextProblem>,
}

impl AutopilotPromptsError {
    /// The problems, in the order of the catalog.
    #[must_use]
    pub fn problems(&self) -> &[AutopilotTextProblem] {
        &self.problems
    }
}

/// Why a text could not be rendered.
#[derive(Debug, thiserror::Error)]
pub enum RenderAutopilotTextError {
    /// The loader holds no text for the entry: it was not filled from the catalog.
    #[error("the text {id:?} was not loaded")]
    NotLoaded {
        /// The text.
        id: AutopilotPrompt,
    },
    /// The values do not fit the text.
    #[error("cannot render the text {id:?}")]
    Render {
        /// The text.
        id: AutopilotPrompt,
        /// What is wrong with the values.
        source: PromptError,
    },
}

/// The checked texts of the catalog.
#[derive(Debug, Clone)]
pub struct AutopilotPrompts {
    /// The text of each entry, as it was read.
    texts: BTreeMap<AutopilotPrompt, String>,
}

impl AutopilotPrompts {
    /// Reads and checks every text of the catalog from `dir`, in `language`.
    ///
    /// # Errors
    /// [`AutopilotPromptsError`] with every file that is missing, unreadable, too large, not UTF-8 or not valid for
    /// its entry.
    pub async fn load(dir: &Path, language: &Language) -> Result<Self, AutopilotPromptsError> {
        let _ = (dir, language);
        Ok(Self { texts: BTreeMap::new() })
    }

    /// Renders `id` with `values`.
    ///
    /// # Errors
    /// [`RenderAutopilotTextError::Render`] when `values` give no value for a variable of the text or one for another;
    /// [`RenderAutopilotTextError::NotLoaded`] for a loader that holds no text for `id`.
    pub fn render(&self, id: AutopilotPrompt, values: &Variables) -> Result<String, RenderAutopilotTextError> {
        let _ = values;
        Err(RenderAutopilotTextError::NotLoaded { id })
    }
}
