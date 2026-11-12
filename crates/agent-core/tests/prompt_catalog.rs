//! The prompt catalog against the files of `prompts/`: every entry must exist and render with its variables, and a
//! failure must name the file.

use std::io;
use std::path::Path;

use agent_core::prompt::{PROMPTS_DIR, PromptId, PromptSpec, Variables, VariablesError};

/// The value every variable gets in the check: the text only has to render.
const PLACEHOLDER_VALUE: &str = "value";

/// The prompts directory of this repository.
fn prompts_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../").join(PROMPTS_DIR)
}

/// Reads a prompt's English file from the repository.
fn read_from_repository(path: &str) -> io::Result<String> {
    std::fs::read_to_string(prompts_dir().join(path))
}

/// A value for each variable `spec` lists.
fn placeholder_values(spec: &PromptSpec) -> Result<Variables, VariablesError> {
    let mut values = Variables::new();
    for name in spec.variables() {
        values = values.with(name, PLACEHOLDER_VALUE)?;
    }
    Ok(values)
}

/// The problems of the catalog, one line each, every line starting with the file's path; `read` gives the content of
/// a file by its path under the prompts directory.
fn catalog_problems(read: impl Fn(&str) -> io::Result<String>) -> Vec<String> {
    let mut problems = Vec::new();
    for id in PromptId::ALL {
        let spec = id.spec();
        let file = format!("{PROMPTS_DIR}/{}", spec.path());
        let text = match read(spec.path()) {
            Ok(text) => text,
            Err(err) => {
                problems.push(format!("{file}: cannot be read: {err}"));
                continue;
            }
        };
        let values = match placeholder_values(&spec) {
            Ok(values) => values,
            Err(err) => {
                problems.push(format!("{file}: the catalog lists a bad variable: {err}"));
                continue;
            }
        };
        match spec.render(&text, &values) {
            Ok(rendered) if rendered.trim().is_empty() => problems.push(format!("{file}: is empty")),
            Ok(_) => {}
            Err(err) => problems.push(format!("{file}: {err}")),
        }
    }
    problems
}

#[test]
fn every_catalog_entry_exists_and_renders() {
    let problems = catalog_problems(read_from_repository);

    assert_eq!(problems, Vec::<String>::new());
}

#[test]
fn a_missing_prompt_file_fails_with_its_name() {
    let missing = PromptId::WorkerAgentRustDeveloper.spec().path();

    let problems = catalog_problems(|path| {
        if path == missing {
            Err(io::Error::from(io::ErrorKind::NotFound))
        } else {
            read_from_repository(path)
        }
    });

    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(
        problems[0].starts_with("prompts/worker-agent/rust-developer.md: cannot be read"),
        "{problems:?}"
    );
}

#[test]
fn a_variable_the_catalog_does_not_list_fails_with_the_files_name() {
    let changed = PromptId::Clarification.spec().path();

    let problems = catalog_problems(|path| {
        let text = read_from_repository(path)?;
        Ok(if path == changed {
            format!("{text}\nAnswer in {{{{language}}}}.\n")
        } else {
            text
        })
    });

    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].starts_with("prompts/clarification.md: "), "{problems:?}");
    assert!(problems[0].contains("language"), "{problems:?}");
}

#[test]
fn a_broken_placeholder_fails_with_the_files_name_and_line() {
    let changed = PromptId::InvestigatorCodeReview.spec().path();

    let problems = catalog_problems(|path| {
        let text = read_from_repository(path)?;
        Ok(if path == changed {
            format!("{{{{ broken\n{text}")
        } else {
            text
        })
    });

    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(
        problems[0].starts_with("prompts/investigator/code-review.md: line 1"),
        "{problems:?}"
    );
}
