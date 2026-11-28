//! `Owner::swap_request`: changes the agent holding a task on the human's word.

use agent_core::decide::ladder::Trouble;
use agent_core::ids::TeamId;
use agent_core::kinds::Role;
use agent_core::protocol::{Client, Failure, FailureKind, Reply, SwapRefusal, SwapRequest};
use agent_core::state::{Choice, Timestamp};
use agent_core::status::TaskStatus;
use agent_runtime::history::log_decision;

use super::{Owner, OwnerError};
use crate::daemon::control::Halt;

impl Owner {
    /// Validates a swap request against configuration and current state, returning the task's
    /// [`TeamId`] on success or the refusal [`Failure`].
    fn check_swap_refusal(&self, request: &SwapRequest) -> Result<TeamId, Failure> {
        // 1. Check target agent: unknown or worker.
        if self.tier_of(&request.agent).is_none() {
            return Err(SwapRefusal::UnknownAgent {
                agent: request.agent.clone(),
            }
            .into());
        }
        if self.tier_of(&request.agent) == Some(agent_core::kinds::Tier::Worker) {
            return Err(SwapRefusal::Worker {
                agent: request.agent.clone(),
            }
            .into());
        }

        // 2. Check the task in state.
        let Some(found) = self.parts.state.find_task(&request.task) else {
            return Err(Failure {
                kind: FailureKind::NotFound,
                message: format!("there is no task {}", request.task),
            });
        };

        let team_id = found.team.id().clone();

        // Role check: only solo, coordinator or member.
        match found.role {
            Role::Solo | Role::Coordinator | Role::Member => {}
            Role::Worker | Role::Reviewer | Role::Investigator => {
                return Err(SwapRefusal::Role {
                    task: request.task.clone(),
                    role: found.role,
                }
                .into());
            }
        }

        // Closed team or task done.
        if self.closed.contains(&team_id) || found.member.status() == TaskStatus::Done {
            return Err(SwapRefusal::Done {
                task: request.task.clone(),
            }
            .into());
        }

        // Already held by this agent.
        if found.member.agent().name() == request.agent.as_str() {
            return Err(SwapRefusal::SameHolder {
                task: request.task.clone(),
                agent: request.agent.clone(),
            }
            .into());
        }

        // In same team.
        let in_team = found
            .team
            .crew()
            .roster()
            .into_iter()
            .map(|(_, m)| m)
            .chain(found.team.reviewers())
            .find(|m| m.agent().name() == request.agent.as_str())
            .map(|m| m.task().clone());
        if let Some(task) = in_team {
            return Err(SwapRefusal::InTeam {
                agent: request.agent.clone(),
                team: team_id,
                task,
            }
            .into());
        }

        // Target agent has a run going in another task.
        let in_run = self
            .parts
            .state
            .teams()
            .iter()
            .flat_map(|team| {
                let crew = team.crew().roster().into_iter().map(|(_, m)| m);
                crew.chain(team.reviewers())
            })
            .find(|m| {
                m.agent().name() == request.agent.as_str()
                    && (m.status() == TaskStatus::Running
                        || m.current_run().is_some()
                        || self.in_flight.contains(m.task()))
            })
            .map(|m| m.task().clone());
        if let Some(task) = in_run {
            return Err(SwapRefusal::InRun {
                agent: request.agent.clone(),
                task,
            }
            .into());
        }

        Ok(team_id)
    }

    /// Handles a swap request from a client: checks the agent and the task, ends a run that goes on,
    /// hands the task over with [`Choice::Human`] and [`Trouble::Swap`], records the decision and log event,
    /// and returns the reply.
    ///
    /// # Errors
    /// [`OwnerError`] when the state cannot be saved or the task is lost.
    pub(super) async fn swap_request(
        &mut self,
        _client: &Client,
        request: SwapRequest,
    ) -> Result<Result<Reply, Failure>, OwnerError> {
        let team_id = match self.check_swap_refusal(&request) {
            Ok(team_id) => team_id,
            Err(failure) => return Ok(Err(failure)),
        };

        let found = self
            .parts
            .state
            .find_task(&request.task)
            .ok_or_else(|| OwnerError::LostTask {
                task: request.task.clone(),
            })?;

        // 3. If a run of the task is going, end it as `agent stop` does.
        let was_running = self.in_flight.contains(&request.task)
            || found.member.status() == TaskStatus::Running
            || found.member.current_run().is_some();

        if was_running {
            self.halt(&request.task, Halt::Stop);
            self.settle_halted(std::slice::from_ref(&request.task)).await?;
        }

        let now = Timestamp::now(&self.parts.clock)?;
        let (held, status) = {
            let found = self
                .parts
                .state
                .find_task(&request.task)
                .ok_or_else(|| OwnerError::LostTask {
                    task: request.task.clone(),
                })?;
            (found.member.agent().clone(), found.member.status())
        };

        // A run may have completed naturally before the halt reached it.
        if status == TaskStatus::Done || self.closed.contains(&team_id) {
            return Ok(Err(SwapRefusal::Done { task: request.task }.into()));
        }

        // 4. Hand the task over with Choice::Human and Trouble::Swap.
        self.hand_task_over(&request.task, &request.agent, &held, Choice::Human, Trouble::Swap, now)?;

        // 5. Only a stopped run needs queuing; keep questions and other final results.
        let mut events = Vec::new();
        if was_running && status == TaskStatus::Stopped {
            self.move_to(&request.task, &team_id, TaskStatus::Queued, &mut events)?;
        }

        // 6. Records: history decision_taken and log task_handed_over.
        log_decision(
            "swap",
            &format!("task={}, from={}", request.task, held.name()),
            &format!("agent={}", request.agent),
        );
        super::takeover::handed_over(
            &team_id,
            &request.task,
            held.name(),
            request.agent.as_str(),
            Trouble::Swap,
        );

        self.save().await?;
        for event in events {
            self.publish(now, event);
        }
        self.publish_summary(now, &team_id);
        self.look_at(&team_id);

        Ok(Ok(Reply::Applied {
            tasks: vec![request.task],
        }))
    }
}

#[cfg(test)]
mod tests;
