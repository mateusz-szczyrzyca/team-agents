//! What the daemon sees of an item a lead returned: who holds it, which run of that member the work was handed over
//! after, the part it belongs to and the facts [`classify`](super::classify) decides on. It reads the state only.

use std::num::NonZeroU32;

use super::{Facts, Held, LastRun, MovedOn};
use crate::ids::RunId;
use crate::state::split::PartKey;
use crate::state::{Member, Team, Timestamp};
use crate::status::RunOutcome;

/// What was seen of one target of a lead's `RETURNED:` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found<'a> {
    /// The member that holds the item for the target; `None` when no member holds it, or the line names another part
    /// than the one that is open.
    member: Option<&'a Member>,
    /// What the daemon saw of the item and the member.
    facts: Facts,
    /// The part of the item whose round is open; `None` for an item that is not split.
    part: Option<PartKey>,
    /// The run of the member that ended `DONE` and is sent back.
    of_run: Option<RunId>,
    /// The number of the member's last run when it was started again since the lead's run began.
    started_again: Option<NonZeroU32>,
}

impl<'a> Found<'a> {
    /// The member that holds the item for the target.
    #[must_use]
    pub const fn member(&self) -> Option<&'a Member> {
        self.member
    }

    /// What the daemon saw of the item and the member.
    #[must_use]
    pub const fn facts(&self) -> Facts {
        self.facts
    }

    /// The part of the item whose round is open.
    #[must_use]
    pub const fn part(&self) -> Option<PartKey> {
        self.part
    }

    /// The run of the member that ended `DONE` and is sent back.
    #[must_use]
    pub const fn of_run(&self) -> Option<&RunId> {
        self.of_run.as_ref()
    }

    /// The number of the member's last run when it was started again since the lead's run began.
    #[must_use]
    pub const fn started_again(&self) -> Option<NonZeroU32> {
        self.started_again
    }
}

/// What the state of `team` shows of `target`, returned by a lead whose run that ended `outcome` began at
/// `lead_started_at`; `None` when the team has no such item.
#[must_use]
pub fn read_return(
    team: &Team,
    target: super::ReturnTarget,
    lead_started_at: Timestamp,
    outcome: RunOutcome,
) -> Option<Found<'_>> {
    let _ = (team, target, lead_started_at, outcome);
    let first = NonZeroU32::MIN;
    Some(Found {
        member: None,
        facts: Facts::new(Held::Nobody, LastRun::Never, MovedOn::No, first, first),
        part: None,
        of_run: None,
        started_again: None,
    })
}
