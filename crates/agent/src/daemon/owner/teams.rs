//! The owner's side of the team's life: starting a team from a `run`, and keeping it going.
//!
//! The decisions are in `daemon/team`; this carries them out: git for the workspaces, the bus for the members' states,
//! the messages and the requests for workers, the state for the assignments, and the notification that a team is done.
//!
//! The owner looks at a team after something happened to it (a run ended, a team started) and on its clock, because
//! the bus wakes a sleeping member only by marking it `waking` and nothing else tells the daemon, and because an item
//! whose branch could not be started is tried again. A look reconciles the members' states on the bus with the tasks, gives out items, serves the requests
//! for workers and closes the team when its work is over. What cannot be done because the bus is down is kept and tried
//! again at the next look; a failure of the bus never ends the daemon.

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::str::FromStr;

use agent_core::cli::{McpServerKind, RunMode};
use agent_core::config::{AbsolutePath, CommandLine, Config};
use agent_core::decide::assign::first_free_worker;
use agent_core::decide::pool::{LastServed, PoolWorker, Requires, next_pool_worker, provider_of};
use agent_core::history::HistoryEvent;
use agent_core::ids::{AgentName, BranchName, RoundId, RunId, TaskId, TeamId};
use agent_core::kinds::{Role, Tier};
use agent_core::log::LogEvent;
use agent_core::plan::PLAN_FILE;
use agent_core::protocol::{EventKind, Failure, FailureKind, Reply, RunRequest};
use agent_core::state::{ItemPhase, RunEnd, Timestamp};
use agent_core::status::{ItemStatus, RunOutcome, TaskStatus};
use agent_runtime::agentbus::{
    AgentBus, AgentBusError, DelegationRequest, DelegationStatus, DelegationUpdate, FailureClass, MemberState,
    Outgoing, Recipient, RequestId, Woken,
};
use agent_runtime::disk::GIBIBYTE;
use agent_runtime::git::{CommitIdentity, DeleteMode, Git, GitError, RemoveMode, Revision, WorktreeBranch};
use agent_runtime::history::log_decision;
use agent_runtime::process::error_chain;
use agent_runtime::request_workspace::{self, RequestWorkspace, WorkspaceRecord};
use agent_runtime::run::compose::{PromptInput, PromptKind, PromptTexts, Serena, compose};
use agent_runtime::run::keep_out_of_git;
use agent_runtime::run::{RunJob, RunVerdict};
use agent_runtime::team_prompts::TeamPrompts;
use serde_json::json;
use tracing::Instrument;

use super::round_state;
use super::wakes::Wake;
use super::{Located, Outcome, Owner, OwnerError};
use crate::daemon::team::WorkerEnd;
use crate::daemon::team::{
    Assigned, BusJob, BusNotice, FormError, Formed, NoticeRenderError, RequestOutcome, Why, Workspace,
    assignable_with_candidates, closing_transitions, failure_reason, form, is_finished, item_in_flight, notice_for,
    reconcile, render_notice, request_outcome, shared_agents, start_item, team_done_notification,
};

/// The name the harness puts on the messages it writes to the bus, as the bus itself does.
pub(super) const BUS_SENDER: &str = "agentbus";

/// Something that is to be done on the bus, with the team it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Outbound {
    /// The team.
    pub(super) team: TeamId,
    /// What is to be done.
    pub(super) job: BusJob,
}

/// A request for a worker that is being served.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Served {
    /// The request.
    pub(super) request: RequestId,
    /// The member that asked and gets the result.
    pub(super) requester: AgentName,
    /// Its task.
    pub(super) requester_task: TaskId,
    /// The worker that serves it.
    pub(super) worker: AgentName,
    /// When the request was given to the worker.
    pub(super) started: Timestamp,
    /// The commit the worker's worktree was at then; `None` when it could not be read.
    pub(super) base: Option<String>,
}

/// The workers of a team and which of them cannot take a request now.
pub(super) struct Workers {
    /// The workers with their tasks, in the configured order.
    pub(super) in_order: Vec<(AgentName, TaskId)>,
    /// The workers that are at work or cannot work.
    pub(super) busy: Vec<AgentName>,
}

/// What is known of a worker agent's availability across the whole installation.
pub(in crate::daemon::owner) struct WorkerAvailability {
    /// Whether the worker has a run going installation-wide.
    pub(in crate::daemon::owner) busy: bool,
    /// How many runs the worker has going installation-wide.
    pub(in crate::daemon::owner) runs: u32,
    /// Whether the worker has an unsettled finished run.
    pub(in crate::daemon::owner) unsettled: bool,
}

/// An owned snapshot used while dispatch mutates the daemon state.
#[derive(Debug)]
struct OwnedPoolWorker {
    /// Configured worker identity.
    name: AgentName,
    /// Provider used for installation-wide rotation.
    provider: String,
    /// Declared requirements this worker can satisfy.
    capabilities: Vec<String>,
    /// Whether another request already reserves this worker.
    busy: bool,
    /// Ordinary runs counted against the configured limit.
    runs: u32,
    /// Whether a result or cleanup still needs settlement.
    unsettled: bool,
}

/// What taking a request from the bus for a worker came to.
enum Taken {
    /// The worker was given a request.
    Served,
    /// A request of another team was taken and is put back; the worker is still free.
    Held,
    /// There is no request, or the bus cannot say now.
    Nothing,
}

/// What one look at a team changed in the state.
#[derive(Debug, Default)]
pub(super) struct Look {
    /// What happened, for the subscribers.
    pub(super) events: Vec<EventKind>,
    /// Whether the state changed and is to be saved.
    pub(super) changed: bool,
}

/// Why a pool worker request cannot be dispatched.
#[derive(Debug, thiserror::Error)]
pub(super) enum PoolDispatchError {
    /// Durable execution ownership could not be established.
    #[error("cannot claim owned worker request")]
    Ownership(#[from] Box<super::OwnerError>),
    /// The requester task was not found in daemon state.
    #[error("requester task {0} not found in state")]
    RequesterNotFound(TaskId),
    /// A task, run, or session ID could not be generated.
    #[error("cannot generate id for pool worker: {0}")]
    GenerateId(#[from] agent_core::ids::GenerateIdError),
    /// A path is invalid or cannot be parsed as an absolute path.
    #[error("invalid path for pool worker: {0}")]
    InvalidPath(#[from] agent_core::config::InvalidValue),
    /// A path is not valid UTF-8.
    #[error("path is not valid UTF-8: {0:?}")]
    NonUtf8Path(std::path::PathBuf),
    /// A git operation on the requester worktree failed.
    #[error("git operation failed on requester worktree: {0}")]
    Git(#[from] agent_runtime::git::GitError),
    /// A request workspace operation failed.
    #[error("workspace error for pool worker: {0}")]
    Workspace(#[from] agent_runtime::request_workspace::WorkspaceError),
    /// The worker agent configuration was not found.
    #[error("worker agent configuration for {0} not found")]
    WorkerConfigNotFound(AgentName),
    /// Prompt composition failed.
    #[error("cannot compose prompt for pool worker: {0}")]
    PromptCompose(#[from] agent_runtime::run::compose::ComposeError),
    /// Prompt shaping failed.
    #[error("cannot shape prompt for pool worker: {0}")]
    PromptShape(#[from] crate::daemon::team::TeamPromptError),
    /// Timestamp retrieval failed.
    #[error("cannot get current timestamp: {0}")]
    Timestamp(#[from] agent_core::state::InvalidTimestamp),
}

impl Owner {
    // ---- starting a team ------------------------------------------------------------------------------------

    /// Refuses starting a team if free disk space is below minimum threshold.
    fn check_disk_for_run(&mut self) -> Result<(), Failure> {
        let Some(free_bytes) = self.check_free_space() else {
            return Err(Failure {
                kind: FailureKind::Refused,
                message: format!(
                    "cannot measure the free disk space of {}: no run starts until it can be measured; see the daemon log",
                    self.parts.store.home().root().display()
                ),
            });
        };
        let min_limit_gb = self.parts.settings.config.disk().min_free_gb;
        let min_limit_bytes = min_limit_gb.saturating_mul(GIBIBYTE);
        if free_bytes < min_limit_bytes {
            let free_gb = free_bytes / GIBIBYTE;
            return Err(Failure {
                kind: FailureKind::Refused,
                message: format!(
                    "not enough free disk space: {free_gb} GB free, minimum is {min_limit_gb} GB; remove build directories or unused worktrees in {}",
                    self.parts.store.home().worktrees_dir().display()
                ),
            });
        }
        Ok(())
    }

    /// Forms and starts the team `request` asks for; the answer is the refusal when it cannot be started.
    ///
    /// # Errors
    /// [`OwnerError`] when the state cannot be saved.
    pub(super) async fn run_request(&mut self, request: RunRequest) -> Result<Result<Reply, Failure>, OwnerError> {
        if let Err(refusal) = self.check_disk_for_run() {
            return Ok(Err(refusal));
        }
        let mut formed = match form(
            &request,
            &self.parts.settings.config,
            self.parts.store.home(),
            &self.parts.clock,
            &mut self.parts.rng,
        ) {
            Ok(formed) => formed,
            Err(refusal) => return Ok(Err(refusal_of(&refusal))),
        };
        if let Err(refusal) = self.preflight_base(&request, &mut formed).await {
            return Ok(Err(refusal));
        }
        let shared = shared_agents(&formed.team, self.parts.state.teams());
        let team_span = tracing::debug_span!("team", team = %formed.team.id());
        // The team goes into a copy of the state first: a team the state refuses leaves nothing behind, and the state the
        // owner holds changes only once the copy is saved.
        let mut next = self.parts.state.clone();
        if let Err(error) = next.insert_team(formed.team.clone()) {
            log_not_started(formed.team.id(), &error_chain(&error), "the state refuses the new team");
            return Ok(Err(internal(&format!(
                "the new team does not fit the state: {}",
                error_chain(&error)
            ))));
        }
        if let Err(failure) = self
            .make_workspaces(&request, &formed)
            .instrument(team_span.clone())
            .await
        {
            return Ok(Err(failure));
        }
        if let Some(bus_team) = &formed.bus
            && let Err(error) = self.parts.bus.team_create(bus_team).instrument(team_span.clone()).await
        {
            tracing::warn!(
                event = LogEvent::BusDown.as_str(),
                team = %formed.team.id(),
                error = %error_chain(&error),
                "the team cannot be created on the bus"
            );
            self.remove_workspaces(&request, &formed.workspaces)
                .instrument(team_span.clone())
                .await;
            return Ok(Err(Failure {
                kind: FailureKind::Internal,
                message: format!("cannot create the team on the agentbus: {}", error_chain(&error)),
            }));
        }
        let team = &formed.team;
        let id = team.id().clone();
        let tasks: Vec<TaskId> = team
            .crew()
            .roster()
            .into_iter()
            .map(|(_, member)| member.task().clone())
            .collect();
        let started = match started_events(team) {
            Ok(started) => started,
            Err(error) => {
                log_not_started(
                    &id,
                    &error_chain(&error),
                    "the events of the new team cannot be made: the team is not started",
                );
                self.undo_start(&request, &formed).instrument(team_span.clone()).await;
                return Ok(Err(internal(&format!(
                    "the new team has no round for an item it started: {}",
                    error_chain(&error)
                ))));
            }
        };
        if let Err(error) = self.parts.store.save(&next).await {
            tracing::error!(
                event = LogEvent::StateSaveFailed.as_str(),
                team = %id,
                error = %error_chain(&error),
                "the state with the new team cannot be saved: the team is not started"
            );
            self.undo_start(&request, &formed).instrument(team_span.clone()).await;
            return Ok(Err(internal(&format!(
                "cannot save the state with the new team: {}",
                error_chain(&error)
            ))));
        }
        self.parts.state = next;
        log_initial_lifecycle(&formed);
        self.record_team_start(&request, &id);
        self.publish_team_started(&id, &tasks, started)?;
        Ok(Ok(Reply::Started {
            team: id,
            tasks,
            shared,
        }))
    }

    /// Publishes the initial events and logs when a team starts.
    fn publish_team_started(
        &mut self,
        id: &TeamId,
        tasks: &[TaskId],
        started: Vec<EventKind>,
    ) -> Result<(), OwnerError> {
        let now = Timestamp::now(&self.parts.clock)?;
        self.publish(now, EventKind::TeamStarted { team: id.clone() });
        for event in started {
            self.publish(now, event);
        }
        self.publish_summary(now, id);
        tracing::info!(
            event = LogEvent::TeamChanged.as_str(),
            team = %id,
            change = "started",
            tasks = tasks.len(),
            "a team is started"
        );
        self.look_at(id);
        Ok(())
    }

    /// Fetches and checks the base before creating any team, task directory or worktree.
    async fn preflight_base(&self, request: &RunRequest, formed: &mut Formed) -> Result<(), Failure> {
        let Some(lead) = formed.workspaces.first() else {
            return Err(internal("the team has no task"));
        };
        let git = checkout_git(&self.parts.settings.config, request, &lead.agent);
        let evidence = git.unpushed_base(&formed.base).await.map_err(|error| Failure {
            kind: FailureKind::Refused,
            message: format!("cannot fetch or compare base {}: {}", formed.base, error_chain(&error)),
        })?;
        if let Some(base) = evidence {
            if !request.allow_unpushed_base {
                return Err(Failure {
                    kind: FailureKind::Refused,
                    message: format!(
                        "base branch {} has {} commits absent from origin/{}; first: {:.7} — {}. Push them, or start again after git reset --hard origin/{}; alternatively pass --allow-unpushed-base to retain this evidence in every pull request",
                        base.branch,
                        base.count,
                        base.branch,
                        base.first.as_str(),
                        base.subject,
                        base.branch
                    ),
                });
            }
            formed.team.record_unpushed_base(base);
        }
        Ok(())
    }

    /// Makes the worktree of every task of `formed`, from the checkout of the request; when one cannot be made, those
    /// that were are removed again.
    async fn make_workspaces(&self, request: &RunRequest, formed: &Formed) -> Result<(), Failure> {
        let Some(lead) = formed.workspaces.first() else {
            return Err(internal("the team has no task"));
        };
        let git = checkout_git(&self.parts.settings.config, request, &lead.agent);
        match git.branch_tip(&formed.base).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                return Err(Failure {
                    kind: FailureKind::Invalid,
                    message: format!(
                        "the base branch {} is not in the checkout {}: the worktrees start from it",
                        formed.base,
                        request.checkout.as_path().display()
                    ),
                });
            }
            Err(error) => return Err(git_failure("cannot look at the base branch of the checkout", &error)),
        }
        let mut made: Vec<Workspace> = Vec::new();
        for workspace in &formed.workspaces {
            let branch = WorktreeBranch::New {
                branch: workspace.branch.clone(),
                start: Revision::Branch(formed.base.clone()),
            };
            if let Err(error) = git
                .worktree_add(&workspace.path, &branch)
                .instrument(tracing::debug_span!("workspace", task = %workspace.task))
                .await
            {
                self.remove_workspaces(request, &made).await;
                return Err(git_failure(
                    &format!("cannot make the worktree of task {}", workspace.task),
                    &error,
                ));
            }
            made.push(workspace.clone());
        }
        // The file is shared by every worktree of the repository: once is enough, before any run could commit a plan.
        let plan = [format!("/{PLAN_FILE}")];
        if let Err(error) = keep_out_of_git(&lead.path, &plan).await {
            self.remove_workspaces(request, &made).await;
            return Err(internal(&format!(
                "cannot keep the plan of the tasks out of git: {}",
                error_chain(&error)
            )));
        }
        Ok(())
    }

    /// Removes what was made for the team of `formed`, which cannot be started: its worktrees and branches, and its team
    /// on the bus, which is marked done because the bus keeps its teams.
    async fn undo_start(&self, request: &RunRequest, formed: &Formed) {
        self.remove_workspaces(request, &formed.workspaces).await;
        if formed.bus.is_some()
            && let Err(error) = self.parts.bus.team_done(formed.team.id()).await
        {
            // A refusal that is being returned already: a bus team that cannot be closed is told, and left for the human.
            tracing::warn!(
                event = LogEvent::BusDown.as_str(),
                team = %formed.team.id(),
                error = %error_chain(&error),
                "the bus team of a refused team cannot be closed"
            );
        }
    }

    /// Removes worktrees and their branches that a request made and cannot use.
    async fn remove_workspaces(&self, request: &RunRequest, made: &[Workspace]) {
        let Some(lead) = made.first() else {
            return;
        };
        let git = checkout_git(&self.parts.settings.config, request, &lead.agent);
        for workspace in made {
            let span = tracing::debug_span!("workspace", task = %workspace.task);
            // A refusal that is being returned already: what cannot be removed is told, and left for the human.
            if let Err(error) = git
                .worktree_remove(&workspace.path, RemoveMode::Force)
                .instrument(span.clone())
                .await
            {
                tracing::warn!(
                    event = LogEvent::GitCallFailed.as_str(),
                    task = %workspace.task,
                    error = %error_chain(&error),
                    "a worktree of a refused team cannot be removed"
                );
            }
            if let Err(error) = git
                .branch_delete(&workspace.branch, DeleteMode::Force)
                .instrument(span)
                .await
            {
                tracing::warn!(
                    event = LogEvent::GitCallFailed.as_str(),
                    task = %workspace.task,
                    error = %error_chain(&error),
                    "a branch of a refused team cannot be removed"
                );
            }
        }
    }

    // ---- looking at teams -----------------------------------------------------------------------------------

    /// Wants to look at `team` at the next turn of the loop.
    pub(super) fn look_at(&mut self, team: &TeamId) {
        self.looks.insert(team.clone());
    }

    /// Wants to look at every team that is not closed.
    pub(super) fn look_at_every_team(&mut self) {
        let open: Vec<TeamId> = self
            .parts
            .state
            .teams()
            .iter()
            .map(|team| team.id().clone())
            .filter(|team| !self.closed.contains(team))
            .collect();
        for team in open {
            self.look_at(&team);
        }
    }

    /// Takes the teams that are finished and closed already, at the start, for closed: the human was told.
    pub(super) fn remember_closed_teams(&mut self) {
        self.closed = self
            .parts
            .state
            .teams()
            .iter()
            .filter(|team| is_finished(team) && closing_transitions(team).is_empty())
            .map(|team| team.id().clone())
            .collect();
    }

    /// What the bus is told and what the teams that were wanted need.
    ///
    /// # Errors
    /// [`OwnerError`] when the state cannot be saved or is found inconsistent.
    pub(super) async fn team_life(&mut self) -> Result<(), OwnerError> {
        self.brief_leads().await;
        self.queue_busy_handover_notices().await?;
        self.flush_outbox().await?;
        self.deliver_due_reminders().await;
        self.drop_closed_waits();
        let looks = std::mem::take(&mut self.looks);
        for team in looks {
            self.look_after(&team).await?;
        }
        Box::pin(self.serve_pool_requests()).await?;
        Ok(())
    }

    /// One look at `team`.
    async fn look_after(&mut self, team: &TeamId) -> Result<(), OwnerError> {
        if self.closed.contains(team) || self.parts.state.team(team).is_none() {
            return Ok(());
        }
        let span = tracing::error_span!("team", team = %team);
        async {
            self.resume_split_proposals(team).await?;
            let mut look = Look::default();
            self.reconcile_bus(team, &mut look).await?;
            self.requeue_underqualified(team, &mut look)?;
            self.pull_requests(team, &mut look).await?;
            self.assign_items(team, &mut look).await?;
            self.refresh_tier_waits(team, &mut look)?;
            self.ask_tier_waits(team).await?;
            self.settle_worker_results(team).await?;
            self.serve_requests(team, &mut look).await?;
            self.close_if_finished(team, &mut look)?;
            self.observe_round_waits(team);
            self.remind_to_commit_in(team).await;
            if look.changed {
                self.parts.state.record_activities(Timestamp::now(&self.parts.clock)?);
                self.save().await?;
                let now = Timestamp::now(&self.parts.clock)?;
                for event in look.events {
                    self.publish(now, event);
                }
                self.publish_summary(now, team);
            }
            if self.closed.contains(team) {
                self.cleanup_merged_worktrees(team).await;
            }
            Ok(())
        }
        .instrument(span)
        .await
    }

    /// Tells the subscribers the team's summary as it is.
    pub(super) fn publish_summary(&self, at: Timestamp, team: &TeamId) {
        if let Some(found) = self.parts.state.team(team) {
            self.publish(
                at,
                EventKind::TeamUpdated {
                    summary: Box::new(self.described(found)),
                },
            );
        }
    }

    // ---- the bus --------------------------------------------------------------------------------------------

    /// Does what is waiting for the bus, oldest first; stops at the first thing the bus does not take.
    pub(super) async fn flush_outbox(&mut self) -> Result<(), OwnerError> {
        while let Some(next) = self.outbox.front().cloned() {
            let carried = match self.carry_out(&next).await {
                Ok(done) => done,
                Err(error) if matches!(&next.job, BusJob::BusyHandOver { .. }) => {
                    if let BusJob::BusyHandOver { task, item } = &next.job {
                        tracing::warn!(
                            event = LogEvent::BusDown.as_str(),
                            team = %next.team,
                            task = %task,
                            item = %item,
                            error = %error_chain(&error),
                            "a busy hand-over notice can no longer be rendered and is settled"
                        );
                    }
                    Ok(())
                }
                Err(error) => return Err(error),
            };
            match carried {
                Ok(()) => {
                    self.outbox.pop_front();
                    // The lead was told of a member's work: nothing is owed to it any more.
                    if let BusJob::Tell {
                        notice: BusNotice::LeadTakesWork(work),
                        ..
                    } = &next.job
                    {
                        self.leads_being_told.remove(&work.task);
                        self.parts.state.lead_told(&work.task)?;
                        self.save().await?;
                    }
                    if let BusJob::BusyHandOver { task, .. } = &next.job {
                        let more = self
                            .outbox
                            .iter()
                            .any(|other| matches!(&other.job, BusJob::BusyHandOver { task: same, .. } if same == task));
                        if !more {
                            self.parts.state.busy_notice_told(task)?;
                            self.save().await?;
                        }
                    }
                }
                // What the bus refuses for good would block everything behind it for ever: it is dropped, once told.
                Err(error) if !is_transient(&error) => {
                    tracing::warn!(
                        event = LogEvent::BusDown.as_str(),
                        team = %next.team,
                        error = %error_chain(&error),
                        job = ?next.job,
                        "the bus refuses a request for good: it is dropped"
                    );
                    self.outbox.pop_front();
                    if let BusJob::BusyHandOver { task, .. } = &next.job {
                        let more = self
                            .outbox
                            .iter()
                            .any(|other| matches!(&other.job, BusJob::BusyHandOver { task: same, .. } if same == task));
                        if !more {
                            self.parts.state.busy_notice_told(task)?;
                            self.save().await?;
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        event = LogEvent::BusDown.as_str(),
                        team = %next.team,
                        error = %error_chain(&error),
                        "the bus does not answer: what is waiting for it is tried again at the next look"
                    );
                    break;
                }
            }
        }
        Ok(())
    }

    /// Does one thing on the bus. The outer error is the daemon's own, the inner one the bus's.
    pub(super) async fn carry_out(&self, outbound: &Outbound) -> Result<Result<(), AgentBusError>, OwnerError> {
        let bus = &self.parts.bus;
        Ok(match &outbound.job {
            BusJob::Tell { to, notice } => {
                let message = render_notice(&self.parts.team_texts, notice)?;
                let from: AgentName = BUS_SENDER.parse()?;
                let to = Recipient::Member {
                    agent: to.clone(),
                    team: outbound.team.clone(),
                };
                bus.send(&from, &to, &message).await.map(drop)
            }
            BusJob::BusyHandOver { task, item } => {
                let text = self.busy_handover_text(task, *item)?;
                let from: AgentName = BUS_SENDER.parse()?;
                let to = Recipient::Team(outbound.team.clone());
                bus.send(&from, &to, &Outgoing::Chat { text }).await.map(drop)
            }
            BusJob::Answer { request, update } => bus.helper_set(*request, update).await,
            BusJob::Finish => bus.team_done(&outbound.team).await,
        })
    }

    /// Makes the members' states on the bus those their tasks have, and starts the tasks of the members the bus woke.
    async fn reconcile_bus(&mut self, team_id: &TeamId, look: &mut Look) -> Result<(), OwnerError> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Ok(());
        };
        if team.crew().coordinator().is_none() {
            return Ok(());
        }
        let on_bus = match self.parts.bus.team_get(team_id).await {
            Ok(on_bus) => on_bus,
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::BusDown.as_str(),
                    team = %team_id,
                    error = %error_chain(&error),
                    "the team cannot be read from the bus: it is tried again at the next look"
                );
                return Ok(());
            }
        };
        let plan = reconcile(team, &on_bus)?;
        let mut held = Vec::new();
        for task in plan.wake {
            if self.wake_member(team_id, &task, look).await? == Wake::Held {
                held.push(task);
            }
        }
        for (member, state) in plan.set {
            // A member that is not woken stays as the bus has it, woken: it is not made active, and not woken again at the
            // next look.
            if state == MemberState::Active
                && held
                    .iter()
                    .any(|task| self.task_on_bus(team_id, &member).as_ref() == Some(task))
            {
                continue;
            }
            match self.parts.bus.team_set(team_id, &member, state).await {
                // A member that went to sleep while something waits for it is woken at once.
                Ok(Woken::Message | Woken::Board) if state == MemberState::Sleeping => {
                    if let Some(task) = self.task_on_bus(team_id, &member) {
                        self.wake_member(team_id, &task, look).await?;
                    }
                }
                // A member that went to sleep with nothing waiting read what woke it.
                Ok(Woken::No) if state == MemberState::Sleeping => {
                    if let Some(task) = self.task_on_bus(team_id, &member) {
                        self.wake_settled(&task, look);
                    }
                }
                Ok(Woken::Message | Woken::Board | Woken::No) => {}
                Err(error) => {
                    tracing::warn!(
                        event = LogEvent::BusDown.as_str(),
                        team = %team_id,
                        %member,
                        error = %error_chain(&error),
                        "the state of a member cannot be set on the bus: it is tried again at the next look"
                    );
                    break;
                }
            }
        }
        Ok(())
    }

    /// The task of the member that the bus of `team` knows as `agent`: the agent its task was first given to, whoever
    /// holds it now.
    fn task_on_bus(&self, team: &TeamId, agent: &AgentName) -> Option<TaskId> {
        self.parts.state.team(team).and_then(|found| {
            found
                .crew()
                .roster()
                .into_iter()
                .find(|(_, member)| member.bus_agent().name() == agent.as_str())
                .map(|(_, member)| member.task().clone())
        })
    }

    /// The task of the member named `agent` in `team`.
    pub(super) fn task_of(&self, team: &TeamId, agent: &AgentName) -> Option<TaskId> {
        self.parts.state.team(team).and_then(|found| {
            found
                .crew()
                .roster()
                .into_iter()
                .find(|(_, member)| member.agent().name() == agent.as_str())
                .map(|(_, member)| member.task().clone())
        })
    }

    /// Queues the task `task` to start with `why`: a waiting task is queued, a task that is about to start
    /// starts with it; a task that is at work is left alone.
    pub(super) fn release(&mut self, task: &TaskId, why: Why, look: &mut Look) -> Result<(), OwnerError> {
        let located = self.locate(task)?;
        let status = self
            .parts
            .state
            .find_task(task)
            .map(|found| found.member.status())
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
        match status {
            TaskStatus::Waiting => {
                self.move_to(task, &located.team, TaskStatus::Queued, &mut look.events)?;
                look.changed = true;
            }
            // About to start: it starts with its reason.
            TaskStatus::Queued => {}
            TaskStatus::Running
            | TaskStatus::Questions
            | TaskStatus::Paused
            | TaskStatus::Stopped
            | TaskStatus::Failed
            | TaskStatus::Done => return Ok(()),
        }
        self.reasons.insert(task.clone(), why);
        Ok(())
    }

    // ---- items and requests ---------------------------------------------------------------------------------

    /// Detaches a parent assignee when approval raised the first part's required tier.
    fn requeue_underqualified(&mut self, team_id: &TeamId, look: &mut Look) -> Result<(), OwnerError> {
        let team = self
            .parts
            .state
            .team(team_id)
            .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
        let items = agent_core::decide::assign::dispatch::underqualified(team);
        for item in items {
            let mut team = self
                .parts
                .state
                .team_mut(team_id)
                .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
            team.item_mut(item)
                .ok_or_else(|| OwnerError::LostItem {
                    team: team_id.clone(),
                    item,
                })?
                .advance(ItemPhase::Pending)
                .map_err(|source| OwnerError::Item {
                    team: team_id.clone(),
                    item,
                    source,
                })?;
            look.changed = true;
        }
        Ok(())
    }

    /// Configured replacement participants with their actual tiers, excluding the current team.
    fn split_replacements(&self, team: &agent_core::state::Team) -> Vec<agent_core::state::Participant> {
        let replacement = self.parts.settings.config.replacement();
        let crew = team.crew().roster();
        let outside: Vec<_> = replacement
            .agents()
            .iter()
            .filter(|name| {
                !crew.iter().any(|(_, member)| member.agent().name() == name.as_str())
                    && !team
                        .reviewers()
                        .iter()
                        .any(|member| member.agent().name() == name.as_str())
            })
            .filter_map(|name| self.participant_of(name))
            .collect();
        outside
    }

    /// Records absent capability once, retaining the wait through daemon recovery.
    fn refresh_tier_waits(&mut self, team_id: &TeamId, look: &mut Look) -> Result<(), OwnerError> {
        let replacement = self.parts.settings.config.replacement();
        let found = self
            .parts
            .state
            .team(team_id)
            .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
        let outside = self.split_replacements(found);
        let busy = self.agents_in_runs();
        let candidates: Vec<_> = outside
            .iter()
            .map(|agent| agent_core::decide::part::Candidate {
                agent,
                status: TaskStatus::Waiting,
                handed_over: false,
                busy: busy.contains(&agent.name()),
            })
            .collect();
        let mut team = self
            .parts
            .state
            .team_mut(team_id)
            .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
        let previous = team.tier_waits().clone();
        let entered = team.refresh_tier_waits_with_candidates(replacement.mode(), replacement.agents(), &candidates);
        look.changed |= previous != *team.tier_waits();
        for (item, tier) in entered {
            tracing::info!(event = LogEvent::TeamChanged.as_str(), kind = "tier_wait", team = %team_id,
                item = %item, required_tier = tier.as_str(),
                "item waits because no crew member meets its required tier");
        }
        Ok(())
    }

    /// Gives the pending items to the members that are free.
    async fn assign_items(&mut self, team_id: &TeamId, look: &mut Look) -> Result<(), OwnerError> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Ok(());
        };
        let replacement = self.parts.settings.config.replacement();
        let outside = self.split_replacements(team);
        let busy = self.agents_in_runs();
        let candidates: Vec<_> = outside
            .iter()
            .map(|agent| agent_core::decide::part::Candidate {
                agent,
                status: TaskStatus::Waiting,
                handed_over: false,
                busy: busy.contains(&agent.name()),
            })
            .collect();
        let given = assignable_with_candidates(team, replacement.mode(), replacement.agents(), &candidates).map_err(
            |source| OwnerError::Assign {
                team: team_id.clone(),
                source,
            },
        )?;
        let repo = team.repo().clone();
        for assigned in given {
            let Some(base) = self
                .parts
                .settings
                .config
                .repo(&repo)
                .map(|repo| repo.base_branch.clone())
            else {
                tracing::error!(
                    event = LogEvent::ConfigurationNameMissing.as_str(),
                    team = %team_id,
                    kind = "repo",
                    name = %repo,
                    "the repository of the team is not in the configuration: no item is given out"
                );
                return Ok(());
            };
            if let Err(error) = self.switch_branch(&repo, &assigned, base).await {
                tracing::warn!(
                    event = LogEvent::GitCallFailed.as_str(),
                    team = %team_id,
                    task = %assigned.task,
                    item = %assigned.item,
                    member = %assigned.agent,
                    error = %error_chain(&error),
                    "the worktree of a member cannot take the branch of its next item: the item waits and is tried again at the next look"
                );
                continue;
            }
            let round = self.activate_assignment(team_id, &assigned, look)?;
            look.events.push(EventKind::ItemStatus {
                team: team_id.clone(),
                item: assigned.item,
                round: round.clone(),
                status: ItemStatus::InProgress,
            });
            look.changed = true;
            let span = tracing::error_span!(
                "item_assign",
                team = %team_id,
                rounds = %round,
                task = %assigned.task
            );
            {
                let _entered = span.enter();
                tracing::info!(
                    event = LogEvent::ItemPhaseChanged.as_str(),
                    item = %assigned.item,
                    phase = "in_progress",
                    member = %assigned.agent,
                    "an item is given to a member"
                );
            }
        }
        Ok(())
    }

    /// Commits the holder, item and round together before a queued task can start.
    fn activate_assignment(
        &mut self,
        team_id: &TeamId,
        assigned: &Assigned,
        look: &mut Look,
    ) -> Result<agent_core::ids::RoundId, OwnerError> {
        let previous = self.parts.state.clone();
        let mut pending = Look::default();
        // No await or persistence occurs inside this transaction. A failed transition
        // restores the holder and queue along with the item and its round.
        let activated = (|| {
            let now = Timestamp::now(&self.parts.clock)?;
            self.open_item_round(team_id, assigned.item, now, &mut pending)?;
            self.record_assignment(team_id, assigned)?;
            self.hand_over_split_task(assigned, team_id, &mut pending)?;
            let round = self.round_of_item(team_id, assigned.item)?;
            self.release(&assigned.task, Why::Assigned(assigned.item), &mut pending)?;
            Ok(round)
        })();
        match activated {
            Ok(round) => {
                look.events.extend(pending.events);
                look.changed |= pending.changed;
                Ok(round)
            }
            Err(error) => {
                self.parts.state = previous;
                Err(error)
            }
        }
    }

    /// Starts a fresh holder session when a split part selects another agent.
    fn hand_over_split_task(
        &mut self,
        assigned: &Assigned,
        team_id: &TeamId,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let found = self
            .parts
            .state
            .find_task(&assigned.task)
            .ok_or_else(|| OwnerError::LostTask {
                task: assigned.task.clone(),
            })?;
        if found.member.agent().name() != assigned.agent.as_str() {
            let from = found.member.agent().name().to_owned();
            let holder = self
                .participant_of(&assigned.agent)
                .ok_or_else(|| OwnerError::LostTask {
                    task: assigned.task.clone(),
                })?;
            let now = Timestamp::now(&self.parts.clock)?;
            self.parts
                .state
                .hand_over_task(&assigned.task, holder, now)
                .map_err(|source| OwnerError::HandOver {
                    task: assigned.task.clone(),
                    source,
                })?;
            self.move_to(&assigned.task, team_id, TaskStatus::Queued, &mut look.events)?;
            look.changed = true;
            tracing::info!(event = LogEvent::TaskHandedOver.as_str(), team = %team_id,
                task = %assigned.task, from, to = %assigned.agent,
                "the next split part goes to a configured replacement in a fresh session");
        }
        Ok(())
    }

    /// Records that the member of `assigned` works on its item.
    fn record_assignment(&mut self, team_id: &TeamId, assigned: &Assigned) -> Result<(), OwnerError> {
        let mut team_mut = self
            .parts
            .state
            .team_mut(team_id)
            .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
        let mut item = team_mut.item_mut(assigned.item).ok_or_else(|| OwnerError::LostItem {
            team: team_id.clone(),
            item: assigned.item,
        })?;
        start_item(&mut item, assigned).map_err(|source| OwnerError::Item {
            team: team_id.clone(),
            item: assigned.item,
            source,
        })
    }

    /// Starts the branch of an item in the worktree of the member that is given it.
    async fn switch_branch(
        &self,
        repo: &agent_core::config::RepoName,
        assigned: &Assigned,
        base: agent_core::ids::BranchName,
    ) -> Result<(), SwitchError> {
        let path = self.parts.store.home().worktree_dir(repo, &assigned.task);
        let text = path.to_str().ok_or_else(|| SwitchError::Path {
            path: path.clone(),
            source: None,
        })?;
        let worktree = AbsolutePath::from_str(text).map_err(|source| SwitchError::Path {
            path: path.clone(),
            source: Some(source),
        })?;
        let identity = CommitIdentity::for_agent(self.parts.settings.config.git(), &assigned.agent);
        let git = Git::new(worktree, identity);
        let start = item_start(&git, base).await.map_err(SwitchError::Git)?;
        git.switch_new_branch(&assigned.branch, &start)
            .await
            .map_err(SwitchError::Git)
    }

    /// Gives the requests for workers to the free workers, the oldest request to the first worker in the configured
    /// order.
    ///
    /// The bus hands out the oldest queued request of every team. A request of another team is held while the queue is
    /// read on, and put back before the next team is looked at: each team's workers serve its own requests, and a team
    /// with a free worker does not keep another team's request from it.
    async fn serve_requests(&mut self, team_id: &TeamId, look: &mut Look) -> Result<(), OwnerError> {
        let Some(Workers {
            in_order: workers,
            mut busy,
        }) = self.workers_of(team_id)?
        else {
            return Ok(());
        };
        let names: Vec<AgentName> = workers.iter().map(|(name, _)| name.clone()).collect();
        let mut held = Vec::new();
        // A request given back for a second try goes to a worker of another provider when one is free.
        while let Some(worker) = self.second_try_pick(team_id, &workers, &busy) {
            match self.take_request(team_id, (&worker, &workers), &mut held, look).await? {
                Taken::Served => busy.push(worker),
                Taken::Held => {}
                Taken::Nothing => break,
            }
        }
        while let Some(worker) = first_free_worker(&names, &busy).cloned() {
            match self.take_request(team_id, (&worker, &workers), &mut held, look).await? {
                Taken::Served => busy.push(worker),
                Taken::Held => {}
                Taken::Nothing => break,
            }
        }
        self.put_back(team_id, held).await;
        Ok(())
    }

    /// Whether the agent `name` is a worker in an active (not stopped) team crew.
    pub(in crate::daemon::owner) fn is_in_active_crew(&self, name: &AgentName) -> bool {
        self.parts.state.teams().iter().any(|t| {
            !round_state::is_stopped(t)
                && t.crew()
                    .roster()
                    .iter()
                    .any(|(role, m)| *role == Role::Worker && m.agent().name() == name.as_str())
        })
    }

    /// Evaluates the busy state, total runs and unsettled status of a worker agent across all teams.
    pub(in crate::daemon::owner) async fn worker_availability(&self, name: &AgentName) -> WorkerAvailability {
        self.worker_availability_except(name, None).await
    }

    /// Checks capacity while allowing the current result to be considered for its own retry.
    pub(in crate::daemon::owner) async fn worker_availability_except(
        &self,
        name: &AgentName,
        request: Option<RequestId>,
    ) -> WorkerAvailability {
        let running_in_teams = agent_core::decide::slot::runs_going(self.parts.state.teams())
            .into_iter()
            .filter(|g| g.agent == *name)
            .count();
        let serving_runs = self.serving.values().filter(|s| s.worker == *name).count();
        let total_runs = u32::try_from(running_in_teams)
            .unwrap_or(u32::MAX)
            .saturating_add(u32::try_from(serving_runs).unwrap_or(u32::MAX));
        let busy = serving_runs > 0;
        let mut unsettled = false;

        for team in self.parts.state.teams() {
            for (_, member) in team.crew().roster() {
                let task_dir = self.parts.store.home().task_dir(member.task());
                match request_workspace::read_each(&task_dir).await {
                    Ok(read) => {
                        for res in read {
                            match res {
                                Ok(workspace) => {
                                    if workspace.record().worker == *name && Some(workspace.record().request) != request
                                    {
                                        unsettled = true;
                                        break;
                                    }
                                }
                                Err(error) => {
                                    tracing::warn!(
                                        event = LogEvent::WorkerRequestDamaged.as_str(),
                                        task = %member.task(),
                                        error = %error_chain(&error),
                                        "workspace record cannot be read: conservatively treating worker as unsettled"
                                    );
                                    unsettled = true;
                                    break;
                                }
                            }
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            event = LogEvent::WorkerRequestDamaged.as_str(),
                            task = %member.task(),
                            error = %error_chain(&error),
                            "workspace dir cannot be read: conservatively treating worker as unsettled"
                        );
                        unsettled = true;
                    }
                }
                if unsettled {
                    break;
                }
            }
            if unsettled {
                break;
            }
        }

        if self
            .cleanup_obligations
            .iter()
            .any(|o| o.workspace.record().worker == *name)
        {
            unsettled = true;
        }

        WorkerAvailability {
            busy,
            runs: total_runs,
            unsettled,
        }
    }

    /// Serves queued requests for workers with free workers of the installation pool.
    #[expect(
        clippy::too_many_lines,
        reason = "loop checks busy state across teams, selects provider turn, dispatches, and handles held requests"
    )]
    pub(super) async fn serve_pool_requests(&mut self) -> Result<(), OwnerError> {
        self.retry_cleanup_obligations().await;
        let pool = self.parts.settings.config.worker_agents().to_vec();
        if pool.is_empty() {
            return Ok(());
        }
        let run_limit = agent_core::decide::slot::limit_for(self.parts.settings.config.limits(), Tier::Worker);

        let open_requests = match self.parts.bus.helper_list(false).await {
            Ok(list) => list,
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::BusDown.as_str(),
                    error = %error_chain(&error),
                    "the queue of requests for pool workers cannot be read"
                );
                return Ok(());
            }
        };
        if open_requests.is_empty() {
            return Ok(());
        }

        // Recover stranded Starting helpers that have no active serving task
        for req in &open_requests {
            if req.status == DelegationStatus::Starting
                && req
                    .worker
                    .as_ref()
                    .is_some_and(|worker| pool.contains(worker) && !self.is_in_active_crew(worker))
                && !self.serving.values().any(|s| s.request == req.id)
            {
                let update = DelegationUpdate {
                    status: Some(DelegationStatus::Queued),
                    task: None,
                    summary: None,
                };
                if let Err(err) = self.parts.bus.helper_set(req.id, &update).await {
                    tracing::warn!(
                        event = LogEvent::BusDown.as_str(),
                        request = %req.id,
                        error = %error_chain(&err),
                        "cannot reset stranded starting request to queued"
                    );
                }
            }
        }

        let mut pool_workers_data = Vec::new();
        for name in &pool {
            let Some(agent_conf) = self.parts.settings.config.agent(name) else {
                continue;
            };
            let provider = provider_of(agent_conf.cli, agent_conf.model.as_ref()).to_owned();
            let capabilities = agent_conf.capabilities.clone();
            let avail = self.worker_availability(name).await;
            pool_workers_data.push(OwnedPoolWorker {
                name: name.clone(),
                provider,
                capabilities,
                busy: avail.busy || self.is_in_active_crew(name),
                runs: avail.runs,
                unsettled: avail.unsettled,
            });
        }

        for req in open_requests {
            if req.status != DelegationStatus::Queued {
                continue;
            }
            let requester_team_id = self
                .parts
                .state
                .teams()
                .iter()
                .find(|t| t.find(&req.from_task).is_some())
                .map(|t| t.id().clone());
            let team_has_workers = requester_team_id.as_ref().is_some_and(|team_id| {
                self.parts
                    .state
                    .team(team_id)
                    .is_some_and(|t| t.crew().roster().iter().any(|(role, _)| *role == Role::Worker))
            });
            if team_has_workers {
                if let Some(ref team_id) = requester_team_id {
                    self.look_at(team_id);
                }
                continue;
            }

            if !self.launch_allowed().await {
                break;
            }
            let requires = Requires {
                capabilities: &req.requires,
            };

            let chosen = {
                let pool_workers: Vec<PoolWorker<'_>> = pool_workers_data
                    .iter()
                    .map(|w| PoolWorker {
                        name: &w.name,
                        provider: &w.provider,
                        capabilities: w.capabilities.as_slice(),
                        busy: w.busy,
                        runs: w.runs,
                        unsettled: w.unsettled,
                    })
                    .collect();

                if let Some(again) = self.again.get(&req.id) {
                    agent_core::decide::pool::second_try_pool_worker(
                        &pool_workers,
                        &requires,
                        &again.worker,
                        &again.failed_provider,
                        run_limit,
                    )
                    .map(|(name, prov)| (name.clone(), prov.to_owned()))
                } else {
                    let last_served = LastServed(self.pool_dispatch.last_served_provider.as_deref());
                    next_pool_worker(&pool_workers, &requires, last_served, run_limit)
                        .map(|(name, prov)| (name.clone(), prov.to_owned()))
                }
            };

            let Some((worker_name, provider)) = chosen else {
                continue;
            };

            // Immediately reserve the chosen worker in pool_workers_data to avoid double dispatch in same pass
            if let Some(entry) = pool_workers_data.iter_mut().find(|w| w.name == worker_name) {
                entry.busy = true;
                entry.runs = entry.runs.saturating_add(1);
            }

            let worker_caps = self
                .parts
                .settings
                .config
                .agent(&worker_name)
                .map(|a| a.capabilities.clone())
                .unwrap_or_default();

            let request = match self
                .parts
                .bus
                .helper_next_with(&worker_name, &worker_caps, Some(req.id))
                .await
            {
                Ok(Some(request)) => request,
                Ok(None) => {
                    let avail = self.worker_availability(&worker_name).await;
                    if let Some(entry) = pool_workers_data.iter_mut().find(|w| w.name == worker_name) {
                        entry.busy = avail.busy;
                        entry.runs = avail.runs;
                        entry.unsettled = avail.unsettled;
                    }
                    continue;
                }
                Err(error) => {
                    tracing::warn!(
                        event = LogEvent::BusDown.as_str(),
                        worker = %worker_name,
                        request = %req.id,
                        error = %error_chain(&error),
                        "the request cannot be claimed from the bus"
                    );
                    let avail = self.worker_availability(&worker_name).await;
                    if let Some(entry) = pool_workers_data.iter_mut().find(|w| w.name == worker_name) {
                        entry.busy = avail.busy;
                        entry.runs = avail.runs;
                        entry.unsettled = avail.unsettled;
                    }
                    continue;
                }
            };

            if let Taken::Served = self.dispatch_pool_request(&worker_name, request).await {
                self.pool_dispatch.last_served_provider = Some(provider);
            }

            let avail = self.worker_availability(&worker_name).await;
            if let Some(entry) = pool_workers_data.iter_mut().find(|w| w.name == worker_name) {
                entry.busy = avail.busy;
                entry.runs = avail.runs;
                entry.unsettled = avail.unsettled;
            }
        }

        Ok(())
    }

    /// Dispatches a single request to a dedicated pool worker workspace.
    async fn dispatch_pool_request(&mut self, worker_name: &AgentName, request: DelegationRequest) -> Taken {
        if let Err(error) = self.try_dispatch_pool_request(worker_name, &request).await {
            tracing::warn!(
                event = LogEvent::WorkerRequestDamaged.as_str(),
                request = %request.id,
                worker = %worker_name,
                error = %error_chain(&error),
                "failed to dispatch pool worker request; returning request to queue"
            );
            let update = DelegationUpdate {
                status: Some(DelegationStatus::Queued),
                task: None,
                summary: None,
            };
            if let Err(bus_err) = self.parts.bus.helper_set(request.id, &update).await {
                tracing::warn!(
                    event = LogEvent::BusDown.as_str(),
                    request = %request.id,
                    error = %error_chain(&bus_err),
                    "cannot reset request status to queued after dispatch failure"
                );
            }
            return Taken::Held;
        }
        Taken::Served
    }

    #[expect(
        clippy::too_many_lines,
        reason = "dispatching a pool worker prepares the dedicated workspace, derives the branch, shapes the prompt, configures MCP, and spawns the job"
    )]
    pub(in crate::daemon::owner) async fn try_dispatch_pool_request(
        &mut self,
        worker_name: &AgentName,
        request: &DelegationRequest,
    ) -> Result<(), PoolDispatchError> {
        let Some(found) = self.parts.state.find_task(&request.from_task) else {
            return Err(PoolDispatchError::RequesterNotFound(request.from_task.clone()));
        };
        let team = found.team.clone();
        let requester_task = request.from_task.clone();
        let requester_repo = team.repo().clone();

        let worker_task = TaskId::generate(&self.parts.clock, &mut self.parts.rng)?;
        let run = RunId::generate(&self.parts.clock, &mut self.parts.rng)?;

        let requester_worktree = self.parts.store.home().worktree_dir(&requester_repo, &requester_task);
        let requester_path_str = requester_worktree
            .to_str()
            .ok_or_else(|| PoolDispatchError::NonUtf8Path(requester_worktree.clone()))?;
        let requester_abs = AbsolutePath::from_str(requester_path_str)?;
        let requester_git = Git::new(
            requester_abs,
            CommitIdentity::for_agent(self.parts.settings.config.git(), &request.from),
        );

        let base_commit = requester_git.rev_parse(&Revision::Head).await?;

        let requester_round = match self.requester_round(team.id(), &requester_task) {
            Some(r) => r,
            None => RoundId::generate(&self.parts.clock, &mut self.parts.rng)?,
        };
        let branch = request_workspace::branch(worker_name, &worker_task)?;

        let workspace = RequestWorkspace::try_from(WorkspaceRecord {
            request: request.id,
            requester: request.from.clone(),
            requester_task: requester_task.clone(),
            requester_round,
            requester_repo: requester_repo.clone(),
            worker: worker_name.clone(),
            worker_task: worker_task.clone(),
            branch: branch.clone(),
            base: base_commit.clone(),
            stage: request_workspace::Stage::Working,
        })?;

        request_workspace::prepare(self.parts.store.home(), &requester_git, &workspace).await?;

        let again_saved = self.again.get(&request.id).cloned();

        let step_result: Result<(), PoolDispatchError> = async {
            let worker_worktree_path =
                request_workspace::worktree_of(self.parts.store.home(), &requester_repo, &worker_task)?;
            let working_dir = worker_worktree_path.clone();

            let Some(agent_conf) = self.parts.settings.config.agent(worker_name) else {
                return Err(PoolDispatchError::WorkerConfigNotFound(worker_name.clone()));
            };
            let setup_cli = agent_conf.cli;
            let setup_model = agent_conf.model.clone();
            let setup_args = agent_conf.args.clone();
            let setup_context = agent_conf.context;
            let setup_output = agent_conf.output;

            let questions_file = self.parts.store.home().questions_file(&worker_task, NonZeroU32::MIN);
            let session_id = super::start::new_session_id(&mut self.parts.rng)?;
            let mode = RunMode::New { session_id };

            let given = self.given_again(team.id(), request, worker_name, &worker_task);

            let input = PromptInput {
                kind: PromptKind::First,
                agent: worker_name.as_str(),
                tier: Tier::Worker,
                role: Role::Worker,
                task: &worker_task,
                repo: &requester_repo,
                worktree: worker_worktree_path.as_path(),
                questions_file: &questions_file,
                number: NonZeroU32::MIN,
                brief: "",
                item: None,
                team_items: &[],
                checkpoint: None,
                serena: Serena::Without,
            };
            let texts = PromptTexts {
                harness: &self.parts.texts,
                rules: &self.parts.rules,
            };
            let composed = compose(&texts, &input)?;

            let prompt = crate::daemon::team::shape_pool_worker(&self.parts.team_texts, &composed, &given)?;

            let binaries = self.parts.settings.config.binaries();
            let home_abs = self.parts.store.home().absolute_root().clone();
            let bus_dir = AgentBus::new(binaries.agentbus.clone(), home_abs.clone()).bus_dir();

            let agentbus_bin = binaries
                .agentbus
                .as_path()
                .to_str()
                .ok_or_else(|| PoolDispatchError::NonUtf8Path(binaries.agentbus.as_path().to_path_buf()))?
                .to_owned();
            let task_dir_buf = self.parts.store.home().task_dir(&worker_task);
            let task_dir_str = task_dir_buf
                .to_str()
                .ok_or_else(|| PoolDispatchError::NonUtf8Path(task_dir_buf.clone()))?
                .to_owned();
            let worktree_str = worker_worktree_path
                .as_path()
                .to_str()
                .ok_or_else(|| PoolDispatchError::NonUtf8Path(worker_worktree_path.as_path().to_path_buf()))?
                .to_owned();
            let home_str = home_abs
                .as_path()
                .to_str()
                .ok_or_else(|| PoolDispatchError::NonUtf8Path(home_abs.as_path().to_path_buf()))?
                .to_owned();
            let bus_dir_str = bus_dir
                .to_str()
                .ok_or_else(|| PoolDispatchError::NonUtf8Path(bus_dir.clone()))?
                .to_owned();

            let mut words = vec![
                agentbus_bin,
                "serve".to_owned(),
                "--agent".to_owned(),
                worker_name.to_string(),
                "--task".to_owned(),
                worker_task.to_string(),
                "--task-dir".to_owned(),
                task_dir_str,
                "--repo".to_owned(),
                requester_repo.to_string(),
                "--worktree".to_owned(),
                worktree_str,
                "--branch".to_owned(),
                branch.to_string(),
                "--tier".to_owned(),
                "worker".to_owned(),
                "--home".to_owned(),
                home_str,
                "--dir".to_owned(),
                bus_dir_str,
            ];
            let workers = self.parts.settings.config.worker_agents();
            if !workers.is_empty() {
                let names: Vec<&str> = workers.iter().map(AgentName::as_str).collect();
                words.extend(["--workers".to_owned(), names.join(",")]);
            }
            let mut mcp_servers = BTreeMap::new();
            let cmd = CommandLine::try_from(words)?;
            mcp_servers.insert(McpServerKind::Agentbus, cmd);

            // Everything fallible must finish before a job owns the workspace.
            let recovery_enabled = self
                .parts
                .bus
                .helper_recovery(request.id)
                .await
                .map_err(|error| Box::new(super::OwnerError::from(error)))?;
            let started = Timestamp::now(&self.parts.clock)?;
            self.start_owned_request(&workspace, &run).await.map_err(Box::new)?;
            let identity = CommitIdentity::for_agent(self.parts.settings.config.git(), worker_name);
            let job = RunJob {
                task: worker_task.clone(),
                run,
                cli: setup_cli,
                model: setup_model,
                extra_args: setup_args,
                mode,
                prompt,
                working_dir,
                identity,
                mcp_servers,
                context: setup_context,
                output: setup_output,
            };

            self.record_pool_run(
                worker_task.clone(),
                Served {
                    request: request.id,
                    requester: request.from.clone(),
                    requester_task,
                    worker: worker_name.clone(),
                    started,
                    base: Some(base_commit.to_string()),
                },
                team.id().clone(),
            );

            let run_span = tracing::error_span!("task", team = %team.id(), task = %worker_task);
            self.spawn_job(worker_task.clone(), job, run_span);

            if !recovery_enabled {
                self.tell_bus(
                    team.id(),
                    request_update(request.id, DelegationStatus::Running, Some(worker_task.clone())),
                );
            }
            tracing::info!(
                event = LogEvent::RequestDelegated.as_str(),
                team = %team.id(),
                request = %request.id,
                %worker_name,
                "a request is given to a pool worker"
            );
            Ok(())
        }
        .await;

        if let Err(err) = step_result {
            if let Some(again) = again_saved {
                self.again.insert(request.id, again);
            }
            if let Err(cleanup_err) =
                request_workspace::release(self.parts.store.home(), &requester_git, &workspace).await
            {
                tracing::warn!(
                    event = LogEvent::WorkerRequestDamaged.as_str(),
                    request = %request.id,
                    error = %error_chain(&cleanup_err),
                    "cannot release workspace after failed dispatch"
                );
                self.cleanup_obligations.push(CleanupObligation {
                    requester_git,
                    workspace,
                });
            }
            return Err(err);
        }

        Ok(())
    }

    /// Retries outstanding cleanup obligations, freeing workers whose workspace release failed earlier.
    pub(in crate::daemon::owner) async fn retry_cleanup_obligations(&mut self) {
        if self.cleanup_obligations.is_empty() {
            return;
        }
        let mut remaining = Vec::new();
        for obligation in std::mem::take(&mut self.cleanup_obligations) {
            match request_workspace::release(
                self.parts.store.home(),
                &obligation.requester_git,
                &obligation.workspace,
            )
            .await
            {
                Ok(()) => {
                    tracing::info!(
                        event = LogEvent::RequestDelegated.as_str(),
                        request = %obligation.workspace.record().request,
                        worker = %obligation.workspace.record().worker,
                        "cleaned up pending workspace obligation after earlier release failure"
                    );
                }
                Err(err) => {
                    tracing::warn!(
                        event = LogEvent::WorkerRequestDamaged.as_str(),
                        request = %obligation.workspace.record().request,
                        error = %error_chain(&err),
                        "retrying workspace release failed; retaining cleanup obligation"
                    );
                    remaining.push(obligation);
                }
            }
        }
        self.cleanup_obligations = remaining;
    }

    /// A run of a pool worker that ended.
    pub(super) fn settle_pool_worker_run(
        &mut self,
        task: &TaskId,
        served: &Served,
        outcome: Outcome,
    ) -> Result<(), OwnerError> {
        let team_id = self
            .parts
            .state
            .find_task(&served.requester_task)
            .map(|f| f.team.id().clone());
        let req_outcome = match outcome {
            Outcome::Verdict(verdict) => match *verdict {
                RunVerdict::Finished(run) => {
                    request_outcome(&run.classified.end, TaskStatus::Done, run.final_message.as_deref())
                }
                RunVerdict::Interrupted { .. } => Some(RequestOutcome::Failed {
                    how: WorkerEnd::Stopped,
                    said: None,
                }),
            },
            Outcome::Failed(reason) => Some(RequestOutcome::Failed {
                how: WorkerEnd::NoStatus,
                said: Some(reason),
            }),
        };
        if let Some(team_id) = team_id {
            match req_outcome {
                Some(RequestOutcome::Done { summary }) => {
                    if let Err(error) = self.keep_worker_result(&team_id, served, task, summary.clone()) {
                        tracing::warn!(
                            event = LogEvent::WorkerRequestDamaged.as_str(),
                            request = %served.request,
                            error = %error_chain(&error),
                            "the result of a pool worker cannot be kept: the requester is given the worker's report unchecked"
                        );
                        Self::drop_try(&self.parts.store.home().task_dir(task), served.request);
                        for job in answer_jobs(&self.parts.team_texts, served, RequestOutcome::Done { summary })? {
                            self.outbox.push_back(Outbound {
                                team: team_id.clone(),
                                job,
                            });
                        }
                        self.answered_requests.insert(served.request);
                    }
                    self.look_at(&team_id);
                }
                Some(outcome @ RequestOutcome::Failed { .. }) => {
                    Self::drop_try(&self.parts.store.home().task_dir(task), served.request);
                    for job in answer_jobs(&self.parts.team_texts, served, outcome)? {
                        self.outbox.push_back(Outbound {
                            team: team_id.clone(),
                            job,
                        });
                    }
                    self.answered_requests.insert(served.request);
                    self.look_at(&team_id);
                }
                None => {
                    self.look_at(&team_id);
                }
            }
        }
        Ok(())
    }

    /// Takes the oldest queued request for `worker` from the bus and starts the worker's run on it.
    async fn take_request(
        &mut self,
        team_id: &TeamId,
        (worker, workers): (&AgentName, &[(AgentName, TaskId)]),
        held: &mut Vec<RequestId>,
        look: &mut Look,
    ) -> Result<Taken, OwnerError> {
        let request = match self.parts.bus.helper_next(worker).await {
            Ok(Some(request)) => request,
            Ok(None) => return Ok(Taken::Nothing),
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::BusDown.as_str(),
                    team = %team_id,
                    %worker,
                    error = %error_chain(&error),
                    "the queue of requests for workers cannot be read: it is tried again at the next look"
                );
                return Ok(Taken::Nothing);
            }
        };
        let Some((_, task)) = workers.iter().find(|(name, _)| name == worker).cloned() else {
            return Ok(Taken::Nothing);
        };
        if !self.is_member(team_id, &request.from_task) {
            held.push(request.id);
            return Ok(Taken::Held);
        }
        log_decision(
            "first_free_worker",
            &json!({"workers": workers.iter().map(|(name, _)| name).collect::<Vec<_>>()}),
            &json!({"worker": worker, "request_id": request.id}),
        );
        // The commit the worker's worktree is at: the files the worker changes are those since.
        let base = self.worker_head(team_id, &task, worker).await;
        let started = Timestamp::now(&self.parts.clock)?;
        let given = self.given_again(team_id, &request, worker, &task);
        self.release(&task, Why::Delegated(Box::new(given)), look)?;
        self.serving.insert(
            task.clone(),
            Served {
                request: request.id,
                requester: request.from.clone(),
                requester_task: request.from_task.clone(),
                worker: worker.clone(),
                started,
                base,
            },
        );
        if !self.parts.bus.helper_recovery(request.id).await? {
            self.tell_bus(
                team_id,
                request_update(request.id, DelegationStatus::Running, Some(task)),
            );
        }
        tracing::info!(
            event = LogEvent::RequestDelegated.as_str(),
            team = %team_id,
            request = %request.id,
            %worker,
            "a request is given to a worker"
        );
        Ok(Taken::Served)
    }

    /// Puts the requests of other teams that the look at `team_id` took from the bus back in the queue at once, so that
    /// the next team looked at finds them; one the bus does not take now is tried again at the next turn of the loop.
    async fn put_back(&mut self, team_id: &TeamId, held: Vec<RequestId>) {
        for request in held {
            let update = DelegationUpdate {
                status: Some(DelegationStatus::Queued),
                task: None,
                summary: None,
            };
            if let Err(error) = self.parts.bus.helper_set(request, &update).await {
                tracing::warn!(
                    event = LogEvent::BusDown.as_str(),
                    team = %team_id,
                    %request,
                    error = %error_chain(&error),
                    "a request of another team cannot be put back now: it is tried again at the next turn"
                );
                self.tell_bus(team_id, BusJob::Answer { request, update });
            }
        }
    }

    /// The workers of `team` in the configured order with their tasks, and those that cannot take a request now;
    /// `None` for a team that is not there or has no workers to ask.
    pub(super) fn workers_of(&self, team_id: &TeamId) -> Result<Option<Workers>, OwnerError> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Ok(None);
        };
        if team.crew().coordinator().is_none() {
            return Ok(None);
        }
        let mut workers = Vec::new();
        let mut busy = Vec::new();
        for (_, member) in team
            .crew()
            .roster()
            .into_iter()
            .filter(|(role, _)| *role == Role::Worker)
        {
            let name: AgentName = member.agent().name().parse()?;
            // A worker whose result is not settled yet is busy: its worktree is what the check reads and the commit comes from.
            if member.status() != TaskStatus::Waiting
                || self.serving.contains_key(member.task())
                || agent_runtime::worker_request::has_pending_except(
                    &self.parts.store.home().task_dir(member.task()),
                    &self.answered_requests,
                )
            {
                busy.push(name.clone());
            }
            workers.push((name, member.task().clone()));
        }
        Ok(Some(Workers {
            in_order: workers,
            busy,
        }))
    }

    /// Whether `task` is a task of `team`.
    fn is_member(&self, team: &TeamId, task: &TaskId) -> bool {
        self.parts
            .state
            .team(team)
            .is_some_and(|found| found.find(task).is_some())
    }

    /// Keeps `job` for the bus: it is done at the next turn of the loop, and again until the bus takes it.
    pub(super) fn tell_bus(&mut self, team: &TeamId, job: BusJob) {
        self.outbox.push_back(Outbound {
            team: team.clone(),
            job,
        });
    }

    // ---- the end of a run -----------------------------------------------------------------------------------

    /// A solo task whose item is under way waits for it, and for the next one, instead of ending.
    pub(super) fn solo_waits_for_its_items(
        &self,
        role: Role,
        team: &TeamId,
        settled: super::Settled,
    ) -> super::Settled {
        match settled {
            super::Settled::Status(TaskStatus::Done)
                if role == Role::Solo && self.parts.state.team(team).is_some_and(item_in_flight) =>
            {
                super::Settled::Status(TaskStatus::Waiting)
            }
            other => other,
        }
    }

    /// A reviewer whose review is given waits for the next one while its team goes on, instead of ending: the team has one
    /// task for an investigator, which reviews every pull request of the team and the same one again after a fix pass.
    pub(super) fn reviewer_waits(&self, role: Role, team: &TeamId, settled: super::Settled) -> super::Settled {
        match settled {
            super::Settled::Status(TaskStatus::Done)
                if role == Role::Reviewer && self.parts.state.team(team).is_some_and(|found| !is_finished(found)) =>
            {
                super::Settled::Status(TaskStatus::Waiting)
            }
            other => other,
        }
    }

    /// What the end of a run of a team's task means for the others: the coordinator is told about a member, a requester
    /// about its request; and the team is looked at.
    ///
    /// # Errors
    /// [`OwnerError`] when a name of the team is not an agent name.
    pub(super) fn after_run(
        &mut self,
        task: &TaskId,
        located: &Located,
        end: &RunEnd,
        message: Option<&str>,
    ) -> Result<(), OwnerError> {
        if let Some(found) = self.parts.state.find_task(task)
            && found.team.crew().coordinator().is_some()
        {
            // A member's work that is its lead's now is told as such, with its briefing, not as a failure: the work is owed
            // to the lead in the state, and the briefing is gathered at the next turn of the loop.
            let told = if found.member.owes_lead() {
                None
            } else if matches!(
                end,
                &RunEnd::Reported {
                    outcome: RunOutcome::Failed
                }
            ) && found.member.status() != TaskStatus::Failed
            {
                // A failed run that the daemon retried for "no tool call" is not a failure of the task: the task is
                // queued again in a new session, and the coordinator is told only if the retried run fails too.
                None
            } else {
                notice_for(found.team, located.role, found.member, end, message)?
            };
            if let Some(told) = told {
                self.outbox.push_back(Outbound {
                    team: located.team.clone(),
                    job: BusJob::Tell {
                        to: told.to,
                        notice: told.notice,
                    },
                });
            }
            let served = self.serving.get(task).cloned();
            if let Some(served) = served
                && let Some(outcome) = request_outcome(end, found.member.status(), message)
            {
                self.serving.remove(task);
                match outcome {
                    // The result of a worker that ended DONE is checked, and its commit brought over, before the requester
                    // is told: the next look at the team settles it.
                    RequestOutcome::Done { summary } => {
                        if let Err(error) = self.keep_worker_result(&located.team, &served, task, summary.clone()) {
                            // Without its file the result cannot be settled: the requester gets the worker's report, as
                            // before, rather than nothing.
                            tracing::warn!(
                                event = LogEvent::WorkerRequestDamaged.as_str(),
                                request = %served.request,
                                error = %error_chain(&error),
                                "the result of a worker cannot be kept: the requester is given the worker's report unchecked"
                            );
                            Self::drop_try(&self.parts.store.home().task_dir(task), served.request);
                            for job in answer_jobs(&self.parts.team_texts, &served, RequestOutcome::Done { summary })? {
                                self.outbox.push_back(Outbound {
                                    team: located.team.clone(),
                                    job,
                                });
                            }
                        }
                    }
                    outcome @ RequestOutcome::Failed { .. } => {
                        // The request is answered here, not from a file: the try that came with it goes with it.
                        Self::drop_try(&self.parts.store.home().task_dir(task), served.request);
                        for job in answer_jobs(&self.parts.team_texts, &served, outcome)? {
                            self.outbox.push_back(Outbound {
                                team: located.team.clone(),
                                job,
                            });
                        }
                    }
                }
            }
        }
        self.look_at(&located.team);
        Ok(())
    }

    // ---- the end of a team ----------------------------------------------------------------------------------

    /// Closes `team` when its work is over: its waiting tasks end, the bus is told, and so is the human, once.
    fn close_if_finished(&mut self, team_id: &TeamId, look: &mut Look) -> Result<(), OwnerError> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Ok(());
        };
        if self.closed.contains(team_id) || !is_finished(team) {
            return Ok(());
        }
        let transitions = closing_transitions(team);
        let has_bus = team.crew().coordinator().is_some();
        let lead = team
            .crew()
            .roster()
            .first()
            .map(|(_, member)| member.task().clone())
            .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
        let now = Timestamp::now(&self.parts.clock)?;
        let context = self.notice_context_for(&lead, now, None, None)?;
        let notification = team_done_notification(&self.parts.team_texts, team, &context)?;
        for (task, status) in transitions {
            self.move_to(&task, team_id, status, &mut look.events)?;
            look.changed = true;
        }
        // The round of the whole work of a list without items, which nothing merges, ends with the team.
        let now = Timestamp::now(&self.parts.clock)?;
        self.abandon_open_rounds(team_id, now, look)?;
        let waiting_reviewers: Vec<TaskId> = self
            .parts
            .state
            .team(team_id)
            .map(|found| {
                found
                    .reviewers()
                    .iter()
                    .filter(|member| member.status() == TaskStatus::Waiting)
                    .map(|member| member.task().clone())
                    .collect()
            })
            .unwrap_or_default();
        for task in waiting_reviewers {
            self.move_to(&task, team_id, TaskStatus::Done, &mut look.events)?;
            look.changed = true;
        }
        if has_bus {
            self.outbox.push_back(Outbound {
                team: team_id.clone(),
                job: BusJob::Finish,
            });
        }
        self.closed.insert(team_id.clone());
        let rounds = self.parts.state.team(team_id).map_or_else(Vec::new, |found| {
            found.rounds().iter().map(|round| round.id().clone()).collect()
        });
        self.record_event_at(now, team_id, rounds, HistoryEvent::TeamEnded {});
        let located = self.locate(&lead)?;
        let now = Timestamp::now(&self.parts.clock)?;
        self.notify(now, &located, &lead, notification);
        tracing::info!(
            event = LogEvent::TeamChanged.as_str(),
            team = %team_id,
            change = "closed",
            "a team is finished and closed"
        );
        Ok(())
    }
}

/// Why the branch of an item cannot be started in the worktree of a member.
#[derive(Debug, thiserror::Error)]
enum SwitchError {
    /// The worktree is not a path git can be run in.
    #[error("the worktree {path:?} cannot be used")]
    Path {
        /// The path.
        path: std::path::PathBuf,
        /// What is wrong with it, if the text was refused.
        source: Option<agent_core::config::InvalidValue>,
    },
    /// Git refused.
    #[error("git cannot start the branch")]
    Git(#[source] GitError),
}

/// Where the branch of a new item starts: the base branch, or the commit the remote's base branch was at when it was last
/// fetched, when that commit has the local base branch in its history. A pull request that was merged on GitHub moved the
/// remote's branch and not the checkout's, and the next item builds on what was merged. A remote branch that is not
/// there, or is not ahead, leaves the local one.
async fn item_start(git: &Git, base: BranchName) -> Result<Revision, GitError> {
    let local = Revision::Branch(base.clone());
    let remote = match git.rev_parse(&Revision::RemoteBranch(base)).await {
        Ok(tip) => tip,
        // No fetch has brought the remote's branch: the local one is all there is.
        Err(GitError::UnknownRevision { .. }) => return Ok(local),
        Err(error) => return Err(error),
    };
    let remote = Revision::Commit(remote);
    if git.is_ancestor(&local, &remote).await? {
        Ok(remote)
    } else {
        Ok(local)
    }
}

/// The git of the checkout of a request, as `agent` commits.
fn checkout_git(config: &Config, request: &RunRequest, agent: &AgentName) -> Git {
    Git::new(request.checkout.clone(), CommitIdentity::for_agent(config.git(), agent))
}

/// The refusal of a request that does not form a team; what the daemon got wrong is logged, and the client is told
/// only that.
fn refusal_of(error: &FormError) -> Failure {
    let failure = error.failure();
    if failure.kind == FailureKind::Internal {
        tracing::error!(
            event = LogEvent::TeamNotStarted.as_str(),
            error = %error_chain(error),
            "a team cannot be formed"
        );
    }
    failure
}

/// Logs that the team `team`, which the daemon formed, is not started because of `error`, as `what` says.
fn log_not_started(team: &TeamId, error: &str, what: &str) {
    tracing::error!(event = LogEvent::TeamNotStarted.as_str(), team = %team, error, "{what}");
}

/// An internal failure with `message`.
fn internal(message: &str) -> Failure {
    Failure {
        kind: FailureKind::Internal,
        message: message.to_owned(),
    }
}

/// The refusal for a git that failed: what was being done, and what git said.
fn git_failure(what: &str, error: &GitError) -> Failure {
    Failure {
        kind: FailureKind::Internal,
        message: format!("{what}: {}", error_chain(error)),
    }
}

/// Whether an error of the bus may pass: a bus that is not there, is slow or has trouble with its files is tried again;
/// one that refuses the request itself is not.
pub(super) fn is_transient(error: &AgentBusError) -> bool {
    matches!(
        error.class(),
        None | Some(FailureClass::Storage | FailureClass::Process | FailureClass::Unclassified)
    )
}

/// Logs the transitions that `form` already applied and the successful state save has now made durable.
fn log_initial_lifecycle(formed: &Formed) {
    let team = &formed.team;
    for round in team.rounds() {
        let span = tracing::error_span!("round", team = %team.id(), rounds = %round.id());
        let _entered = span.enter();
        tracing::info!(
            event = LogEvent::RoundChanged.as_str(),
            team = %team.id(),
            round = %round.id(),
            change = "opened",
            item = ?round.item(),
            "a round is opened"
        );
    }
    for progress in team.items() {
        let ItemPhase::InProgress { assignee, .. } = progress.phase() else {
            continue;
        };
        let Some(round) = team.round_of(progress.id()) else {
            continue;
        };
        let Some(workspace) = formed.workspaces.iter().find(|workspace| workspace.agent == *assignee) else {
            continue;
        };
        let span = tracing::error_span!(
            "item_assign",
            team = %team.id(),
            rounds = %round.id(),
            task = %workspace.task
        );
        let _entered = span.enter();
        tracing::info!(
            event = LogEvent::ItemPhaseChanged.as_str(),
            item = %progress.id(),
            phase = "in_progress",
            member = %assignee,
            "an item is given to a member"
        );
    }
}

/// The events of a new team: its rounds, which were opened with it, and the items it starts with — those that have a
/// member already.
///
/// # Errors
/// [`OwnerError::LostRound`] for an item that has started and has no round: a mistake of the daemon.
fn started_events(team: &agent_core::state::Team) -> Result<Vec<EventKind>, OwnerError> {
    let mut events: Vec<EventKind> = team
        .rounds()
        .iter()
        .map(|round| EventKind::RoundOpened {
            team: team.id().clone(),
            round: round.id().clone(),
            item: round.item(),
        })
        .collect();
    for progress in team
        .items()
        .iter()
        .filter(|progress| progress.status() != ItemStatus::Pending && progress.status() != ItemStatus::Done)
    {
        let round = team
            .round_of(progress.id())
            .ok_or_else(|| OwnerError::LostRound {
                team: team.id().clone(),
                item: progress.id(),
            })?
            .id()
            .clone();
        events.push(EventKind::ItemStatus {
            team: team.id().clone(),
            item: progress.id(),
            round,
            status: progress.status(),
        });
    }
    Ok(events)
}

/// The change of the request `request` to `status`, for the bus, with the task of the worker that serves it.
fn request_update(request: RequestId, status: DelegationStatus, task: Option<TaskId>) -> BusJob {
    BusJob::Answer {
        request,
        update: DelegationUpdate {
            status: Some(status),
            task,
            summary: None,
        },
    }
}

/// What a finished request is for the bus: the request closed with its result, and the requester told; `texts` word
/// a failure.
fn answer_jobs(
    texts: &TeamPrompts,
    served: &Served,
    outcome: RequestOutcome,
) -> Result<Vec<BusJob>, NoticeRenderError> {
    let (status, summary, notice) = match outcome {
        RequestOutcome::Done { summary } => (
            DelegationStatus::Done,
            summary.clone(),
            BusNotice::HelperResult {
                request: served.request,
                worker: served.worker.clone(),
                summary,
            },
        ),
        RequestOutcome::Failed { how, said } => {
            let reason = failure_reason(texts, how, said.as_deref())?;
            (
                DelegationStatus::Failed,
                reason.clone(),
                BusNotice::HelperFailed {
                    request: served.request,
                    worker: served.worker.clone(),
                    reason,
                },
            )
        }
    };
    Ok(vec![
        BusJob::Answer {
            request: served.request,
            update: DelegationUpdate {
                status: Some(status),
                task: None,
                summary: Some(summary),
            },
        },
        BusJob::Tell {
            to: served.requester.clone(),
            notice,
        },
    ])
}

/// An unfulfilled workspace release obligation after dispatch failure.
#[derive(Debug, Clone)]
pub(in crate::daemon::owner) struct CleanupObligation {
    pub(in crate::daemon::owner) requester_git: Git,
    pub(in crate::daemon::owner) workspace: RequestWorkspace,
}

#[cfg(test)]
mod tests;
