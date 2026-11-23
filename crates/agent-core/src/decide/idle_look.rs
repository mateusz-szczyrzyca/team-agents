//! The types a look at a team gives the idleness decision.

use crate::ids::TeamId;
use crate::state::Timestamp;

/// What a look at a team found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Look {
    /// The team.
    pub team: TeamId,
    /// The moment of the look.
    pub at: Timestamp,
    /// The last activity of the team: a run that started or ended, a commit on one of its branches, or its creation.
    pub last: Timestamp,
    /// The states of its tasks that are not its own work.
    pub holds: Holds,
    /// The team waits for the merge of a pull request that is not one of its own rounds: another team's.
    pub waits_for_another_team: bool,
}

/// What holds a team that has done no work for the bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holds {
    /// No task runs, waits for a quota, or is paused or stopped by the human. A question to the human is not a hold.
    Nothing,
    /// A run of one of its tasks is going.
    Running,
    /// A task waits for an exhausted quota or a rate limit to reset.
    QuotaWait,
    /// The human paused or stopped it.
    HumanHold,
}

/// Why a team is kept, for the log and the notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// A run of one of its tasks is going.
    Running,
    /// A task waits for a quota or a rate limit to reset.
    QuotaWait,
    /// The human paused or stopped it.
    HumanHold,
    /// It waits for the merge of another team's pull request.
    AnotherTeam,
    /// Less than the bound has passed since its last activity.
    RecentlyActive,
}
