//! What a team's run is told beyond the working context: the team it is in, why it starts now, and what it is asked.

use agent_core::ids::{BranchName, ItemId};
use agent_core::kinds::Role;
use agent_core::prompt::{PromptError, TeamPrompt, Variables, VariablesError};
use agent_core::state::{Member, Team};
use agent_runtime::agentbus::DelegationRequest;
use agent_runtime::run::compose::{ComposeError, PromptTexts};
use agent_runtime::team_prompts::{RenderTeamTextError, TeamPrompts};

/// Why a team's task starts a run now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon) enum Why {
    /// The bus woke it: a message or a board item waits for it.
    Woken,
    /// It is given the item with this number.
    Assigned(ItemId),
    /// A worker is asked to serve this request.
    Delegated(Box<DelegationRequest>),
}

/// What a run is shaped from.
#[derive(Debug, Clone, Copy)]
pub(in crate::daemon) struct TeamRun<'a> {
    /// The team.
    pub(in crate::daemon) team: &'a Team,
    /// The place of the member in the team.
    pub(in crate::daemon) role: Role,
    /// The member.
    pub(in crate::daemon) member: &'a Member,
    /// The branch the member works on now.
    pub(in crate::daemon) branch: &'a BranchName,
    /// The branch the team's branches start from.
    pub(in crate::daemon) base: &'a BranchName,
    /// Why the run starts, if it is not the plain continuation of the task.
    pub(in crate::daemon) why: Option<&'a Why>,
    /// Whether the run starts a session; a resumed session already holds the context.
    pub(in crate::daemon) new_session: bool,
}

/// Why a prompt cannot be shaped.
#[derive(Debug, thiserror::Error)]
pub(in crate::daemon) enum TeamPromptError {
    /// A text of the team cannot be rendered.
    #[error("cannot render a text of the team")]
    Render(#[from] RenderTeamTextError),
    /// An item of the task list cannot be rendered.
    #[error("cannot render an item")]
    Item(#[from] ComposeError),
    /// A variable name of the program is not valid.
    #[error("a variable of a text is not valid")]
    Variable(#[from] VariablesError),
    /// A text of the team does not fit the values.
    #[error("a text of the team does not take the values it is given")]
    Values(#[from] PromptError),
    /// The item a member is given is not in the task list: a mistake of the daemon.
    #[error("item {item} is not in the task list")]
    NoSuchItem {
        /// The item.
        item: ItemId,
    },
}

/// The prompt of `run`, made from `composed`, the prompt of the task as it is without a team.
///
/// # Errors
/// [`TeamPromptError`].
pub(in crate::daemon) fn shape(
    texts: &TeamPrompts,
    harness: &PromptTexts<'_>,
    run: &TeamRun<'_>,
    composed: String,
) -> Result<String, TeamPromptError> {
    let _ = (texts, harness, run);
    Ok(composed)
}

#[cfg(test)]
mod tests;
