//! Reviews: who reviews a pull request, what a reviewer is asked and what its report says.
//!
//! The reviewers are investigators from outside the team. A review is a task of the team (`Team::reviewers`) that ends
//! `waiting`, so that the same investigator can review the next pull request and the same one again after a fix pass.
//! Everything here decides from the state and the text of a report; the owner carries it out (`owner/rounds.rs`).

use std::collections::HashSet;

use agent_core::config::Config;
use agent_core::decide::REVIEWS_REQUIRED;
use agent_core::ids::{InvestigatorName, TaskId, TeamId};
use agent_core::kinds::UnknownKeyword;
use agent_core::state::{ItemPhase, Member, Participant, ReviewOutcome, State, Team};
use agent_core::status::{ReviewVerdict, TaskStatus};
use rand::{Rng, RngExt};

use super::team::is_finished;

/// The longest report the harness keeps, posts and hands on, in characters: far above any real report, and under the
/// limit of a comment on GitHub.
pub(super) const MAX_REPORT_CHARS: usize = 20_000;

/// How many characters of an unreadable verdict a [`VerdictProblem`] keeps.
const MAX_FOUND_CHARS: usize = 80;

/// What a verdict line starts with, whatever the case.
const VERDICT_KEY: &str = "VERDICT:";

/// What a status line starts with.
const STATUS_KEY: &str = "STATUS:";

/// Why a reviewer's final message gives no verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum VerdictProblem {
    /// No line of the message is a `VERDICT:` line.
    Missing,
    /// The first `VERDICT:` line has something other than one of the two verdicts.
    Unreadable {
        /// What the line says after `VERDICT:`, cut.
        found: String,
    },
}

/// The verdict of a reviewer's final message: the first `VERDICT:` line, once the markup characters are dropped, says
/// `APPROVE` or `REQUEST_CHANGES` and nothing else.
///
/// # Errors
/// [`VerdictProblem`].
pub(super) fn parse_verdict(message: &str) -> Result<ReviewVerdict, VerdictProblem> {
    let Some(value) = message.lines().find_map(|line| after_key(line, VERDICT_KEY)) else {
        return Err(VerdictProblem::Missing);
    };
    match value.to_ascii_uppercase().parse::<ReviewVerdict>() {
        Ok(verdict) => Ok(verdict),
        // Anything else on the line is not a verdict: the report is asked for again, and the line is told.
        Err(UnknownKeyword { .. }) => Err(VerdictProblem::Unreadable {
            found: value.chars().take(MAX_FOUND_CHARS).collect(),
        }),
    }
}

/// What follows `key` on `line` when the line starts with it, once the markup characters (`*` and `` ` ``), the
/// indentation and the marks of a heading, a quotation or a list item are dropped; the key is read in any case.
fn after_key(line: &str, key: &str) -> Option<String> {
    let plain: String = line.chars().filter(|c| !matches!(c, '*' | '`')).collect();
    let start = plain.trim_start_matches(|c: char| c.is_whitespace() || matches!(c, '#' | '>' | '-'));
    let (head, rest) = (start.get(..key.len())?, start.get(key.len()..)?);
    head.eq_ignore_ascii_case(key).then(|| rest.trim().to_owned())
}

/// The report in a reviewer's final message, as it is posted and kept: without the `STATUS:` line, at most
/// [`MAX_REPORT_CHARS`] characters.
pub(super) fn report_text(message: &str) -> String {
    let mut lines: Vec<&str> = message.lines().collect();
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    if lines.last().is_some_and(|line| after_key(line, STATUS_KEY).is_some()) {
        lines.pop();
    }
    lines.join("\n").trim().chars().take(MAX_REPORT_CHARS).collect()
}

/// Where the review of an investigator is carried out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Slot {
    /// The investigator has a review task in the team that waits: the same task reviews again.
    Task(TaskId),
    /// The investigator has no task in the team yet: a new one is made.
    New,
}

/// An investigator that is free to review a pull request of a team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Reviewer {
    /// The investigator.
    pub(super) name: InvestigatorName,
    /// Where it reviews.
    pub(super) slot: Slot,
}

/// The investigators that may review a pull request of `team` now, by name: configured, outside its crew, with no task
/// that is going or about to, no review to give and no team to lead or serve that is not finished.
pub(super) fn free_reviewers(state: &State, config: &Config, team: &TeamId) -> Vec<Reviewer> {
    let Some(reviewed) = state.team(team) else {
        return Vec::new();
    };
    let busy = busy_investigators(state);
    config
        .investigators()
        .keys()
        .filter(|name| !busy.contains(*name) && !in_crew(reviewed, name))
        .filter_map(|name| {
            slot_in(reviewed, name).map(|slot| Reviewer {
                name: name.clone(),
                slot,
            })
        })
        .collect()
}

/// Whether the investigator `name` is in the crew of `team`.
fn in_crew(team: &Team, name: &InvestigatorName) -> bool {
    team.crew()
        .roster()
        .into_iter()
        .any(|(_, member)| investigator_of(member).is_some_and(|crewing| crewing == name))
}

/// The investigator that works in the task of `member`, if it is one.
fn investigator_of(member: &Member) -> Option<&InvestigatorName> {
    match member.agent() {
        Participant::Investigator(name) => Some(name),
        Participant::Normal(_) | Participant::Worker(_) => None,
    }
}

/// Where the investigator `name` reviews for `team`: its review task of the team when it waits, a new one when it has
/// none, nowhere when its task of the team is over (the team has one task for an investigator, and it does not end its
/// review to be used again).
fn slot_in(team: &Team, name: &InvestigatorName) -> Option<Slot> {
    let own = team
        .reviewers()
        .iter()
        .find(|member| investigator_of(member).is_some_and(|reviewing| reviewing == name));
    match own {
        None => Some(Slot::New),
        Some(member) => match member.status() {
            TaskStatus::Waiting => Some(Slot::Task(member.task().clone())),
            TaskStatus::Queued
            | TaskStatus::Running
            | TaskStatus::Questions
            | TaskStatus::Paused
            | TaskStatus::Stopped
            | TaskStatus::Failed
            | TaskStatus::Done => None,
        },
    }
}

/// The investigators that have something to do in any team: a task that is going, about to start, asking or paused; a
/// review to give; or a place in the crew of a team that is not finished.
fn busy_investigators(state: &State) -> HashSet<InvestigatorName> {
    let mut busy = HashSet::new();
    for team in state.teams() {
        let finished = is_finished(team);
        for (_, member) in team.crew().roster() {
            if let Some(name) = investigator_of(member)
                && engaged(member.status(), !finished)
            {
                busy.insert(name.clone());
            }
        }
        for member in team.reviewers() {
            if let Some(name) = investigator_of(member)
                && engaged(member.status(), has_pending_review(team, member))
            {
                busy.insert(name.clone());
            }
        }
    }
    busy
}

/// Whether a task with `status` keeps its investigator from another task; `waiting_matters` says whether a task that
/// waits still has something to wait for.
const fn engaged(status: TaskStatus, waiting_matters: bool) -> bool {
    match status {
        TaskStatus::Queued | TaskStatus::Running | TaskStatus::Questions | TaskStatus::Paused => true,
        TaskStatus::Waiting => waiting_matters,
        TaskStatus::Stopped | TaskStatus::Failed | TaskStatus::Done => false,
    }
}

/// Whether the reviewer `member` has a review to give for an item of `team` that is in review.
fn has_pending_review(team: &Team, member: &Member) -> bool {
    team.items().iter().any(|item| match item.phase() {
        ItemPhase::InReview { reviews, .. } => reviews
            .iter()
            .any(|review| review.task == *member.task() && matches!(review.outcome, ReviewOutcome::Pending)),
        ItemPhase::Pending
        | ItemPhase::InProgress { .. }
        | ItemPhase::Fixing { .. }
        | ItemPhase::Approved { .. }
        | ItemPhase::Done { .. } => false,
    })
}

/// [`REVIEWS_REQUIRED`] of the `free` reviewers, drawn at random; `None` while fewer are free: the pull request waits.
pub(super) fn choose_reviewers(mut free: Vec<Reviewer>, rng: &mut impl Rng) -> Option<Vec<Reviewer>> {
    if free.len() < REVIEWS_REQUIRED {
        return None;
    }
    let mut chosen = Vec::with_capacity(REVIEWS_REQUIRED);
    for _ in 0..REVIEWS_REQUIRED {
        let index = rng.random_range(0..free.len());
        chosen.push(free.swap_remove(index));
    }
    Some(chosen)
}

#[cfg(test)]
mod tests;
