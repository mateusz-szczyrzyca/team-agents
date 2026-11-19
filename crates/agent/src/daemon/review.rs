//! Reviews: who reviews a pull request, what a reviewer is asked and what its report says.
//!
//! The reviewers are investigators from outside the team. A review is a task of the team (`Team::reviewers`) that ends
//! `waiting`, so that the same investigator can review the next pull request and the same one again after a fix pass.
//! Everything here decides from the state and the text of a report; the owner carries it out (`owner/rounds.rs`).

use std::collections::HashSet;

use agent_core::config::{Config, ReviewCount};
use agent_core::ids::{InvestigatorName, TaskId, TeamId};
use agent_core::kinds::UnknownKeyword;
use agent_core::state::{ItemPhase, Member, Participant, ReviewOutcome, State, Team};
use agent_core::status::{ReviewVerdict, TaskStatus};
use rand::seq::SliceRandom;
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
    /// Structured findings or verification answers are malformed.
    Findings(agent_core::review_findings::ReportError),
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

/// Removes only the first verdict line used by [`parse_verdict`], preserving all other report bytes.
pub(super) fn report_without_verdict(report: &str) -> String {
    let mut removed = false;
    report
        .split_inclusive('\n')
        .filter(|line| {
            if !removed && after_key(line, VERDICT_KEY).is_some() {
                removed = true;
                false
            } else {
                true
            }
        })
        .collect()
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

/// Two reviewers for a pull request opened under the preceding protocol.
///
/// The reviewers that review for the team already (they wait in a review task of it) are drawn before new ones: they
/// know the pull request and look at what became of their own findings.
pub(super) fn choose_reviewers(free: Vec<Reviewer>, rng: &mut impl Rng) -> Option<Vec<Reviewer>> {
    choose_reviewers_for(free, ReviewCount::TWO, rng)
}

/// Draws a complete configured pass, preferring the team's existing available review tasks.
pub(super) fn choose_reviewers_for(
    free: Vec<Reviewer>,
    required: ReviewCount,
    rng: &mut impl Rng,
) -> Option<Vec<Reviewer>> {
    if free.len() < required.get() {
        return None;
    }
    let (mut known, mut new): (Vec<Reviewer>, Vec<Reviewer>) = free
        .into_iter()
        .partition(|reviewer| matches!(reviewer.slot, Slot::Task(_)));
    let mut chosen = Vec::with_capacity(required.get());
    for _ in 0..required.get() {
        let pool = if known.is_empty() { &mut new } else { &mut known };
        let index = rng.random_range(0..pool.len());
        chosen.push(pool.swap_remove(index));
    }
    Some(chosen)
}

#[cfg(test)]
mod tests;

/// Whether this task is an independent first-pass reviewer, including replacement and correction runs.
///
/// The pass belongs to the pull request, not to one item: an item that joined a fix pass has none of its own, so the
/// pass is the first one only when none of the items this task reviews has had a fix pass.
pub(super) fn independent_first_review(team: &Team, task: &TaskId) -> bool {
    let reviewed = team.items().iter().filter(|item| {
        matches!(item.phase(), ItemPhase::InReview { reviews, .. }
            if reviews.iter().any(|review| review.task == *task && matches!(review.outcome, ReviewOutcome::Pending)))
    });
    let mut independent = false;
    for item in reviewed {
        if !item.fix_passes().is_empty() {
            return false;
        }
        independent |= item.findings().is_some_and(|ledger| ledger.review_count().is_some());
    }
    independent
}

/// Draws verifiers, taking investigators never used on this PR before the least recently used.
pub(super) fn choose_verifiers(
    free: Vec<Reviewer>,
    required: ReviewCount,
    history: &[agent_core::state::FixPass],
    rng: &mut impl Rng,
) -> Option<Vec<Reviewer>> {
    if free.len() < required.get() {
        return None;
    }
    let mut pools = std::collections::BTreeMap::<Option<usize>, Vec<Reviewer>>::new();
    for reviewer in free {
        let last = history
            .iter()
            .rposition(|pass| pass.requested_by.iter().any(|r| r.reviewer == reviewer.name));
        pools.entry(last).or_default().push(reviewer);
    }
    let mut chosen = Vec::with_capacity(required.get());
    for mut pool in pools.into_values() {
        pool.shuffle(rng);
        let remaining = required.get().saturating_sub(chosen.len());
        chosen.extend(pool.into_iter().take(remaining));
        if chosen.len() == required.get() {
            break;
        }
    }
    Some(chosen)
}

/// Draws one independent opinion from the free investigators outside the answered pass.
pub(super) fn choose_opinion(
    free: Vec<Reviewer>,
    excluded: &HashSet<InvestigatorName>,
    rng: &mut impl Rng,
) -> Option<Reviewer> {
    let mut eligible: Vec<_> = free
        .into_iter()
        .filter(|reviewer| !excluded.contains(&reviewer.name))
        .collect();
    if eligible.is_empty() {
        return None;
    }
    let index = rng.random_range(0..eligible.len());
    Some(eligible.swap_remove(index))
}
