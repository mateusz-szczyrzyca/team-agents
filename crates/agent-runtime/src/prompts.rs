//! The prompt loader: reads the catalog's files from the prompts directory, checks all of them at startup, applies the
//! language override and renders.
//!
//! The catalog ([`PromptId`]) says which files exist, who reads them and which variables each takes; this module reads
//! them. [`PromptLoader::load`] reads every file once and reports *all* the problems it finds, not the first, so that
//! an installation with several broken prompts is fixed in one go. After that, [`PromptLoader::render`] can only fail
//! on the values it is given.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::string::FromUtf8Error;

use agent_core::config::Language;
use agent_core::prompt::{PromptError, PromptId, Variables, VariablesError};
use tokio::io::AsyncReadExt;

/// The most bytes a prompt file may have: a bound on input from the disk, far above any prompt of the catalog.
pub const MAX_PROMPT_BYTES: usize = 1 << 20;

/// What is wrong with the file of one prompt.
#[derive(Debug, thiserror::Error)]
pub enum PromptProblem {
    /// None of the files the prompt may come from exists.
    #[error("the prompt {id:?} has no file: tried {tried:?}")]
    Missing {
        /// The prompt.
        id: PromptId,
        /// The files that were tried, in order, as paths in the prompts directory.
        tried: Vec<PathBuf>,
    },
    /// A file exists and cannot be read.
    #[error("cannot read the file {path:?} of the prompt {id:?}")]
    Read {
        /// The prompt.
        id: PromptId,
        /// The file.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// A file is longer than [`MAX_PROMPT_BYTES`].
    #[error("the file {path:?} of the prompt {id:?} is longer than {limit} bytes")]
    TooLarge {
        /// The prompt.
        id: PromptId,
        /// The file.
        path: PathBuf,
        /// The limit.
        limit: usize,
    },
    /// A file is not UTF-8 text.
    #[error("the file {path:?} of the prompt {id:?} is not UTF-8 text")]
    NotUtf8 {
        /// The prompt.
        id: PromptId,
        /// The file.
        path: PathBuf,
        /// Where the bytes stop being UTF-8.
        source: FromUtf8Error,
    },
    /// A file is not a valid text for its prompt: not a template, or it uses another set of variables than the
    /// catalog lists.
    #[error("the file {path:?} is not a valid text for the prompt {id:?}")]
    Invalid {
        /// The prompt.
        id: PromptId,
        /// The file.
        path: PathBuf,
        /// What is wrong with it.
        source: PromptError,
    },
    /// The catalog lists a variable name that is not valid: a mistake of the program, not of the files.
    #[error("the catalog lists an invalid variable for the prompt {id:?}")]
    Catalog {
        /// The prompt.
        id: PromptId,
        /// What is wrong with the name.
        source: VariablesError,
    },
}

/// Every problem the prompt files have.
#[derive(Debug, thiserror::Error)]
#[error("{} prompt files cannot be used", problems.len())]
pub struct PromptLoadError {
    /// The problems, in the order of the catalog; never empty.
    problems: Vec<PromptProblem>,
}

impl PromptLoadError {
    /// The problems, in the order of the catalog.
    #[must_use]
    pub fn problems(&self) -> &[PromptProblem] {
        &self.problems
    }
}

/// Why a prompt could not be rendered.
#[derive(Debug, thiserror::Error)]
pub enum RenderPromptError {
    /// The loader holds no text for the prompt: it was not filled from the catalog.
    #[error("the prompt {id:?} was not loaded")]
    NotLoaded {
        /// The prompt.
        id: PromptId,
    },
    /// The values do not fit the prompt.
    #[error("cannot render the prompt {id:?}")]
    Prompt {
        /// The prompt.
        id: PromptId,
        /// What is wrong with the values.
        source: PromptError,
    },
}

/// The checked texts of the catalog.
#[derive(Debug, Clone)]
pub struct PromptLoader {
    /// The text of each prompt of the catalog, as it was read.
    texts: BTreeMap<PromptId, String>,
}

impl PromptLoader {
    /// Reads and checks every prompt of the catalog from `dir`, in `language`.
    ///
    /// # Errors
    /// [`PromptLoadError`] with every file that is missing, unreadable, too large, not UTF-8 or not valid for its
    /// prompt.
    pub async fn load(dir: &Path, language: &Language) -> Result<Self, PromptLoadError> {
        let mut texts = BTreeMap::new();
        let mut problems = Vec::new();
        for id in PromptId::ALL {
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
            Err(PromptLoadError { problems })
        }
    }

    /// Renders `id` with `values`.
    ///
    /// # Errors
    /// [`RenderPromptError::Prompt`] (with [`PromptError::Render`]) when `values` give no value for a variable of
    /// the prompt or one for another; [`RenderPromptError::NotLoaded`] for a loader that holds no text for `id`.
    pub fn render(&self, id: PromptId, values: &Variables) -> Result<String, RenderPromptError> {
        let text = self.texts.get(&id).ok_or(RenderPromptError::NotLoaded { id })?;
        id.spec()
            .render(text, values)
            .map_err(|source| RenderPromptError::Prompt { id, source })
    }
}

/// Reads the text of `id` and checks it against the catalog.
async fn load_one(dir: &Path, id: PromptId, language: &Language) -> Result<String, PromptProblem> {
    let spec = id.spec();
    let (path, text) = read_first(dir, id, &spec.candidates(language)).await?;
    // The text has to render with a value for each variable the catalog lists, and with nothing else: the check of
    // the text against the catalog is the check the later rendering will make, with empty values.
    let values = spec
        .variables()
        .iter()
        .try_fold(Variables::new(), |values, name| values.with(name, ""))
        .map_err(|source| PromptProblem::Catalog { id, source })?;
    spec.render(&text, &values)
        .map_err(|source| PromptProblem::Invalid { id, path, source })?;
    Ok(text)
}

/// Reads the first of `candidates` (paths in `dir`) that exists.
///
/// A file that exists but cannot be read is a problem, never a reason to try the next one: a broken translation must
/// not silently turn into English.
async fn read_first(dir: &Path, id: PromptId, candidates: &[PathBuf]) -> Result<(PathBuf, String), PromptProblem> {
    for candidate in candidates {
        let path = dir.join(candidate);
        if let Some(text) = read_file(id, &path).await? {
            return Ok((path, text));
        }
    }
    Err(PromptProblem::Missing {
        id,
        tried: candidates.to_vec(),
    })
}

/// The text of the file at `path`, or `None` when there is no such file.
async fn read_file(id: PromptId, path: &Path) -> Result<Option<String>, PromptProblem> {
    let read_error = |source| PromptProblem::Read {
        id,
        path: path.to_path_buf(),
        source,
    };
    let file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        // A candidate that is not there is the specified way to fall back to the next one.
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(read_error(source)),
    };
    // `usize` is never wider than `u64` on a platform Rust supports, so the fallback is not reachable.
    let read_limit = u64::try_from(MAX_PROMPT_BYTES).map_or(u64::MAX, |limit| limit.saturating_add(1));
    let mut bytes = Vec::new();
    file.take(read_limit)
        .read_to_end(&mut bytes)
        .await
        .map_err(read_error)?;
    if bytes.len() > MAX_PROMPT_BYTES {
        return Err(PromptProblem::TooLarge {
            id,
            path: path.to_path_buf(),
            limit: MAX_PROMPT_BYTES,
        });
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|source| PromptProblem::NotUtf8 {
            id,
            path: path.to_path_buf(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use agent_core::prompt::{RenderError, TemplateError, VariableName};

    use super::*;

    fn english() -> Language {
        Language::default()
    }

    fn language(text: &str) -> Language {
        Language::from_str(text).unwrap()
    }

    /// The prompts directory of this repository.
    fn repository_prompts() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts")
    }

    /// A valid text for `id`: it uses exactly the variables the catalog lists.
    fn valid_text(id: PromptId) -> String {
        let spec = id.spec();
        let variables: Vec<String> = spec.variables().iter().map(|name| format!("{{{{{name}}}}}")).collect();
        format!("Text of {}. {}\n", spec.path(), variables.join(" "))
    }

    /// A prompts directory with a valid file for every prompt of the catalog.
    fn catalog_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for id in PromptId::ALL {
            write(dir.path(), id.spec().path(), &valid_text(*id));
        }
        dir
    }

    fn write(dir: &Path, relative: &str, text: &str) {
        let path = dir.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn problems_of(result: Result<PromptLoader, PromptLoadError>) -> Vec<PromptProblem> {
        result.unwrap_err().problems
    }

    // Rule: the prompts of this repository are a valid catalog, and each renders to the text of its file.
    #[tokio::test]
    async fn the_prompts_of_the_repository_load_and_render_to_their_files() {
        let loader = PromptLoader::load(&repository_prompts(), &english()).await.unwrap();

        for id in PromptId::ALL {
            let file = std::fs::read_to_string(repository_prompts().join(id.spec().path())).unwrap();
            let values = id
                .spec()
                .variables()
                .iter()
                .fold(Variables::new(), |values, name| values.with(name, "value").unwrap());
            let rendered = loader.render(*id, &values).unwrap();
            assert_eq!(rendered.is_empty(), file.is_empty(), "{id:?}");
            if id.spec().variables().is_empty() {
                assert_eq!(rendered, file, "{id:?}");
            }
        }
    }

    // Rule: a prompt of the model's audience is English, whatever the language of the installation.
    #[tokio::test]
    async fn a_language_does_not_change_the_prompts_models_read() {
        let polish = PromptLoader::load(&repository_prompts(), &language("Polish"))
            .await
            .unwrap();
        let english = PromptLoader::load(&repository_prompts(), &english()).await.unwrap();

        for id in PromptId::ALL {
            let values = Variables::new();
            assert_eq!(
                polish.render(*id, &values).unwrap(),
                english.render(*id, &values).unwrap(),
                "{id:?}"
            );
        }
    }

    // Rule: every file of the catalog is read from the directory that was given.
    #[tokio::test]
    async fn the_texts_come_from_the_given_directory() {
        let dir = catalog_dir();
        write(dir.path(), PromptId::Clarification.spec().path(), "Ask. Then wait.\n");

        let loader = PromptLoader::load(dir.path(), &english()).await.unwrap();

        assert_eq!(
            loader.render(PromptId::Clarification, &Variables::new()).unwrap(),
            "Ask. Then wait.\n"
        );
        assert_eq!(
            loader
                .render(PromptId::InvestigatorCodeReview, &Variables::new())
                .unwrap(),
            valid_text(PromptId::InvestigatorCodeReview)
        );
    }

    // Rule: a missing file is reported with the prompt and where it was looked for — and so is every other problem.
    #[tokio::test]
    async fn every_problem_is_reported_not_only_the_first() {
        let dir = catalog_dir();
        std::fs::remove_file(dir.path().join(PromptId::Clarification.spec().path())).unwrap();
        write(
            dir.path(),
            PromptId::AgentRustDeveloper.spec().path(),
            "Uses {{nothing}} here.",
        );
        write(
            dir.path(),
            PromptId::WorkerAgentRustDeveloper.spec().path(),
            "An unclosed {{placeholder",
        );

        let problems = problems_of(PromptLoader::load(dir.path(), &english()).await);

        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(
            matches!(&problems[0], PromptProblem::Missing { id: PromptId::Clarification, tried } if tried == &[PathBuf::from("clarification.md")]),
            "{:?}",
            problems[0]
        );
        assert!(
            matches!(
                &problems[1],
                PromptProblem::Invalid { id: PromptId::AgentRustDeveloper, path, source: PromptError::Undeclared { name } }
                    if path == &dir.path().join("agent/rust-developer.md") && *name == VariableName::from_str("nothing").unwrap()
            ),
            "{:?}",
            problems[1]
        );
        assert!(
            matches!(
                &problems[2],
                PromptProblem::Invalid {
                    id: PromptId::WorkerAgentRustDeveloper,
                    source: PromptError::Template(TemplateError::Unclosed { .. }),
                    ..
                }
            ),
            "{:?}",
            problems[2]
        );
    }

    // Rule: a file that is not UTF-8 is a problem of its own.
    #[tokio::test]
    async fn a_file_that_is_not_utf8_is_reported() {
        let dir = catalog_dir();
        std::fs::write(
            dir.path().join(PromptId::Clarification.spec().path()),
            [0xff, 0xfe, b'x'],
        )
        .unwrap();

        let problems = problems_of(PromptLoader::load(dir.path(), &english()).await);

        assert!(
            matches!(
                &problems[..],
                [PromptProblem::NotUtf8 {
                    id: PromptId::Clarification,
                    ..
                }]
            ),
            "{problems:?}"
        );
    }

    // Rule: a file over the bound is refused without being read whole.
    #[tokio::test]
    async fn a_file_over_the_limit_is_reported() {
        let dir = catalog_dir();
        let file = std::fs::File::create(dir.path().join(PromptId::Clarification.spec().path())).unwrap();
        file.set_len(u64::try_from(MAX_PROMPT_BYTES).unwrap() + 1).unwrap();

        let problems = problems_of(PromptLoader::load(dir.path(), &english()).await);

        assert!(
            matches!(&problems[..], [PromptProblem::TooLarge { id: PromptId::Clarification, limit, .. }] if *limit == MAX_PROMPT_BYTES),
            "{problems:?}"
        );
    }

    // Rule: a file that is at the bound is read.
    #[tokio::test]
    async fn a_file_at_the_limit_is_read() {
        let dir = catalog_dir();
        write(
            dir.path(),
            PromptId::Clarification.spec().path(),
            &"a".repeat(MAX_PROMPT_BYTES),
        );

        let loader = PromptLoader::load(dir.path(), &english()).await.unwrap();

        assert_eq!(
            loader.render(PromptId::Clarification, &Variables::new()).unwrap().len(),
            MAX_PROMPT_BYTES
        );
    }

    // Rule: a file that cannot be read is a read problem with its path.
    #[tokio::test]
    async fn a_file_that_cannot_be_read_is_reported() {
        let dir = catalog_dir();
        let path = dir.path().join(PromptId::Clarification.spec().path());
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();

        let problems = problems_of(PromptLoader::load(dir.path(), &english()).await);

        assert!(
            matches!(&problems[..], [PromptProblem::Read { id: PromptId::Clarification, path: found, .. }] if *found == path),
            "{problems:?}"
        );
    }

    // Rule: values that do not fit the prompt are a render error, not a wrong text.
    #[tokio::test]
    async fn rendering_with_a_value_the_prompt_does_not_have_fails() {
        let dir = catalog_dir();
        let loader = PromptLoader::load(dir.path(), &english()).await.unwrap();
        let values = Variables::new().with("extra", "x").unwrap();

        let result = loader.render(PromptId::Clarification, &values);

        assert!(
            matches!(
                &result,
                Err(RenderPromptError::Prompt {
                    id: PromptId::Clarification,
                    source: PromptError::Render(RenderError::Unknown { .. })
                })
            ),
            "{result:?}"
        );
    }

    // Rule: a loader that was not filled from the catalog says so instead of rendering nothing.
    #[test]
    fn rendering_a_prompt_that_was_not_loaded_fails() {
        let loader = PromptLoader { texts: BTreeMap::new() };

        let result = loader.render(PromptId::Clarification, &Variables::new());

        assert!(
            matches!(
                &result,
                Err(RenderPromptError::NotLoaded {
                    id: PromptId::Clarification
                })
            ),
            "{result:?}"
        );
    }

    // ---- the language override ----------------------------------------------------------------------------------

    /// The files a human-facing prompt of the language Polish may come from.
    fn candidates() -> Vec<PathBuf> {
        vec![
            PathBuf::from("lang/polish/harness/notify/done.md"),
            PathBuf::from("harness/notify/done.md"),
        ]
    }

    // Rule: the file of the language wins over English when it exists.
    #[tokio::test]
    async fn the_file_of_the_language_wins() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "harness/notify/done.md", "Done.");
        write(dir.path(), "lang/polish/harness/notify/done.md", "Gotowe.");

        let (path, text) = read_first(dir.path(), PromptId::Clarification, &candidates())
            .await
            .unwrap();

        assert_eq!(
            (path, text),
            (
                dir.path().join("lang/polish/harness/notify/done.md"),
                "Gotowe.".to_owned()
            )
        );
    }

    // Rule: English is the fallback when the language has no file.
    #[tokio::test]
    async fn english_is_the_fallback() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "harness/notify/done.md", "Done.");

        let (path, text) = read_first(dir.path(), PromptId::Clarification, &candidates())
            .await
            .unwrap();

        assert_eq!(
            (path, text),
            (dir.path().join("harness/notify/done.md"), "Done.".to_owned())
        );
    }

    // Rule: a broken file of the language is a problem, never a silent fall back to English.
    #[tokio::test]
    async fn a_file_of_the_language_that_cannot_be_read_is_not_replaced_by_english() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "harness/notify/done.md", "Done.");
        std::fs::create_dir_all(dir.path().join("lang/polish/harness/notify/done.md")).unwrap();

        let result = read_first(dir.path(), PromptId::Clarification, &candidates()).await;

        assert!(matches!(&result, Err(PromptProblem::Read { .. })), "{result:?}");
    }

    // Rule: no file at all lists every place that was tried.
    #[tokio::test]
    async fn no_file_lists_the_candidates() {
        let dir = tempfile::tempdir().unwrap();

        let result = read_first(dir.path(), PromptId::Clarification, &candidates()).await;

        assert!(
            matches!(&result, Err(PromptProblem::Missing { tried, .. }) if *tried == candidates()),
            "{result:?}"
        );
    }
}
