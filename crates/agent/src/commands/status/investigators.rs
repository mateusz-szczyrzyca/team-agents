//! What `agent status --investigators` says of each configured investigator: the last two activities in the teams that are
//! not closed, newest first, or that it is free.
//!
//! The teams come from the daemon's answer to `status` and the investigators from `agents.toml`: both are passed in, so
//! that the view is a pure function of what the daemon and the configuration say.

use std::cmp::Reverse;

use agent_core::ids::InvestigatorName;
use agent_core::kinds::ActivityKind;
use agent_core::protocol::TeamSummary;
use agent_core::state::{Activity, ActivityEnd, ActivityOutcome, RoundLife, Timestamp};
use time::UtcOffset;

use super::clock_of;
use super::reviewer::{ordinal, ordinal_text, stopped};

/// How many activities of an investigator the view shows.
const SHOWN: usize = 2;

/// The view of the investigators `configured` in `teams`: a line for each of the last two activities of each of them in
/// the teams that are not closed, newest first, and a line for the one that has none. The times are the local ones at
/// `offset`.
pub(crate) fn render(teams: &[TeamSummary], configured: &[InvestigatorName], offset: UtcOffset) -> String {
    let mut lines = Vec::new();
    for investigator in configured {
        let mut found = Shown::all_of(investigator, teams);
        found.sort_by_key(|shown| Reverse(shown.asked));
        if found.is_empty() {
            lines.push(format!("{investigator}  free, no activity in an open team"));
            continue;
        }
        lines.extend(found.into_iter().take(SHOWN).map(|shown| shown.text(offset)));
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// An activity of an investigator in a team that is not closed, with its team.
#[derive(Debug)]
struct Shown<'a> {
    /// When it was asked: the view is newest first.
    asked: Timestamp,
    /// The activity.
    activity: &'a Activity,
    /// Its team, whose activities give the ordinal of a review.
    team: &'a TeamSummary,
}

impl<'a> Shown<'a> {
    /// Every activity of `investigator` in a team of `teams` that is not closed.
    fn all_of(investigator: &InvestigatorName, teams: &'a [TeamSummary]) -> Vec<Self> {
        teams
            .iter()
            .filter(|team| team.rounds.iter().any(|round| *round.life() == RoundLife::Open))
            .flat_map(|team| {
                team.activities
                    .iter()
                    .filter(|activity| activity.investigator() == investigator)
                    .map(|activity| Self {
                        asked: activity.asked_at(),
                        activity,
                        team,
                    })
            })
            .collect()
    }

    /// The line of the activity at the local time `offset` gives.
    fn text(&self, offset: UtcOffset) -> String {
        format!(
            "{}  {}  {}  for {}  {}",
            self.activity.task(),
            self.activity.request().kind().as_str(),
            self.activity.investigator(),
            self.team.id,
            self.state(offset),
        )
    }

    /// What the activity did or does: what it is doing, when it was done and what, or when it stopped without its
    /// report and why, in the words of a reviewer's line.
    fn state(&self, offset: UtcOffset) -> String {
        match self.activity.outcome() {
            ActivityOutcome::Working => match self.activity.request().kind() {
                ActivityKind::Reviewer => format!("working, {} review", self.review()),
                ActivityKind::TeamMember | ActivityKind::Investigator | ActivityKind::TaskAllocator => {
                    "working".to_owned()
                }
            },
            ActivityOutcome::Finished {
                at,
                end: ActivityEnd::Reported,
            } => format!("done at {}, {}", clock_of(*at, offset), self.done()),
            ActivityOutcome::Finished { at, end } => stopped(*at, end, offset),
        }
    }

    /// What a reported activity did, in the words of its kind.
    fn done(&self) -> String {
        match self.activity.request().kind() {
            ActivityKind::Reviewer => format!("{} review finished", self.review()),
            ActivityKind::TeamMember => "work in the team finished".to_owned(),
            ActivityKind::Investigator => "investigation finished".to_owned(),
            ActivityKind::TaskAllocator => "tasks allocated".to_owned(),
        }
    }

    /// The ordinal of the review among those its investigator gave for its rounds: `1st`, `2nd`, ...
    fn review(&self) -> String {
        ordinal_text(ordinal(self.activity, &self.team.activities))
    }
}

#[cfg(test)]
mod tests;
