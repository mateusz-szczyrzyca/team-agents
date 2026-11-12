//! The loader of the harness's own texts: the run prompts and the notifications of the catalog [`HarnessPrompt`].
//!
//! It works like [`PromptLoader`](crate::prompts::PromptLoader), for the second catalog: [`HarnessPrompts::load`]
//! reads every file once, applies the language override of the texts for the human and reports *all* the problems it
//! finds; after that [`HarnessPrompts::render`] can only fail on the values it is given.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::string::FromUtf8Error;

use agent_core::config::Language;
use agent_core::prompt::{HarnessPrompt, PromptError, Variables, VariablesError};

use crate::text_file::{TextFileError, read_text_file};

/// The most bytes a text file may have: a bound on input from the disk, far above any text of the catalog.
pub const MAX_TEXT_BYTES: usize = 1 << 20;

/// Why a file of the catalog cannot be used.
#[derive(Debug, thiserror::Error)]
pub enum ProblemCause {
    /// None of the files the text may come from exists.
    #[error("none of the candidate files exists")]
    Missing {
        /// The files that were tried, in order, as paths in the prompts directory.
        tried: Vec<PathBuf>,
    },
    /// The file exists and cannot be read.
    #[error("the file cannot be read")]
    Read(#[source] io::Error),
    /// The file is longer than [`MAX_TEXT_BYTES`].
    #[error("the file is longer than {MAX_TEXT_BYTES} bytes")]
    TooLarge,
    /// The file is not UTF-8 text.
    #[error("the file is not UTF-8 text")]
    NotUtf8(#[source] FromUtf8Error),
    /// The file is not a valid template for its entry: not a template, or another set of variables than the catalog's.
    #[error("the file is not a valid text for its entry")]
    Invalid(#[source] PromptError),
    /// The catalog lists a variable name that is not valid: a mistake of the program, not of the files.
    #[error("the catalog lists an invalid variable")]
    Catalog(#[source] VariablesError),
}

/// What is wrong with the file of one text.
#[derive(Debug, thiserror::Error)]
#[error("the text {id:?} cannot be used (file {path:?})")]
pub struct TextProblem {
    /// The text.
    pub id: HarnessPrompt,
    /// The file, in the prompts directory; the English one when none exists.
    pub path: PathBuf,
    /// What is wrong.
    #[source]
    pub cause: ProblemCause,
}

/// Every problem the files of the catalog have.
#[derive(Debug, thiserror::Error)]
#[error("{} text files cannot be used{}", problems.len(), listed(problems))]
pub struct HarnessPromptsError {
    /// The prompts directory the files were read from.
    dir: PathBuf,
    /// The problems, in the order of the catalog; never empty.
    problems: Vec<TextProblem>,
}

/// Each problem on one line: its file and what is wrong with it.
fn listed(_problems: &[TextProblem]) -> String {
    String::new()
}

impl HarnessPromptsError {
    /// The problems, in the order of the catalog.
    #[must_use]
    pub fn problems(&self) -> &[TextProblem] {
        &self.problems
    }
}

/// Why a text could not be rendered.
#[derive(Debug, thiserror::Error)]
pub enum RenderTextError {
    /// The loader holds no text for the entry: it was not filled from the catalog.
    #[error("the text {id:?} was not loaded")]
    NotLoaded {
        /// The text.
        id: HarnessPrompt,
    },
    /// The values do not fit the text.
    #[error("cannot render the text {id:?}")]
    Render {
        /// The text.
        id: HarnessPrompt,
        /// What is wrong with the values.
        source: PromptError,
    },
}

/// The checked texts of the catalog.
#[derive(Debug, Clone)]
pub struct HarnessPrompts {
    /// The text of each entry, as it was read.
    texts: BTreeMap<HarnessPrompt, String>,
}

impl HarnessPrompts {
    /// Reads and checks every text of the catalog from `dir`, in `language`.
    ///
    /// # Errors
    /// [`HarnessPromptsError`] with every file that is missing, unreadable, too large, not UTF-8 or not valid for its
    /// entry.
    pub async fn load(dir: &Path, language: &Language) -> Result<Self, HarnessPromptsError> {
        let mut texts = BTreeMap::new();
        let mut problems = Vec::new();
        for id in HarnessPrompt::ALL {
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
            Err(HarnessPromptsError {
                dir: dir.to_path_buf(),
                problems,
            })
        }
    }

    /// Renders `id` with `values`.
    ///
    /// # Errors
    /// [`RenderTextError::Render`] when `values` give no value for a variable of the text or one for another;
    /// [`RenderTextError::NotLoaded`] for a loader that holds no text for `id`.
    pub fn render(&self, id: HarnessPrompt, values: &Variables) -> Result<String, RenderTextError> {
        let text = self.texts.get(&id).ok_or(RenderTextError::NotLoaded { id })?;
        id.spec()
            .render(text, values)
            .map_err(|source| RenderTextError::Render { id, source })
    }
}

/// Reads the text of `id` and checks it against the catalog.
async fn load_one(dir: &Path, id: HarnessPrompt, language: &Language) -> Result<String, TextProblem> {
    let spec = id.spec();
    let candidates = spec.candidates(language);
    let problem = |path: &Path, cause| TextProblem {
        id,
        path: path.to_path_buf(),
        cause,
    };
    let english = Path::new(spec.path());
    let (path, text) = match read_first(dir, &candidates).await {
        Ok(Some(found)) => found,
        Ok(None) => {
            return Err(problem(english, ProblemCause::Missing { tried: candidates }));
        }
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

/// Reads the first of `candidates` (paths in `dir`) that exists: its path and its text, or `None` when none exists.
///
/// A file that exists but cannot be read is a problem, never a reason to try the next one: a broken translation must
/// not silently turn into English.
async fn read_first(dir: &Path, candidates: &[PathBuf]) -> Result<Option<(PathBuf, String)>, (PathBuf, ProblemCause)> {
    for candidate in candidates {
        let path = dir.join(candidate);
        match read_file(&path).await {
            Ok(Some(text)) => return Ok(Some((path, text))),
            // A candidate that is not there is the specified way to fall back to the next one.
            Ok(None) => {}
            Err(cause) => return Err((path, cause)),
        }
    }
    Ok(None)
}

/// The text of the file at `path`, or `None` when there is no such file.
async fn read_file(path: &Path) -> Result<Option<String>, ProblemCause> {
    let file = read_text_file(path, MAX_TEXT_BYTES)
        .await
        .map_err(|error| match error {
            TextFileError::Read(source) => ProblemCause::Read(source),
            TextFileError::TooLarge { .. } => ProblemCause::TooLarge,
            TextFileError::NotUtf8(source) => ProblemCause::NotUtf8(source),
        })?;
    Ok(file.map(|file| file.text))
}
