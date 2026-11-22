//! What an agent is told when its commit is refused: the findings, each as a line of the catalog, in the frame that says
//! how to make the commit the rule allows.

use agent_core::decide::commit_guard::Refusal;
use agent_core::prompt::PromptError;

use crate::harness_prompts::{HarnessPrompts, RenderTextError};

/// Why the text of a refusal cannot be made.
#[derive(Debug, thiserror::Error)]
pub enum RefusalTextError {
    /// A value does not fit the text's variable.
    #[error("cannot give a variable to the text of a refusal")]
    Variable(#[source] PromptError),
    /// A text of the catalog cannot be rendered.
    #[error("cannot render the text of a refusal")]
    Render(#[source] RenderTextError),
}

/// The text that tells an agent why the commit with these `refusals` was not made and what to do instead.
///
/// # Errors
/// [`RefusalTextError`] when a text of the catalog is not loaded or does not fit its values.
pub fn refusal_text(_texts: &HarnessPrompts, _refusals: &[Refusal]) -> Result<String, RefusalTextError> {
    Ok(String::new())
}
