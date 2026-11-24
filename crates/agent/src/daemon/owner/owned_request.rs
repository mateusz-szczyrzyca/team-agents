//! Execution ownership shared by pool and team request settlement.

use crate::daemon::team::{BusNotice, render_notice};
use agent_core::decide::worker_result::WorkerEnding;
use agent_core::ids::{RunId, TaskId};
use agent_runtime::agentbus::{DelegationStatus, Outgoing, OwnedChange, OwnedOutcome, RequestOwner, RequestResult};
use agent_runtime::request_workspace::RequestWorkspace;
use agent_runtime::worker_request::Pending;
use agent_runtime::worker_request::recovery::{self, Journal, Phase, Transition};

use super::{Owner, OwnerError};

impl Owner {
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
    pub(super) async fn start_owned_team_request(&self, task: &TaskId, run: &RunId) -> Result<(), OwnerError> {
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
        let workspace = RequestWorkspace::try_from(agent_runtime::request_workspace::WorkspaceRecord {
            request: served.request,
            requester: served.requester.clone(),
            requester_task: served.requester_task.clone(),
            requester_round: self
                .requester_round(found.team.id(), &served.requester_task)
                .ok_or(OwnerError::RequestOwnership)?,
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
        let record = workspace.record();
        if !self.parts.bus.helper_recovery(record.request).await? {
            return Ok(());
        }
        let dir = self.parts.store.home().task_dir(&record.requester_task);
        if recovery::load(&dir, record.request).await?.is_none() {
            recovery::prepare(&dir, workspace).await?;
        }
        recovery::advance(
            &dir,
            record.request,
            Transition::Assigned {
                workspace: workspace.clone(),
                run: run.clone(),
            },
        )
        .await?;
        let result = self
            .parts
            .bus
            .helper_owned(
                record.request,
                RequestOwner {
                    worker: &record.worker,
                    task: &record.worker_task,
                    run,
                },
                OwnedChange::Start,
            )
            .await?;
        if result == OwnedOutcome::Stale {
            return Err(OwnerError::RequestOwnership);
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
            self.keep_worker_result(&team, &served, task, summary)?;
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
    ) -> Result<bool, OwnerError> {
        let Some(journal) = self.owned_journal(&pending.requester_task, pending.request).await? else {
            return Ok(false);
        };
        let Some(run) = assigned_run(&journal, &pending.worker_task, &pending.worker) else {
            tracing::warn!(event = "worker_result_dropped", request = %pending.request, "stale settlement dropped");
            return Ok(true);
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
        Ok(true)
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
                        journal.ending().unwrap_or(WorkerEnding::Done),
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
                return (owner == worker && &journal.workspace().record().worker_task == task).then_some(run);
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
