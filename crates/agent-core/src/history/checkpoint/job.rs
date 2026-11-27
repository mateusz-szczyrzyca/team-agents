//! What a checkpoint says of a job that is not an item of a round: a worker's request, a review and an investigation.
//!
//! The three parts are made here from plain facts the daemon gathers, as [`super::state`] does for an item: what is
//! meant is the request, the pull request under review or the question; what is done is what the worktree holds; what is
//! left is what the job still has to do, and `next:` says what that is.

use super::state::{cut, one_line};
use super::{CheckpointContent, CommitLine};
use crate::ids::ItemId;
use crate::state::PrNumber;
use crate::task_list::requirements::Requirement;

/// The result of the check of a worker's request, as it last ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckResult<'a> {
    /// Whether the check passed.
    pub passed: bool,
    /// The line of the check's output that says it.
    pub line: &'a str,
}

/// The item a review is against.
#[derive(Debug, Clone, Copy)]
pub struct ReviewedItem<'a> {
    /// The item.
    pub id: ItemId,
    /// Its title.
    pub title: &'a str,
    /// Its requirements, numbered.
    pub requirements: &'a [Requirement],
}

/// What a job has done so far, which its worktree and its files say.
#[derive(Debug, Clone, Copy)]
pub enum JobWork<'a> {
    /// A worker's request.
    Request {
        /// How its check last ran; `None` when it did not run yet or the request names none.
        last_check: Option<CheckResult<'a>>,
        /// The commits of the request's worktree since it was made.
        commits: &'a [CommitLine],
        /// The files that differ from where the worktree started.
        changed: &'a [String],
    },
    /// A review.
    Review {
        /// The review pass.
        pass: u32,
        /// The findings written so far, as the reviewer's findings file has them; empty when it has none.
        findings: &'a str,
    },
    /// An investigation.
    Investigation {
        /// The commits of the investigator's worktree since it was made.
        commits: &'a [CommitLine],
        /// The files that differ from where the worktree started: the investigator's notes and changes.
        changed: &'a [String],
    },
}

/// What is meant by a worker's request: its text, and its check when it names one.
#[must_use]
pub fn request_meant(request: &str, check: Option<&str>) -> String {
    let mut lines = vec![format!("R1. {request}")];
    lines.extend(check.map(|check| format!("{CHECK_LABEL}{check}")));
    lines.join("\n")
}

/// What is meant by a review: the pull request at its head and the pass, against the requirements of its item.
#[must_use]
pub fn review_meant(pull_request: PrNumber, head: &str, pass: u32, item: &ReviewedItem<'_>) -> String {
    let mut lines = vec![format!(
        "R1. Review pull request #{pull_request} at head {head}, pass {pass}, against the requirements of item {} ({}):",
        item.id, item.title
    )];
    lines.extend(
        item.requirements
            .iter()
            .map(|requirement| format!("- {}. {}", requirement.id, requirement.text)),
    );
    lines.join("\n")
}

/// What is meant by an investigation: the question of the request.
#[must_use]
pub fn investigation_meant(question: &str) -> String {
    format!("R1. Answer the question: {question}")
}

/// The start of the line of a request's check in what is meant.
const CHECK_LABEL: &str = "K1. ";

/// The three parts of the checkpoint of a job whose `meant` part is `meant`, with `work` done so far.
#[must_use]
pub fn job_content(meant: String, work: &JobWork<'_>) -> CheckpointContent {
    let (done, left) = match work {
        JobWork::Request {
            last_check,
            commits,
            changed,
        } => (
            request_done(*last_check, commits, changed),
            request_left(last_check.is_some_and(|result| result.passed), has_check(&meant)),
        ),
        JobWork::Review { pass, findings } => (
            review_done(findings),
            format!("next: finish the review of pass {pass} and submit it with a verdict"),
        ),
        JobWork::Investigation { commits, changed } => (
            work_lines(commits, changed).join("\n"),
            "next: answer the question".to_owned(),
        ),
    };
    CheckpointContent { meant, done, left }
}

/// Whether `meant` is of a request that names a check.
fn has_check(meant: &str) -> bool {
    meant.lines().any(|line| line.starts_with(CHECK_LABEL))
}

/// The lines that tell the commits and the changed files of a worktree.
fn work_lines(commits: &[CommitLine], changed: &[String]) -> Vec<String> {
    let mut lines = vec![format!("Commits: {}", commits.len())];
    lines.extend(cut(commits.iter().map(|commit| format!("- {}", commit.id)).collect()));
    lines.push(if changed.is_empty() {
        "Changed files: none".to_owned()
    } else {
        format!("Changed files: {}", changed.join(", "))
    });
    lines
}

/// What a worker's request has done: the work of its worktree and how its check last ran.
fn request_done(last_check: Option<CheckResult<'_>>, commits: &[CommitLine], changed: &[String]) -> String {
    let mut lines = work_lines(commits, changed);
    lines.push(match last_check {
        None => "Check: not run yet".to_owned(),
        Some(result) => format!(
            "Check: {} — {}",
            if result.passed { "passed" } else { "failed" },
            one_line(result.line)
        ),
    });
    lines.join("\n")
}

/// What is left of a worker's request: its check, until it passes.
fn request_left(passed: bool, checked: bool) -> String {
    match (checked, passed) {
        (true, true) => "Nothing is left of the request: its check passed.\nnext: report the result".to_owned(),
        (true, false) => "K1 has not passed yet.\nnext: make the check pass and commit".to_owned(),
        (false, _) => "R1 is not met yet.\nnext: do what the request asks and commit".to_owned(),
    }
}

/// What a review has done: the findings its file holds, counted by their numbers.
fn review_done(findings: &str) -> String {
    let written = findings.lines().filter(|line| is_finding_line(line)).count();
    let mut lines = vec![format!("Findings written so far: {written}")];
    lines.extend(cut(findings.lines().map(str::to_owned).collect()));
    lines.join("\n")
}

/// Whether `line` starts a finding of a report: a number and a full stop.
fn is_finding_line(line: &str) -> bool {
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && line.chars().nth(digits) == Some('.')
}

#[cfg(test)]
mod tests;
