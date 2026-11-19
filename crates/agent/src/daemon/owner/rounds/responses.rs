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
use agent_core::review_response::opinion::{LeadAnswer, OpinionError, Permission};
use agent_core::state::{ItemPhase, RunEnd, RunState, Timestamp};
use agent_core::status::RunOutcome;

impl Owner {
    /// Reads the handed-over tip before accepting a second-opinion response.
    pub(in crate::daemon::owner) async fn read_fix_responses(
        &mut self,
        pulls: &Pulls,
        team: &TeamId,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let lead = self.lead_of(team)?;
        let rotating = self.parts.state.team(team).is_some_and(|team| {
            team.items().iter().any(|item| {
                matches!(item.phase(), ItemPhase::Fixing { .. })
                    && item.findings().is_some_and(|ledger| ledger.verification().is_some())
            })
        });
        if !rotating {
            return self.check_fix_responses(pulls, team, look);
        }
        let head = self
            .worktree_git(&lead.repo, &lead.task, &lead.agent)
            .map_err(super::OpinionDaemonError::from)?
            .rev_parse(&agent_runtime::git::Revision::Head)
            .await
            .map_err(super::OpinionDaemonError::from)?;
        self.check_responses(pulls, team, look, Some(&head))
    }

    /// Validates legacy responses without the independent-opinion protocol.
    pub(in crate::daemon::owner) fn check_fix_responses(
        &mut self,
        pulls: &Pulls,
        team: &TeamId,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        self.check_responses(pulls, team, look, None)
    }

    /// Validates the latest completed fix response once, without consuming another fix pass.
    fn check_responses(
        &mut self,
        pulls: &Pulls,
        team_id: &TeamId,
        look: &mut Look,
        head: Option<&agent_core::state::CommitId>,
    ) -> Result<(), OwnerError> {
        let lead = self.lead_of(team_id)?;
        let Some(team) = self.parts.state.team(team_id) else {
            return Ok(());
        };
        let Some(run) = team
            .find(&lead.task)
            .and_then(|(_, member)| member.runs().last())
            .filter(|run| {
                matches!(
                    run.state(),
                    RunState::Ended {
                        end: RunEnd::Reported {
                            outcome: RunOutcome::Done
                        },
                        ..
                    }
                )
            })
            .cloned()
        else {
            return Ok(());
        };
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
            let answer = fix_answer(
                pulls,
                &mut ledger,
                &message,
                pass,
                head,
                self.parts.settings.config.review().second_opinion(),
            )?;
            let correction = match answer {
                FixAnswer::Accepted(responses) => {
                    if let ItemPhase::Fixing { pr, .. } = item.phase()
                        && explained.insert(pr.number)
                    {
                        self.record_explanations(team_id, &item, &lead, responses);
                    }
                    None
                }
                FixAnswer::Correction(text) => Some(text),
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
                self.correct_response(pulls, team_id, id, text, bounces, look)?;
            }
        }
        Ok(())
    }
    /// Returns an invalid response within the shared correction budget, or notifies the human.
    fn correct_response(
        &mut self,
        pulls: &Pulls,
        team_id: &TeamId,
        id: agent_core::ids::ItemId,
        text: String,
        bounces: u32,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let lead = self.lead_of(team_id)?;
        tracing::warn!(event = LogEvent::ChecksDecided.as_str(), task = %lead.task, bounces, "invalid lead finding response: verification remains held");
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
                .flat_map(agent_core::state::Team::items)
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
    use agent_core::review_response::Correction;
    let instruction = match problem.correction {
        Correction::OnePerFinding => AutopilotPrompt::ReviewResponseNumbers,
        Correction::FixedOrExplained => AutopilotPrompt::ReviewResponseExplain,
        Correction::FixedOnly => AutopilotPrompt::ReviewResponseFixed,
    };
    let allowed = pulls.texts.render(instruction, &Variables::new())?;
    let values = Variables::new()
        .with("line", problem.line.to_string())?
        .with("number", problem.number.to_string())?
        .with("allowed", allowed.trim_end())?;
    Ok(pulls.texts.render(AutopilotPrompt::ReviewResponseInvalid, &values)?)
}

/// Renders the alternative available for an unused enabled blocker.
fn opinion_correction(pulls: &Pulls, line: usize, number: std::num::NonZeroU32) -> Result<String, ResponseTextError> {
    let allowed = pulls
        .texts
        .render(AutopilotPrompt::ReviewResponseOpinion, &Variables::new())?;
    Ok(pulls.texts.render(
        AutopilotPrompt::ReviewResponseInvalid,
        &Variables::new()
            .with("line", line.to_string())?
            .with("number", number.to_string())?
            .with("allowed", allowed.trim_end())?,
    )?)
}

/// Validates a complete message and selects its precise correction text.
fn fix_answer(
    pulls: &Pulls,
    ledger: &mut agent_core::state::Findings,
    message: &str,
    pass: u32,
    head: Option<&agent_core::state::CommitId>,
    permission: Permission,
) -> Result<FixAnswer, OwnerError> {
    let response = match head {
        Some(head) => ledger
            .answer_opinions(
                message,
                std::num::NonZeroU32::new(pass).ok_or(agent_core::state::FindingNumbersExhausted::Exhausted)?,
                head,
                permission,
            )
            .map(|answers| {
                answers
                    .into_iter()
                    .filter_map(|(number, answer)| match answer {
                        LeadAnswer::Fixed => Some((number, agent_core::review_response::Response::Fixed)),
                        LeadAnswer::Explained(why) => {
                            Some((number, agent_core::review_response::Response::Explained(why)))
                        }
                        LeadAnswer::SecondOpinion(_) => None,
                    })
                    .collect()
            }),
        None => ledger.answer_lead(message, pass).map_err(OpinionError::from),
    };
    let correction = match response {
        Ok(responses) => {
            return Ok(FixAnswer::Accepted(responses));
        }
        Err(OpinionError::Response(problem)) => {
            let eligible = head.is_some()
                && permission == Permission::Enabled
                && ledger.entries().iter().any(|entry| {
                    entry.number == problem.number
                        && entry.finding.severity == agent_core::review_findings::Severity::Blocker
                        && !ledger.opinions().contains_key(&entry.number)
                });
            if eligible {
                opinion_correction(pulls, problem.line, problem.number)?
            } else {
                response_correction(pulls, &problem)?
            }
        }
        Err(OpinionError::Forbidden { number, line, .. }) => {
            let correction = match ledger
                .entries()
                .iter()
                .find(|entry| entry.number == number)
                .map(|entry| entry.finding.severity)
            {
                Some(agent_core::review_findings::Severity::Major)
                    if !ledger.disputes().is_some_and(|d| d.explanations.contains_key(&number)) =>
                {
                    agent_core::review_response::Correction::FixedOrExplained
                }
                Some(agent_core::review_findings::Severity::Minor) | None => {
                    agent_core::review_response::Correction::OnePerFinding
                }
                Some(agent_core::review_findings::Severity::Major | agent_core::review_findings::Severity::Blocker) => {
                    agent_core::review_response::Correction::FixedOnly
                }
            };
            response_correction(
                pulls,
                &agent_core::review_response::ResponseError {
                    number,
                    line,
                    correction,
                },
            )?
        }
        Err(OpinionError::MissingReason { number, line }) => opinion_correction(pulls, line, number)?,
        Err(OpinionError::MissingVerdict | OpinionError::MissingEvidence) => {
            opinion_correction(pulls, 1, std::num::NonZeroU32::MIN)?
        }
    };
    Ok(FixAnswer::Correction(correction))
}

/// A response is either accepted in full or returned for correction.
#[derive(Debug)]
enum FixAnswer {
    /// Answers whose durable changes were accepted atomically.
    Accepted(Vec<(std::num::NonZeroU32, agent_core::review_response::Response)>),
    /// Complete instructions for correcting the response.
    Correction(String),
}
