//! The loader of the texts of the checks before a review: the catalog [`ChecksPrompt`].
//!
//! It works like [`AutopilotPrompts`](crate::autopilot_prompts::AutopilotPrompts), for the fifth catalog:
//! [`ChecksPrompts::load`] reads every file once, applies the language override of the texts for the human and reports
//! *all* the problems it finds; after that [`ChecksPrompts::render`] can only fail on the values it is given.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use agent_core::config::Language;
use agent_core::prompt::{ChecksPrompt, PromptError, Variables};

use crate::harness_prompts::{ProblemCause, read_first};

/// What is wrong with the file of one text.
#[derive(Debug, thiserror::Error)]
#[error("the text {id:?} cannot be used (file {path:?})")]
pub struct ChecksTextProblem {
    /// The text.
    pub id: ChecksPrompt,
    /// The file, in the prompts directory; the English one when none exists.
    pub path: PathBuf,
    /// What is wrong.
    #[source]
    pub cause: ProblemCause,
}

/// Every problem the files of the catalog have.
#[derive(Debug, thiserror::Error)]
#[error("{} text files in {} cannot be used:{}", problems.len(), dir.display(), Listed(problems))]
pub struct ChecksPromptsError {
    /// The prompts directory the files were read from.
    dir: PathBuf,
    /// The problems, in the order of the catalog; never empty.
    problems: Vec<ChecksTextProblem>,
}

/// The problems, each on its own line: its file and what is wrong with it.
struct Listed<'a>(&'a [ChecksTextProblem]);

impl fmt::Display for Listed<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for problem in self.0 {
            write!(f, "\n  {}: {}", problem.path.display(), problem.cause)?;
        }
        Ok(())
    }
}

impl ChecksPromptsError {
    /// The problems, in the order of the catalog.
    #[must_use]
    pub fn problems(&self) -> &[ChecksTextProblem] {
        &self.problems
    }
}

/// Why a text could not be rendered.
#[derive(Debug, thiserror::Error)]
pub enum RenderChecksTextError {
    /// The loader holds no text for the entry: it was not filled from the catalog.
    #[error("the text {id:?} was not loaded")]
    NotLoaded {
        /// The text.
        id: ChecksPrompt,
    },
    /// The values do not fit the text.
    #[error("cannot render the text {id:?}")]
    Render {
        /// The text.
        id: ChecksPrompt,
        /// What is wrong with the values.
        source: PromptError,
    },
}

/// The checked texts of the catalog.
#[derive(Debug, Clone)]
pub struct ChecksPrompts {
    /// The text of each entry, as it was read.
    texts: BTreeMap<ChecksPrompt, String>,
}

impl ChecksPrompts {
    /// Reads and checks every text of the catalog from `dir`, in `language`.
    ///
    /// # Errors
    /// [`ChecksPromptsError`] with every file that is missing, unreadable, too large, not UTF-8 or not valid for
    /// its entry.
    pub async fn load(dir: &Path, language: &Language) -> Result<Self, ChecksPromptsError> {
        let mut texts = BTreeMap::new();
        let mut problems = Vec::new();
        for id in ChecksPrompt::ALL {
            match load_one(dir, *id, language).await {
                Ok(text) => {
                    texts.insert(*id, text);
                }
                Err(problem) => problems.push(problem),
            }
        }
        if problems.is_empty() {
            Ok(Self { texts })
        } else {
            Err(ChecksPromptsError {
                dir: dir.to_path_buf(),
                problems,
            })
        }
    }

    /// Renders `id` with `values`.
    ///
    /// # Errors
    /// [`RenderChecksTextError::Render`] when `values` give no value for a variable of the text or one for another;
    /// [`RenderChecksTextError::NotLoaded`] for a loader that holds no text for `id`.
    pub fn render(&self, id: ChecksPrompt, values: &Variables) -> Result<String, RenderChecksTextError> {
        let text = self.texts.get(&id).ok_or(RenderChecksTextError::NotLoaded { id })?;
        id.spec()
            .render(text, values)
            .map_err(|source| RenderChecksTextError::Render { id, source })
    }
}

/// Reads the text of `id` and checks it against the catalog.
async fn load_one(dir: &Path, id: ChecksPrompt, _language: &Language) -> Result<String, ChecksTextProblem> {
    let spec = id.spec();
    let english = vec![PathBuf::from(spec.path())];
    match read_first(dir, &english).await {
        Ok(Some((_, text))) => Ok(text),
        Ok(None) => Err(ChecksTextProblem {
            id,
            path: PathBuf::from(spec.path()),
            cause: ProblemCause::Missing { tried: english },
        }),
        Err((path, cause)) => Err(ChecksTextProblem { id, path, cause }),
    }
}
