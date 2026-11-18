//! The verdicts: what a reviewer's final message leads to — its report is kept and posted, its verdict recorded, and the
//! round machine says what follows.

use agent_core::config::ModelName;
use agent_core::decide::round::{RoundError, RoundPolicy};
use agent_core::history::HistoryEvent;
use agent_core::ids::{AgentName, ItemId, TaskId, TeamId};
use agent_core::log::LogEvent;
use agent_core::prompt::{AutopilotPrompt, Variables};
use agent_core::protocol::EventKind;
use agent_core::state::{ItemPhase, PullRequest, ReviewOutcome, RoundEnd, Team, Timestamp};
use agent_core::status::ReviewVerdict;
use agent_runtime::git::Revision;
use agent_runtime::github::CommentReview;
use agent_runtime::history::log_decision;
use agent_runtime::process::error_chain;

use super::super::teams::Look;
use crate::daemon::merge::{Next, VerdictError, after_verdict, count};
use crate::daemon::review::{VerdictProblem, parse_verdict, report_text, report_without_verdict};
use crate::daemon::team::Why;

use super::super::{Located, Owner, OwnerError};
use super::{Announcement, AutopilotNotice, Lead, PublishError, Pulls};

/// How many times a report without a verdict is sent back before the human is told.
pub(super) const MAX_VERDICT_BOUNCES: u32 = 2;

/// Why a review does not come, for the human, when its reviewer reports without a verdict.
const NO_VERDICT_LINE: &str =
    "it ended its review again and again without a `VERDICT:` line, so its review does not count.";

/// A review that was asked of a reviewer, as the state keeps it, and where it was found.
pub(super) struct Offered {
    /// When the review was asked.
    pub(super) requested_at: Timestamp,
    /// The item it was found in.
    pub(super) item: ItemId,
    /// The pull request.
    pub(super) pr: PullRequest,
    /// The investigator that reviews.
    pub(super) reviewer: agent_core::ids::InvestigatorName,
    /// The fix passes the pull request had had when the review was asked.
    pub(super) pass: usize,
}

/// The review a reviewer's final message answers: the pull request, its items and when it was asked.
pub(super) struct Asked {
    /// The pull request.
    pub(super) pr: PullRequest,
    /// The items of the pull request that have the review pending, in the order of the list.
    pub(super) items: Vec<ItemId>,
    /// The fix passes the pull request has had.
    pub(super) passes: usize,
    /// The investigator that reviews.
    pub(super) reviewer: agent_core::ids::InvestigatorName,
}

impl Owner {
    /// Takes the final message of the reviewer `task`, whose run ended with `STATUS: DONE`: its verdict is recorded, posted
    /// on the pull request and acted on.
    ///
    /// # Errors
    /// [`OwnerError`] when the state or a file of the home cannot be written.
    pub(in crate::daemon::owner) async fn review_ended(
        &mut self,
        task: &TaskId,
        located: &Located,
        message: Option<&str>,
    ) -> Result<(), OwnerError> {
        let Some(pulls) = self.pulls.clone() else {
            return Ok(());
        };
        let Some(message) = message else {
            return Ok(());
        };
        let started = self
            .parts
            .state
            .find_task(task)
            .and_then(|found| found.member.runs().last().map(agent_core::state::Run::started_at));
        let Some(started) = started else {
            return Ok(());
        };
        // A review pass waits for every review that was asked, so a report answers a review that is pending or none: one
        // of a reviewer that was replaced, or that was woken with nothing to review.
        let Some(asked) = self.answered_by(&located.team, task, started) else {
            tracing::info!(
                event = LogEvent::ReviewReportLate.as_str(),
                team = %located.team,
                task = %task,
                "a reviewer ended with nothing that it was asked to review: its report is not taken"
            );
            return Ok(());
        };
        match parse_verdict(message) {
            Ok(verdict) => {
                if let Some(ledger) = asked
                    .items
                    .first()
                    .and_then(|id| self.parts.state.team(&located.team)?.item(*id)?.findings())
                    && let Err(problem) = agent_core::review_findings::parse_report(message, verdict, &ledger.open())
                {
                    return self
                        .send_back(&pulls, task, located, &asked, &VerdictProblem::Findings(problem))
                        .await;
                }
                self.watch.bounces.remove(task);
                self.take_verdict(&pulls, task, located, &asked, verdict, message).await
            }
            Err(problem) => self.send_back(&pulls, task, located, &asked, &problem).await,
        }
    }

    /// The review that a run of the reviewer `task` that started at `started` answers: the latest review asked of it at or
    /// before that time that is still pending.
    pub(super) fn answered_by(&self, team: &TeamId, task: &TaskId, started: Timestamp) -> Option<Asked> {
        let team = self.parts.state.team(team)?;
        answer_among(offered_reviews(team, task), started)
    }

    /// A report without a verdict: the reviewer is asked for it again, and after [`MAX_VERDICT_BOUNCES`] times the human is
    /// told.
    pub(super) async fn send_back(
        &mut self,
        pulls: &Pulls,
        task: &TaskId,
        located: &Located,
        asked: &Asked,
        problem: &VerdictProblem,
    ) -> Result<(), OwnerError> {
        let count = self.watch.bounces.entry(task.clone()).or_insert(0);
        *count = count.saturating_add(1);
        let count = *count;
        tracing::warn!(
            event = LogEvent::RunEndedWithoutStatus.as_str(),
            task = %task,
            pr = %asked.pr.number,
            ?problem,
            count,
            "a reviewer's report has no verdict"
        );
        if count <= MAX_VERDICT_BOUNCES {
            let mut look = Look::default();
            let why = match problem {
                VerdictProblem::Findings(problem) => Why::ReviewFormat(problem.clone()),
                VerdictProblem::Missing | VerdictProblem::Unreadable { .. } => Why::VerdictMissing,
            };
            self.release(task, why, &mut look)?;
            return self.finish_look(&located.team, look).await;
        }
        let announcement = Announcement::NoVerdict { task: task.clone() };
        if !self.watch.announced.insert(announcement) {
            return Ok(());
        }
        let now = Timestamp::now(&self.parts.clock)?;
        let lead = self.lead_of(&located.team)?;
        let values = Variables::new()
            .with("reviewer", asked.reviewer.as_str())
            .and_then(|values| values.with("pr", asked.pr.number.to_string()))
            .and_then(|values| values.with("task", task.as_str()))
            .and_then(|values| values.with("why", NO_VERDICT_LINE));
        self.tell_human(pulls, AutopilotNotice::NoVerdict, values, &lead, now);
        Ok(())
    }

    /// Records the verdict `verdict` of the reviewer `task`, keeps and posts its report, and does what the round does next.
    pub(super) async fn take_verdict(
        &mut self,
        pulls: &Pulls,
        task: &TaskId,
        located: &Located,
        asked: &Asked,
        verdict: ReviewVerdict,
        message: &str,
    ) -> Result<(), OwnerError> {
        let team_id = &located.team;
        let now = Timestamp::now(&self.parts.clock)?;
        let report = report_text(message);
        let passes = count(asked.passes);
        self.parts
            .store
            .save_review_report(task, asked.pr.number, passes, &report)
            .await?;
        let lead = self.lead_of(team_id)?;
        let next = self.decide(team_id, asked, verdict);
        let mut look = Look::default();
        self.record_verdict(team_id, task, asked, verdict, now, &mut look)?;
        let completed = self.record_findings(team_id, task, asked, verdict, message)?;
        if completed {
            self.post_findings(pulls, &lead, team_id, asked).await;
        }
        self.record_event_at(
            now,
            team_id,
            self.rounds_of_items(team_id, &asked.items),
            HistoryEvent::ReviewSubmitted {
                pr: asked.pr.number,
                pass: passes,
                reviewer: asked.reviewer.clone(),
                task: task.clone(),
                verdict,
                report: report.clone(),
            },
        );
        self.post_review(pulls, &lead, task, asked, verdict, &report).await;
        if let Some(next) = next {
            self.apply_next(team_id, &lead, asked, next, &mut look)?;
        }
        look.changed = true;
        self.finish_look(team_id, look).await
    }

    /// Stages a parsed report and finalizes numbers only after every requested report is in.
    fn record_findings(
        &mut self,
        team_id: &TeamId,
        task: &TaskId,
        asked: &Asked,
        verdict: ReviewVerdict,
        message: &str,
    ) -> Result<bool, OwnerError> {
        let mut completed = false;
        for id in &asked.items {
            let mut team = self
                .parts
                .state
                .team_mut(team_id)
                .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
            let mut item = team.item_mut(*id).ok_or_else(|| OwnerError::LostItem {
                team: team_id.clone(),
                item: *id,
            })?;
            let reviews = match item.phase() {
                ItemPhase::InReview { reviews, .. } => reviews.clone(),
                ItemPhase::Pending
                | ItemPhase::InProgress { .. }
                | ItemPhase::Fixing { .. }
                | ItemPhase::Approved { .. }
                | ItemPhase::Done { .. } => continue,
            };
            let Some(ledger) = item.findings_mut() else {
                continue;
            };
            let report = agent_core::review_findings::parse_report(message, verdict, &ledger.open())?;
            ledger.record(task.clone(), report);
            if reviews
                .iter()
                .all(|review| matches!(review.outcome, ReviewOutcome::Submitted { .. }))
            {
                let pass = std::num::NonZeroU32::new(count(asked.passes).saturating_add(1))
                    .ok_or(agent_core::state::FindingNumbersExhausted::Exhausted)?;
                ledger.finish(&reviews, pass)?;
                completed = true;
            }
        }
        Ok(completed)
    }

    /// What the verdict leads to, by the round machine; `None` when the round does not take it, which is told: the
    /// verdict is recorded all the same.
    pub(super) fn decide(&self, team_id: &TeamId, asked: &Asked, verdict: ReviewVerdict) -> Option<Next> {
        let team = self.parts.state.team(team_id)?;
        let progress = asked.items.first().and_then(|id| team.item(*id))?;
        let policy = RoundPolicy::new(team.merge(), self.parts.settings.config.review());
        match after_verdict(progress, &asked.reviewer, verdict, policy) {
            Ok(next) => {
                log_decision(
                    "after_verdict",
                    &format!(
                        "pr={}, reviewer={}, verdict={verdict:?}",
                        asked.pr.number, asked.reviewer
                    ),
                    &format!("{next:?}"),
                );
                Some(next)
            }
            Err(VerdictError::Round(RoundError::NotExpected { .. })) => {
                tracing::info!(
                    event = LogEvent::ReviewReportLate.as_str(),
                    team = %team_id,
                    pr = %asked.pr.number,
                    "a verdict came after the review pass was decided: it is recorded and changes nothing"
                );
                None
            }
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::RunEndedWithoutStatus.as_str(),
                    team = %team_id,
                    pr = %asked.pr.number,
                    error = %error_chain(&error),
                    "the round does not take a verdict: it is recorded and changes nothing"
                );
                None
            }
        }
    }

    /// Records the outcome of the review of `task` in every item of the pull request.
    pub(super) fn record_verdict(
        &mut self,
        team_id: &TeamId,
        task: &TaskId,
        asked: &Asked,
        verdict: ReviewVerdict,
        now: Timestamp,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        for id in &asked.items {
            let mut team_mut = self
                .parts
                .state
                .team_mut(team_id)
                .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
            let mut item = team_mut.item_mut(*id).ok_or_else(|| OwnerError::LostItem {
                team: team_id.clone(),
                item: *id,
            })?;
            item.submit_review(task, verdict, now)
                .map_err(|source| OwnerError::Review {
                    team: team_id.clone(),
                    item: *id,
                    source,
                })?;
            let round = self.round_of_item(team_id, *id)?;
            look.events.push(EventKind::ReviewSubmitted {
                team: team_id.clone(),
                item: *id,
                round,
                reviewer: asked.reviewer.clone(),
                verdict,
            });
        }
        Ok(())
    }

    /// Posts the report of the reviewer `task` on the pull request as a comment review that names the commit it reviewed.
    /// The harness opens the pull request and posts the reviews with one account, which GitHub does not let approve its
    /// own pull request: the verdict is in the report.
    ///
    /// A review that cannot be posted is logged and does not hold the round: the verdict counts, and the report is in the
    /// task's directory.
    pub(super) async fn post_review(
        &mut self,
        pulls: &Pulls,
        lead: &Lead,
        task: &TaskId,
        asked: &Asked,
        verdict: ReviewVerdict,
        report: &str,
    ) {
        if let Err(error) = self.try_post_review(pulls, lead, task, asked, verdict, report).await {
            tracing::warn!(
                event = LogEvent::GithubCallFailed.as_str(),
                task = %task,
                pr = %asked.pr.number,
                error = %error_chain(&error),
                "a review cannot be posted on the pull request: its verdict counts and its report is kept"
            );
        }
    }

    /// See [`Owner::post_review`].
    pub(super) async fn try_post_review(
        &mut self,
        pulls: &Pulls,
        lead: &Lead,
        task: &TaskId,
        asked: &Asked,
        verdict: ReviewVerdict,
        report: &str,
    ) -> Result<(), PublishError> {
        let agent: AgentName = asked.reviewer.as_str().parse()?;
        let git = self.worktree_git(&lead.repo, task, &agent)?;
        let head = git.rev_parse(&Revision::Head).await?;
        let github = self.github_for(pulls, &lead.repo, &git).await?;
        let client = match self.parts.settings.config.investigator(&asked.reviewer) {
            Some(inv) => crate::daemon::notice::client_label(Some(inv.cli), inv.model.as_ref().map(ModelName::as_str)),
            None => String::new(),
        };
        let verdict_line = match verdict {
            ReviewVerdict::Approve => pulls.texts.render(
                AutopilotPrompt::ReviewVerdictApprove,
                // One mark on GitHub: the double one is the notification's "ready to merge", which one review is not.
                &Variables::new().with("icon", agent_core::notice_status::NoticeStatus::Done.icon())?,
            )?,
            ReviewVerdict::RequestChanges => pulls.texts.render(
                AutopilotPrompt::ReviewVerdictRequestChanges,
                &Variables::new().with("icon", agent_core::notice_status::NoticeStatus::Failed.icon())?,
            )?,
        };
        let pass = count(asked.passes).saturating_add(1).to_string();
        let stripped_report = report_without_verdict(report);
        let body = pulls.texts.render(
            AutopilotPrompt::ReviewComment,
            &Variables::new()
                .with("reviewer", asked.reviewer.as_str())?
                .with("client", &client)?
                .with("verdict", verdict_line.trim_end())?
                .with("pass", &pass)?
                .with("commit", head.as_str())?
                .with("report", &stripped_report)?,
        )?;
        let body = self.link_commits(&git, &head, &body).await;
        github
            .post_comment_review(
                asked.pr.number,
                &CommentReview {
                    body: body.trim_end().to_owned(),
                    commit: head,
                },
            )
            .await?;
        Ok(())
    }

    /// Does what the verdict led to, in every item of the pull request.
    pub(super) fn apply_next(
        &mut self,
        team_id: &TeamId,
        lead: &Lead,
        asked: &Asked,
        next: Next,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let mut approved = false;
        for id in &asked.items {
            let mut team_mut = self
                .parts
                .state
                .team_mut(team_id)
                .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
            let mut item = team_mut.item_mut(*id).ok_or_else(|| OwnerError::LostItem {
                team: team_id.clone(),
                item: *id,
            })?;
            let ItemPhase::InReview { pr, reviews } = item.phase().clone() else {
                continue;
            };
            let after = match next {
                Next::Wait | Next::AskHuman => continue,
                Next::Merge | Next::HandToHuman => ItemPhase::Approved { pr, reviews },
                Next::FixPass { .. } => ItemPhase::Fixing {
                    assignee: lead.agent.clone(),
                    branch: pr.branch.clone(),
                    pr,
                },
            };
            let status = after.kind().status();
            item.advance(after).map_err(|source| OwnerError::Item {
                team: team_id.clone(),
                item: *id,
                source,
            })?;
            // One approval for the pull request, though its items move one by one; before the round of a hand-over closes.
            if !approved && matches!(next, Next::Merge | Next::HandToHuman) {
                approved = true;
                let rounds = self.rounds_of_items(team_id, &asked.items);
                self.record_event(
                    team_id,
                    rounds.clone(),
                    HistoryEvent::PullApproved { pr: asked.pr.number },
                );
                if matches!(next, Next::HandToHuman) {
                    self.record_event(team_id, rounds, HistoryEvent::PullHandedOver { pr: asked.pr.number });
                }
            }
            let round = self.round_of_item(team_id, *id)?;
            {
                let span = tracing::error_span!("round", team = %team_id, rounds = %round);
                let _entered = span.enter();
                tracing::info!(
                    event = LogEvent::ItemPhaseChanged.as_str(),
                    team = %team_id,
                    item = %id,
                    phase = %status.as_str(),
                    pr = %asked.pr.number,
                    "an item changed phase after a review verdict"
                );
            }
            look.events.push(EventKind::ItemStatus {
                team: team_id.clone(),
                item: *id,
                round,
                status,
            });
            // The pull request of a team that does not merge by itself is the human's from its approval on: the round
            // ends there.
            if matches!(next, Next::HandToHuman) {
                let now = Timestamp::now(&self.parts.clock)?;
                self.close_item_round(team_id, *id, RoundEnd::Approved, now, look)?;
            }
        }
        Ok(())
    }
}

/// The reviews of the reviewer `task` that are pending in `team`.
fn offered_reviews(team: &Team, task: &TaskId) -> Vec<Offered> {
    let mut offered = Vec::new();
    for progress in team.items() {
        let ItemPhase::InReview { pr, reviews } = progress.phase() else {
            continue;
        };
        let pending = reviews
            .iter()
            .find(|review| review.task == *task && matches!(review.outcome, ReviewOutcome::Pending));
        if let Some(review) = pending {
            offered.push(Offered {
                requested_at: review.requested_at,
                item: progress.id(),
                pr: pr.clone(),
                reviewer: review.reviewer.clone(),
                pass: progress.fix_passes().len(),
            });
        }
    }
    offered
}

/// What a run that started at `started` answers among the `offered` reviews: those of one pull request asked at the
/// latest time that is not after it.
fn answer_among(offered: Vec<Offered>, started: Timestamp) -> Option<Asked> {
    let latest = offered
        .iter()
        .map(|found| found.requested_at)
        .filter(|requested| *requested <= started)
        .max()?;
    let answered: Vec<Offered> = offered
        .into_iter()
        .filter(|found| found.requested_at == latest)
        .collect();
    let first = answered.first()?;
    Some(Asked {
        pr: first.pr.clone(),
        items: answered
            .iter()
            .filter(|found| found.pr.number == first.pr.number)
            .map(|found| found.item)
            .collect(),
        passes: first.pass,
        reviewer: first.reviewer.clone(),
    })
}
