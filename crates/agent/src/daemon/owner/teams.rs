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

use std::str::FromStr;

use agent_core::config::{AbsolutePath, Config};
use agent_core::decide::assign::first_free_worker;
use agent_core::ids::{AgentName, BranchName, TaskId, TeamId};
use agent_core::kinds::Role;
use agent_core::protocol::{EventKind, Failure, FailureKind, Reply, RunRequest, TeamSummary};
use agent_core::state::{RunEnd, Timestamp};
use agent_core::status::{ItemStatus, TaskStatus};
use agent_runtime::agentbus::{
    AgentBusError, DelegationStatus, DelegationUpdate, FailureClass, MemberState, Recipient, RequestId, Woken,
};
use agent_runtime::git::{CommitIdentity, DeleteMode, Git, GitError, RemoveMode, Revision, WorktreeBranch};
use agent_runtime::process::error_chain;
use agent_runtime::team_prompts::TeamPrompts;

use super::{Located, Notice, Owner, OwnerError};
use crate::daemon::team::{
    Assigned, BusJob, BusNotice, FormError, Formed, NoticeRenderError, RequestOutcome, Why, Workspace, assignable,
    closing_transitions, failure_reason, form, is_finished, item_in_flight, notice_for, reconcile, render_notice,
    request_outcome, start_item, team_done_notification,
};

/// The name the harness puts on the messages it writes to the bus, as the bus itself does.
const BUS_SENDER: &str = "agentbus";

/// Something that is to be done on the bus, with the team it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Outbound {
    /// The team.
    team: TeamId,
    /// What is to be done.
    job: BusJob,
}

/// A request for a worker that is being served.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Served {
    /// The request.
    request: RequestId,
    /// The member that asked and gets the result.
    requester: AgentName,
    /// The worker that serves it.
    worker: AgentName,
}

/// The workers of a team and which of them cannot take a request now.
struct Workers {
    /// The workers with their tasks, in the configured order.
    in_order: Vec<(AgentName, TaskId)>,
    /// The workers that are at work or cannot work.
    busy: Vec<AgentName>,
}

/// What one look at a team changed in the state.
#[derive(Debug, Default)]
pub(super) struct Round {
    /// What happened, for the subscribers.
    pub(super) events: Vec<EventKind>,
    /// Whether the state changed and is to be saved.
    pub(super) changed: bool,
}

impl Owner {
    // ---- starting a team ------------------------------------------------------------------------------------

    /// Forms and starts the team `request` asks for; the answer is the refusal when it cannot be started.
    ///
    /// # Errors
    /// [`OwnerError`] when the state cannot be saved.
    pub(super) async fn run_request(&mut self, request: RunRequest) -> Result<Result<Reply, Failure>, OwnerError> {
        let formed = match form(
            &request,
            &self.parts.settings.config,
            self.parts.store.home(),
            &self.parts.clock,
            &mut self.parts.rng,
        ) {
            Ok(formed) => formed,
            Err(refusal) => return Ok(Err(refusal_of(&refusal))),
        };
        // The team goes into a copy of the state first: a team the state refuses leaves nothing behind, and the state the
        // owner holds changes only once the copy is saved.
        let mut next = self.parts.state.clone();
        if let Err(error) = next.insert_team(formed.team.clone()) {
            tracing::error!(team = %formed.team.id(), error = %error_chain(&error), "the state refuses the new team");
            return Ok(Err(internal(&format!(
                "the new team does not fit the state: {}",
                error_chain(&error)
            ))));
        }
        if let Err(failure) = self.make_workspaces(&request, &formed).await {
            return Ok(Err(failure));
        }
        if let Some(bus_team) = &formed.bus
            && let Err(error) = self.parts.bus.team_create(bus_team).await
        {
            tracing::error!(team = %formed.team.id(), error = %error_chain(&error), "the team cannot be created on the bus");
            self.remove_workspaces(&request, &formed.workspaces).await;
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
        let started = started_items(team);
        if let Err(error) = self.parts.store.save(&next).await {
            tracing::error!(team = %id, error = %error_chain(&error), "the state with the new team cannot be saved: the team is not started");
            self.undo_start(&request, &formed).await;
            return Ok(Err(internal(&format!(
                "cannot save the state with the new team: {}",
                error_chain(&error)
            ))));
        }
        self.parts.state = next;
        let now = Timestamp::now(&self.parts.clock)?;
        self.publish(now, EventKind::TeamStarted { team: id.clone() });
        for event in started {
            self.publish(now, event);
        }
        self.publish_summary(now, &id);
        tracing::info!(team = %id, tasks = tasks.len(), "a team is started");
        self.look_at(&id);
        Ok(Ok(Reply::Started { team: id, tasks }))
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
            if let Err(error) = git.worktree_add(&workspace.path, &branch).await {
                self.remove_workspaces(request, &made).await;
                return Err(git_failure(
                    &format!("cannot make the worktree of task {}", workspace.task),
                    &error,
                ));
            }
            made.push(workspace.clone());
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
            tracing::warn!(team = %formed.team.id(), error = %error_chain(&error), "the bus team of a refused team cannot be closed");
        }
    }

    /// Removes worktrees and their branches that a request made and cannot use.
    async fn remove_workspaces(&self, request: &RunRequest, made: &[Workspace]) {
        let Some(lead) = made.first() else {
            return;
        };
        let git = checkout_git(&self.parts.settings.config, request, &lead.agent);
        for workspace in made {
            // A refusal that is being returned already: what cannot be removed is told, and left for the human.
            if let Err(error) = git.worktree_remove(&workspace.path, RemoveMode::Force).await {
                tracing::warn!(task = %workspace.task, error = %error_chain(&error), "a worktree of a refused team cannot be removed");
            }
            if let Err(error) = git.branch_delete(&workspace.branch, DeleteMode::Force).await {
                tracing::warn!(task = %workspace.task, error = %error_chain(&error), "a branch of a refused team cannot be removed");
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
        self.flush_outbox().await?;
        let looks = std::mem::take(&mut self.looks);
        for team in looks {
            self.look_after(&team).await?;
        }
        Ok(())
    }

    /// One look at `team`.
    async fn look_after(&mut self, team: &TeamId) -> Result<(), OwnerError> {
        if self.closed.contains(team) || self.parts.state.team(team).is_none() {
            return Ok(());
        }
        let mut round = Round::default();
        self.reconcile_bus(team, &mut round).await?;
        self.pull_requests(team, &mut round).await?;
        self.assign_items(team, &mut round).await?;
        self.serve_requests(team, &mut round).await?;
        self.close_if_finished(team, &mut round)?;
        if round.changed {
            self.save().await?;
            let now = Timestamp::now(&self.parts.clock)?;
            for event in round.events {
                self.publish(now, event);
            }
            self.publish_summary(now, team);
        }
        Ok(())
    }

    /// Tells the subscribers the team's summary as it is.
    pub(super) fn publish_summary(&self, at: Timestamp, team: &TeamId) {
        if let Some(found) = self.parts.state.team(team) {
            self.publish(
                at,
                EventKind::TeamUpdated {
                    summary: Box::new(TeamSummary::from(found)),
                },
            );
        }
    }

    // ---- the bus --------------------------------------------------------------------------------------------

    /// Does what is waiting for the bus, oldest first; stops at the first thing the bus does not take.
    async fn flush_outbox(&mut self) -> Result<(), OwnerError> {
        while let Some(next) = self.outbox.front().cloned() {
            match self.carry_out(&next).await? {
                Ok(()) => {
                    self.outbox.pop_front();
                }
                // What the bus refuses for good would block everything behind it for ever: it is dropped, once told.
                Err(error) if !is_transient(&error) => {
                    tracing::error!(team = %next.team, error = %error_chain(&error), job = ?next.job, "the bus refuses a request for good: it is dropped");
                    self.outbox.pop_front();
                }
                Err(error) => {
                    tracing::warn!(team = %next.team, error = %error_chain(&error), "the bus does not answer: what is waiting for it is tried again at the next look");
                    break;
                }
            }
        }
        Ok(())
    }

    /// Does one thing on the bus. The outer error is the daemon's own, the inner one the bus's.
    async fn carry_out(&self, outbound: &Outbound) -> Result<Result<(), AgentBusError>, OwnerError> {
        let bus = &self.parts.bus;
        Ok(match &outbound.job {
            BusJob::Tell { to, notice } => {
                let message = render_notice(&self.parts.team_texts, notice)?;
                let from: AgentName = BUS_SENDER.parse()?;
                bus.send(&from, &Recipient::Agent(to.clone()), &message).await.map(drop)
            }
            BusJob::Answer { request, update } => bus.helper_set(*request, update).await,
            BusJob::Finish => bus.team_done(&outbound.team).await,
        })
    }

    /// Makes the members' states on the bus those their tasks have, and starts the tasks of the members the bus woke.
    async fn reconcile_bus(&mut self, team_id: &TeamId, round: &mut Round) -> Result<(), OwnerError> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Ok(());
        };
        if team.crew().coordinator().is_none() {
            return Ok(());
        }
        let on_bus = match self.parts.bus.team_get(team_id).await {
            Ok(on_bus) => on_bus,
            Err(error) => {
                tracing::warn!(team = %team_id, error = %error_chain(&error), "the team cannot be read from the bus: it is tried again at the next look");
                return Ok(());
            }
        };
        let plan = reconcile(team, &on_bus)?;
        for task in plan.wake {
            self.release(&task, Why::Woken, round)?;
        }
        for (member, state) in plan.set {
            match self.parts.bus.team_set(team_id, &member, state).await {
                // A member that went to sleep while something waits for it is woken at once.
                Ok(Woken::Message | Woken::Board) if state == MemberState::Sleeping => {
                    if let Some(task) = self.task_of(team_id, &member) {
                        self.release(&task, Why::Woken, round)?;
                    }
                }
                Ok(Woken::Message | Woken::Board | Woken::No) => {}
                Err(error) => {
                    tracing::warn!(team = %team_id, %member, error = %error_chain(&error), "the state of a member cannot be set on the bus: it is tried again at the next look");
                    break;
                }
            }
        }
        Ok(())
    }

    /// The task of the member named `agent` in `team`.
    fn task_of(&self, team: &TeamId, agent: &AgentName) -> Option<TaskId> {
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
    pub(super) fn release(&mut self, task: &TaskId, why: Why, round: &mut Round) -> Result<(), OwnerError> {
        let located = self.locate(task)?;
        let status = self
            .parts
            .state
            .find_task(task)
            .map(|found| found.member.status())
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
        match status {
            TaskStatus::Waiting => {
                self.move_to(task, &located.team, TaskStatus::Queued, &mut round.events)?;
                round.changed = true;
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

    /// Gives the pending items to the members that are free.
    async fn assign_items(&mut self, team_id: &TeamId, round: &mut Round) -> Result<(), OwnerError> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Ok(());
        };
        let given = assignable(team).map_err(|source| OwnerError::Assign {
            team: team_id.clone(),
            source,
        })?;
        let repo = team.repo().clone();
        for assigned in given {
            let Some(base) = self
                .parts
                .settings
                .config
                .repo(&repo)
                .map(|repo| repo.base_branch.clone())
            else {
                tracing::error!(team = %team_id, %repo, "the repository of the team is not in the configuration: no item is given out");
                return Ok(());
            };
            if let Err(error) = self.switch_branch(&repo, &assigned, base).await {
                tracing::warn!(team = %team_id, item = %assigned.item, member = %assigned.agent, error = %error_chain(&error), "the worktree of a member cannot take the branch of its next item: the item waits and is tried again at the next look");
                continue;
            }
            self.record_assignment(team_id, &assigned)?;
            round.events.push(EventKind::ItemStatus {
                team: team_id.clone(),
                item: assigned.item,
                status: ItemStatus::InProgress,
            });
            round.changed = true;
            tracing::info!(team = %team_id, item = %assigned.item, member = %assigned.agent, "an item is given to a member");
            self.release(&assigned.task, Why::Assigned(assigned.item), round)?;
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
    async fn serve_requests(&mut self, team_id: &TeamId, round: &mut Round) -> Result<(), OwnerError> {
        let Some(Workers {
            in_order: workers,
            mut busy,
        }) = self.workers_of(team_id)?
        else {
            return Ok(());
        };
        let names: Vec<AgentName> = workers.iter().map(|(name, _)| name.clone()).collect();
        let mut held = Vec::new();
        while let Some(worker) = first_free_worker(&names, &busy).cloned() {
            let request = match self.parts.bus.helper_next(&worker).await {
                Ok(Some(request)) => request,
                Ok(None) => break,
                Err(error) => {
                    tracing::warn!(team = %team_id, %worker, error = %error_chain(&error), "the queue of requests for workers cannot be read: it is tried again at the next look");
                    break;
                }
            };
            let Some((_, task)) = workers.iter().find(|(name, _)| *name == worker).cloned() else {
                break;
            };
            if !self.is_member(team_id, &request.from_task) {
                held.push(request.id);
                continue;
            }
            self.release(&task, Why::Delegated(Box::new(request.clone())), round)?;
            self.serving.insert(
                task.clone(),
                Served {
                    request: request.id,
                    requester: request.from.clone(),
                    worker: worker.clone(),
                },
            );
            self.tell_bus(
                team_id,
                request_update(request.id, DelegationStatus::Running, Some(task)),
            );
            tracing::info!(team = %team_id, request = %request.id, %worker, "a request is given to a worker");
            busy.push(worker);
        }
        self.put_back(team_id, held).await;
        Ok(())
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
                tracing::warn!(team = %team_id, %request, error = %error_chain(&error), "a request of another team cannot be put back now: it is tried again at the next turn");
                self.tell_bus(team_id, BusJob::Answer { request, update });
            }
        }
    }

    /// The workers of `team` in the configured order with their tasks, and those that cannot take a request now;
    /// `None` for a team that is not there or has no workers to ask.
    fn workers_of(&self, team_id: &TeamId) -> Result<Option<Workers>, OwnerError> {
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
            if member.status() != TaskStatus::Waiting || self.serving.contains_key(member.task()) {
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
    fn tell_bus(&mut self, team: &TeamId, job: BusJob) {
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
            if let Some(told) = notice_for(found.team, located.role, found.member, end, message)? {
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
                for job in answer_jobs(&self.parts.team_texts, &served, outcome)? {
                    self.outbox.push_back(Outbound {
                        team: located.team.clone(),
                        job,
                    });
                }
            }
        }
        self.look_at(&located.team);
        Ok(())
    }

    // ---- the end of a team ----------------------------------------------------------------------------------

    /// Closes `team` when its work is over: its waiting tasks end, the bus is told, and so is the human, once.
    fn close_if_finished(&mut self, team_id: &TeamId, round: &mut Round) -> Result<(), OwnerError> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Ok(());
        };
        if self.closed.contains(team_id) || !is_finished(team) {
            return Ok(());
        }
        let transitions = closing_transitions(team);
        let has_bus = team.crew().coordinator().is_some();
        let notification = team_done_notification(&self.parts.team_texts, team)?;
        let lead = team
            .crew()
            .roster()
            .first()
            .map(|(_, member)| member.task().clone())
            .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
        for (task, status) in transitions {
            self.move_to(&task, team_id, status, &mut round.events)?;
            round.changed = true;
        }
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
            self.move_to(&task, team_id, TaskStatus::Done, &mut round.events)?;
            round.changed = true;
        }
        if has_bus {
            self.outbox.push_back(Outbound {
                team: team_id.clone(),
                job: BusJob::Finish,
            });
        }
        self.closed.insert(team_id.clone());
        let located = self.locate(&lead)?;
        let now = Timestamp::now(&self.parts.clock)?;
        self.notify(now, &located, &lead, Notice::Prepared(notification));
        tracing::info!(team = %team_id, "a team is finished and closed");
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
        tracing::error!(error = %error_chain(error), "a team cannot be formed");
    }
    failure
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
fn is_transient(error: &AgentBusError) -> bool {
    matches!(
        error.class(),
        None | Some(FailureClass::Storage | FailureClass::Process | FailureClass::Unclassified)
    )
}

/// The events of the items a new team starts with: those that have a member already.
fn started_items(team: &agent_core::state::Team) -> Vec<EventKind> {
    team.items()
        .iter()
        .filter(|progress| progress.status() != ItemStatus::Pending && progress.status() != ItemStatus::Done)
        .map(|progress| EventKind::ItemStatus {
            team: team.id().clone(),
            item: progress.id(),
            status: progress.status(),
        })
        .collect()
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

#[cfg(test)]
mod tests;
