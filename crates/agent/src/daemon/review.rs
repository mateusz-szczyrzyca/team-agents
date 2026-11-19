//! Reviews: who reviews a pull request, what a reviewer is asked and what its report says.
//!
//! The reviewers are investigators from outside the team. A review is a task of the team (`Team::reviewers`) that ends
//! `waiting`, so that the same investigator can review the next pull request and the same one again after a fix pass.
//! Everything here decides from the state and the text of a report; the owner carries it out (`owner/rounds.rs`).

use std::collections::HashSet;

use agent_core::config::Config;
use agent_core::decide::investigator::{Activity, choose};
use agent_core::ids::{InvestigatorName, ItemId, TaskId, TeamId};
use agent_core::kinds::UnknownKeyword;
use agent_core::state::{ItemPhase, Member, Participant, ReviewOutcome, State, Team};
use agent_core::status::{ReviewVerdict, TaskStatus};
use std::collections::BTreeSet;
use std::num::NonZeroUsize;

/// The longest report the harness keeps, posts and hands on, in characters: far above any real report, and under the
/// limit of a comment on GitHub.
pub(super) const MAX_REPORT_CHARS: usize = 20_000;

/// How many characters of an unreadable verdict a [`VerdictProblem`] keeps.
const MAX_FOUND_CHARS: usize = 80;

/// What a verdict line starts with, whatever the case.
const VERDICT_KEY: &str = "VERDICT:";

/// What a status line starts with.
const STATUS_KEY: &str = "STATUS:";

/// The item whose findings refuse a report, together with its precise protocol diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("review report refused for item {item}")]
pub(crate) struct ReportRefusal {
    /// The ledger whose allowed answers the correction must describe.
    pub(crate) item: ItemId,
    /// The refused source line and the rule it violated.
    #[source]
    pub(crate) problem: agent_core::review_findings::ReportError,
}

/// Why a reviewer's final message gives no verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum VerdictProblem {
    /// No line of the message is a `VERDICT:` line.
    Missing,
    /// Structured findings or verification answers are malformed.
    Findings(ReportRefusal),
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
/// that is running or queued. A sleeping crew member may serve another team.
pub(super) fn free_reviewers(state: &State, config: &Config, team: &TeamId) -> Vec<Reviewer> {
    let Some(reviewed) = state.team(team) else {
        return Vec::new();
    };
    let busy = occupied_investigators(state);
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

/// Investigators with a running or queued run, including accepted review requests
/// waiting for the scheduler to queue their reusable task.
fn occupied_investigators(state: &State) -> HashSet<InvestigatorName> {
    state
        .teams()
        .iter()
        .flat_map(|team| {
            team.crew()
                .roster()
                .into_iter()
                .map(|(_, member)| member)
                .chain(team.reviewers().iter())
                .filter(|member| {
                    member.current_run().is_some()
                        || matches!(member.status(), TaskStatus::Queued | TaskStatus::Running)
                        || (member.status() == TaskStatus::Waiting && has_pending_review(team, member))
                })
                .filter_map(investigator_of)
                .cloned()
        })
        .collect()
}

/// Applies the common activity selector and preserves each chosen investigator's
/// team-specific worktree slot. Caller exclusions are never relaxed.
pub(super) fn select_reviewers(
    state: &State,
    config: &Config,
    team: &TeamId,
    required: NonZeroUsize,
    excluded: &HashSet<InvestigatorName>,
) -> Option<Vec<Reviewer>> {
    let free = free_reviewers(state, config, team);
    let configured = config.investigators().keys().cloned().collect();
    let available = free.iter().map(|reviewer| reviewer.name.clone()).collect();
    let activities: Vec<_> = state
        .teams()
        .iter()
        .flat_map(|team| {
            team.activities().iter().map(|activity| Activity {
                investigator: activity.investigator(),
                team: team.id(),
                kind: activity.request().kind(),
                asked_at: activity.asked_at(),
            })
        })
        .collect();
    let excluded: BTreeSet<_> = excluded.iter().cloned().collect();
    let chosen = choose(&configured, &available, &activities, team, &excluded, required)?;
    let order: std::collections::BTreeMap<_, _> = chosen.into_iter().zip(0_usize..).collect();
    let mut selected: Vec<_> = free
        .into_iter()
        .filter(|reviewer| order.contains_key(&reviewer.name))
        .collect();
    selected.sort_by_key(|reviewer| order.get(&reviewer.name).copied());
    Some(selected)
}

/// Whether the reviewer `member` has a review to give for an item of `team` that is in review.
fn has_pending_review(team: &Team, member: &Member) -> bool {
    if pending_opinion(team, member.task()).is_some() {
        return true;
    }
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

/// The pending second opinion owned by a task, while its round is being fixed.
pub(super) fn pending_opinion<'a>(
    team: &'a Team,
    task: &TaskId,
) -> Option<(
    agent_core::ids::ItemId,
    std::num::NonZeroU32,
    &'a agent_core::state::SecondOpinion,
)> {
    team.items()
        .iter()
        .filter(|item| matches!(item.phase(), ItemPhase::Fixing { .. }))
        .find_map(|item| {
            item.findings()?.opinions().iter().find_map(|(number, opinion)| {
                matches!(&opinion.stage, agent_core::state::OpinionStage::Assigned { task: owner, .. } if owner == task)
                    .then_some((item.id(), *number, opinion))
            })
        })
}
