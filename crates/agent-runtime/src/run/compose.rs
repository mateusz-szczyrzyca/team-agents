//! The prompt of a run: its sections, each rendered from its template, joined.
//!
//! A first run gets the working context, its role, the rules of its tier, the brief of the task list, its item and
//! its last checkpoint; a run that resumes a session gets what the interruption was, because the session already holds
//! the rest. No wording lives here: every section is a template of the catalogs (`prompts/harness/run/`,
//! `prompts/agent/` and its siblings), and this module only chooses and fills them.

use std::num::NonZeroU32;
use std::path::Path;

use agent_core::config::{Minutes, RepoName};
use agent_core::ids::TaskId;
use agent_core::kinds::{Role, Tier};
use agent_core::prompt::{HarnessPrompt, PromptId, Variables, VariablesError};
use agent_core::task_list::Item;

use crate::harness_prompts::{HarnessPrompts, RenderTextError};
use crate::prompts::{PromptLoader, RenderPromptError};

/// Why a run is resumed instead of started: what its prompt tells the agent about the interruption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeReason {
    /// The watchdog killed the last run after this long without activity.
    Hang {
        /// The watchdog's limit.
        after: Minutes,
    },
    /// The harness was restarted while the run was going.
    Restart,
    /// The run stopped on a provider's limit, which is over now.
    Limit,
}

/// Whether the prompt starts a session or continues one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    /// A new session: the whole context.
    First,
    /// A session that goes on: only what happened.
    Resume(ResumeReason),
}

/// What a prompt is made of.
#[derive(Debug, Clone, Copy)]
pub struct PromptInput<'a> {
    /// Starting or resuming.
    pub kind: PromptKind,
    /// The agent's name.
    pub agent: &'a str,
    /// The agent's tier: it decides which developer rules it gets.
    pub tier: Tier,
    /// The agent's place in its team.
    pub role: Role,
    /// The task.
    pub task: &'a TaskId,
    /// The repository.
    pub repo: &'a RepoName,
    /// The git worktree the agent works in.
    pub worktree: &'a Path,
    /// The file the agent writes its next round of questions to.
    pub questions_file: &'a Path,
    /// The number of that round.
    pub round: NonZeroU32,
    /// The brief of the task list; may be empty.
    pub brief: &'a str,
    /// The item the agent works on, if it has one.
    pub item: Option<&'a Item>,
    /// The agent's last checkpoint, if there is one.
    pub checkpoint: Option<&'a str>,
}

/// The two loaded catalogs a prompt is made from.
#[derive(Debug, Clone, Copy)]
pub struct PromptTexts<'a> {
    /// The run prompts of the harness.
    pub harness: &'a HarnessPrompts,
    /// The rules of the developer and of the reviewer.
    pub rules: &'a PromptLoader,
}

/// Why a prompt could not be composed.
#[derive(Debug, thiserror::Error)]
pub enum ComposeError {
    /// A text of the harness could not be rendered.
    #[error("cannot render a text of the harness")]
    Harness(#[from] RenderTextError),
    /// The rules could not be rendered.
    #[error("cannot render the rules")]
    Rules(#[from] RenderPromptError),
    /// A variable name of the program is not valid.
    #[error("a variable of a text is not valid")]
    Variable(#[from] VariablesError),
}

/// The prompt of `input`.
///
/// # Errors
/// [`ComposeError`] when a text does not take the values this module gives it: a mistake of the installation's
/// prompt files that [`HarnessPrompts::load`] checks at start, or of the program.
pub fn compose(texts: &PromptTexts<'_>, input: &PromptInput<'_>) -> Result<String, ComposeError> {
    let _ = (texts, input);
    Ok(String::new())
}
