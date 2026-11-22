//! What the daemon sees of an item a lead returned: who holds it, which run of that member the work was handed over
//! after, the part it belongs to and the facts [`classify`](super::classify) decides on. It reads the state only.

use std::num::NonZeroU32;

use super::{Answered, Facts, Held, LeadEnd, handed_over_run, run_number, run_started_since, runs_of_round};
use crate::ids::RunId;
use crate::kinds::Role;
use crate::state::split::PartKey;
use crate::state::{ItemPhase, Member, RoundLife, Team, Timestamp};
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
///
/// The member is the one that holds the item, when the line names the part whose round is open (or no part). Only its runs
/// for that round are read, so the runs for items it held before are no hand-over of this one. The hand-over is the last
/// `DONE` among the runs that began before the lead's run: a run that began after it is what the return started.
#[must_use]
pub fn read_return(
    team: &Team,
    target: super::ReturnTarget,
    lead_started_at: Timestamp,
    outcome: RunOutcome,
) -> Option<Found<'_>> {
    let progress = team.item(target.item())?;
    let round = team
        .rounds()
        .iter()
        .find(|round| round.item() == Some(progress.id()) && matches!(round.life(), RoundLife::Open));
    let part = round.and_then(crate::state::Round::part);
    let assignee = match progress.phase() {
        ItemPhase::InProgress { assignee, .. } | ItemPhase::Fixing { assignee, .. } => Some(assignee.as_str()),
        ItemPhase::Pending | ItemPhase::InReview { .. } | ItemPhase::Approved { .. } | ItemPhase::Done { .. } => None,
    };
    // A line that names a part counts only for the part the item is at: the work of another part is not what is open.
    let names_the_open_part = target.part().is_none() || target.part() == part;
    let member = assignee
        .and_then(|name| member_named(team, name))
        .filter(|_| names_the_open_part && round.is_some());
    let runs = match (member, round) {
        (Some(member), Some(round)) => runs_of_round(member.runs(), round.id()),
        _ => Vec::new(),
    };
    let before = runs.partition_point(|run| run.started_at() < lead_started_at);
    let of_run = handed_over_run(runs.get(..before).unwrap_or(&runs)).cloned();
    let pending = progress
        .rejections()
        .is_some_and(|recorded| recorded.iter().any(|rejection| rejection.counted_in().is_none()));
    let answered = if pending || of_run.as_ref().is_some_and(|run| progress.was_rejected(run)) {
        Answered::Yes
    } else {
        Answered::No
    };
    let held = if member.is_some() {
        Held::ByTheMember
    } else {
        Held::Nobody
    };
    let facts = Facts::from_runs_before(held, answered, &runs, lead_started_at).with_lead(LeadEnd::Settled(outcome));
    // The number is the position among all the runs of the member's task, as the daemon counts the runs it starts.
    let started_again =
        member.and_then(|member| run_started_since(&runs, lead_started_at).map(|_| run_number(member.runs().len())));
    Some(Found {
        member,
        facts,
        part,
        of_run,
        started_again,
    })
}

/// The member of the crew of `team` whose agent is called `name`.
fn member_named<'a>(team: &'a Team, name: &str) -> Option<&'a Member> {
    team.crew()
        .roster()
        .into_iter()
        .find(|(role, member)| *role == Role::Member && member.agent().name() == name)
        .map(|(_, member)| member)
}
