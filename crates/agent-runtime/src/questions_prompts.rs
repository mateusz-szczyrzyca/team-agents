//! The loader of the texts of the questions and their answers: the catalog [`QuestionsPrompt`].
//!
//! It works like [`TeamPrompts`](crate::team_prompts::TeamPrompts), for the fourth catalog.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use agent_core::config::Language;
use agent_core::prompt::{PromptError, QuestionsPrompt, Variables};

use crate::harness_prompts::{ProblemCause, read_first};

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
#[error("{} text files in {} cannot be used:{}", problems.len(), dir.display(), Listed(problems))]
pub struct QuestionsPromptsError {
    /// The prompts directory the files were read from.
    dir: PathBuf,
    /// The problems, in the order of the catalog; never empty.
    problems: Vec<QuestionsTextProblem>,
}

/// The problems, each on its own line: its file and what is wrong with it.
struct Listed<'a>(&'a [QuestionsTextProblem]);

impl fmt::Display for Listed<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for problem in self.0 {
            write!(f, "\n  {}: {}", problem.path.display(), problem.cause)?;
        }
        Ok(())
    }
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
        let mut texts = BTreeMap::new();
        let mut problems = Vec::new();
        for id in QuestionsPrompt::ALL {
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
            Err(QuestionsPromptsError {
                dir: dir.to_path_buf(),
                problems,
            })
        }
    }

    /// Renders `id` with `values`.
    ///
    /// # Errors
    /// [`RenderQuestionsTextError::Render`] when `values` give no value for a variable of the text or one for another;
    /// [`RenderQuestionsTextError::NotLoaded`] for a loader that holds no text for `id`.
    pub fn render(&self, id: QuestionsPrompt, values: &Variables) -> Result<String, RenderQuestionsTextError> {
        let text = self.texts.get(&id).ok_or(RenderQuestionsTextError::NotLoaded { id })?;
        id.spec()
            .render(text, values)
            .map_err(|source| RenderQuestionsTextError::Render { id, source })
    }
}

/// Reads the text of `id` and checks it against the catalog.
async fn load_one(dir: &Path, id: QuestionsPrompt, language: &Language) -> Result<String, QuestionsTextProblem> {
    let spec = id.spec();
    let candidates = spec.candidates(language);
    let problem = |path: &Path, cause| QuestionsTextProblem {
        id,
        path: path.to_path_buf(),
        cause,
    };
    let english = Path::new(spec.path());
    let (path, text) = match read_first(dir, &candidates).await {
        Ok(Some(found)) => found,
        Ok(None) => return Err(problem(english, ProblemCause::Missing { tried: candidates })),
        Err((path, cause)) => return Err(problem(&path, cause)),
    };
    // The text has to render with a value for each variable the catalog lists, and with nothing else: the check of
    // the text against the catalog is the check the later rendering will make, with empty values.
    let values = spec
        .variables()
        .iter()
        .try_fold(Variables::new(), |values, name| values.with(name, ""))
        .map_err(|source| problem(&path, ProblemCause::Catalog(source)))?;
    spec.render(&text, &values)
        .map_err(|source| problem(&path, ProblemCause::Invalid(source)))?;
    Ok(text)
}
