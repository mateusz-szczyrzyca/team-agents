//! Durable lead-response validation, including bounded correction and restart recovery.
use super::super::teams::Look;
use super::super::{Owner, OwnerError};
/// Why a lead correction cannot be rendered.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ResponseTextError {
    /// A catalog variable is invalid.
    #[error("invalid fix response variable")]
    Variable(#[from] agent_core::prompt::VariablesError),
    /// The catalog text cannot be rendered.
    #[error("cannot render fix response text")]
    Render(#[from] agent_runtime::autopilot_prompts::RenderAutopilotTextError),
}
use super::{AutopilotNotice, Pulls};
use crate::daemon::team::Why;
use agent_core::ids::TeamId;
use agent_core::log::LogEvent;
use agent_core::prompt::{AutopilotPrompt, Variables};
use agent_core::state::{ItemPhase, RunEnd, RunState, Timestamp};
use agent_core::status::RunOutcome;

impl Owner {
    /// Validates the latest completed fix response once, without consuming another fix pass.
    pub(in crate::daemon::owner) fn check_fix_responses(
        &mut self,
        pulls: &Pulls,
        team_id: &TeamId,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let lead = self.lead_of(team_id)?;
        let Some(team) = self.parts.state.team(team_id) else {
            return Ok(());
        };
        let Some(run) = team.find(&lead.task).and_then(|(_, m)| m.runs().last()).cloned() else {
            return Ok(());
        };
        if !matches!(
            run.state(),
            RunState::Ended {
                end: RunEnd::Reported {
                    outcome: RunOutcome::Done
                },
                ..
            }
        ) {
            return Ok(());
        }
        let fixing: Vec<_> = team
            .items()
            .iter()
            .filter(|i| matches!(i.phase(), ItemPhase::Fixing { .. }))
            .map(|i| (i.id(), i.clone()))
            .collect();
        let mut explained = std::collections::HashSet::new();
        for (id, item) in fixing {
            let pass = crate::daemon::merge::count(item.fix_passes().len());
            let Some(mut ledger) = item.findings().cloned() else {
                continue;
            };
            let Some(disputes) = ledger.disputes() else {
                continue;
            };
            if disputes.answered_pass == pass || disputes.checked_run.as_ref() == Some(run.id()) {
                continue;
            }
            // An earlier DONE run is not a response to reviews requested after it started.
            if item
                .fix_passes()
                .last()
                .is_some_and(|p| p.requested_by.iter().any(|r| r.requested_at > run.started_at()))
            {
                continue;
            }
            let message = ledger
                .verification()
                .and_then(|v| v.lead_message.as_deref())
                .unwrap_or_default()
                .to_owned();
            let response = ledger.answer_lead(&message, pass);
            let correction = match response {
                Ok(responses) => {
                    if let ItemPhase::Fixing { pr, .. } = item.phase()
                        && explained.insert(pr.number)
                    {
                        self.record_explanations(team_id, &item, &lead, responses);
                    }
                    None
                }
                Err(problem) => Some(response_correction(pulls, &problem)?),
            };
            let Some(disputes) = ledger.disputes_mut() else {
                continue;
            };
            disputes.checked_run = Some(run.id().clone());
            if let Some(text) = &correction {
                disputes.bounces = disputes.bounces.saturating_add(1);
                disputes.correction = Some(text.clone());
            }
            let bounces = disputes.bounces;
            let mut team = self
                .parts
                .state
                .team_mut(team_id)
                .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
            let mut item = team.item_mut(id).ok_or_else(|| OwnerError::LostItem {
                team: team_id.clone(),
                item: id,
            })?;
            if let Some(saved) = item.findings_mut() {
                *saved = ledger;
            }
            look.changed = true;
            if let Some(text) = correction {
                tracing::warn!(event = LogEvent::ChecksDecided.as_str(), task = %lead.task, pass, bounces, "invalid lead finding response: verification remains held");
                if bounces <= super::verdicts::MAX_VERDICT_BOUNCES {
                    self.release(&lead.task, Why::Gate(text), look)?;
                } else {
                    let values = Variables::new()
                        .with("team", team_id.as_str())
                        .and_then(|v| v.with("items", id.to_string()))
                        .and_then(|v| v.with("reason", text));
                    let now = Timestamp::now(&self.parts.clock)?;
                    self.tell_human(pulls, AutopilotNotice::PublishFailed, values, &lead, now);
                }
            }
        }
        Ok(())
    }
    /// Keeps each new explanation in the round history before verification.
    fn record_explanations(
        &mut self,
        team_id: &TeamId,
        item: &agent_core::state::ItemProgress,
        lead: &super::Lead,
        responses: Vec<(std::num::NonZeroU32, agent_core::review_response::Response)>,
    ) {
        let pass = crate::daemon::merge::count(item.fix_passes().len());
        if let ItemPhase::Fixing { pr, .. } = item.phase() {
            let items: Vec<_> = self
                .parts
                .state
                .team(team_id)
                .into_iter()
                .flat_map(|team| team.items())
                .filter(|item| matches!(item.phase(), ItemPhase::Fixing { pr: known, .. } if known.number == pr.number))
                .map(agent_core::state::ItemProgress::id)
                .collect();
            let rounds = self.rounds_of_items(team_id, &items);
            for (number, response) in responses {
                match response {
                    agent_core::review_response::Response::Fixed => {}
                    agent_core::review_response::Response::Explained(why) => self.record_event(
                        team_id,
                        rounds.clone(),
                        agent_core::history::HistoryEvent::FindingExplained {
                            pr: pr.number,
                            number,
                            pass,
                            by: lead.agent.clone(),
                            why,
                        },
                    ),
                }
            }
        }
    }
}

/// Renders a precise correction through the harness catalog.
fn response_correction(
    pulls: &Pulls,
    problem: &agent_core::review_response::ResponseError,
) -> Result<String, ResponseTextError> {
    let values = Variables::new()
        .with("line", problem.line.to_string())?
        .with("number", problem.number.to_string())?
        .with("allowed", problem.allowed)?;
    Ok(pulls.texts.render(AutopilotPrompt::ReviewResponseInvalid, &values)?)
}
