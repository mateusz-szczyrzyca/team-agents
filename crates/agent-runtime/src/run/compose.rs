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
    let mut sections = match input.kind {
        PromptKind::First => first_sections(texts, input)?,
        PromptKind::Resume(reason) => vec![resume_section(texts, reason)?],
    };
    if let Some(checkpoint) = input.checkpoint {
        let values = Variables::new().with("checkpoint", checkpoint)?;
        sections.push(texts.harness.render(HarnessPrompt::Checkpoint, &values)?);
    }
    let mut prompt = sections
        .iter()
        .map(|section| section.trim_end())
        .collect::<Vec<_>>()
        .join(SECTION_SEPARATOR);
    prompt.push('\n');
    Ok(prompt)
}

/// Between two sections: a blank line.
const SECTION_SEPARATOR: &str = "\n\n";

/// The sections of a first run, except the checkpoint.
fn first_sections(texts: &PromptTexts<'_>, input: &PromptInput<'_>) -> Result<Vec<String>, ComposeError> {
    let context = Variables::new()
        .with("agent", input.agent)?
        .with("task", input.task.as_str())?
        .with("repo", input.repo.as_str())?
        .with("worktree", input.worktree.display().to_string())?
        .with("questions_file", input.questions_file.display().to_string())?
        .with("round", input.round.to_string())?;
    let mut sections = vec![
        texts.harness.render(HarnessPrompt::Context, &context)?,
        texts.harness.render(role_prompt(input.role), &Variables::new())?,
        texts
            .rules
            .render(rules_prompt(input.role, input.tier), &Variables::new())?,
    ];
    if !input.brief.trim().is_empty() {
        let values = Variables::new().with("brief", input.brief)?;
        sections.push(texts.harness.render(HarnessPrompt::Brief, &values)?);
    }
    if let Some(item) = input.item {
        sections.extend(item_sections(texts, item)?);
    }
    Ok(sections)
}

/// The sections of an item: the item itself, then its expected result and its checks, those it has.
fn item_sections(texts: &PromptTexts<'_>, item: &Item) -> Result<Vec<String>, ComposeError> {
    let values = Variables::new()
        .with("item_id", item.id.to_string())?
        .with("item_title", &item.title)?
        .with("item_description", &item.description)?;
    let mut sections = vec![texts.harness.render(HarnessPrompt::Item, &values)?];
    if let Some(expected) = &item.expected_result {
        let values = Variables::new().with("expected", expected)?;
        sections.push(texts.harness.render(HarnessPrompt::ItemExpected, &values)?);
    }
    if let Some(checks) = &item.checks {
        let values = Variables::new().with("checks", checks)?;
        sections.push(texts.harness.render(HarnessPrompt::ItemChecks, &values)?);
    }
    Ok(sections)
}

/// What a resumed run is told about its interruption.
fn resume_section(texts: &PromptTexts<'_>, reason: ResumeReason) -> Result<String, ComposeError> {
    let section = match reason {
        ResumeReason::Hang { after } => {
            let minutes = after.as_duration().as_secs() / SECS_PER_MINUTE;
            let values = Variables::new().with("minutes", minutes.to_string())?;
            texts.harness.render(HarnessPrompt::ResumeHang, &values)?
        }
        ResumeReason::Restart => texts.harness.render(HarnessPrompt::ResumeRestart, &Variables::new())?,
        ResumeReason::Limit => texts.harness.render(HarnessPrompt::ResumeLimit, &Variables::new())?,
    };
    Ok(section)
}

/// Seconds in a minute.
const SECS_PER_MINUTE: u64 = 60;

/// The text that tells an agent its role.
const fn role_prompt(role: Role) -> HarnessPrompt {
    match role {
        Role::Solo => HarnessPrompt::RoleSolo,
        Role::Coordinator => HarnessPrompt::RoleCoordinator,
        Role::Member => HarnessPrompt::RoleMember,
        Role::Worker => HarnessPrompt::RoleWorker,
        Role::Reviewer => HarnessPrompt::RoleReviewer,
    }
}

/// The rules an agent works by: a reviewer the review rules, everyone else the developer rules of its tier.
const fn rules_prompt(role: Role, tier: Tier) -> PromptId {
    match (role, tier) {
        (Role::Reviewer, _) => PromptId::InvestigatorCodeReview,
        (Role::Solo | Role::Coordinator | Role::Member | Role::Worker, Tier::Normal) => PromptId::AgentRustDeveloper,
        (Role::Solo | Role::Coordinator | Role::Member | Role::Worker, Tier::Worker) => {
            PromptId::WorkerAgentRustDeveloper
        }
        (Role::Solo | Role::Coordinator | Role::Member | Role::Worker, Tier::Investigator) => {
            PromptId::InvestigatorRustDeveloper
        }
    }
}
