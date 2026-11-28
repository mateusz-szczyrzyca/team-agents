//! Lead task shifts on minute ticks: rotates coordinator and solo tasks to the next free agent.

use agent_core::decide::ladder::Trouble;
use agent_core::decide::shift::{ShiftCheck, ShiftLimits, shift_target};
use agent_core::history::CheckpointCause;
use agent_core::ids::{AgentName, ItemId, TaskId, TeamId};
use agent_core::kinds::Role;
use agent_core::log::LogEvent;
use agent_core::protocol::EventKind;
use agent_core::state::{Choice, ItemPhase, Timestamp};
use agent_core::status::TaskStatus;
use agent_runtime::agentbus::{Outgoing, Recipient};
use agent_runtime::history::log_decision;
use agent_runtime::process::error_chain;

use super::{Halt, Owner, OwnerError};

impl Owner {
    /// On its minute tick, checks live lead tasks and rotates their holders when shifts are due.
    ///
    /// # Errors
    /// [`OwnerError`] when saving state fails or tasks are lost.
    pub(super) async fn check_lead_shifts(&mut self) -> Result<(), OwnerError> {
        let shift_minutes = self.parts.settings.config.limits().lead_shift_minutes;
        if shift_minutes == 0 {
            return Ok(());
        }
        let now = Timestamp::now(&self.parts.clock)?;
        let replacement_agents = self.parts.settings.config.replacement().agents().to_vec();

        let candidates: Vec<(TeamId, TaskId, Role, AgentName)> = self
            .parts
            .state
            .teams()
            .iter()
            .filter(|team| !self.closed.contains(team.id()))
            .flat_map(|team| {
                team.crew()
                    .roster()
                    .into_iter()
                    .filter(|(role, lead)| {
                        matches!(role, Role::Solo | Role::Coordinator)
                            && !matches!(lead.status(), TaskStatus::Done | TaskStatus::Paused)
                    })
                    .map(|(role, lead)| (team.id().clone(), lead.task().clone(), role, lead.agent().agent_name()))
            })
            .collect();

        for (team_id, task, role, holder) in candidates {
            let Some(team) = self.parts.state.team(&team_id) else {
                continue;
            };
            let limits = ShiftLimits {
                lead_shift_minutes: shift_minutes,
                now,
                replacement_agents: &replacement_agents,
            };
            let check = shift_target(&task, team, &self.parts.state, &limits);
            match check {
                ShiftCheck::NotDue => {
                    self.shift_no_free_logged.remove(&task);
                    self.gate_wait_since.remove(&task);
                }
                ShiftCheck::NoneFree => {
                    self.gate_wait_since.remove(&task);
                    if self.shift_no_free_logged.insert(task.clone()) {
                        let worked_secs = team
                            .find(&task)
                            .map_or(0, |(_, lead)| agent_core::decide::shift::worked_secs(lead, now));
                        let worked_minutes = worked_secs / 60;
                        tracing::info!(
                            event = LogEvent::ShiftNoFreeAgent.as_str(),
                            task = %task,
                            holder = holder.as_str(),
                            worked_minutes = worked_minutes,
                            "a lead task reached its shift boundary but no eligible replacement agent was free"
                        );
                    }
                }
                ShiftCheck::To(candidate) => {
                    let is_gating = self.gate_answer_tasks.values().any(|t| t == &task);
                    if is_gating {
                        let wait_start = *self.gate_wait_since.entry(task.clone()).or_insert(now);
                        let waiting_duration = now.as_utc() - wait_start.as_utc();
                        if waiting_duration < time::Duration::minutes(15) {
                            continue;
                        }
                    }
                    self.shift_no_free_logged.remove(&task);
                    self.gate_wait_since.remove(&task);

                    if role == Role::Solo {
                        self.execute_solo_shift(&team_id, &task, &candidate, now).await?;
                    } else {
                        self.execute_team_shift(&team_id, &task, &candidate, now).await?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Hands over a solo task to the next replacement agent with reason `shift`.
    async fn execute_solo_shift(
        &mut self,
        team_id: &TeamId,
        task: &TaskId,
        candidate: &AgentName,
        now: Timestamp,
    ) -> Result<(), OwnerError> {
        let was_running = self.in_flight.contains(task)
            || self
                .parts
                .state
                .find_task(task)
                .is_some_and(|f| f.member.status() == TaskStatus::Running || f.member.current_run().is_some());
        if was_running {
            self.halt(task, Halt::Stop);
            self.settle_halted(std::slice::from_ref(task)).await?;
        }

        let (held, status) = {
            let found = self
                .parts
                .state
                .find_task(task)
                .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
            (found.member.agent().clone(), found.member.status())
        };

        if status == TaskStatus::Done || self.closed.contains(team_id) {
            return Ok(());
        }

        self.hand_task_over(task, candidate, &held, Choice::Human, Trouble::Shift, now)?;

        let mut events: Vec<EventKind> = Vec::new();
        if self
            .parts
            .state
            .find_task(task)
            .is_some_and(|f| f.member.status() != TaskStatus::Queued)
        {
            self.move_to(task, team_id, TaskStatus::Queued, &mut events)?;
        }

        log_decision(
            "shift",
            &format!("task={task}, from={}", held.name()),
            &format!("agent={candidate}"),
        );
        super::takeover::handed_over(team_id, task, held.name(), candidate.as_str(), Trouble::Shift);

        self.save().await?;
        for event in events {
            self.publish(now, event);
        }
        self.publish_summary(now, team_id);
        self.look_at(team_id);
        Ok(())
    }

    /// Trades the coordinator task and a member task in one change, notifying the team on the bus.
    async fn execute_team_shift(
        &mut self,
        team_id: &TeamId,
        coord_task: &TaskId,
        candidate: &AgentName,
        now: Timestamp,
    ) -> Result<(), OwnerError> {
        let (member_task, agent_b) = {
            let team = self
                .parts
                .state
                .team(team_id)
                .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
            let (_, member) = team
                .crew()
                .roster()
                .into_iter()
                .find(|(_, m)| m.agent().agent_name() == *candidate)
                .ok_or_else(|| OwnerError::LostTask { task: coord_task.clone() })?;
            (member.task().clone(), member.agent().clone())
        };

        let tasks_to_halt: Vec<TaskId> = [coord_task.clone(), member_task.clone()]
            .into_iter()
            .filter(|t| {
                self.in_flight.contains(t)
                    || self
                        .parts
                        .state
                        .find_task(t)
                        .is_some_and(|f| f.member.status() == TaskStatus::Running || f.member.current_run().is_some())
            })
            .collect();
        for t in &tasks_to_halt {
            self.halt(t, Halt::Stop);
        }
        if !tasks_to_halt.is_empty() {
            self.settle_halted(&tasks_to_halt).await?;
        }

        let (agent_a, status_coord, status_member) = {
            let team = self
                .parts
                .state
                .team(team_id)
                .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
            let (_, coord) = team
                .crew()
                .coordinator()
                .map(|c| (Role::Coordinator, c))
                .ok_or_else(|| OwnerError::LostTask { task: coord_task.clone() })?;
            let (_, member) = team
                .crew()
                .roster()
                .into_iter()
                .find(|(_, m)| m.task() == &member_task)
                .ok_or_else(|| OwnerError::LostTask { task: member_task.clone() })?;
            (coord.agent().clone(), coord.status(), member.status())
        };

        if status_coord == TaskStatus::Done || status_member == TaskStatus::Done || self.closed.contains(team_id) {
            return Ok(());
        }

        let items_a: Vec<(ItemId, agent_core::ids::BranchName)> = self
            .parts
            .state
            .team(team_id)
            .map(|t| {
                t.items()
                    .iter()
                    .filter_map(|p| match p.phase() {
                        ItemPhase::InProgress { assignee, branch } | ItemPhase::Fixing { assignee, branch, .. }
                            if assignee.as_str() == agent_a.name() => Some((p.id(), branch.clone())),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let items_b: Vec<(ItemId, agent_core::ids::BranchName)> = self
            .parts
            .state
            .team(team_id)
            .map(|t| {
                t.items()
                    .iter()
                    .filter_map(|p| match p.phase() {
                        ItemPhase::InProgress { assignee, branch } | ItemPhase::Fixing { assignee, branch, .. }
                            if assignee.as_str() == agent_b.name() => Some((p.id(), branch.clone())),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();

        self.parts
            .state
            .trade_team_tasks(team_id, coord_task, &member_task, now)
            .map_err(|source| OwnerError::HandOver {
                task: coord_task.clone(),
                source,
            })?;

        if let Some(mut team) = self.parts.state.team_mut(team_id) {
            for (id, branch) in items_a {
                if let Some(mut item) = team.item_mut(id) {
                    let _ = item.reassign(agent_b.agent_name(), branch);
                }
            }
            for (id, branch) in items_b {
                if let Some(mut item) = team.item_mut(id) {
                    let _ = item.reassign(agent_a.agent_name(), branch);
                }
            }
        }

        self.parts.state.begin_holder_session(coord_task, Trouble::Shift, now)?;
        self.parts.state.begin_holder_session(&member_task, Trouble::Shift, now)?;

        self.queue_for_task(coord_task, CheckpointCause::HolderChanged);
        self.queue_for_task(&member_task, CheckpointCause::HolderChanged);

        let mut events: Vec<EventKind> = Vec::new();
        if self
            .parts
            .state
            .find_task(coord_task)
            .is_some_and(|f| f.member.status() != TaskStatus::Queued)
        {
            self.move_to(coord_task, team_id, TaskStatus::Queued, &mut events)?;
        }
        if self
            .parts
            .state
            .find_task(&member_task)
            .is_some_and(|f| f.member.status() != TaskStatus::Queued)
        {
            self.move_to(&member_task, team_id, TaskStatus::Queued, &mut events)?;
        }

        log_decision(
            "shift",
            &format!("task={coord_task}, from={}", agent_a.name()),
            &format!("agent={}", agent_b.name()),
        );
        log_decision(
            "shift",
            &format!("task={member_task}, from={}", agent_b.name()),
            &format!("agent={}", agent_a.name()),
        );
        super::takeover::handed_over(team_id, coord_task, agent_a.name(), agent_b.name(), Trouble::Shift);
        super::takeover::handed_over(team_id, &member_task, agent_b.name(), agent_a.name(), Trouble::Shift);

        if let Err(error) = self.parts.bus.claim_lead(team_id, &agent_b.agent_name()).await {
            tracing::warn!(
                event = LogEvent::BusDown.as_str(),
                error = %error_chain(&error),
                "cannot set lead on bus"
            );
        }
        let from_sender: AgentName = super::teams::BUS_SENDER
            .parse()
            .unwrap_or_else(|_| "agentbus".parse().expect("valid sender"));
        let to = Recipient::Team(team_id.clone());
        let announcement = format!(
            "Coordination shifted: {} holds coordinator task {}, {} holds member task {}.",
            agent_b.name(),
            coord_task,
            agent_a.name(),
            member_task
        );
        if let Err(error) = self
            .parts
            .bus
            .send(&from_sender, &to, &Outgoing::Chat { text: announcement })
            .await
        {
            tracing::warn!(
                event = LogEvent::BusDown.as_str(),
                error = %error_chain(&error),
                "cannot send shift announcement on bus"
            );
        }

        self.save().await?;
        for event in events {
            self.publish(now, event);
        }
        self.publish_summary(now, team_id);
        self.look_at(team_id);
        Ok(())
    }
}
