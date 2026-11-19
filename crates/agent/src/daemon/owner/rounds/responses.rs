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
            response_run(team, &lead.task).is_some_and(|run| {
                team.items().iter().any(|item| {
                    needs_response(item, run, crate::daemon::merge::pull_passes(team, item))
                        && item.findings().is_some_and(|ledger| ledger.verification().is_some())
                })
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
        let Some(run) = response_run(team, &lead.task).cloned() else {
            return Ok(());
        };
        // The items of one pull request share its ledger, which counts the pull request's passes: each is answered with
        // that pass, also one that joined a fix pass and has fewer of its own.
        let fixing: Vec<_> = team
            .items()
            .iter()
            .map(|item| (item.clone(), crate::daemon::merge::pull_passes(team, item)))
            .filter(|(item, pass)| needs_response(item, &run, *pass))
            .collect();
        let mut explained = std::collections::HashSet::new();
        for (item, pass) in fixing {
            let id = item.id();
            let Some(mut ledger) = item.findings().cloned() else {
                continue;
            };
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
                        self.record_explanations(team_id, &item, pass, &lead, responses);
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
                .and_then(|v| v.with("reason", text))
                .and_then(|v| v.with("bounces", bounces.to_string()))
                .and_then(|v| v.with("task", lead.task.as_str()))
                .and_then(|v| v.with("reviewer", lead.agent.as_str()))
                .and_then(|v| {
                    v.with(
                        "pr",
                        self.parts
                            .state
                            .team(team_id)
                            .and_then(|team| team.item(id))
                            .map_or_else(String::new, |item| match item.phase() {
                                ItemPhase::Fixing { pr, .. } => pr.number.to_string(),
                                ItemPhase::Pending
                                | ItemPhase::InProgress { .. }
                                | ItemPhase::InReview { .. }
                                | ItemPhase::Approved { .. }
                                | ItemPhase::Done { .. } => String::new(),
                            }),
                    )
                });
            let now = Timestamp::now(&self.parts.clock)?;
            self.tell_human(pulls, AutopilotNotice::ResponseRefused, values, &lead, now);
        }
        Ok(())
    }
    /// Keeps each new explanation in the round history before verification, as given in fix pass `pass` of the pull
    /// request.
    fn record_explanations(
        &mut self,
        team_id: &TeamId,
        item: &agent_core::state::ItemProgress,
        pass: u32,
        lead: &super::Lead,
        responses: Vec<(std::num::NonZeroU32, agent_core::review_response::Response)>,
    ) {
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

/// The completed lead run that can supply a fix response.
fn response_run<'a>(
    team: &'a agent_core::state::Team,
    task: &agent_core::ids::TaskId,
) -> Option<&'a agent_core::state::Run> {
    team.find(task)
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
}

/// Whether this item still needs that run's response to the fix pass `pass` of its pull request.
fn needs_response(item: &agent_core::state::ItemProgress, run: &agent_core::state::Run, pass: u32) -> bool {
    if !matches!(item.phase(), ItemPhase::Fixing { .. }) {
        return false;
    }
    let Some(disputes) = item.findings().and_then(agent_core::state::Findings::disputes) else {
        return false;
    };
    disputes.answered_pass != pass
        && disputes.checked_run.as_ref() != Some(run.id())
        && !item
            .fix_passes()
            .last()
            .is_some_and(|pass| pass.requested_by.iter().any(|r| r.requested_at > run.started_at()))
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
    let Some(number) = problem.detail.number else {
        // A label that is no number names no finding: the first open one is not the one meant.
        let values = Variables::new().with("line", problem.line.to_string())?;
        return Ok(pulls.texts.render(AutopilotPrompt::ReviewResponseUnnumbered, &values)?);
    };
    let allowed = pulls.texts.render(instruction, &Variables::new())?;
    let values = Variables::new()
        .with("line", problem.line.to_string())?
        .with("number", number.to_string())?
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
    let (correction, detail) = match response {
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
            let correction =
                if eligible && problem.detail.mistake != agent_core::review_findings::AnswerMistake::UnknownNumber {
                    opinion_correction(pulls, problem.line, problem.number)?
                } else {
                    response_correction(pulls, &problem)?
                };
            (correction, problem.detail)
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
            let detail = agent_core::review_findings::AnswerDiagnostic {
                source_line: message.lines().nth(line.saturating_sub(1)).map(str::to_owned),
                number: Some(number),
                mistake: agent_core::review_findings::AnswerMistake::NotAllowed,
            };
            let correction = response_correction(
                pulls,
                &agent_core::review_response::ResponseError {
                    number,
                    line,
                    correction,
                    detail: detail.clone(),
                },
            )?;
            (correction, detail)
        }
        Err(OpinionError::MissingReason { number, line }) => (
            opinion_correction(pulls, line, number)?,
            agent_core::review_findings::AnswerDiagnostic {
                source_line: message.lines().nth(line.saturating_sub(1)).map(str::to_owned),
                number: Some(number),
                mistake: agent_core::review_findings::AnswerMistake::NotAllowed,
            },
        ),
        Err(OpinionError::MissingVerdict | OpinionError::MissingEvidence) => {
            return Ok(FixAnswer::Correction(unlocated_correction(&pulls.texts)?));
        }
    };
    let forms = lead_forms(&pulls.texts, ledger, permission)?;
    let diagnostic = answer_diagnostic(&pulls.texts, &detail, &forms)?;
    Ok(FixAnswer::Correction(
        [correction.trim_end(), diagnostic.trim_end()].join("\n\n"),
    ))
}

/// A response is either accepted in full or returned for correction.
#[derive(Debug)]
enum FixAnswer {
    /// Answers whose durable changes were accepted atomically.
    Accepted(Vec<(std::num::NonZeroU32, agent_core::review_response::Response)>),
    /// Complete instructions for correcting the response.
    Correction(String),
}

/// Requests a complete response when no finding or line was identified.
fn unlocated_correction(
    texts: &agent_runtime::autopilot_prompts::AutopilotPrompts,
) -> Result<String, ResponseTextError> {
    Ok(texts.render(AutopilotPrompt::ReviewResponseNumbers, &Variables::new())?)
}

#[cfg(test)]
mod tests;

/// Renders source evidence and the complete list of permitted answers from the catalog.
pub(super) fn answer_diagnostic(
    texts: &agent_runtime::autopilot_prompts::AutopilotPrompts,
    detail: &agent_core::review_findings::AnswerDiagnostic,
    forms: &str,
) -> Result<String, ResponseTextError> {
    use agent_core::review_findings::AnswerMistake;
    let prompt = match detail.mistake {
        AnswerMistake::UnknownNumber => AutopilotPrompt::ReviewMistakeUnknown,
        AnswerMistake::Duplicate => AutopilotPrompt::ReviewMistakeDuplicate,
        AnswerMistake::AfterStatus => AutopilotPrompt::ReviewMistakeAfterStatus,
        AnswerMistake::NotAllowed => AutopilotPrompt::ReviewMistakeForbidden,
        AnswerMistake::Missing => AutopilotPrompt::ReviewMistakeMissing,
        AnswerMistake::AfterFindings => AutopilotPrompt::ReviewMistakeAfterFindings,
        AnswerMistake::Malformed => AutopilotPrompt::ReviewMistakeMalformed,
        AnswerMistake::Verdict => AutopilotPrompt::ReviewMistakeVerdict,
    };
    let mistake = texts.render(prompt, &Variables::new())?;
    let values = Variables::new()
        .with("source", detail.source_line.as_deref().unwrap_or_default())?
        .with("mistake", mistake.trim_end())?
        .with("forms", forms)?;
    Ok(match detail.number {
        Some(number) => texts.render(
            AutopilotPrompt::ReviewAnswerDiagnostic,
            &values.with("number", number.to_string())?,
        )?,
        None => texts.render(AutopilotPrompt::ReviewAnswerDiagnosticUnnumbered, &values)?,
    })
}

/// Lists the forms the lead may use for each open blocking finding.
fn lead_forms(
    texts: &agent_runtime::autopilot_prompts::AutopilotPrompts,
    ledger: &agent_core::state::Findings,
    permission: Permission,
) -> Result<String, ResponseTextError> {
    use agent_core::review_findings::Severity;
    let mut forms = String::new();
    for entry in ledger
        .entries()
        .iter()
        .filter(|entry| entry.status == agent_core::state::FindingStatus::Open && entry.finding.severity.blocks())
    {
        let instruction = match entry.finding.severity {
            Severity::Blocker
                if permission == Permission::Enabled && !ledger.opinions().contains_key(&entry.number) =>
            {
                AutopilotPrompt::ReviewResponseOpinion
            }
            Severity::Major
                if !ledger
                    .disputes()
                    .is_some_and(|d| d.explanations.contains_key(&entry.number)) =>
            {
                AutopilotPrompt::ReviewResponseExplain
            }
            Severity::Blocker | Severity::Major | Severity::Minor => AutopilotPrompt::ReviewResponseFixed,
        };
        let allowed = texts.render(instruction, &Variables::new())?;
        forms.push_str(
            &texts.render(
                AutopilotPrompt::ReviewResponseForm,
                &Variables::new()
                    .with("number", entry.number.to_string())?
                    .with("allowed", allowed.trim_end())?,
            )?,
        );
    }
    Ok(forms)
}

/// Lists the verifier's forms for every open finding and pending explanation: a verifier answers the minor ones too.
pub(super) fn verifier_forms(
    texts: &agent_runtime::autopilot_prompts::AutopilotPrompts,
    ledger: &agent_core::state::Findings,
) -> Result<String, ResponseTextError> {
    let mut forms = String::new();
    for answers in ledger.verifier_answers() {
        forms.push_str(
            &texts.render(
                AutopilotPrompt::ReviewResponseForm,
                &Variables::new()
                    .with("number", answers.number.to_string())?
                    .with("allowed", verifier_allowed(texts, answers)?)?,
            )?,
        );
    }
    Ok(forms)
}

/// The answers a verifier may give for one finding, as one line of the catalog's words: what [`verifier_forms`] and the
/// brief of a verifier both name.
pub(super) fn verifier_allowed(
    texts: &agent_runtime::autopilot_prompts::AutopilotPrompts,
    answers: agent_core::state::VerifierAnswers,
) -> Result<String, ResponseTextError> {
    use agent_core::state::{Decision, Raise};
    let decision = match answers.decision {
        Decision::Explanation => AutopilotPrompt::ReviewVerifierPending,
        Decision::Outcome => AutopilotPrompt::ReviewVerifierResolved,
    };
    let mut allowed = texts.render(decision, &Variables::new())?.trim_end().to_owned();
    let raise = match answers.raise {
        Raise::Never => None,
        Raise::ToBlocker => Some(AutopilotPrompt::ReviewRaiseBlocker),
        Raise::ToMajorOrBlocker => Some(AutopilotPrompt::ReviewRaiseMajor),
    };
    if let Some(raise) = raise {
        allowed.push(' ');
        allowed.push_str(texts.render(raise, &Variables::new())?.trim_end());
    }
    Ok(allowed)
}
