//! Execution ownership shared by pool and team request settlement.

use crate::daemon::team::{BusNotice, render_notice};
use agent_core::decide::worker_result::WorkerEnding;
use agent_core::ids::{RoundId, RunId, TaskId};
use agent_runtime::agentbus::{DelegationStatus, Outgoing, OwnedChange, OwnedOutcome, RequestOwner, RequestResult};
use agent_runtime::request_workspace::RequestWorkspace;
use agent_runtime::worker_request::Pending;
use agent_runtime::worker_request::recovery::{self, Journal, Phase, Transition};

use super::{Outcome, Owner, OwnerError, RunEnd, RunOutcome, RunVerdict};

/// Whether dispatch owns a newly prepared workspace or a retained one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WorkspaceUse {
    /// Prepared for a first or wrong-result try.
    New,
    /// Preserved across an interruption.
    Retained,
}

/// Whether startup ownership reconciliation permits worker dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RequestRecovery {
    /// Ownership is reconciled, or this owner has not begun startup recovery.
    Ready,
    /// The bus must become available before worker requests can be dispatched.
    Pending,
}

impl Owner {
    /// Records an owned run's declared ending or interruption before ordinary task settlement.
    pub(super) async fn settle_owned_run(
        &mut self,
        task: &TaskId,
        run_id: &RunId,
        outcome: &Outcome,
    ) -> Result<bool, OwnerError> {
        let (ending, summary) = match outcome {
            Outcome::Verdict(verdict) => match verdict.as_ref() {
                RunVerdict::Finished(run) => {
                    let ending = match &run.classified.end {
                        RunEnd::Reported {
                            outcome: RunOutcome::Done,
                        } => agent_core::decide::worker_result::WorkerEnding::Done,
                        RunEnd::Reported {
                            outcome: RunOutcome::Failed,
                        } => agent_core::decide::worker_result::WorkerEnding::Failed,
                        RunEnd::Reported {
                            outcome: RunOutcome::Questions,
                        } => agent_core::decide::worker_result::WorkerEnding::Cannot,
                        RunEnd::Reported { .. }
                        | RunEnd::NoStatus { .. }
                        | RunEnd::QuotaExhausted { .. }
                        | RunEnd::RateLimited
                        | RunEnd::Hung
                        | RunEnd::TurnLimit
                        | RunEnd::Stopped => agent_core::decide::worker_result::WorkerEnding::NoStatus,
                    };
                    (ending, run.final_message.clone().unwrap_or_default())
                }
                RunVerdict::Interrupted { .. } => (
                    agent_core::decide::worker_result::WorkerEnding::NoStatus,
                    "worker interrupted".to_owned(),
                ),
            },
            Outcome::Failed(reason) => (
                agent_core::decide::worker_result::WorkerEnding::NoStatus,
                reason.clone(),
            ),
        };
        let owned = if ending == agent_core::decide::worker_result::WorkerEnding::NoStatus {
            let cause = match outcome {
                Outcome::Verdict(verdict) => match verdict.as_ref() {
                    RunVerdict::Finished(run) => match &run.classified.end {
                        RunEnd::QuotaExhausted { .. } => agent_core::decide::worker_recovery::Cause::Quota,
                        RunEnd::RateLimited => agent_core::decide::worker_recovery::Cause::Refused,
                        RunEnd::Reported { .. }
                        | RunEnd::NoStatus { .. }
                        | RunEnd::Hung
                        | RunEnd::TurnLimit
                        | RunEnd::Stopped => agent_core::decide::worker_recovery::Cause::NoStatus,
                    },
                    RunVerdict::Interrupted { .. } => {
                        if self.shutdown.is_cancelled() {
                            agent_core::decide::worker_recovery::Cause::RestartRunning
                        } else {
                            agent_core::decide::worker_recovery::Cause::NoStatus
                        }
                    }
                },
                Outcome::Failed(_) => agent_core::decide::worker_recovery::Cause::Died,
            };
            self.interrupt_owned_request(task, run_id, cause).await?
        } else {
            self.observe_owned_request(task, run_id, ending, summary).await?
        };
        Ok(owned)
    }

    /// Retries startup reconciliation without making a bus outage fatal to unrelated tasks.
    pub(super) async fn retry_owned_recovery(&mut self) -> Result<(), OwnerError> {
        if self.request_recovery == RequestRecovery::Ready {
            return Ok(());
        }
        match self.recover_owned_requests().await {
            Ok(()) => self.request_recovery = RequestRecovery::Ready,
            Err(OwnerError::RequestBus(error)) => {
                // Worker dispatch remains fenced until a later look reconciles every saved attempt.
                tracing::warn!(
                    event = agent_core::log::LogEvent::BusDown.as_str(),
                    error = %agent_runtime::process::error_chain(&error),
                    "owned request recovery is waiting for the bus"
                );
            }
            Err(error) => return Err(error),
        }
        Ok(())
    }

    /// Reconciles durable owned attempts before the queue may dispatch a replacement.
    pub(super) async fn recover_owned_requests(&mut self) -> Result<(), OwnerError> {
        for entry in self.parts.bus.helper_list(false).await? {
            let Some(mut journal) = self.owned_journal(&entry.from_task, entry.id).await? else {
                // Requests created before recovery was enabled retain their original lifecycle.
                continue;
            };
            if journal.phase() == &Phase::Queued && entry.status == DelegationStatus::Running {
                let run = self
                    .parts
                    .bus
                    .helper_attempt_run(entry.id)
                    .await?
                    .ok_or(OwnerError::RequestOwnership)?;
                let worker = entry.worker.as_ref().ok_or(OwnerError::RequestOwnership)?;
                let workspace = journal.current_workspace();
                if assigned_run(&journal, &workspace.record().worker_task, worker) != Some(&run) {
                    // The bus claim is written first; a crash may precede its journal assignment.
                    let transition = if journal.events().is_empty() {
                        Transition::Assigned {
                            workspace: workspace.clone(),
                            run,
                        }
                    } else {
                        Transition::Started {
                            worker: worker.clone(),
                            run,
                        }
                    };
                    journal = recovery::advance(
                        &self.parts.store.home().task_dir(&entry.from_task),
                        entry.id,
                        transition,
                    )
                    .await?;
                }
            }
            let workspace = journal.current_workspace().record();
            let task = workspace.worker_task.clone();
            let worker = entry.worker.as_ref().ok_or(OwnerError::RequestOwnership)?;
            let served = super::teams::Served {
                request: entry.id,
                requester: entry.from.clone(),
                requester_task: entry.from_task.clone(),
                worker: worker.clone(),
                started: agent_core::state::Timestamp::now(&self.parts.clock)?,
                base: Some(workspace.base.to_string()),
            };
            match journal.phase() {
                Phase::Running(attempt) if journal.ending().is_none() => {
                    let run = attempt.run.clone();
                    self.serving.insert(task.clone(), served);
                    self.interrupt_owned_request(
                        &task,
                        &run,
                        agent_core::decide::worker_recovery::Cause::RestartRunning,
                    )
                    .await?;
                }
                Phase::Queued
                    if matches!(
                        journal.events().last(),
                        Some(Transition::Interrupted { .. } | Transition::RestartedQueued { .. })
                    ) =>
                {
                    let run = assigned_run(&journal, &task, worker).ok_or(OwnerError::RequestOwnership)?;
                    if entry.status == DelegationStatus::Running {
                        // The journal write survived but its bus update did not. Replay without charging another return.
                        self.finish_owned_interruption(&journal, &served, &task, run).await?;
                    } else {
                        self.restart_queued_request(&journal, &served, &task, run).await?;
                    }
                }
                Phase::Exhausted => {
                    let run = assigned_run(&journal, &task, worker).ok_or(OwnerError::RequestOwnership)?;
                    self.finish_owned_interruption(&journal, &served, &task, run).await?;
                }
                Phase::Running(_) | Phase::Queued | Phase::Settled => {}
            }
        }
        Ok(())
    }

    /// Snapshots queued partial work before replaying its retained assignment.
    async fn restart_queued_request(
        &mut self,
        journal: &Journal,
        served: &super::teams::Served,
        task: &TaskId,
        run: &RunId,
    ) -> Result<(), OwnerError> {
        let workspace = journal.current_workspace().record();
        let team = self
            .parts
            .state
            .find_task(&served.requester_task)
            .ok_or(OwnerError::RequestOwnership)?
            .team
            .id()
            .clone();
        let git = self
            .git_of(&team, task, &served.worker)
            .ok_or(OwnerError::RequestOwnership)?;
        let snapshot = git
            .snapshot(&workspace.requester_round, task)
            .await
            .map_err(OwnerError::RequestSnapshot)?;
        let updated = recovery::advance(
            &self.parts.store.home().task_dir(&served.requester_task),
            served.request,
            Transition::RestartedQueued {
                snapshot: recovery::SnapshotIdentity {
                    round: workspace.requester_round.clone(),
                    task: task.clone(),
                    number: std::num::NonZeroU32::new(snapshot.number).ok_or(OwnerError::RequestOwnership)?,
                    commit: snapshot.commit,
                    head: snapshot.head,
                },
            },
        )
        .await?;
        self.finish_owned_interruption(&updated, served, task, run).await?;
        Ok(())
    }

    /// Checks the durable observation before checking or bringing over an owned result.
    pub(super) async fn owns_observed_result(&self, pending: &Pending) -> Result<bool, OwnerError> {
        let Some(journal) = self.owned_journal(&pending.requester_task, pending.request).await? else {
            return Ok(true);
        };
        Ok(
            matches!(journal.phase(), Phase::Running(attempt) if attempt.worker == pending.worker)
                && journal.current_workspace().record().worker_task == pending.worker_task
                && journal.ending().is_some(),
        )
    }

    /// Reads the journal from the requester's task, independently of the serving task.
    pub(super) async fn owned_journal(
        &self,
        task: &TaskId,
        request: agent_runtime::agentbus::RequestId,
    ) -> Result<Option<Journal>, OwnerError> {
        Ok(recovery::load(&self.parts.store.home().task_dir(task), request).await?)
    }

    /// Binds a legacy crew worker's existing worktree without creating or releasing it.
    pub(super) async fn start_owned_team_request(&mut self, task: &TaskId, run: &RunId) -> Result<(), OwnerError> {
        let Some(served) = self.serving.get(task) else {
            return Ok(());
        };
        if !self.parts.bus.helper_recovery(served.request).await? {
            return Ok(());
        }
        let found = self.parts.state.find_task(task).ok_or(OwnerError::RequestOwnership)?;
        let git = self
            .git_of(found.team.id(), task, &served.worker)
            .ok_or(OwnerError::RequestOwnership)?;
        let base = git
            .rev_parse(&agent_runtime::git::Revision::Head)
            .await
            .map_err(OwnerError::RequestSnapshot)?;
        let requester_round = match self.requester_round(found.team.id(), &served.requester_task) {
            Some(round) => round,
            None => RoundId::generate(&self.parts.clock, &mut self.parts.rng)?,
        };
        let workspace = RequestWorkspace::try_from(agent_runtime::request_workspace::WorkspaceRecord {
            request: served.request,
            requester: served.requester.clone(),
            requester_task: served.requester_task.clone(),
            requester_round,
            requester_repo: found.team.repo().clone(),
            worker: served.worker.clone(),
            worker_task: task.clone(),
            branch: agent_runtime::request_workspace::branch(&served.worker, task)?,
            base,
            stage: agent_runtime::request_workspace::Stage::Working,
        })?;
        self.start_owned_request(&workspace, run).await
    }

    /// Claims a new opted-in request before its CLI can run.
    pub(super) async fn start_owned_request(
        &self,
        workspace: &RequestWorkspace,
        run: &RunId,
    ) -> Result<(), OwnerError> {
        self.start_owned_attempt(workspace, &workspace.record().worker, run, WorkspaceUse::New)
            .await
    }

    /// Claims a fresh execution while preserving the original workspace branch identity.
    pub(super) async fn start_owned_attempt(
        &self,
        workspace: &RequestWorkspace,
        worker: &agent_core::ids::AgentName,
        run: &RunId,
        usage: WorkspaceUse,
    ) -> Result<(), OwnerError> {
        let record = workspace.record();
        if !self.parts.bus.helper_recovery(record.request).await? {
            return Ok(());
        }
        let dir = self.parts.store.home().task_dir(&record.requester_task);
        if recovery::load(&dir, record.request).await?.is_none() {
            recovery::prepare(&dir, workspace).await?;
        }
        let result = self
            .parts
            .bus
            .helper_owned(
                record.request,
                RequestOwner {
                    worker,
                    task: &record.worker_task,
                    run,
                },
                OwnedChange::Start,
            )
            .await?;
        if result == OwnedOutcome::Stale {
            return Err(OwnerError::RequestOwnership);
        }
        recovery::advance(
            &dir,
            record.request,
            if usage == WorkspaceUse::Retained {
                Transition::Started {
                    worker: worker.clone(),
                    run: run.clone(),
                }
            } else {
                Transition::Assigned {
                    workspace: workspace.clone(),
                    run: run.clone(),
                }
            },
        )
        .await?;
        Ok(())
    }

    /// Snapshots a missing result before releasing ownership back to the queue.
    pub(super) async fn interrupt_owned_request(
        &mut self,
        task: &TaskId,
        run: &RunId,
        cause: agent_core::decide::worker_recovery::Cause,
    ) -> Result<bool, OwnerError> {
        let Some(served) = self.serving.get(task).cloned() else {
            return Ok(false);
        };
        let Some(journal) = self.owned_journal(&served.requester_task, served.request).await? else {
            return Ok(false);
        };
        if !matches!(journal.phase(), Phase::Running(attempt) if &attempt.run == run && attempt.worker == served.worker)
        {
            tracing::warn!(event = "worker_result_dropped", request = %served.request, %run, "interrupted execution no longer owns request");
            return Ok(true);
        }
        let team = self
            .parts
            .state
            .find_task(&served.requester_task)
            .ok_or(OwnerError::RequestOwnership)?
            .team
            .id()
            .clone();
        let git = self
            .git_of(&team, task, &served.worker)
            .ok_or(OwnerError::RequestOwnership)?;
        let round = &journal.current_workspace().record().requester_round;
        let snapshot = git.snapshot(round, task).await.map_err(OwnerError::RequestSnapshot)?;
        let updated = recovery::advance(
            &self.parts.store.home().task_dir(&served.requester_task),
            served.request,
            Transition::Interrupted {
                run: run.clone(),
                cause,
                snapshot: recovery::SnapshotIdentity {
                    round: round.clone(),
                    task: task.clone(),
                    number: std::num::NonZeroU32::new(snapshot.number).ok_or(OwnerError::RequestOwnership)?,
                    commit: snapshot.commit,
                    head: snapshot.head,
                },
            },
        )
        .await?;
        self.finish_owned_interruption(&updated, &served, task, run).await?;
        Ok(true)
    }

    /// Replays the bus half of a durable interruption, including a final bounded return.
    async fn finish_owned_interruption(
        &mut self,
        updated: &Journal,
        served: &super::teams::Served,
        task: &TaskId,
        run: &RunId,
    ) -> Result<(), OwnerError> {
        let team = self
            .parts
            .state
            .find_task(&served.requester_task)
            .ok_or(OwnerError::RequestOwnership)?
            .team
            .id()
            .clone();
        let causes;
        let notice;
        let change = if updated.phase() == &Phase::Exhausted {
            causes = updated
                .events()
                .iter()
                .filter_map(|event| match event {
                    Transition::Interrupted { cause, .. } => Some(match cause {
                        agent_core::decide::worker_recovery::Cause::Died => "died",
                        agent_core::decide::worker_recovery::Cause::Refused => "refused",
                        agent_core::decide::worker_recovery::Cause::Quota => "quota",
                        agent_core::decide::worker_recovery::Cause::NoStatus => "no_status",
                        agent_core::decide::worker_recovery::Cause::RestartQueued => "restart_queued",
                        agent_core::decide::worker_recovery::Cause::RestartRunning => "restart_running",
                    }),
                    Transition::RestartedQueued { .. } => Some("restart_queued"),
                    Transition::Started { .. }
                    | Transition::Assigned { .. }
                    | Transition::Rejected { .. }
                    | Transition::Observed { .. }
                    | Transition::Settled { .. } => None,
                })
                .collect::<Vec<_>>()
                .join(", ");
            let entries = self.parts.bus.helper_list(false).await?;
            let entry = entries
                .iter()
                .find(|entry| entry.id == served.request)
                .ok_or(OwnerError::RequestOwnership)?;
            let step = match &entry.step {
                Some(step) => step.as_str(),
                None => "unlabelled",
            };
            let values = agent_core::prompt::Variables::new()
                .with("request_id", served.request.number().to_string())
                .map_err(super::NoticeRenderError::from)?
                .with("worker", served.worker.to_string())
                .map_err(super::NoticeRenderError::from)?
                .with("step", step)
                .map_err(super::NoticeRenderError::from)?
                .with("causes", &causes)
                .map_err(super::NoticeRenderError::from)?;
            notice = self
                .parts
                .team_texts
                .render(agent_core::prompt::TeamPrompt::WorkerInterruptedReturned, &values)
                .map_err(super::NoticeRenderError::from)?;
            OwnedChange::Settle {
                result: RequestResult::Failed,
                summary: &causes,
                notice: &notice,
            }
        } else {
            OwnedChange::Retry
        };
        let outcome = self
            .parts
            .bus
            .helper_owned(
                served.request,
                RequestOwner {
                    worker: &served.worker,
                    task,
                    run,
                },
                change,
            )
            .await?;
        if outcome == OwnedOutcome::Stale {
            tracing::warn!(event = "worker_result_dropped", request = %served.request, %run, "interruption lost request ownership");
        }
        self.serving.remove(task);
        self.look_at(&team);
        if updated.phase() == &Phase::Exhausted {
            tracing::info!(event = "worker_request_returned", request = %served.request, %run, "worker request exhausted its interruption bound");
        } else {
            tracing::info!(event = "worker_request_requeued", request = %served.request, %run, "worker request retained for another execution");
        }
        Ok(())
    }

    /// Keeps a declared ending only for the execution that still owns this request.
    pub(super) async fn observe_owned_request(
        &mut self,
        task: &TaskId,
        run: &RunId,
        ending: WorkerEnding,
        summary: String,
    ) -> Result<bool, OwnerError> {
        let Some(served) = self.serving.get(task).cloned() else {
            return Ok(false);
        };
        let Some(journal) = self.owned_journal(&served.requester_task, served.request).await? else {
            return Ok(false);
        };
        let current = matches!(journal.phase(), Phase::Running(attempt) if &attempt.run == run && attempt.worker == served.worker)
            && &journal.current_workspace().record().worker_task == task;
        if !current {
            tracing::warn!(event = "worker_result_dropped", request = %served.request, %run, "late worker result dropped: execution no longer owns the request");
            return Ok(true);
        }
        let dir = self.parts.store.home().task_dir(&served.requester_task);
        recovery::advance(
            &dir,
            served.request,
            Transition::Observed {
                run: run.clone(),
                ending,
            },
        )
        .await?;
        if let Some(team) = self
            .parts
            .state
            .find_task(&served.requester_task)
            .map(|found| found.team.id().clone())
        {
            self.keep_worker_result(&team, &served, task, crate::daemon::team::worker_report(&summary))?;
            self.look_at(&team);
        }
        self.serving.remove(task);
        Ok(true)
    }
    /// Publishes an owned final notice atomically, including crash replays.
    pub(super) async fn settle_owned_request(
        &self,
        pending: &Pending,
        status: DelegationStatus,
        summary: &str,
        notice: &BusNotice,
    ) -> Result<Option<OwnedOutcome>, OwnerError> {
        let Some(journal) = self.owned_journal(&pending.requester_task, pending.request).await? else {
            return Ok(None);
        };
        let Some(run) = assigned_run(&journal, &pending.worker_task, &pending.worker) else {
            tracing::warn!(event = "worker_result_dropped", request = %pending.request, "stale settlement dropped");
            return Ok(Some(OwnedOutcome::Stale));
        };
        let result = match status {
            DelegationStatus::Done => RequestResult::Done,
            DelegationStatus::Failed => RequestResult::Failed,
            DelegationStatus::Unclear => RequestResult::Unclear,
            DelegationStatus::Queued | DelegationStatus::Starting | DelegationStatus::Running => {
                return Err(OwnerError::RequestOwnership);
            }
        };
        let text = match render_notice(&self.parts.team_texts, notice)? {
            Outgoing::Notice { text, .. } | Outgoing::Chat { text } => text,
        };
        let outcome = self
            .parts
            .bus
            .helper_owned(
                pending.request,
                RequestOwner {
                    worker: &pending.worker,
                    task: &pending.worker_task,
                    run,
                },
                OwnedChange::Settle {
                    result,
                    summary,
                    notice: &text,
                },
            )
            .await?;
        match outcome {
            OwnedOutcome::Applied | OwnedOutcome::Replay => {
                if matches!(journal.phase(), Phase::Running(_)) {
                    recovery::advance(
                        &self.parts.store.home().task_dir(&pending.requester_task),
                        pending.request,
                        Transition::Settled { run: run.clone() },
                    )
                    .await?;
                }
                if outcome == OwnedOutcome::Replay {
                    tracing::info!(event = "worker_result_dropped", request = %pending.request, %run, "settlement replay suppressed");
                }
            }
            OwnedOutcome::Stale => {
                tracing::warn!(event = "worker_result_dropped", request = %pending.request, %run, "late settlement dropped");
            }
        }
        Ok(Some(outcome))
    }

    /// Retains a snapshot and advances wrong-result ownership before the queue is released.
    pub(super) async fn retry_owned_request(&self, pending: &Pending) -> Result<Option<bool>, OwnerError> {
        let Some(journal) = self.owned_journal(&pending.requester_task, pending.request).await? else {
            return Ok(None);
        };
        let Some(run) = assigned_run(&journal, &pending.worker_task, &pending.worker) else {
            return Ok(Some(true));
        };
        let dir = self.parts.store.home().task_dir(&pending.requester_task);
        if matches!(journal.phase(), Phase::Running(_)) {
            let git = self
                .git_of(&pending.team, &pending.worker_task, &pending.worker)
                .ok_or(OwnerError::RequestOwnership)?;
            let round = &journal.current_workspace().record().requester_round;
            let snapshot = git
                .snapshot(round, &pending.worker_task)
                .await
                .map_err(OwnerError::RequestSnapshot)?;
            let identity = recovery::SnapshotIdentity {
                round: round.clone(),
                task: pending.worker_task.clone(),
                number: std::num::NonZeroU32::new(snapshot.number).ok_or(OwnerError::RequestOwnership)?,
                commit: snapshot.commit,
                head: snapshot.head,
            };
            recovery::advance(
                &dir,
                pending.request,
                Transition::Rejected {
                    run: run.clone(),
                    snapshot: identity,
                    wrong: super::worker_result::rejection_causes(
                        match &pending.stage {
                            agent_runtime::worker_request::Stage::Checked(checked)
                            | agent_runtime::worker_request::Stage::Settled { checked, .. } => checked,
                            agent_runtime::worker_request::Stage::Ended => return Err(OwnerError::RequestOwnership),
                        },
                        journal.ending().ok_or(OwnerError::RequestOwnership)?,
                        pending.run_seconds,
                        self.parts.settings.config.workers().seconds(),
                    ),
                },
            )
            .await?;
        } else if journal.phase() != &Phase::Queued {
            return Ok(Some(true));
        }
        let outcome = self
            .parts
            .bus
            .helper_owned(
                pending.request,
                RequestOwner {
                    worker: &pending.worker,
                    task: &pending.worker_task,
                    run,
                },
                OwnedChange::Retry,
            )
            .await?;
        Ok(Some(outcome == OwnedOutcome::Stale))
    }
}

/// Finds the last assignment, never an earlier execution's ownership.
fn assigned_run<'a>(journal: &'a Journal, task: &TaskId, worker: &agent_core::ids::AgentName) -> Option<&'a RunId> {
    for event in journal.events().iter().rev() {
        match event {
            Transition::Assigned { workspace, run } => {
                return (&workspace.record().worker_task == task && &workspace.record().worker == worker)
                    .then_some(run);
            }
            Transition::Started { worker: owner, run } => {
                return (owner == worker && &journal.current_workspace().record().worker_task == task).then_some(run);
            }
            Transition::Interrupted { .. }
            | Transition::RestartedQueued { .. }
            | Transition::Rejected { .. }
            | Transition::Observed { .. }
            | Transition::Settled { .. } => {}
        }
    }
    None
}

#[cfg(test)]
mod tests;
