//! What an agent is told when its commit is refused: the findings, each as a line of the catalog, in the frame that says
//! how to make the commit the rule allows.

use agent_core::decide::commit_guard::{Refusal, Taken};
use agent_core::prompt::{HarnessPrompt, Variables, VariablesError};
use agent_core::repo_path::RepoPath;
use agent_core::state::CommitId;

use crate::harness_prompts::{HarnessPrompts, RenderTextError};

/// Why the text of a refusal cannot be made.
#[derive(Debug, thiserror::Error)]
pub enum RefusalTextError {
    /// A value does not fit the text's variable.
    #[error("cannot give a variable to the text of a refusal")]
    Variable(#[source] VariablesError),
    /// A text of the catalog cannot be rendered.
    #[error("cannot render the text of a refusal")]
    Render(#[source] RenderTextError),
}

/// How many digits of a commit ID name it in a text.
const SHORT_ID_LEN: usize = 7;

/// A commit ID as it is written in a text: its first digits.
fn short(id: &CommitId) -> String {
    id.as_str().chars().take(SHORT_ID_LEN).collect()
}

/// `files` as the words of a sentence: each in backticks, with commas between.
fn listed(files: &[RepoPath]) -> String {
    files
        .iter()
        .map(|file| format!("`{file}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The line of the catalog that tells `refusal`.
fn finding(texts: &HarnessPrompts, refusal: &Refusal) -> Result<String, RefusalTextError> {
    let render = |id, values: Variables| texts.render(id, &values).map_err(RefusalTextError::Render);
    let with =
        |values: Variables, name: &str, value: &str| values.with(name, value).map_err(RefusalTextError::Variable);
    match refusal {
        Refusal::FrozenTestEdited {
            path,
            red,
            taken: Taken::Lines(count),
        } => {
            let values = with(Variables::new(), "path", path.as_str())?;
            let values = with(values, "red", &short(red))?;
            let values = with(values, "count", &count.to_string())?;
            render(HarnessPrompt::CommitRefusedLines, values)
        }
        Refusal::FrozenTestEdited {
            path,
            red,
            taken: Taken::WholeFile,
        } => {
            let values = with(Variables::new(), "path", path.as_str())?;
            let values = with(values, "red", &short(red))?;
            render(HarnessPrompt::CommitRefusedWhole, values)
        }
        Refusal::TestFixTouchesOtherFiles { others } => {
            let values = with(Variables::new(), "files", &listed(others))?;
            render(HarnessPrompt::CommitRefusedMixed, values)
        }
    }
}

/// The text that tells an agent why the commit with these `refusals` was not made and what to do instead.
///
/// # Errors
/// [`RefusalTextError`] when a text of the catalog is not loaded or does not fit its values.
pub fn refusal_text(texts: &HarnessPrompts, refusals: &[Refusal]) -> Result<String, RefusalTextError> {
    let mut findings = String::new();
    for refusal in refusals {
        findings.push_str(finding(texts, refusal)?.trim_end());
        findings.push('\n');
    }
    let values = Variables::new()
        .with("findings", findings.trim_end())
        .map_err(RefusalTextError::Variable)?;
    texts
        .render(HarnessPrompt::CommitRefused, &values)
        .map_err(RefusalTextError::Render)
}
