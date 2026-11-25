//! The wait of a round whose task the human stopped: the task, and when it was stopped.

use agent_core::ids::TaskId;
use agent_core::kinds::Role;
use agent_core::state::{Round, RunState, Team, Timestamp};
use agent_core::status::TaskStatus;

use crate::daemon::owner::Owner;

/// The task of a round that the human stopped and that holds the round: only `agent resume` advances it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::owner) struct StoppedTask {
    /// The stopped task.
    pub(in crate::daemon::owner) task: TaskId,
    /// When the run it was stopped in ended, or `None` when it never ran.
    pub(in crate::daemon::owner) stopped_at: Option<Timestamp>,
}

impl Owner {
    /// The task `round` waits to be resumed, or `None` when it waits for anybody else.
    ///
    /// The round waits for a resume only when everything that holds it is stopped: an open question is an answer the human
    /// owes, a task that failed or that the human paused is the human's as it was, and a reviewer whose review is asked is
    /// looked at for replacement first. A reviewer whose review is no longer asked holds nothing.
    pub(in crate::daemon::owner) fn stopped_task(team: &Team, round: &Round) -> Option<StoppedTask> {
        let members = team.members_in(round.id());
        if members.iter().any(|(_, member)| member.open_question().is_some()) {
            return None;
        }
        let mut stopped = None;
        for (role, member) in members {
            let stuck = matches!(
                member.status(),
                TaskStatus::Questions | TaskStatus::Paused | TaskStatus::Stopped | TaskStatus::Failed
            );
            if !stuck {
                continue;
            }
            if role == Role::Reviewer {
                if crate::daemon::merge::review_asked(team, member.task()) {
                    return None;
                }
                continue;
            }
            if member.status() != TaskStatus::Stopped {
                return None;
            }
            stopped.get_or_insert_with(|| StoppedTask {
                task: member.task().clone(),
                stopped_at: member.runs().last().and_then(|run| match run.state() {
                    RunState::Ended { at, .. } => Some(*at),
                    RunState::Running => None,
                }),
            });
        }
        stopped
    }

    /// The wait line of a round that waits for `stopped` to be resumed; `now` is when it was looked at, `None` when the clock
    /// cannot be read, which shows the moment with its date.
    pub(in crate::daemon::owner) fn resume_line(&self, stopped: &StoppedTask, now: Option<Timestamp>) -> String {
        let at = stopped.stopped_at.map(|at| self.local_moment(at, now));
        agent_core::decide::wait::resume_line(&stopped.task, at.as_deref())
    }
}
