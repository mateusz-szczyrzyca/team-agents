//! The values of the variables of `harness/run/checkpoint-state.md`: how a checkpoint is told to an agent or a human,
//! wherever it is told.

use agent_core::history::{CheckpointCause, CheckpointContent};
use agent_core::prompt::{Variables, VariablesError};
use agent_core::state::{CommitId, Timestamp};

/// What the text of a checkpoint is made of: the facts of one checkpoint record that
/// [`HarnessPrompt::CheckpointState`](agent_core::prompt::HarnessPrompt::CheckpointState) tells.
#[derive(Debug, Clone, Copy)]
pub struct CheckpointFacts<'a> {
    /// The step the checkpoint was taken after.
    pub cause: CheckpointCause,
    /// When it was taken.
    pub at: Timestamp,
    /// The tip of the holder's branch then; `None` when git could not say.
    pub head: Option<&'a CommitId>,
    /// What is meant, done and left; `None` for a checkpoint that kept no account of the work.
    pub content: Option<&'a CheckpointContent>,
}

/// What a checkpoint with no account of the work says in the place of its three parts.
pub const NO_CONTENT: &str = "(this checkpoint kept no account of the work)";

/// The values of the variables of the text that tells `facts`.
///
/// # Errors
/// [`VariablesError`] when a value cannot be given to its variable.
pub fn checkpoint_state_variables(facts: &CheckpointFacts<'_>) -> Result<Variables, VariablesError> {
    let kept = |part: fn(&CheckpointContent) -> &str| facts.content.map_or(NO_CONTENT, part).to_owned();
    Variables::new()
        .with("meant", kept(|content| &content.meant))?
        .with("done", kept(|content| &content.done))?
        .with("left", kept(|content| &content.left))?
        .with("cause", facts.cause.to_string())?
        .with("time", facts.at.to_string())?
        .with(
            "head",
            facts.head.map_or_else(|| "none".to_owned(), ToString::to_string),
        )
}

#[cfg(test)]
mod tests;
