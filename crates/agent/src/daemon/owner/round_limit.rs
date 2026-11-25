//! The limit of a round, `[limits] round_minutes`: the run of a task whose item has gone on longer than the limit without
//! its pull request in review is cut, and the task is handed over through `[replacement]` as a task whose runs went bad
//! is — to another agent, briefed in a new session, or to the lead. With nobody to take it over the task fails and the
//! human is told.
//!
//! What the daemon keeps between the cut and the end of the run is in memory only: a restart of the daemon resumes the cut
//! run as any interrupted run, and the next look cuts it again when the round is still overdue.

use agent_core::decide::commit_reminder::is_reminded;
use agent_core::decide::ladder::Trouble;
use agent_core::decide::round_limit::{Overdue, RoundClock, RunMark, round_overdue};
use agent_core::ids::{ItemId, RoundId, TaskId};
use agent_core::log::LogEvent;
use agent_core::protocol::EventKind;
use agent_core::state::{ItemPhase, Round, RunEnd, RunState, State, Timestamp};
use agent_core::status::TaskStatus;
use agent_runtime::process::error_chain;

use super::takeover::Takeover;
use super::{Located, Owner, OwnerError};
use crate::daemon::notice::Notice;

/// A cut of a run whose round is overdue, kept from the cut until the run has ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OverdueCut {
    /// The round that is overdue.
    pub(super) round: RoundId,
    /// The item the round is for.
    pub(super) item: ItemId,
    /// The whole minutes the round had gone on at the cut, counted from the start of its clock.
    pub(super) minutes: u64,
    /// `[limits] round_minutes` at the cut.
    pub(super) limit: u32,
}

/// The round of `task` that its run works for and its holder works on with no pull request yet, with what its clock reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Timed {
    /// The round.
    pub(super) round: RoundId,
    /// The item the round is for.
    pub(super) item: ItemId,
    /// When the round was opened.
    pub(super) opened_at: Timestamp,
    /// When the holder took the task over; `None` when it held it from the start.
    pub(super) held_since: Option<Timestamp>,
    /// The task's runs, oldest first.
    pub(super) runs: Vec<RunMark>,
}

impl Timed {
    /// The clock of the round, as the decision reads it.
    pub(super) fn clock(&self) -> RoundClock<'_> {
        RoundClock {
            opened_at: self.opened_at,
            held_since: self.held_since,
            runs: &self.runs,
        }
    }
}

/// The round whose time limit applies to the run going of `task`: an open round the run works for whose item is in
/// progress with the task's agent as its assignee. None for a worker and a reviewer, a task with no run going, a round in
/// review, approved, fixed or closed, and an item another agent works on.
pub(super) fn timed_round(state: &State, task: &TaskId) -> Option<Timed> {
    let found = state.find_task(task)?;
    if !is_reminded(found.role) {
        return None;
    }
    let run = found.member.current_run()?;
    let holder = found.member.agent().agent_name();
    let round = run
        .rounds()
        .iter()
        .filter_map(|id| found.team.round(id))
        .filter(|round| Round::is_open(round))
        .find(|round| {
            round.item().and_then(|item| found.team.item(item)).is_some_and(
                |item| matches!(item.phase(), ItemPhase::InProgress { assignee, .. } if *assignee == holder),
            )
        })?;
    let runs = found
        .member
        .runs()
        .iter()
        .map(|run| RunMark {
            started_at: run.started_at(),
            stopped: matches!(
                run.state(),
                RunState::Ended {
                    end: RunEnd::Stopped,
                    ..
                }
            ),
        })
        .collect();
    Some(Timed {
        round: round.id().clone(),
        item: round.item()?,
        opened_at: round.opened_at(),
        held_since: found.member.past_holders().last().map(|holding| holding.until),
        runs,
    })
}

impl Owner {
    /// Cuts the run going of `task` when the round it works on is over `[limits] round_minutes`: as a stop of the human
    /// does but without a halt, so that the end of the run hands the task over (see [`Owner::hand_over_overdue`]). A run
    /// the human halts, or the daemon cut already, is left alone.
    pub(super) fn cut_overdue_round(&mut self, task: &TaskId) {
        let limit = self.parts.settings.config.limits().round_minutes;
        if limit == 0
            || self.overdue_cuts.contains_key(task)
            || self.halts.contains_key(task)
            || self.commit_cuts.contains_key(task)
        {
            return;
        }
        let Some(timed) = timed_round(&self.parts.state, task) else {
            return;
        };
        let now = match Timestamp::now(&self.parts.clock) {
            Ok(now) => now,
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::DaemonTaskFailed.as_str(),
                    task = %task,
                    operation = "round_limit",
                    error = %error_chain(&error),
                    "the clock gives no time: the round was not timed"
                );
                return;
            }
        };
        let Overdue::Due { minutes, since } = round_overdue(&timed.clock(), now, limit) else {
            return;
        };
        agent_runtime::history::log_decision(
            "round_overdue",
            &format!(
                "task={task}, round={}, item={}, since={since}, limit={limit}",
                timed.round, timed.item
            ),
            &format!("cut after {minutes} minutes"),
        );
        self.overdue_cuts.insert(
            task.clone(),
            OverdueCut {
                round: timed.round,
                item: timed.item,
                minutes,
                limit,
            },
        );
        if let Some(cancel) = self.cancels.get(task) {
            cancel.cancel();
        }
    }

    /// Ends the run `cut` was made for and hands its task over under `[replacement]`, as a task whose runs went bad with
    /// [`Trouble::RoundOverdue`]: to another agent, which is briefed in a new session; to the lead, which is given the
    /// work; or to nobody, and the task fails with a notice to the human. The run ended as a stop, which no retry, turn
    /// count or rejection counts; the briefing of the agent or the lead that takes it over shows what it left.
    ///
    /// # Errors
    /// [`OwnerError`] when the run cannot be ended, the task handed over or moved, or the state saved.
    pub(super) async fn hand_over_overdue(
        &mut self,
        task: &TaskId,
        located: &Located,
        now: Timestamp,
        cut: OverdueCut,
        mut events: Vec<EventKind>,
    ) -> Result<(), OwnerError> {
        self.end_run(task, &located.team, now, RunEnd::Stopped, false, None, &mut events)?;
        let takeover = self.take_over(task, located.role, Trouble::RoundOverdue)?;
        let (status, notice) = match takeover {
            Takeover::Agent => (TaskStatus::Queued, None),
            Takeover::Lead => (TaskStatus::Stopped, None),
            Takeover::Nobody => (
                TaskStatus::Failed,
                Some(Notice::RunFailed {
                    reason: format!(
                        "round {} of item {} went on {} minutes without its pull request in review, over \
                         `[limits] round_minutes = {}`, and no agent of `[replacement]` can take the task over.",
                        cut.round, cut.item, cut.minutes, cut.limit
                    ),
                }),
            ),
        };
        self.move_to(task, &located.team, status, &mut events)?;
        self.commit(task, located, now, events, notice).await?;
        // The lead is told of the work that is its own now, and of a task that failed.
        if takeover != Takeover::Agent {
            self.after_run(task, located, &RunEnd::Stopped, None)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
