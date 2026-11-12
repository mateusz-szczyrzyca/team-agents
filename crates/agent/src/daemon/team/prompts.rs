//! What a team's run is told beyond the working context: the team it is in, why it starts now, and what it is asked.

use agent_core::ids::{BranchName, ItemId};
use agent_core::kinds::Role;
use agent_core::prompt::{PromptError, TeamPrompt, Variables, VariablesError};
use agent_core::state::Team;
use agent_runtime::agentbus::DelegationRequest;
use agent_runtime::run::compose::{ComposeError, PromptTexts, item_sections};
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

/// Between two sections: a blank line, as in the prompt of a task.
const SECTION_SEPARATOR: &str = "\n\n";

/// What the names of nobody are written as.
const NOBODY: &str = "none";

/// The prompt of `run`, made from `composed`, the prompt of the task as it is without a team.
///
/// A session that starts is told the team after its prompt; a session that goes on already knows it. What starts the run
/// follows: a wake-up, an item, a request. A resumed session gets only that, because the rest is in it.
///
/// # Errors
/// [`TeamPromptError`].
pub(in crate::daemon) fn shape(
    texts: &TeamPrompts,
    harness: &PromptTexts<'_>,
    run: &TeamRun<'_>,
    composed: String,
) -> Result<String, TeamPromptError> {
    if !matches!(run.role, Role::Coordinator | Role::Member | Role::Worker) {
        return Ok(composed);
    }
    let reason = match run.why {
        None => Vec::new(),
        Some(Why::Woken) => vec![wake(texts, run)?],
        // The prompt of a session that starts names the item its member has; only a resumed one is told of a new item.
        Some(Why::Assigned(_)) if run.new_session => Vec::new(),
        Some(Why::Assigned(item)) => assignment(texts, harness, run, *item)?,
        Some(Why::Delegated(request)) => vec![delegation(texts, request)?],
    };
    let mut sections = Vec::new();
    if run.new_session {
        sections.push(composed);
        sections.push(context(texts, run)?);
    } else if reason.is_empty() {
        return Ok(composed);
    }
    sections.extend(reason);
    let mut prompt = sections
        .iter()
        .map(|section| section.trim_end())
        .collect::<Vec<_>>()
        .join(SECTION_SEPARATOR);
    prompt.push('\n');
    Ok(prompt)
}

/// The team the member is in: who, and on which branch it works.
fn context(texts: &TeamPrompts, run: &TeamRun<'_>) -> Result<String, TeamPromptError> {
    let crew = run.team.crew();
    let names = |role: Role| -> String {
        let named: Vec<&str> = crew
            .roster()
            .into_iter()
            .filter(|(of, _)| *of == role)
            .map(|(_, member)| member.agent().name())
            .collect();
        if named.is_empty() {
            NOBODY.to_owned()
        } else {
            named.join(", ")
        }
    };
    let values = Variables::new()
        .with("team", run.team.id().as_str())?
        .with("coordinator", names(Role::Coordinator))?
        .with("members", names(Role::Member))?
        .with("workers", names(Role::Worker))?
        .with("branch", run.branch.as_str())?
        .with("base", run.base.as_str())?;
    Ok(texts.render(TeamPrompt::Context, &values)?)
}

/// What a member whose sleep a message ended is told.
fn wake(texts: &TeamPrompts, run: &TeamRun<'_>) -> Result<String, TeamPromptError> {
    let values = Variables::new().with("team", run.team.id().as_str())?;
    Ok(texts.render(TeamPrompt::Wake, &values)?)
}

/// What a resumed member is told when it is given `item`: the new branch, then the item.
fn assignment(
    texts: &TeamPrompts,
    harness: &PromptTexts<'_>,
    run: &TeamRun<'_>,
    item: ItemId,
) -> Result<Vec<String>, TeamPromptError> {
    let listed = run
        .team
        .task_list()
        .list()
        .item(item)
        .ok_or(TeamPromptError::NoSuchItem { item })?;
    let values = Variables::new()
        .with("item_id", item.to_string())?
        .with("branch", run.branch.as_str())?
        .with("base", run.base.as_str())?;
    let mut sections = vec![texts.render(TeamPrompt::Assignment, &values)?];
    sections.extend(item_sections(harness, listed)?);
    Ok(sections)
}

/// What a worker is told when it is asked to serve `request`.
fn delegation(texts: &TeamPrompts, request: &DelegationRequest) -> Result<String, TeamPromptError> {
    let values = Variables::new()
        .with("requester", request.from.as_str())?
        .with("request_id", request.id.number().to_string())?
        .with("requester_worktree", request.worktree.display().to_string())?
        .with("requester_branch", request.branch.as_str())?
        .with("request", &request.request)?
        .with("context", &request.context)?;
    Ok(texts.render(TeamPrompt::Delegation, &values)?)
}

#[cfg(test)]
mod tests;
