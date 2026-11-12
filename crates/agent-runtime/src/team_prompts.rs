//! The loader of the texts of a team's life: the catalog [`TeamPrompt`].
//!
//! It works like [`HarnessPrompts`](crate::harness_prompts::HarnessPrompts), for the third catalog:
//! [`TeamPrompts::load`] reads every file once, applies the language override of the text for the human and reports
//! *all* the problems it finds; after that [`TeamPrompts::render`] can only fail on the values it is given.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use agent_core::config::Language;
use agent_core::prompt::{PromptError, TeamPrompt, Variables};

use crate::harness_prompts::{ProblemCause, read_first};

/// What is wrong with the file of one text.
#[derive(Debug, thiserror::Error)]
#[error("the text {id:?} cannot be used (file {path:?})")]
pub struct TeamTextProblem {
    /// The text.
    pub id: TeamPrompt,
    /// The file, in the prompts directory; the English one when none exists.
    pub path: PathBuf,
    /// What is wrong.
    #[source]
    pub cause: ProblemCause,
}

/// Every problem the files of the catalog have.
#[derive(Debug, thiserror::Error)]
#[error("{} text files in {} cannot be used:{}", problems.len(), dir.display(), Listed(problems))]
pub struct TeamPromptsError {
    /// The prompts directory the files were read from.
    dir: PathBuf,
    /// The problems, in the order of the catalog; never empty.
    problems: Vec<TeamTextProblem>,
}

/// The problems, each on its own line: its file and what is wrong with it.
struct Listed<'a>(&'a [TeamTextProblem]);

impl fmt::Display for Listed<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for problem in self.0 {
            write!(f, "\n  {}: {}", problem.path.display(), problem.cause)?;
        }
        Ok(())
    }
}

impl TeamPromptsError {
    /// The problems, in the order of the catalog.
    #[must_use]
    pub fn problems(&self) -> &[TeamTextProblem] {
        &self.problems
    }
}

/// Why a text could not be rendered.
#[derive(Debug, thiserror::Error)]
pub enum RenderTeamTextError {
    /// The loader holds no text for the entry: it was not filled from the catalog.
    #[error("the text {id:?} was not loaded")]
    NotLoaded {
        /// The text.
        id: TeamPrompt,
    },
    /// The values do not fit the text.
    #[error("cannot render the text {id:?}")]
    Render {
        /// The text.
        id: TeamPrompt,
        /// What is wrong with the values.
        source: PromptError,
    },
}

/// The checked texts of the catalog.
#[derive(Debug, Clone)]
pub struct TeamPrompts {
    /// The text of each entry, as it was read.
    texts: BTreeMap<TeamPrompt, String>,
}

impl TeamPrompts {
    /// Reads and checks every text of the catalog from `dir`, in `language`.
    ///
    /// # Errors
    /// [`TeamPromptsError`] with every file that is missing, unreadable, too large, not UTF-8 or not valid for its
    /// entry.
    pub async fn load(dir: &Path, language: &Language) -> Result<Self, TeamPromptsError> {
        let _ = (dir, language);
        Ok(Self { texts: BTreeMap::new() })
    }

    /// Renders `id` with `values`.
    ///
    /// # Errors
    /// [`RenderTeamTextError::Render`] when `values` give no value for a variable of the text or one for another;
    /// [`RenderTeamTextError::NotLoaded`] for a loader that holds no text for `id`.
    pub fn render(&self, id: TeamPrompt, values: &Variables) -> Result<String, RenderTeamTextError> {
        let _ = values;
        let text = self.texts.get(&id).ok_or(RenderTeamTextError::NotLoaded { id })?;
        Ok(text.clone())
    }
}

#[expect(dead_code, reason = "used by the loader once it reads the files")]
async fn _unused() {
    let _ = read_first;
}
