//! What a checkpoint says of the work, told against the requirements of the item: what is meant, what is done of it and
//! what is left of it.
//!
//! The text is made here from plain facts the daemon gathers (the plan, the judgements of its steps, the commits with
//! their `Step:` trailers, the findings), so that the same facts always give the same text.

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use super::CheckpointContent;
use crate::ids::ItemId;
use crate::plan::judge::JudgementState;
use crate::plan::{Plan, StepState};
use crate::review_findings::Severity;
use crate::state::{FindingStatus, PrNumber};
use crate::task_list::requirements::{Requirement, RequirementId, RequirementKind};

#[cfg(test)]
mod tests;

/// The most lines of one list of a checkpoint that are kept; the rest is counted.
pub const LIST_LIMIT: usize = 40;

/// A commit of the holder's branch, as the checkpoint tells it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitLine {
    /// The abbreviated hash of the commit.
    pub id: String,
    /// The step its `Step:` trailer names; `None` when it has none or it does not read as a step.
    pub step: Option<ItemId>,
}

/// A finding of the reviews of the round's pull request, as the checkpoint tells it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingLine {
    /// Its permanent number.
    pub number: NonZeroU32,
    /// How serious the reviewer found it.
    pub severity: Severity,
    /// Where the reviewer found it.
    pub place: String,
    /// What the reviewer said.
    pub text: String,
    /// Whether it is open, resolved or not confirmed.
    pub status: FindingStatus,
}

/// What the holder does next, which the phase of the round decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// There is no pull request yet.
    HandOver,
    /// The reviews of the pass are awaited.
    WaitForReviews {
        /// The review pass.
        pass: u32,
    },
    /// The open findings are fixed in the pull request.
    Fix {
        /// The pull request.
        pull_request: PrNumber,
    },
    /// The round waits for the human.
    WaitForHuman,
    /// The pull request is approved and waits for its merge.
    WaitForMerge,
    /// The item is done: its pull request is merged.
    Nothing,
}

/// The facts a checkpoint tells.
#[derive(Debug, Clone)]
pub struct StateInput<'a> {
    /// The task's short description.
    pub description: &'a str,
    /// The item's number.
    pub item: ItemId,
    /// The item's title.
    pub title: &'a str,
    /// The requirements of the item, numbered.
    pub requirements: &'a [Requirement],
    /// The plan of the holder; `None` when it has none that reads.
    pub plan: Option<&'a Plan>,
    /// What the harness judged of the steps that are marked done.
    pub judgements: &'a BTreeMap<ItemId, JudgementState>,
    /// The commits of the holder's branch, oldest first.
    pub commits: &'a [CommitLine],
    /// The findings of the round's pull request.
    pub findings: &'a [FindingLine],
    /// How many review passes passed.
    pub passes_passed: u32,
    /// The tip of the holder's branch, abbreviated.
    pub head: Option<&'a str>,
    /// What comes next.
    pub next: Next,
}

/// The numbers that the first line of the `done` and of the `left` part of a checkpoint carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    /// How many commits the branch has.
    pub commits: u32,
    /// How many steps are done.
    pub steps_done: u32,
    /// How many steps are left.
    pub steps_left: u32,
    /// How many findings are open.
    pub findings_open: u32,
}

/// How a step of the plan stands: what the checkpoint says of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Standing {
    /// Marked done and its check passed.
    Confirmed,
    /// Marked done, and no check ran or it names none.
    Declared,
    /// Marked done and its check failed.
    Mismatch,
    /// In hand.
    InHand,
    /// Still to do.
    NotDone,
}

impl Standing {
    /// What the checkpoint calls it in a list of steps.
    const fn word(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Declared => "declared",
            Self::Mismatch => "mismatch",
            Self::InHand => "in hand",
            Self::NotDone => "not done",
        }
    }

    /// Whether the step is done: marked so and not a mismatch.
    const fn is_done(self) -> bool {
        matches!(self, Self::Confirmed | Self::Declared)
    }
}

/// A step of the plan that is not given up, with what the commits say of it.
#[derive(Debug, Clone)]
struct StepFacts {
    /// The step's ID.
    id: ItemId,
    /// How it stands.
    standing: Standing,
    /// The requirements of the item it names; empty for a step that is not tied to one.
    requirements: Vec<RequirementId>,
    /// Whether a commit names it in its `Step:` trailer.
    has_commit: bool,
}

impl StepFacts {
    /// Whether the step counts as done: it is, and a step tied to a requirement has a commit of its own.
    fn counts_done(&self) -> bool {
        self.standing.is_done() && (self.requirements.is_empty() || self.has_commit)
    }

    /// What the list of steps left says of it.
    fn left_word(&self) -> &'static str {
        if self.standing.is_done() {
            "no commit"
        } else {
            self.standing.word()
        }
    }
}

/// How far a requirement is met.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Met {
    /// Every step for it that is not given up counts as done.
    Fully,
    /// Some do.
    Partly,
    /// None does.
    NotStarted,
}

/// The steps of `input`'s plan that are not given up.
fn steps_of(input: &StateInput<'_>) -> Vec<StepFacts> {
    let Some(plan) = input.plan else {
        return Vec::new();
    };
    plan.steps()
        .filter(|step| step.state() != StepState::GivenUp)
        .map(|step| {
            let id = step.id();
            let standing = match (step.state(), input.judgements.get(&id)) {
                (StepState::Done, Some(JudgementState::Confirmed)) => Standing::Confirmed,
                (StepState::Done, Some(JudgementState::Mismatch)) => Standing::Mismatch,
                (StepState::Done, Some(JudgementState::Declared) | None) => Standing::Declared,
                (StepState::InHand, _) => Standing::InHand,
                (StepState::Todo | StepState::GivenUp, _) => Standing::NotDone,
            };
            // A `For:` line that names no requirement of the item reads as none: the plan reader sends such a plan back.
            let requirements = step
                .requirements()
                .unwrap_or_default()
                .into_iter()
                .filter(|named| input.requirements.iter().any(|known| known.id == *named))
                .collect();
            StepFacts {
                id,
                standing,
                requirements,
                has_commit: input.commits.iter().any(|commit| commit.step == Some(id)),
            }
        })
        .collect()
}

/// Joins `parts` with `, `.
fn joined(parts: impl IntoIterator<Item = String>) -> String {
    parts.into_iter().collect::<Vec<_>>().join(", ")
}

/// A requirement with its steps, how far it is met, and the line that tells it in the `done` part.
struct Told<'a> {
    /// The requirement.
    requirement: &'a Requirement,
    /// Its steps.
    steps: Vec<&'a StepFacts>,
    /// How far it is met.
    met: Met,
}

impl<'a> Told<'a> {
    /// The requirement `requirement` against the `steps` of the plan.
    fn of(requirement: &'a Requirement, steps: &'a [StepFacts]) -> Self {
        let steps: Vec<&StepFacts> = steps
            .iter()
            .filter(|step| step.requirements.contains(&requirement.id))
            .collect();
        let counted = steps.iter().filter(|step| step.counts_done()).count();
        let met = if steps.is_empty() || counted == 0 {
            Met::NotStarted
        } else if counted == steps.len() {
            Met::Fully
        } else {
            Met::Partly
        };
        Self {
            requirement,
            steps,
            met,
        }
    }

    /// The line of the `done` part: the state and its evidence.
    fn done_line(&self, input: &StateInput<'_>) -> String {
        let id = self.requirement.id;
        let done: Vec<&&StepFacts> = self.steps.iter().filter(|step| step.counts_done()).collect();
        let confirmed = done.iter().all(|step| step.standing == Standing::Confirmed);
        match (self.met, input.head) {
            (Met::NotStarted, _) => format!("{id} not started"),
            (Met::Fully, Some(head)) if confirmed && id.kind() == RequirementKind::Check => {
                format!("{id} met — its check passed at {head}")
            }
            (met, _) => {
                let word = if met == Met::Fully { "met" } else { "partly met" };
                let steps = joined(
                    done.iter()
                        .map(|step| format!("{} ({})", step.id, step.standing.word())),
                );
                let commits = joined(
                    input
                        .commits
                        .iter()
                        .filter(|commit| {
                            commit
                                .step
                                .is_some_and(|named| self.steps.iter().any(|step| step.id == named))
                        })
                        .map(|commit| commit.id.clone()),
                );
                format!("{id} {word} — steps {steps}; commits {commits}")
            }
        }
    }

    /// The line of the `left` part, or `None` for a requirement that is met.
    fn left_line(&self) -> Option<String> {
        let id = self.requirement.id;
        let left: Vec<String> = self
            .steps
            .iter()
            .filter(|step| !step.counts_done())
            .map(|step| format!("{} ({})", step.id, step.left_word()))
            .collect();
        match (self.met, left.is_empty()) {
            (Met::Fully, _) => None,
            (Met::Partly, _) => Some(format!("{id} partly met — steps left: {}", joined(left))),
            (Met::NotStarted, true) => Some(format!("{id} not started — no step names it")),
            (Met::NotStarted, false) => Some(format!("{id} not started — steps left: {}", joined(left))),
        }
    }
}

/// `lines` with all but the first [`LIST_LIMIT`] of them replaced by the number left out.
fn cut(mut lines: Vec<String>) -> Vec<String> {
    if let Some(left_out) = lines.len().checked_sub(LIST_LIMIT).filter(|left_out| *left_out > 0) {
        lines.truncate(LIST_LIMIT);
        lines.push(format!("{left_out} more left out"));
    }
    lines
}

/// What a finding says in one line: its words, whatever its line breaks.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The requirement of the item that `text` names, if it names one: the first word that is the number of one.
fn named_requirement(text: &str, requirements: &[Requirement]) -> Option<RequirementId> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter_map(|word| word.parse::<RequirementId>().ok())
        .find(|named| requirements.iter().any(|known| known.id == *named))
}

/// The `meant` part: the task, the item and its requirements, each with its number.
fn meant(input: &StateInput<'_>) -> String {
    let mut lines = vec![
        format!("Task: {}", input.description),
        format!("Item {}: {}", input.item, input.title),
    ];
    lines.extend(
        input
            .requirements
            .iter()
            .map(|requirement| format!("{}. {}", requirement.id, requirement.text)),
    );
    lines.join("\n")
}

/// The commits that are tied to no requirement: with no trailer, or naming a step no requirement has.
fn untied_commits<'a>(input: &'a StateInput<'_>, steps: &[StepFacts]) -> Vec<&'a CommitLine> {
    input
        .commits
        .iter()
        .filter(|commit| {
            !commit.step.is_some_and(|named| {
                steps
                    .iter()
                    .any(|step| step.id == named && !step.requirements.is_empty())
            })
        })
        .collect()
}

/// The `done` part.
fn done(input: &StateInput<'_>, told: &[Told<'_>], steps: &[StepFacts]) -> String {
    let steps_done = steps.iter().filter(|step| step.counts_done()).count();
    let mut lines: Vec<String> = told.iter().map(|told| told.done_line(input)).collect();
    for finding in input.findings {
        let word = match finding.status {
            FindingStatus::Open => continue,
            FindingStatus::Resolved => "resolved",
            FindingStatus::NotConfirmed => "not confirmed",
        };
        lines.push(format!(
            "Finding #{} {word} — {}",
            finding.number,
            one_line(&finding.place)
        ));
    }
    if input.passes_passed > 0 {
        lines.push(format!("Review passes passed: {}", input.passes_passed));
    }
    let untied_steps: Vec<String> = steps
        .iter()
        .filter(|step| step.requirements.is_empty() && step.counts_done())
        .map(|step| format!("{} ({})", step.id, step.standing.word()))
        .collect();
    let untied = untied_commits(input, steps);
    if !untied_steps.is_empty() || !untied.is_empty() {
        lines.push("Not tied to a requirement:".to_owned());
        if !untied_steps.is_empty() {
            lines.push(format!("steps {}", joined(untied_steps)));
        }
        if !untied.is_empty() {
            lines.push(format!(
                "commits {}",
                joined(untied.iter().map(|commit| commit.id.clone()))
            ));
        }
    }
    let mut text = vec![format!("Totals: {} commits, {steps_done} steps", input.commits.len())];
    text.extend(cut(lines));
    text.join("\n")
}

/// The line of the `left` part that says what comes next.
fn next_line(input: &StateInput<'_>, open: usize) -> String {
    match input.next {
        Next::HandOver => "next: commit what is finished, run the gate and hand the round over".to_owned(),
        Next::WaitForReviews { pass } => format!("next: wait for the reviews of pass {pass}"),
        Next::Fix { pull_request } => format!(
            "next: fix the {open} open findings in pull request #{pull_request} and answer each one",
            pull_request = pull_request.get()
        ),
        Next::WaitForHuman => "next: wait for the human's instruction; read the human's messages".to_owned(),
        Next::WaitForMerge => "next: wait for the merge".to_owned(),
        Next::Nothing => "next: nothing; the pull request is merged".to_owned(),
    }
}

/// The `left` part.
fn left(input: &StateInput<'_>, told: &[Told<'_>], steps: &[StepFacts]) -> String {
    let mut lines: Vec<String> = told.iter().filter_map(Told::left_line).collect();
    let untied: Vec<String> = steps
        .iter()
        .filter(|step| step.requirements.is_empty() && !step.counts_done())
        .map(|step| format!("{} ({})", step.id, step.left_word()))
        .collect();
    if !untied.is_empty() {
        lines.push("Not tied to a requirement:".to_owned());
        lines.push(format!("steps left: {}", joined(untied)));
    }
    let open: Vec<&FindingLine> = input
        .findings
        .iter()
        .filter(|finding| finding.status == FindingStatus::Open)
        .collect();
    for finding in &open {
        let line = format!(
            "Finding #{} [{}] {} — {}",
            finding.number,
            finding.severity.word(),
            one_line(&finding.place),
            one_line(&finding.text)
        );
        lines.push(match named_requirement(&finding.text, input.requirements) {
            Some(named) => format!("{line} ({named})"),
            None => line,
        });
    }
    let steps_left = steps.iter().filter(|step| !step.counts_done()).count();
    let mut text = vec![format!("Totals: {steps_left} steps, {} findings", open.len())];
    text.extend(cut(lines));
    text.push(next_line(input, open.len()));
    text.join("\n")
}

/// The three parts of the checkpoint that tells `input`.
#[must_use]
pub fn checkpoint_content(input: &StateInput<'_>) -> CheckpointContent {
    let steps = steps_of(input);
    let told: Vec<Told<'_>> = input
        .requirements
        .iter()
        .map(|requirement| Told::of(requirement, &steps))
        .collect();
    CheckpointContent {
        meant: meant(input),
        done: done(input, &told, &steps),
        left: left(input, &told, &steps),
    }
}

/// The number in `line` that stands before `unit`, in a `Totals:` line made by this module.
fn total_before(line: &str, unit: &str) -> Option<u32> {
    let rest = line.strip_prefix("Totals: ")?;
    rest.split(", ")
        .find_map(|part| part.strip_suffix(unit).and_then(|number| number.trim().parse().ok()))
}

impl CheckpointContent {
    /// The numbers in the first lines of the `done` and `left` parts; `None` for a content that has none.
    #[must_use]
    pub fn counts(&self) -> Option<Counts> {
        let done = self.done.lines().next()?;
        let left = self.left.lines().next()?;
        Some(Counts {
            commits: total_before(done, " commits")?,
            steps_done: total_before(done, " steps")?,
            steps_left: total_before(left, " steps")?,
            findings_open: total_before(left, " findings")?,
        })
    }
}
