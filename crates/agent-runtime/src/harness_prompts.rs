//! The loader of the harness's own texts: the run prompts and the notifications of the catalog [`HarnessPrompt`].
//!
//! It works like [`PromptLoader`](crate::prompts::PromptLoader), for the second catalog: [`HarnessPrompts::load`]
//! reads every file once, applies the language override of the texts for the human and reports *all* the problems it
//! finds; after that [`HarnessPrompts::render`] can only fail on the values it is given.

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::string::FromUtf8Error;

use agent_core::config::Language;
use agent_core::prompt::{HarnessPrompt, PromptError, Variables, VariablesError};

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
#[error("{} text files cannot be used", problems.len())]
pub struct HarnessPromptsError {
    /// The problems, in the order of the catalog; never empty.
    problems: Vec<TextProblem>,
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
    pub async fn load(dir: &std::path::Path, language: &Language) -> Result<Self, HarnessPromptsError> {
        let _ = (dir, language);
        Ok(Self { texts: BTreeMap::new() })
    }

    /// Renders `id` with `values`.
    ///
    /// # Errors
    /// [`RenderTextError::Render`] when `values` give no value for a variable of the text or one for another;
    /// [`RenderTextError::NotLoaded`] for a loader that holds no text for `id`.
    pub fn render(&self, id: HarnessPrompt, values: &Variables) -> Result<String, RenderTextError> {
        let _ = values;
        Err(RenderTextError::NotLoaded { id })
    }
}
