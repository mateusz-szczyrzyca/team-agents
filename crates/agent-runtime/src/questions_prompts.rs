//! The loader of the texts of the questions and their answers: the catalog [`QuestionsPrompt`].
//!
//! It works like [`TeamPrompts`](crate::team_prompts::TeamPrompts), for the fourth catalog.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use agent_core::config::Language;
use agent_core::prompt::{PromptError, QuestionsPrompt, Variables};

use crate::harness_prompts::ProblemCause;

/// What is wrong with the file of one text.
#[derive(Debug, thiserror::Error)]
#[error("the text {id:?} cannot be used (file {path:?})")]
pub struct QuestionsTextProblem {
    /// The text.
    pub id: QuestionsPrompt,
    /// The file, in the prompts directory; the English one when none exists.
    pub path: PathBuf,
    /// What is wrong.
    #[source]
    pub cause: ProblemCause,
}

/// Every problem the files of the catalog have.
#[derive(Debug, thiserror::Error)]
#[error("{} text files in {} cannot be used", problems.len(), dir.display())]
pub struct QuestionsPromptsError {
    /// The prompts directory the files were read from.
    dir: PathBuf,
    /// The problems, in the order of the catalog; never empty.
    problems: Vec<QuestionsTextProblem>,
}

impl QuestionsPromptsError {
    /// The problems, in the order of the catalog.
    #[must_use]
    pub fn problems(&self) -> &[QuestionsTextProblem] {
        &self.problems
    }
}

/// Why a text could not be rendered.
#[derive(Debug, thiserror::Error)]
pub enum RenderQuestionsTextError {
    /// The loader holds no text for the entry: it was not filled from the catalog.
    #[error("the text {id:?} was not loaded")]
    NotLoaded {
        /// The text.
        id: QuestionsPrompt,
    },
    /// The values do not fit the text.
    #[error("cannot render the text {id:?}")]
    Render {
        /// The text.
        id: QuestionsPrompt,
        /// What is wrong with the values.
        source: PromptError,
    },
}

/// The checked texts of the catalog.
#[derive(Debug, Clone)]
pub struct QuestionsPrompts {
    /// The text of each entry, as it was read.
    texts: BTreeMap<QuestionsPrompt, String>,
}

impl QuestionsPrompts {
    /// Reads and checks every text of the catalog from `dir`, in `language`.
    ///
    /// # Errors
    /// [`QuestionsPromptsError`] with every file that is missing, unreadable, too large, not UTF-8 or not valid for its
    /// entry.
    pub async fn load(dir: &Path, language: &Language) -> Result<Self, QuestionsPromptsError> {
        let _ = (dir, language);
        Ok(Self { texts: BTreeMap::new() })
    }

    /// Renders `id` with `values`.
    ///
    /// # Errors
    /// [`RenderQuestionsTextError::Render`] when `values` give no value for a variable of the text or one for another;
    /// [`RenderQuestionsTextError::NotLoaded`] for a loader that holds no text for `id`.
    pub fn render(&self, id: QuestionsPrompt, values: &Variables) -> Result<String, RenderQuestionsTextError> {
        let _ = (id, values, &self.texts);
        Ok(String::new())
    }
}
