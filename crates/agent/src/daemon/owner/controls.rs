//! The owner's side of the controls: `stop`, `pause`, `unpause`, `resume` and `say` for a task or a team.
//!
//! Each task of the target is looked at by itself (`control::decide`): a task with no run going changes its status at
//! once, a task with a run going has the run stopped, and the end of the run sets the status — the owner waits for that
//! before it answers, so a client that has its answer sees the status it asked for. What was reached is recorded where a
//! person looks: the event, the activity log of each task, with the client that did it.

use agent_core::cli::ActivityEvent;
use agent_core::ids::{AgentName, TaskId, TeamId};
use agent_core::protocol::{Client, ControlAction, EventKind, Failure, FailureKind, Reply, SayRequest, Target};
use agent_core::state::{RunEnd, Timestamp};
use agent_core::status::TaskStatus;
use agent_runtime::agentbus::{Outgoing, Recipient};
use agent_runtime::process::error_chain;

use super::{Owner, OwnerError};
use crate::daemon::control::{Halt, Skipped, Steer, Step, decide};

/// The name the human's messages carry on the bus.
const HUMAN: &str = "human";

/// A task of a target, with its team.
struct Aimed {
    /// The task.
    task: TaskId,
    /// Its team.
    team: TeamId,
}

impl Owner {
    // ---- stop, pause, unpause, resume -----------------------------------------------------------------------

    /// Carries out `steer` on `target`; the answer names the tasks it reached, or is the refusal when it reached none.
    ///
    /// # Errors
    /// [`OwnerError`] when the state cannot be saved.
    pub(super) async fn control_request(
        &mut self,
        client: &Client,
        steer: Steer,
        target: &Target,
    ) -> Result<Result<Reply, Failure>, OwnerError> {
        let aimed = match self.aim(target, true) {
            Ok(aimed) => aimed,
            Err(failure) => return Ok(Err(failure)),
        };
        let now = Timestamp::now(&self.parts.clock)?;
        let mut reached = Vec::new();
        let mut halted = Vec::new();
        let mut skipped = Vec::new();
        let mut events = Vec::new();
        for Aimed { task, team } in &aimed {
            let status = self.status_of(task)?;
            match decide(steer, status) {
                Step::Move(next) => {
                    self.move_task(task, team, steer, next, &mut events)?;
                    reached.push(task.clone());
                }
                Step::Halt(halt) => {
                    self.halt(task, halt);
                    halted.push(task.clone());
                    reached.push(task.clone());
                }
                Step::Skip(why) => skipped.push((task.clone(), status, why)),
            }
        }
        if reached.is_empty() {
            return Ok(Err(nothing_done(steer, target, &skipped)));
        }
        self.save().await?;
        for event in events {
            self.publish(now, event);
        }
        for Aimed { team, .. } in &aimed {
            self.publish_summary(now, team);
        }
        self.settle_halted(&halted).await?;
        for Aimed { task, team } in &aimed {
            if reached.contains(task) {
                self.after_move(task, team, steer)?;
            }
        }
        self.record_control(client, steer.into(), target, &aimed, &reached)
            .await?;
        Ok(Ok(Reply::Applied { tasks: reached }))
    }

    /// The tasks of `target`: a task, or the tasks of a team — its crew and, when `reviewers` is set, its reviewers.
    fn aim(&self, target: &Target, reviewers: bool) -> Result<Vec<Aimed>, Failure> {
        match target {
            Target::Task(task) => {
                let found = self.parts.state.find_task(task).ok_or_else(|| Failure {
                    kind: FailureKind::NotFound,
                    message: format!("there is no task {task}"),
                })?;
                Ok(vec![Aimed {
                    task: task.clone(),
                    team: found.team.id().clone(),
                }])
            }
            Target::Team(team) => {
                let found = self.parts.state.team(team).ok_or_else(|| Failure {
                    kind: FailureKind::NotFound,
                    message: format!("there is no team {team}"),
                })?;
                let crew = found.crew().roster().into_iter().map(|(_, member)| member);
                let tasks: Vec<TaskId> = if reviewers {
                    crew.chain(found.reviewers().iter())
                        .map(|member| member.task().clone())
                        .collect()
                } else {
                    crew.map(|member| member.task().clone()).collect()
                };
                Ok(tasks
                    .into_iter()
                    .map(|task| Aimed {
                        task,
                        team: team.clone(),
                    })
                    .collect())
            }
        }
    }

    /// The status of `task`.
    fn status_of(&self, task: &TaskId) -> Result<TaskStatus, OwnerError> {
        self.parts
            .state
            .find_task(task)
            .map(|found| found.member.status())
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })
    }

    /// Changes the status of `task`, which has no run going, to `next`.
    fn move_task(
        &mut self,
        task: &TaskId,
        team: &TeamId,
        steer: Steer,
        next: TaskStatus,
        events: &mut Vec<EventKind>,
    ) -> Result<(), OwnerError> {
        self.move_to(task, team, next, events)?;
        // What waited for a time or for a hang count that no longer applies: the human decided.
        self.wakes.remove(task);
        self.hangs.remove(task);
        // A team that was finished and is resumed has work again, and is looked at again.
        if steer == Steer::Resume {
            self.closed.remove(team);
        }
        Ok(())
    }

    /// What follows a task that changed its status without a run: its team is looked at, and a request a worker was to serve
    /// is answered when the worker is stopped.
    fn after_move(&mut self, task: &TaskId, team: &TeamId, steer: Steer) -> Result<(), OwnerError> {
        if steer == Steer::Stop {
            let located = self.locate(task)?;
            self.after_run(task, &located, &RunEnd::Stopped, None)?;
        }
        self.look_at(team);
        Ok(())
    }

    /// Has the run of `task` stopped; the end of the run sets the status of the task.
    fn halt(&mut self, task: &TaskId, halt: Halt) {
        self.halts.insert(task.clone(), halt);
        if let Some(cancel) = self.cancels.get(task) {
            cancel.cancel();
        }
    }

    /// Waits until the runs of `halted` have ended and their tasks are recorded.
    ///
    /// The runs of other tasks that end meanwhile are recorded as they come.
    async fn settle_halted(&mut self, halted: &[TaskId]) -> Result<(), OwnerError> {
        while halted.iter().any(|task| self.in_flight.contains(task)) {
            let Some(joined) = self.runs.join_next_with_id().await else {
                break;
            };
            self.settle_joined(joined).await?;
        }
        Ok(())
    }

    /// Records a control of the human: its event, once for the target, and a line in the activity log of each task it
    /// reached.
    async fn record_control(
        &mut self,
        client: &Client,
        action: ControlAction,
        target: &Target,
        aimed: &[Aimed],
        reached: &[TaskId],
    ) -> Result<(), OwnerError> {
        let now = Timestamp::now(&self.parts.clock)?;
        let Some(first) = aimed.first() else {
            return Ok(());
        };
        let task = match target {
            Target::Task(task) => Some(task.clone()),
            Target::Team(_) => None,
        };
        self.publish(
            now,
            EventKind::Controlled {
                team: first.team.clone(),
                task,
                action,
                by: client.clone(),
            },
        );
        for task in reached {
            let event = ActivityEvent::Control {
                action,
                by: client.clone(),
            };
            self.log_activity(task, now, event).await;
        }
        tracing::info!(action = ?action, tasks = reached.len(), "the human used a control");
        Ok(())
    }

    // ---- say ------------------------------------------------------------------------------------------------

    /// Puts what the human says on the bus, for the agent of the task or for the team.
    ///
    /// # Errors
    /// [`OwnerError`] when the state cannot be saved or a name is not an agent name.
    pub(super) async fn say_request(
        &mut self,
        client: &Client,
        request: SayRequest,
    ) -> Result<Result<Reply, Failure>, OwnerError> {
        let SayRequest { target, text } = request;
        let aimed = match self.aim(&target, false) {
            Ok(aimed) => aimed,
            Err(failure) => return Ok(Err(failure)),
        };
        let mut hearing = Vec::new();
        for Aimed { task, .. } in &aimed {
            if self.status_of(task)? != TaskStatus::Done {
                hearing.push(task.clone());
            }
        }
        if hearing.is_empty() {
            return Ok(Err(Failure {
                kind: FailureKind::Conflict,
                message: format!("nobody is left to hear it: {} is done", describe(&target)),
            }));
        }
        let recipients = self.recipients(&target, &hearing)?;
        let from: AgentName = HUMAN.parse()?;
        let message = Outgoing::Chat {
            text: text.as_str().to_owned(),
        };
        for recipient in &recipients {
            if let Err(error) = self.parts.bus.send(&from, recipient, &message).await {
                return Ok(Err(Failure {
                    kind: FailureKind::Internal,
                    message: format!("cannot put the message on the agentbus: {}", error_chain(&error)),
                }));
            }
        }
        self.record_control(client, ControlAction::Say, &target, &aimed, &hearing)
            .await?;
        for Aimed { team, .. } in &aimed {
            // A member asleep is woken by the message, which the look at its team finds.
            self.look_at(team);
        }
        Ok(Ok(Reply::Applied { tasks: hearing }))
    }

    /// Who the bus is told to deliver to: the agent of a task; for a team, the team when it has one on the bus, else each
    /// of its agents.
    fn recipients(&self, target: &Target, hearing: &[TaskId]) -> Result<Vec<Recipient>, OwnerError> {
        match target {
            Target::Task(task) => Ok(vec![Recipient::Agent(self.agent_of(task)?)]),
            Target::Team(team) => {
                let on_bus = self
                    .parts
                    .state
                    .team(team)
                    .is_some_and(|found| found.crew().coordinator().is_some());
                if on_bus {
                    return Ok(vec![Recipient::Team(team.clone())]);
                }
                hearing
                    .iter()
                    .map(|task| self.agent_of(task).map(Recipient::Agent))
                    .collect()
            }
        }
    }

    /// The agent of `task`.
    fn agent_of(&self, task: &TaskId) -> Result<AgentName, OwnerError> {
        let found = self
            .parts
            .state
            .find_task(task)
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
        Ok(found.member.agent().name().parse()?)
    }
}

/// What a target is called in a message: the task or the team.
fn describe(target: &Target) -> String {
    match target {
        Target::Task(task) => format!("task {task}"),
        Target::Team(team) => format!("every task of team {team}"),
    }
}

/// The refusal of a control that did nothing to any task of `target`: why, for each task.
fn nothing_done(steer: Steer, target: &Target, skipped: &[(TaskId, TaskStatus, Skipped)]) -> Failure {
    let message = match (target, skipped) {
        (Target::Task(task), [(_, status, why)]) => format!("task {task} is {status}: {}", why.because(steer)),
        (Target::Task(_) | Target::Team(_), _) => {
            let lines: Vec<String> = skipped
                .iter()
                .map(|(task, status, why)| format!("- task {task} is {status}: {}", why.because(steer)))
                .collect();
            format!(
                "{} does nothing to {}:\n{}",
                action_word(steer),
                describe(target),
                lines.join("\n")
            )
        }
    };
    Failure {
        kind: FailureKind::Conflict,
        message,
    }
}

/// The command of `steer`, as the human types it.
const fn action_word(steer: Steer) -> &'static str {
    match steer {
        Steer::Stop => "stop",
        Steer::Pause => "pause",
        Steer::Unpause => "unpause",
        Steer::Resume => "resume",
    }
}

#[cfg(test)]
mod tests;
