//! What a member's run ending tells the coordinator.

use agent_core::ids::{AgentName, BranchName, ItemId};
use agent_core::kinds::Role;
use agent_core::state::{ItemPhase, Member, RunEnd, Team};
use agent_core::status::{RunOutcome, TaskStatus};
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

/// How a request for a worker ended, for the requester.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon) enum RequestOutcome {
    /// The worker did it.
    Done {
        /// The worker's report, without its status line.
        summary: String,
    },
    /// The worker could not.
    Failed {
        /// How the worker's run ended.
        how: WorkerEnd,
        /// The end of what the worker said; `None` when it said nothing.
        said: Option<String>,
    },
}

/// How the run of a worker that could not do a request ended; each has its sentence for the requester in
/// `prompts/harness/team/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon) enum WorkerEnd {
    /// Its status line says it failed.
    Failed,
    /// It said nothing a requester can use: it went to sleep, excluded itself, or asked questions that cannot be used.
    NoResult,
    /// It ended without a status line, hit a provider's limit it cannot wait for, or its run could not be started or
    /// carried out.
    NoStatus,
    /// The human stopped it.
    Stopped,
    /// The watchdog gave up on it.
    Hung,
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
pub(crate) enum NoticeError {
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

/// How the request a worker served ended, given the end `end` of the worker's run and the status `status` its task has
/// after it; `None` while the request is not over: the worker is paused, asks the human or is queued to go on.
pub(in crate::daemon) fn request_outcome(
    end: &RunEnd,
    status: TaskStatus,
    message: Option<&str>,
) -> Option<RequestOutcome> {
    let said = message.unwrap_or_default();
    if goes_on(end) && status != TaskStatus::Failed {
        return None;
    }
    let fail = |how: WorkerEnd| {
        let said = (!said.trim().is_empty()).then(|| end_of(said));
        Some(RequestOutcome::Failed { how, said })
    };
    match end {
        RunEnd::Reported {
            outcome: RunOutcome::Done,
        } => Some(RequestOutcome::Done { summary: report(said) }),
        RunEnd::Reported {
            outcome: RunOutcome::Failed,
        } => fail(WorkerEnd::Failed),
        // Questions that cannot be used fail the task: the worker gave nothing to the requester either.
        RunEnd::Reported {
            outcome: RunOutcome::Sleeping | RunOutcome::Excluded | RunOutcome::Questions,
        } => fail(WorkerEnd::NoResult),
        // A provider's limit that cannot be waited for is settled as a run without a status.
        RunEnd::NoStatus { .. } | RunEnd::QuotaExhausted { .. } | RunEnd::RateLimited => fail(WorkerEnd::NoStatus),
        RunEnd::Stopped => fail(WorkerEnd::Stopped),
        RunEnd::Hung => Some(RequestOutcome::Failed {
            how: WorkerEnd::Hung,
            said: None,
        }),
    }
}

/// Whether the worker goes on after a run that ended with `end`, unless its task failed after all: the human answers
/// its questions, its provider's limit ends, the watchdog lets it continue.
const fn goes_on(end: &RunEnd) -> bool {
    matches!(
        end,
        RunEnd::Reported {
            outcome: RunOutcome::Questions,
        } | RunEnd::QuotaExhausted { .. }
            | RunEnd::RateLimited
            | RunEnd::Hung
    )
}

/// The worker's report without its closing `STATUS:` line; the whole message when nothing else is in it, so that a
/// report is never empty.
fn report(message: &str) -> String {
    let trimmed = message.trim_end();
    let without = match trimmed.rsplit_once('\n') {
        Some((before, last)) if last.trim_start().starts_with(STATUS_PREFIX) => before.trim_end(),
        None if trimmed.trim_start().starts_with(STATUS_PREFIX) => "",
        _ => trimmed,
    };
    if without.is_empty() {
        trimmed.to_owned()
    } else {
        without.to_owned()
    }
}

/// What a status line starts with.
const STATUS_PREFIX: &str = "STATUS:";

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
