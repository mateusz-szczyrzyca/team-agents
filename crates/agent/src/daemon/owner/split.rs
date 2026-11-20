//! Durable coordinator proposals, whole-plan approval and bounded corrections.

use agent_core::decide::split::approval::{AskReason, ProposalDecision, ProposalFault, ProposalRead, decide_proposal};
use agent_core::ids::{ItemId, TaskId, TeamId};
use agent_core::kinds::Role;
use agent_core::prompt::{TeamPrompt, Variables, VariablesError};
use agent_core::questions::{MAX_QUESTIONS_BYTES, quote};
use agent_core::state::split::keeping::{ProposalKeeping, ProposalStage};
use agent_core::state::split::proposal::{ReadProposalError, ScopeChange, read_proposal};
use agent_core::state::{RecordSplitError, Timestamp};
use agent_core::status::TaskStatus;
use agent_runtime::team_prompts::{RenderTeamTextError, TeamPrompts};
use agent_runtime::text_file::{ReplaceError, read_text_file, replace_text_file};

use super::{Asked, Located, Owner, OwnerError};
use crate::daemon::settle::Settled;

/// A proposal decision could not be persisted or communicated.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ProposalError {
    /// Saved input exceeds its bound.
    #[error("cannot keep the proposal")]
    Form(#[from] ReadProposalError),
    /// The parent cannot hold another proposal.
    #[error("cannot record the proposal")]
    Keep(#[from] RecordSplitError),
    /// Values supplied to a catalog text are invalid.
    #[error("cannot prepare the proposal text")]
    Variables(#[from] VariablesError),
    /// A catalog text cannot be rendered.
    #[error("cannot render the proposal text")]
    Render(#[from] RenderTeamTextError),
    /// The example cannot be serialized.
    #[error("cannot serialize the approval example")]
    Example(#[from] serde_json::Error),
    /// The questions file cannot be replaced atomically.
    #[error("cannot write the split question")]
    Question(#[from] ReplaceError),
}

impl Owner {
    /// Recognizes explicit submissions and durably saves input before deciding.
    pub(super) async fn split_at_end(
        &mut self,
        task: &TaskId,
        located: &Located,
        message: Option<&str>,
    ) -> Result<Option<Settled>, OwnerError> {
        if located.role != Role::Coordinator {
            return Ok(None);
        }
        let team = self
            .parts
            .state
            .team(&located.team)
            .ok_or_else(|| OwnerError::LostTeam {
                team: located.team.clone(),
            })?;
        let submitted = team
            .items()
            .iter()
            .find(|item| {
                !team.splits().iter().any(|plan| plan.parent() == item.id())
                    && message
                        .is_some_and(|text| text.lines().any(|line| line.trim() == format!("SPLIT: {}", item.id())))
            })
            .map(agent_core::state::ItemProgress::id);
        let Some(item) = submitted else { return Ok(None) };
        let mut keeping = team.split_proposal(item).cloned();
        let path = self
            .parts
            .store
            .home()
            .task_dir(task)
            .join(format!("split-proposal-{item}.json"));
        let text = match read_text_file(&path, MAX_QUESTIONS_BYTES).await {
            Ok(Some(file)) => file.text,
            Ok(None) => String::new(),
            Err(error) => {
                // Unreadable submissions receive the same bounded correction as malformed JSON.
                tracing::warn!(path = %path.display(), error = %agent_runtime::process::error_chain(&error), "split proposal is unreadable");
                String::new()
            }
        };
        match &mut keeping {
            Some(saved) => saved.resubmit(text).map_err(ProposalError::Form)?,
            None => keeping = Some(ProposalKeeping::new(text).map_err(ProposalError::Form)?),
        }
        let keeping = keeping.ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
        self.keep_proposal(&located.team, item, keeping.clone())?;
        self.save().await?;
        self.decide_split(task, located, item, keeping).await.map(Some)
    }

    /// Applies one saved pending proposal; all outcomes are committed with the task status.
    async fn decide_split(
        &mut self,
        task: &TaskId,
        located: &Located,
        item: ItemId,
        mut keeping: ProposalKeeping,
    ) -> Result<Settled, OwnerError> {
        let parsed = read_proposal(keeping.text());
        let read = match &parsed {
            Ok(proposal) if proposal.parent() == item => ProposalRead::Parsed(proposal),
            Ok(_) => ProposalRead::Unreadable { part: None },
            Err(error) => {
                // The diagnostic is retained in the log while the coordinator receives a catalog correction.
                tracing::warn!(task = %task, error = %agent_runtime::process::error_chain(error), "split proposal does not read");
                ProposalRead::Unreadable { part: None }
            }
        };
        let decision = decide_proposal(
            read,
            self.parts.settings.config.split(),
            self.parts.settings.config.split_unit(),
            keeping.corrections(),
        );
        match decision {
            ProposalDecision::Approve(plan) => {
                self.parts
                    .state
                    .team_mut(&located.team)
                    .ok_or_else(|| OwnerError::LostTeam {
                        team: located.team.clone(),
                    })?
                    .activate_split(plan)?;
                Ok(Settled::Status(TaskStatus::Queued))
            }
            ProposalDecision::SendBack(fault) => {
                keeping.send_back(fault);
                self.keep_proposal(&located.team, item, keeping)?;
                Ok(Settled::Status(TaskStatus::Queued))
            }
            ProposalDecision::AskHuman(reason) => {
                let number = super::start::next_number(&self.member_mut(task)?.1)
                    .ok_or_else(|| OwnerError::QuestionNumbers { task: task.clone() })?;
                let reason = match reason {
                    AskReason::CorrectionsUsed(fault) => fault_text(&self.parts.team_texts, &fault)?,
                    AskReason::ScopeChanged(change) => match change {
                        ScopeChange::Less { reason }
                        | ScopeChange::More { reason }
                        | ScopeChange::Different { reason } => reason,
                        ScopeChange::Unchanged => String::new(),
                    },
                };
                // The legacy approving verdict has no scope field; its parts retain estimate and tier.
                let mut example = match parsed {
                    Ok(proposal) => serde_json::to_value(proposal).map_err(ProposalError::Example)?,
                    Err(_) => serde_json::json!({"parent": item, "parts": []}),
                };
                if let Some(object) = example.as_object_mut() {
                    object.remove("scope");
                }
                let example = serde_json::to_string_pretty(&example).map_err(ProposalError::Example)?;
                let values = Variables::new()
                    .with("task", task.as_str())
                    .and_then(|v| v.with("item", item.to_string()))
                    .and_then(|v| v.with("reason", quote(&reason)))
                    .and_then(|v| v.with("proposal", quote(keeping.text())))
                    .and_then(|v| v.with("example", example))
                    .map_err(ProposalError::Variables)?;
                let text = self
                    .parts
                    .team_texts
                    .render(TeamPrompt::NoticeSplitQuestion, &values)
                    .map_err(ProposalError::Render)?;
                replace_text_file(&self.parts.store.home().questions_file(task, number), &text, 0o600)
                    .await
                    .map_err(ProposalError::Question)?;
                keeping.asked(number);
                self.keep_proposal(&located.team, item, keeping)?;
                Ok(Settled::Questions)
            }
        }
    }

    /// Keeps a complete pending input or decision without changing the approved plan.
    fn keep_proposal(&mut self, team: &TeamId, item: ItemId, keeping: ProposalKeeping) -> Result<(), OwnerError> {
        self.parts
            .state
            .team_mut(team)
            .ok_or_else(|| OwnerError::LostTeam { team: team.clone() })?
            .keep_split_proposal(item, keeping)
            .map_err(ProposalError::Keep)?;
        Ok(())
    }

    /// Resumes decisions saved before a crash, never replaying a correction or an existing question.
    pub(super) async fn resume_split_proposals(&mut self, team: &TeamId) -> Result<(), OwnerError> {
        let Some(found) = self.parts.state.team(team) else {
            return Ok(());
        };
        let pending: Vec<_> = found
            .items()
            .iter()
            .filter_map(|item| {
                found
                    .split_proposal(item.id())
                    .filter(|saved| matches!(saved.stage(), ProposalStage::Pending))
                    .map(|saved| (item.id(), saved.clone()))
            })
            .collect();
        let Some(coordinator) = found.crew().coordinator() else {
            return Ok(());
        };
        let task = coordinator.task().clone();
        for (item, saved) in pending {
            let located = self.locate(&task)?;
            let settled = self.decide_split(&task, &located, item, saved).await?;
            let now = Timestamp::now(&self.parts.clock)?;
            let mut events = Vec::new();
            let (status, notice) = match settled {
                Settled::Questions => match self.ask(&task, &located, now, &mut events).await? {
                    Asked::Recorded(notice) => (TaskStatus::Questions, Some(notice)),
                    Asked::Unusable(notice) => (TaskStatus::Failed, Some(notice)),
                },
                Settled::Status(status) => (status, None),
                _ => return Err(OwnerError::LostTask { task: task.clone() }),
            };
            self.move_to(&task, &located.team, status, &mut events)?;
            self.commit(&task, &located, now, events, notice).await?;
        }
        Ok(())
    }
}

/// Renders the catalog's explanation for one policy fault.
fn fault_text(texts: &TeamPrompts, fault: &ProposalFault) -> Result<String, ProposalError> {
    let part = |part: &agent_core::state::split::PartKey| Variables::new().with("part", part.to_string());
    let (id, values) = match fault {
        ProposalFault::Unreadable { part: key } => (
            TeamPrompt::SplitUnreadable,
            Variables::new().with("part", key.map_or_else(|| "unknown".to_owned(), |key| key.to_string()))?,
        ),
        ProposalFault::MissingChecks { part: key } => (TeamPrompt::SplitChecks, part(key)?),
        ProposalFault::Cycle { part: key } => (TeamPrompt::SplitCycle, part(key)?),
        ProposalFault::FirstPartDepends { part: key } => (TeamPrompt::SplitFirst, part(key)?),
        ProposalFault::UnknownDependency { part: key, dependency } => (
            TeamPrompt::SplitUnknown,
            part(key)?.with("dependency", dependency.to_string())?,
        ),
        ProposalFault::DuplicateDependency { part: key, dependency } => (
            TeamPrompt::SplitDuplicate,
            part(key)?.with("dependency", dependency.to_string())?,
        ),
        ProposalFault::EmptyTitle { part: key } => (TeamPrompt::SplitTitle, part(key)?),
        ProposalFault::EmptyDeliverable { part: key } => (TeamPrompt::SplitDeliverable, part(key)?),
        ProposalFault::PartCount { count } => (
            TeamPrompt::SplitCount,
            Variables::new().with("count", count.to_string())?,
        ),
        ProposalFault::AboveLimit {
            part: key,
            estimate,
            limit,
        } => (
            TeamPrompt::SplitLimit,
            part(key)?
                .with("amount", estimate.amount().to_string())?
                .with("unit", estimate.unit())?
                .with("limit", limit.to_string())?,
        ),
    };
    Ok(texts.render(id, &values)?)
}

/// Adds a saved correction to the next coordinator prompt.
pub(super) fn with_correction(
    texts: &TeamPrompts,
    team: &agent_core::state::Team,
    role: Role,
    prompt: String,
) -> Result<String, ProposalError> {
    if role != Role::Coordinator {
        return Ok(prompt);
    }
    let mut prompt = prompt;
    for item in team.items() {
        if let Some(saved) = team.split_proposal(item.id())
            && let ProposalStage::Correction { fault } = saved.stage()
        {
            let values = Variables::new()
                .with("item", item.id().to_string())?
                .with("reason", fault_text(texts, fault)?)?;
            let text = texts.render(TeamPrompt::SplitCorrection, &values)?;
            prompt.push_str("\n\n");
            prompt.push_str(&text);
        }
    }
    Ok(prompt)
}

#[cfg(test)]
#[path = "split/tests.rs"]
mod tests;
