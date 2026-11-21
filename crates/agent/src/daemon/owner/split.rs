//! Durable coordinator proposals, whole-plan approval and bounded corrections.

use agent_core::decide::split::approval::{AskReason, ProposalDecision, ProposalFault, ProposalRead, decide_proposal};
use agent_core::history::{HistoryEvent, SplitDecision};
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

/// The two task transitions a proposal can request.
#[derive(Debug)]
enum ProposalEnd {
    /// Continue the coordinator after approval or correction.
    Queued,
    /// Wait for the existing human question.
    Questions,
}

impl From<ProposalEnd> for Settled {
    fn from(end: ProposalEnd) -> Self {
        match end {
            ProposalEnd::Queued => Self::Status(TaskStatus::Queued),
            ProposalEnd::Questions => Self::Questions,
        }
    }
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
        let keeping = team.split_proposal(item).cloned();
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
        let keeping = match keeping {
            Some(mut saved) => {
                saved.resubmit(text).map_err(ProposalError::Form)?;
                saved
            }
            None => ProposalKeeping::new(text).map_err(ProposalError::Form)?,
        };
        self.keep_proposal(&located.team, item, keeping.clone())?;
        self.save().await?;
        self.decide_split(task, located, item, keeping)
            .await
            .map(|end| Some(end.into()))
    }

    /// Applies one saved pending proposal; all outcomes are committed with the task status.
    async fn decide_split(
        &mut self,
        task: &TaskId,
        located: &Located,
        item: ItemId,
        mut keeping: ProposalKeeping,
    ) -> Result<ProposalEnd, OwnerError> {
        let parsed = read_proposal(keeping.text());
        let read = proposal_read(&parsed, item, task);
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
                    .activate_split(plan.with_approval(agent_core::state::split::SplitApproval::Harness))?;
                self.record_split_decision(task, located, item, &keeping, SplitDecision::Approved, String::new());
                Ok(ProposalEnd::Queued)
            }
            ProposalDecision::SendBack(fault) => {
                let reason = fault_text(&self.parts.team_texts, &fault)?;
                keeping.send_back(fault);
                self.record_split_decision(task, located, item, &keeping, SplitDecision::Corrected, reason);
                self.keep_proposal(&located.team, item, keeping)?;
                Ok(ProposalEnd::Queued)
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
                    Err(_) => unreadable_example(keeping.text(), item),
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
                self.record_split_decision(task, located, item, &keeping, SplitDecision::Asked, reason);
                self.keep_proposal(&located.team, item, keeping)?;
                Ok(ProposalEnd::Questions)
            }
        }
    }

    /// Captures the policy outcome in the structured log and the append-only history.
    fn record_split_decision(
        &self,
        task: &TaskId,
        located: &Located,
        item: ItemId,
        keeping: &ProposalKeeping,
        decision: SplitDecision,
        reason: String,
    ) {
        agent_runtime::history::log_decision(
            "split_proposal",
            &serde_json::json!({"task": task, "item": item, "corrections": keeping.corrections()}),
            &serde_json::json!({"decision": decision, "reason": reason}),
        );
        let question = match keeping.stage() {
            ProposalStage::Asked { question } => Some(*question),
            ProposalStage::Pending | ProposalStage::Correction { .. } => None,
        };
        self.record_event(
            &located.team,
            self.rounds_of_tasks(std::slice::from_ref(task)),
            HistoryEvent::SplitDecision {
                task: task.clone(),
                item,
                decision,
                corrections: keeping.corrections(),
                question,
                reason,
            },
        );
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
                ProposalEnd::Questions => match self.ask(&task, &located, now, &mut events).await? {
                    Asked::Recorded(notice) => (TaskStatus::Questions, Some(notice)),
                    Asked::Unusable(notice) => (TaskStatus::Failed, Some(notice)),
                },
                ProposalEnd::Queued => (TaskStatus::Queued, None),
            };
            self.move_to(&task, &located.team, status, &mut events)?;
            self.commit(&task, &located, now, events, notice).await?;
        }
        Ok(())
    }
}

/// The approving-verdict example for a submission that cannot be read: the parts it contains, cut down to the
/// fields an approving verdict accepts, so the human can correct and approve them.
fn unreadable_example(text: &str, item: ItemId) -> serde_json::Value {
    /// The part fields an approving verdict accepts; anything else would make the verdict invalid.
    const VERDICT_FIELDS: [&str; 6] = ["title", "delivers", "checks", "depends_on", "estimate", "tier"];
    let parts: Vec<serde_json::Value> = match serde_json::from_str::<serde_json::Value>(text) {
        Ok(value) => value
            .get("parts")
            .and_then(serde_json::Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(serde_json::Value::as_object)
                    .map(|part| {
                        serde_json::Value::Object(
                            part.iter()
                                .filter(|(name, _)| VERDICT_FIELDS.contains(&name.as_str()))
                                .map(|(name, field)| (name.clone(), field.clone()))
                                .collect(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        // Text that is not JSON has no parts to keep; the question quotes it as submitted.
        Err(_) => Vec::new(),
    };
    serde_json::json!({"parent": item, "parts": parts})
}

/// Preserves a known faulty part identity while retaining the parser's cause in diagnostics.
fn proposal_read<'a>(
    parsed: &'a Result<agent_core::state::split::proposal::Proposal, ReadProposalError>,
    item: ItemId,
    task: &TaskId,
) -> ProposalRead<'a> {
    match parsed {
        Ok(proposal) if proposal.parent() == item => ProposalRead::Parsed(proposal),
        Ok(_) => ProposalRead::Unreadable { part: None },
        Err(error) => {
            // The diagnostic is retained in the log while the coordinator receives a catalog correction.
            tracing::warn!(task = %task, error = %agent_runtime::process::error_chain(error), "split proposal does not read");
            let part = match error {
                ReadProposalError::Estimate { part } => part
                    .checked_sub(1)
                    .and_then(agent_core::state::split::PartKey::at_position),
                ReadProposalError::TooLarge | ReadProposalError::Json { .. } => None,
            };
            ProposalRead::Unreadable { part }
        }
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
