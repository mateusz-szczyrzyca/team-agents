//! The verdicts: what a reviewer's final message leads to — its report is kept and posted, its verdict recorded, and the
//! round machine says what follows.

use agent_core::decide::round::{RoundError, RoundPolicy};
use agent_core::ids::{AgentName, ItemId, TaskId, TeamId};
use agent_core::prompt::{AutopilotPrompt, Variables};
use agent_core::protocol::EventKind;
use agent_core::state::{Completion, ItemPhase, PullRequest, Review, ReviewOutcome, Team, Timestamp};
use agent_core::status::ReviewVerdict;
use agent_runtime::git::Revision;
use agent_runtime::github::CommentReview;
use agent_runtime::process::error_chain;

use super::super::teams::Look;
use crate::daemon::merge::{Next, VerdictError, after_verdict, count};
use crate::daemon::review::{VerdictProblem, parse_verdict, report_text};
use crate::daemon::team::Why;

use super::super::{Located, Owner, OwnerError};
use super::{Announcement, Lead, PublishError, Pulls};

/// How many times a report without a verdict is sent back before the human is told.
pub(super) const MAX_VERDICT_BOUNCES: u32 = 2;

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
    /// Whether it is the review pass that is going.
    pub(super) current: bool,
}

/// What a reviewer's report answers.
pub(super) enum Answer {
    /// A review of the review pass that is going.
    Current(Asked),
    /// A review of a review pass the pull request has left behind.
    Late(Late),
}

/// A review of a review pass that is decided.
pub(super) struct Late {
    /// The pull request.
    pub(super) pr: PullRequest,
    /// The fix passes the pull request had had when the review was asked.
    pub(super) pass: usize,
    /// The investigator that reviews.
    pub(super) reviewer: agent_core::ids::InvestigatorName,
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
        let asked = match self.answered_by(&located.team, task, started) {
            Some(Answer::Current(asked)) => asked,
            Some(Answer::Late(late)) => return self.keep_late_review(&pulls, task, located, &late, message).await,
            None => {
                tracing::debug!(task = %task, "a reviewer ended with nothing that it was asked to review: its report is not taken");
                return Ok(());
            }
        };
        match parse_verdict(message) {
            Ok(verdict) => {
                self.watch.bounces.remove(task);
                self.take_verdict(&pulls, task, located, &asked, verdict, message).await
            }
            Err(problem) => self.send_back(&pulls, task, located, &asked, &problem).await,
        }
    }

    /// The review that a run of the reviewer `task` that started at `started` answers: the latest review asked of it at or
    /// before that time that is still pending, in the review pass that is going or in one the pull request has left behind.
    pub(super) fn answered_by(&self, team: &TeamId, task: &TaskId, started: Timestamp) -> Option<Answer> {
        let team = self.parts.state.team(team)?;
        answer_among(offered_reviews(team, task), started)
    }

    /// A report that answers a review of a review pass the pull request has left behind, because the reviewer was slow and
    /// the other reviewer had asked for changes: the review pass is decided, and the report is kept and posted all the same.
    pub(super) async fn keep_late_review(
        &mut self,
        pulls: &Pulls,
        task: &TaskId,
        located: &Located,
        late: &Late,
        message: &str,
    ) -> Result<(), OwnerError> {
        tracing::info!(task = %task, pr = %late.pr.number, "a reviewer ended a review of a review pass that is decided: its report is kept and posted");
        let report = report_text(message);
        let pass = count(late.pass);
        self.parts
            .store
            .save_review_report(task, late.pr.number, pass, &report)
            .await?;
        let lead = self.lead_of(&located.team)?;
        let asked = Asked {
            pr: late.pr.clone(),
            items: Vec::new(),
            passes: late.pass,
            reviewer: late.reviewer.clone(),
        };
        self.post_review(pulls, &lead, task, &asked, &report).await;
        Ok(())
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
        tracing::warn!(task = %task, pr = %asked.pr.number, ?problem, count, "a reviewer's report has no verdict");
        if count <= MAX_VERDICT_BOUNCES {
            let mut look = Look::default();
            self.release(task, Why::VerdictMissing, &mut look)?;
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
            .and_then(|values| values.with("task", task.as_str()));
        self.tell_human(pulls, AutopilotPrompt::NoticeNoVerdict, values, &lead, now);
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
        self.post_review(pulls, &lead, task, asked, &report).await;
        if let Some(next) = next {
            self.apply_next(team_id, &lead, asked, next, &mut look)?;
        }
        look.changed = true;
        self.finish_look(team_id, look).await
    }

    /// What the verdict leads to, by the round machine; `None` when the round does not take it, which is told: the
    /// verdict is recorded all the same.
    pub(super) fn decide(&self, team_id: &TeamId, asked: &Asked, verdict: ReviewVerdict) -> Option<Next> {
        let team = self.parts.state.team(team_id)?;
        let progress = asked.items.first().and_then(|id| team.item(*id))?;
        let policy = RoundPolicy::new(team.merge(), self.parts.settings.config.review());
        match after_verdict(progress, &asked.reviewer, verdict, policy) {
            Ok(next) => Some(next),
            Err(VerdictError::Round(RoundError::NotExpected { .. })) => {
                tracing::info!(team = %team_id, pr = %asked.pr.number, "a verdict came after the review pass was decided: it is recorded and changes nothing");
                None
            }
            Err(error) => {
                tracing::warn!(team = %team_id, pr = %asked.pr.number, error = %error_chain(&error), "the round does not take a verdict: it is recorded and changes nothing");
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
            look.events.push(EventKind::ReviewSubmitted {
                team: team_id.clone(),
                item: *id,
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
    pub(super) async fn post_review(&mut self, pulls: &Pulls, lead: &Lead, task: &TaskId, asked: &Asked, report: &str) {
        if let Err(error) = self.try_post_review(pulls, lead, task, asked, report).await {
            tracing::warn!(task = %task, pr = %asked.pr.number, error = %error_chain(&error), "a review cannot be posted on the pull request: its verdict counts and its report is kept");
        }
    }

    /// See [`Owner::post_review`].
    pub(super) async fn try_post_review(
        &mut self,
        pulls: &Pulls,
        lead: &Lead,
        task: &TaskId,
        asked: &Asked,
        report: &str,
    ) -> Result<(), PublishError> {
        let agent: AgentName = asked.reviewer.as_str().parse()?;
        let git = self.worktree_git(&lead.repo, task, &agent)?;
        let head = git.rev_parse(&Revision::Head).await?;
        let github = self.github_for(pulls, &lead.repo, &git).await?;
        let body = pulls.texts.render(
            AutopilotPrompt::ReviewComment,
            &Variables::new()
                .with("reviewer", asked.reviewer.as_str())?
                .with("head", head.as_str())?
                .with("report", report)?,
        )?;
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
            look.events.push(EventKind::ItemStatus {
                team: team_id.clone(),
                item: *id,
                status,
            });
        }
        Ok(())
    }
}

/// The reviews of the reviewer `task` that are pending in `team`, in the review pass that is going and in those the pull requests
/// have left behind.
fn offered_reviews(team: &Team, task: &TaskId) -> Vec<Offered> {
    let pending = |reviews: &[Review]| {
        reviews
            .iter()
            .find(|review| review.task == *task && matches!(review.outcome, ReviewOutcome::Pending))
            .map(|review| (review.requested_at, review.reviewer.clone()))
    };
    let mut offered = Vec::new();
    for progress in team.items() {
        if let ItemPhase::InReview { pr, reviews } = progress.phase()
            && let Some((requested_at, reviewer)) = pending(reviews)
        {
            offered.push(Offered {
                requested_at,
                item: progress.id(),
                pr: pr.clone(),
                reviewer,
                pass: progress.fix_passes().len(),
                current: true,
            });
        }
        let pr = match progress.phase() {
            ItemPhase::InReview { pr, .. }
            | ItemPhase::Fixing { pr, .. }
            | ItemPhase::Approved { pr, .. }
            | ItemPhase::Done {
                completion: Completion::Merged { pr, .. },
            } => pr,
            ItemPhase::Done {
                completion: Completion::Listed,
            }
            | ItemPhase::Pending
            | ItemPhase::InProgress { .. } => continue,
        };
        for (index, pass) in progress.fix_passes().iter().enumerate() {
            if let Some((requested_at, reviewer)) = pending(&pass.requested_by) {
                offered.push(Offered {
                    requested_at,
                    item: progress.id(),
                    pr: pr.clone(),
                    reviewer,
                    pass: index,
                    current: false,
                });
            }
        }
    }
    offered
}

/// What a run that started at `started` answers among the `offered` reviews: those asked at the latest time that is not
/// after it; of the review pass that is going when one of them is, else of a review pass that is decided.
fn answer_among(offered: Vec<Offered>, started: Timestamp) -> Option<Answer> {
    let latest = offered
        .iter()
        .map(|found| found.requested_at)
        .filter(|requested| *requested <= started)
        .max()?;
    let answered: Vec<Offered> = offered
        .into_iter()
        .filter(|found| found.requested_at == latest)
        .collect();
    let current: Vec<&Offered> = answered.iter().filter(|found| found.current).collect();
    if let Some(first) = current.first() {
        return Some(Answer::Current(Asked {
            pr: first.pr.clone(),
            items: current
                .iter()
                .filter(|found| found.pr.number == first.pr.number)
                .map(|found| found.item)
                .collect(),
            passes: first.pass,
            reviewer: first.reviewer.clone(),
        }));
    }
    answered.into_iter().next().map(|found| {
        Answer::Late(Late {
            pr: found.pr,
            pass: found.pass,
            reviewer: found.reviewer,
        })
    })
}
