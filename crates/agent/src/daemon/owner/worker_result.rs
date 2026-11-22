//! What becomes of the result of a worker: its check, the bringing over of its commit and the answer to the requester.
//!
//! When a worker's run ends `STATUS: DONE`, [`Owner::keep_worker_result`] writes the request to a file of the worker's
//! task directory and the next look at the team settles it, stage by stage, writing the file again after each stage:
//! the harness runs the request's check in the worker's worktree, [`decide`] says what the result is worth, a result that
//! is good is brought over to the requester's branch, and the requester is told. A daemon that stops between two stages
//! goes on from the stage the file names, so that the check runs once and a commit is brought over once.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use agent_core::config::AbsolutePath;
use agent_core::decide::worker_result::{CheckOutcome, Elsewhere, Facts, Scope, Verdict, WorkerEnding, Wrong, decide};
use agent_core::ids::{AgentName, ItemId, TaskId, TeamId};
use agent_core::kinds::Role;
use agent_core::log::LogEvent;
use agent_core::plan::Check;
use agent_core::plan::judge::JudgementState;
use agent_core::state::{CommitId, Timestamp};
use agent_runtime::agentbus::{DelegationStatus, DelegationUpdate};
use agent_runtime::git::{BringOver, CommitIdentity, Git, Revision};
use agent_runtime::plan::check::{CheckContext, execute_request_check, read_check_tail, request_check_outcome};
use agent_runtime::process::error_chain;
use agent_runtime::team_prompts::TeamPrompts;
use agent_runtime::worker_request::{
    Checked, Found, Pending, PendingError, Settlement, Stage, pending_path, read_pending, write_pending,
};

use super::teams::Served;
use super::{Owner, OwnerError};
use crate::daemon::team::{BusJob, BusNotice, NoticeRenderError};

/// What the requester reads of a check's output when the check printed nothing.
const NO_OUTPUT: &str = "(the check printed nothing)";

/// What the files of a request, as the bus has them, say it asks for.
fn scope_of(files: &[String]) -> Scope {
    if files.is_empty() { Scope::Facts } else { Scope::Files }
}

/// The facts of a result for [`decide`]: no other provider is looked for in this version, so a wrong result is returned
/// to the requester at once.
fn facts_of(checked: &Checked, run_seconds: u64, limit_seconds: u64) -> Facts {
    Facts {
        ending: WorkerEnding::Done,
        check: checked
            .found
            .as_ref()
            .map_or(CheckOutcome::Absent, |found| found.outcome),
        changed_outside_files: !checked.outside.is_empty(),
        past_seconds: limit_seconds > 0 && run_seconds > limit_seconds,
        scope: scope_of(&checked.files),
        try_number: 1,
        tries: 1,
        elsewhere: Elsewhere::None,
    }
}

/// The reasons a result is wrong, in the words the requester reads.
fn reasons_of(checked: &Checked, wrong: &[Wrong], limit_seconds: u64) -> Vec<String> {
    wrong
        .iter()
        .map(|wrong| match wrong {
            Wrong::EndedOtherwise(_) => "the worker did not end its run with STATUS: DONE".to_owned(),
            Wrong::CheckFailed => match &checked.found {
                Some(found) => format!(
                    "the check `{}` failed{}",
                    found.line,
                    found
                        .reason
                        .as_ref()
                        .map_or_else(String::new, |reason| format!(" ({reason})"))
                ),
                None => "the check failed".to_owned(),
            },
            Wrong::CheckTimedOut => "the check ran past its time limit".to_owned(),
            Wrong::OutsideFiles => format!(
                "the worker changed files the request did not name: {}",
                checked.outside.join(", ")
            ),
            Wrong::NoCheck => "the request names files but no check, so nothing shows the commit right".to_owned(),
            Wrong::PastSeconds => format!("the worker's run lasted past {limit_seconds} seconds"),
        })
        .collect()
}

impl Owner {
    /// Keeps the request `served` of `team`, whose worker's run ended `STATUS: DONE` with the report `summary`, for the
    /// next look at the team to settle.
    ///
    /// # Errors
    /// [`OwnerError`] when the file cannot be written or the clock gives a time that cannot be recorded.
    pub(super) fn keep_worker_result(
        &mut self,
        team: &TeamId,
        served: &Served,
        worker_task: &TaskId,
        summary: String,
    ) -> Result<(), OwnerError> {
        let now = Timestamp::now(&self.parts.clock)?;
        let elapsed = now.as_utc() - served.started.as_utc();
        let pending = Pending {
            request: served.request,
            team: team.clone(),
            requester: served.requester.clone(),
            requester_task: served.requester_task.clone(),
            worker: served.worker.clone(),
            worker_task: worker_task.clone(),
            summary,
            run_seconds: u64::try_from(elapsed.whole_seconds()).unwrap_or(0),
            base: served.base.clone(),
            stage: Stage::Ended,
        };
        let task_dir = self.parts.store.home().task_dir(worker_task);
        write_pending(&task_dir, &pending)?;
        Ok(())
    }

    /// The git handle of the worktree of `task`, a task of `team`, for `agent`; `None` when the team or the path is not
    /// there.
    fn git_of(&self, team: &TeamId, task: &TaskId, agent: &AgentName) -> Option<Git> {
        let path = self.worker_worktree(team, task)?;
        let worktree = AbsolutePath::from_str(path.to_str()?).ok()?;
        Some(Git::new(
            worktree,
            CommitIdentity::for_agent(self.parts.settings.config.git(), agent),
        ))
    }

    /// The worktree of `task`, a task of `team`.
    fn worker_worktree(&self, team: &TeamId, task: &TaskId) -> Option<PathBuf> {
        let repo = self.parts.state.team(team)?.repo().clone();
        Some(self.parts.store.home().worktree_dir(&repo, task))
    }

    /// The commit the worktree of the worker task `task` is at, as text.
    pub(super) async fn worker_head(&self, team: &TeamId, task: &TaskId, agent: &AgentName) -> Option<String> {
        let git = self.git_of(team, task, agent)?;
        git.rev_parse(&Revision::Head).await.ok().map(|head| head.to_string())
    }

    /// The round the requester's last run worked for, which a snapshot of its worktree is kept under.
    fn requester_round(&self, team: &TeamId, task: &TaskId) -> Option<agent_core::ids::RoundId> {
        let (_, member) = self.parts.state.team(team)?.find(task)?;
        member.runs().last()?.rounds().first().cloned()
    }

    /// Settles the requests of the workers of `team` whose runs ended: one stage after another, each written down before
    /// the next, and the answer at the end. A stage the bus cannot take now is tried again at the next look.
    ///
    /// # Errors
    /// [`OwnerError`] when a file cannot be read or written.
    pub(super) async fn settle_worker_results(&mut self, team: &TeamId) -> Result<(), OwnerError> {
        let Some(found) = self.parts.state.team(team) else {
            return Ok(());
        };
        let workers: Vec<TaskId> = found
            .crew()
            .roster()
            .into_iter()
            .filter(|(role, _)| *role == Role::Worker)
            .map(|(_, member)| member.task().clone())
            .collect();
        for task in workers {
            let task_dir = self.parts.store.home().task_dir(&task);
            for pending in read_pending(&task_dir).await? {
                self.settle_one(&task_dir, pending).await?;
            }
        }
        Ok(())
    }

    /// Takes `pending` through the stages that remain.
    async fn settle_one(&mut self, task_dir: &Path, mut pending: Pending) -> Result<(), OwnerError> {
        loop {
            let next = match &pending.stage {
                Stage::Ended => match self.check_result(&pending).await {
                    Ok(checked) => Stage::Checked(Box::new(checked)),
                    Err(error) => {
                        tracing::warn!(
                            event = LogEvent::BusDown.as_str(),
                            request = %pending.request,
                            error = %error_chain(&error),
                            "what a request asks cannot be read now: it is tried again at the next look"
                        );
                        return Ok(());
                    }
                },
                Stage::Checked(checked) => {
                    let settlement = self.settle_result(&pending, checked).await;
                    Stage::Settled {
                        checked: checked.clone(),
                        settlement,
                    }
                }
                Stage::Settled { checked, settlement } => {
                    self.answer_requester(&pending, checked, settlement).await?;
                    // A removed file that was never there is the end the request wants.
                    if let Err(error) = std::fs::remove_file(pending_path(task_dir, pending.request))
                        && error.kind() != std::io::ErrorKind::NotFound
                    {
                        return Err(PendingError::Write {
                            path: pending_path(task_dir, pending.request),
                            source: error,
                        }
                        .into());
                    }
                    return Ok(());
                }
            };
            pending.stage = next;
            write_pending(task_dir, &pending)?;
        }
    }

    /// Reads what the request asks and checks the worker's result.
    ///
    /// # Errors
    /// [`agent_runtime::agentbus::AgentBusError`] when the bus cannot say what the request asks.
    async fn check_result(&self, pending: &Pending) -> Result<Checked, agent_runtime::agentbus::AgentBusError> {
        let work = self.parts.bus.helper_work(pending.request).await?;
        let files: Vec<String> = work.files.iter().map(|path| path.as_str().to_owned()).collect();
        let worker_git = self.git_of(&pending.team, &pending.worker_task, &pending.worker);
        let head = match &worker_git {
            Some(git) => git.rev_parse(&Revision::Head).await.ok(),
            None => None,
        };
        let base = pending.base.as_deref().and_then(|base| CommitId::from_str(base).ok());
        let committed = match (&head, &base) {
            (Some(head), Some(base)) => head != base,
            (Some(_), None) => true,
            (None, _) => false,
        };
        let outside = match (&worker_git, &head, &base) {
            (Some(git), Some(head), Some(base)) if committed => match git.changed_between(base, head).await {
                Ok(changed) => changed
                    .into_iter()
                    .filter(|path| !work.files.contains(path))
                    .map(|path| path.as_str().to_owned())
                    .collect(),
                Err(error) => {
                    tracing::warn!(error = %error_chain(&error), "the files of a worker's commits cannot be listed");
                    Vec::new()
                }
            },
            _ => Vec::new(),
        };
        let found = match (&work.check, &work.step) {
            (Some(text), Some(step)) => self.run_check(pending, step, text).await,
            _ => None,
        };
        Ok(Checked {
            step: work.step,
            files,
            head: head.map(|head| head.to_string()),
            committed,
            outside,
            found,
        })
    }

    /// Runs the check `text` of the request in the worker's worktree.
    async fn run_check(&self, pending: &Pending, step: &ItemId, text: &str) -> Option<Found> {
        let Ok(check) = Check::from_str(text) else {
            return Some(Found {
                outcome: CheckOutcome::Failed,
                line: text.to_owned(),
                tail: String::new(),
                reason: Some("the check is not one the harness can run".to_owned()),
                duration_ms: 0,
                output: None,
            });
        };
        let home = self.parts.store.home();
        let worktree = self.worker_worktree(&pending.team, &pending.worker_task)?;
        let task_dir = home.task_dir(&pending.worker_task);
        let config = &self.parts.settings.config;
        let context = CheckContext {
            worktree: &worktree,
            task_dir: &task_dir,
            run: None,
            check_seconds: config.plan().check_seconds(),
            check_refuse: config.plan().check_refuse(),
            envs: &[],
        };
        match execute_request_check(&context, step, &check).await {
            Ok(result) => {
                let tail = match &result.output_path {
                    Some(path) => read_check_tail(&task_dir.join(path)).await.unwrap_or_default(),
                    None => result.tail.clone(),
                };
                Some(Found {
                    outcome: request_check_outcome(&result),
                    line: result.check_line.clone().unwrap_or_else(|| text.to_owned()),
                    tail,
                    reason: result.reason.clone(),
                    duration_ms: result.duration_ms,
                    output: result.output_path.clone(),
                })
            }
            Err(error) => Some(Found {
                outcome: CheckOutcome::Failed,
                line: text.to_owned(),
                tail: String::new(),
                reason: Some(error_chain(&error)),
                duration_ms: 0,
                output: None,
            }),
        }
    }

    /// Decides what the checked result is worth and, when it is good, brings the worker's commit over.
    async fn settle_result(&self, pending: &Pending, checked: &Checked) -> Settlement {
        if checked.step.is_none() || (checked.files.is_empty() && checked.found.is_none()) {
            return Settlement::Report;
        }
        let limit = self.parts.settings.config.workers().seconds();
        let facts = facts_of(checked, pending.run_seconds, limit);
        match decide(&facts) {
            Verdict::Accept if facts.scope == Scope::Facts => Settlement::Report,
            Verdict::Accept if !checked.committed => Settlement::Returned {
                reasons: vec!["the worker committed nothing, so there is nothing to bring over".to_owned()],
            },
            Verdict::Accept => self.bring_over(pending, checked).await,
            Verdict::GiveToAnother { wrong } | Verdict::Return { wrong } => Settlement::Returned {
                reasons: reasons_of(checked, &wrong, limit),
            },
        }
    }

    /// Brings the worker's commit to the requester's branch, once: a commit that is there already is not brought again.
    async fn bring_over(&self, pending: &Pending, checked: &Checked) -> Settlement {
        let returned = |reason: String| Settlement::Returned { reasons: vec![reason] };
        let Some(head) = checked.head.as_deref().and_then(|head| CommitId::from_str(head).ok()) else {
            return returned("the worker's commit cannot be read".to_owned());
        };
        let Some(git) = self.git_of(&pending.team, &pending.requester_task, &pending.requester) else {
            return returned("the requester's worktree cannot be reached".to_owned());
        };
        match git.brought_over(&head).await {
            Ok(Some(commit)) => {
                return Settlement::Brought {
                    commit: commit.to_string(),
                };
            }
            Ok(None) => {}
            Err(error) => {
                return returned(format!(
                    "the requester's branch cannot be read: {}",
                    error_chain(&error)
                ));
            }
        }
        let Some(round) = self.requester_round(&pending.team, &pending.requester_task) else {
            return returned("the requester has no round to keep a snapshot of its worktree under".to_owned());
        };
        match git.bring_over(&head, &round, &pending.requester_task).await {
            Ok(BringOver::Applied { commit, .. }) => Settlement::Brought {
                commit: commit.to_string(),
            },
            Ok(BringOver::Conflicted { files, .. } | BringOver::InTheWay { files, .. }) => Settlement::Conflict {
                files: files.iter().map(|path| path.as_str().to_owned()).collect(),
            },
            Err(error) => returned(format!("the commit cannot be brought over: {}", error_chain(&error))),
        }
    }

    /// Answers the requester and the bus for a settled request.
    async fn answer_requester(
        &mut self,
        pending: &Pending,
        checked: &Checked,
        settlement: &Settlement,
    ) -> Result<(), OwnerError> {
        let (status, summary, notice) = match self.words_of(pending, checked, settlement) {
            Ok(words) => words,
            Err(error) => {
                tracing::error!(error = %error_chain(&error), "the answer to a request cannot be worded");
                return Ok(());
            }
        };
        if let (Settlement::Brought { commit }, Some(step)) = (settlement, checked.step) {
            self.record_confirmed(pending, checked, step, commit).await?;
        }
        tracing::info!(
            request = %pending.request,
            worker = %pending.worker,
            requester = %pending.requester,
            step = ?checked.step.map(|step| step.to_string()),
            outcome = settlement.outcome(),
            run_seconds = pending.run_seconds,
            "the result of a worker is settled"
        );
        let jobs = [
            BusJob::Answer {
                request: pending.request,
                update: DelegationUpdate {
                    status: Some(status),
                    task: None,
                    summary: Some(summary),
                },
            },
            BusJob::Tell {
                to: pending.requester.clone(),
                notice,
            },
        ];
        for job in jobs {
            self.tell_bus(&pending.team, job);
        }
        Ok(())
    }

    /// The status, the summary and the notice of the answer.
    fn words_of(
        &self,
        pending: &Pending,
        checked: &Checked,
        settlement: &Settlement,
    ) -> Result<(DelegationStatus, String, BusNotice), NoticeRenderError> {
        let texts: &TeamPrompts = &self.parts.team_texts;
        let step = checked.step.map_or_else(String::new, |step| step.to_string());
        let tail = checked
            .found
            .as_ref()
            .map(|found| found.tail.trim().to_owned())
            .filter(|tail| !tail.is_empty())
            .unwrap_or_else(|| NO_OUTPUT.to_owned());
        let files = checked.files.join(", ");
        let (status, text) = match settlement {
            Settlement::Report => (DelegationStatus::Done, pending.summary.clone()),
            Settlement::Brought { commit } => (
                DelegationStatus::Done,
                crate::daemon::team::worker_result_brought(texts, &pending.worker, &step, commit, &files, &tail)?,
            ),
            Settlement::Conflict { files } => (
                DelegationStatus::Failed,
                crate::daemon::team::worker_result_conflict(
                    texts,
                    &pending.worker,
                    &step,
                    checked.head.as_deref().unwrap_or_default(),
                    &files.join(", "),
                )?,
            ),
            Settlement::Returned { reasons } => (
                DelegationStatus::Failed,
                crate::daemon::team::worker_step_returned(texts, pending.request, &step, reasons)?,
            ),
        };
        let notice = match status {
            DelegationStatus::Done => BusNotice::HelperResult {
                request: pending.request,
                worker: pending.worker.clone(),
                summary: text.clone(),
            },
            _ => BusNotice::HelperFailed {
                request: pending.request,
                worker: pending.worker.clone(),
                reason: text.clone(),
            },
        };
        Ok((status, text, notice))
    }

    /// Records in the history that the check of step `step` of the requester's plan passed on the worker's commit.
    async fn record_confirmed(
        &mut self,
        pending: &Pending,
        checked: &Checked,
        step: ItemId,
        commit: &str,
    ) -> Result<(), OwnerError> {
        let at = Timestamp::now(&self.parts.clock)?;
        let record = agent_runtime::history::JudgementRecord {
            task: pending.requester_task.clone(),
            team: pending.team.clone(),
            run: None,
            at,
            step,
            state: JudgementState::Confirmed,
            check: checked.found.as_ref().map(|found| found.line.clone()),
            reason: Some(format!(
                "done by the worker {} (H{}), commit {commit}",
                pending.worker,
                pending.request.number()
            )),
            duration_ms: checked.found.as_ref().map_or(0, |found| found.duration_ms),
            output_path: checked.found.as_ref().and_then(|found| found.output.clone()),
        };
        self.record_judgements(&pending.requester_task, vec![record])
            .await
            .map(drop)
    }
}

#[cfg(test)]
mod tests;
