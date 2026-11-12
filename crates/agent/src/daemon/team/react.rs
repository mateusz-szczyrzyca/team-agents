//! What a member's run ending tells the coordinator.

use agent_core::ids::{AgentName, BranchName, ItemId};
use agent_core::kinds::Role;
use agent_core::state::{ItemPhase, Member, RunEnd, Team};
use agent_core::status::RunOutcome;
use agent_runtime::agentbus::RequestId;

use super::assign::{agent_name, task_branch};
use crate::daemon::settle::end_of;

/// A notice for the bus: what happened to a member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon) enum BusNotice {
    /// A member reported `DONE`: its branch is ready to be integrated.
    MemberFinished {
        /// The member.
        member: AgentName,
        /// The item it worked on, if the team has items.
        item: Option<ItemId>,
        /// The branch it worked on: its item's, or its task's.
        branch: BranchName,
    },
    /// A member reported `FAILED`.
    MemberFailed {
        /// The member.
        member: AgentName,
        /// The end of its final message.
        message: String,
    },
    /// A member ended its turn without a status line.
    MemberNoStatus {
        /// The member.
        member: AgentName,
    },
    /// A worker finished a request of a member: the result goes to the member.
    HelperResult {
        /// The request.
        request: RequestId,
        /// The worker.
        worker: AgentName,
        /// What the worker reported.
        summary: String,
    },
    /// A worker could not do a request of a member.
    HelperFailed {
        /// The request.
        request: RequestId,
        /// The worker.
        worker: AgentName,
        /// Why, for the requester.
        reason: String,
    },
}

/// A notice and the member of the team it is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon) struct Told {
    /// Who is told: the coordinator.
    pub(in crate::daemon) to: AgentName,
    /// What it is told.
    pub(in crate::daemon) notice: BusNotice,
}

/// Why a notice cannot be made.
#[derive(Debug, thiserror::Error)]
pub(in crate::daemon) enum NoticeError {
    /// A name of the team is not an agent name.
    #[error("the name of a member of the team is not an agent name")]
    Name(#[from] agent_core::ids::InvalidName),
    /// The branch of the task is not a valid branch name.
    #[error("the branch of a task of the team is not a valid branch name")]
    Branch(#[from] agent_core::ids::InvalidBranchName),
}

/// What the end `end` of a run of `member` (role `role`) tells the coordinator of `team`; `message` is the run's final
/// message.
///
/// # Errors
/// [`NoticeError`].
pub(in crate::daemon) fn notice_for(
    team: &Team,
    role: Role,
    member: &Member,
    end: &RunEnd,
    message: Option<&str>,
) -> Result<Option<Told>, NoticeError> {
    if role != Role::Member {
        return Ok(None);
    }
    let Some(coordinator) = team.crew().coordinator() else {
        return Ok(None);
    };
    let to = agent_name(coordinator.agent())?;
    let who = agent_name(member.agent())?;
    let notice = match end {
        RunEnd::Reported {
            outcome: RunOutcome::Done,
        } => {
            let (item, branch) = match worked_on(team, member) {
                Some((item, branch)) => (Some(item), branch),
                None => (None, task_branch(&who, member.task())?),
            };
            BusNotice::MemberFinished {
                member: who,
                item,
                branch,
            }
        }
        RunEnd::Reported {
            outcome: RunOutcome::Failed,
        } => BusNotice::MemberFailed {
            member: who,
            message: end_of(message.unwrap_or_default()),
        },
        RunEnd::NoStatus { .. } => BusNotice::MemberNoStatus { member: who },
        RunEnd::Reported { .. }
        | RunEnd::QuotaExhausted { .. }
        | RunEnd::RateLimited
        | RunEnd::Hung
        | RunEnd::Stopped => return Ok(None),
    };
    Ok(Some(Told { to, notice }))
}

/// The item `member` works on and its branch, if it has one.
fn worked_on(team: &Team, member: &Member) -> Option<(ItemId, BranchName)> {
    team.items().iter().find_map(|progress| match progress.phase() {
        ItemPhase::InProgress { assignee, branch } | ItemPhase::Fixing { assignee, branch, .. }
            if assignee.as_str() == member.agent().name() =>
        {
            Some((progress.id(), branch.clone()))
        }
        ItemPhase::Pending
        | ItemPhase::InProgress { .. }
        | ItemPhase::InReview { .. }
        | ItemPhase::Fixing { .. }
        | ItemPhase::Approved { .. }
        | ItemPhase::Done { .. } => None,
    })
}

#[cfg(test)]
mod tests;
