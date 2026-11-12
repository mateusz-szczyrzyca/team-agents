//! What a member's run ending tells the coordinator.

use agent_core::ids::{AgentName, BranchName, ItemId};
use agent_core::kinds::Role;
use agent_core::state::{Member, RunEnd, Team};

/// A notice for the bus: what happened to a member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon) enum BusNotice {
    /// A member reported `DONE`: its branch is ready to be integrated.
    MemberFinished {
        /// The member.
        member: AgentName,
        /// The item it worked on, if the team has items.
        item: Option<ItemId>,
        /// The branch it worked on.
        branch: Option<BranchName>,
    },
    /// A member reported `FAILED`.
    MemberFailed {
        /// The member.
        member: AgentName,
        /// The item it worked on, if the team has items.
        item: Option<ItemId>,
        /// The end of its final message.
        message: String,
    },
    /// A member ended its turn without a status line.
    MemberNoStatus {
        /// The member.
        member: AgentName,
        /// The item it worked on, if the team has items.
        item: Option<ItemId>,
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
    let _ = (team, role, member, end, message);
    Ok(None)
}

#[cfg(test)]
mod tests;
