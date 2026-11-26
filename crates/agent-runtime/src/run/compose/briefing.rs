//! The briefing of a task started in a new session: the facts of the work the runs before it left behind, as the text
//! of [`HarnessPrompt::Briefing`](agent_core::prompt::HarnessPrompt::Briefing).

use agent_core::plan::judge::JudgementState;
use agent_core::prompt::{Variables, VariablesError};

mod review;

pub use review::{BriefingFindings, BriefingPullRequest, BriefingReview, ReviewStanding};

/// One run of the task, as the briefing lists it: when it started, the agent that ran it and how it ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BriefingRun<'a> {
    /// When the run started, as the briefing shows the time.
    pub started: &'a str,
    /// The agent that ran it.
    pub agent: &'a str,
    /// How it ended, in words: `DONE`, `QUESTIONS`, `no status`, `hang`, `turn limit`, `stopped` or `failed`.
    pub end: &'a str,
}

/// One question of the task that the human answered, as the briefing lists it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BriefingAnswer<'a> {
    /// The question's number in its file.
    pub number: &'a str,
    /// The rest of the question's heading line.
    pub title: &'a str,
    /// The human's verdict: `ok` for the recommendation, or the human's own answer.
    pub verdict: &'a str,
}

/// The facts a briefing adds to [`Briefing`]: the runs of the task, the human's answers, what the harness judged of the
/// steps, and the notes that older ones are left out. Its default is the briefing of a task with none of them.
#[derive(Debug, Clone, Copy, Default)]
pub struct BriefingNotes<'a> {
    /// The runs of the task that its briefing lists, oldest first.
    pub runs: &'a [BriefingRun<'a>],
    /// The note that older runs are left out of [`BriefingNotes::runs`], when they are.
    pub runs_left_out: Option<&'a str>,
    /// The human's answers to the questions of the task, oldest first.
    pub answers: &'a [BriefingAnswer<'a>],
    /// The note that older answers are left out of [`BriefingNotes::answers`], when they are.
    pub answers_left_out: Option<&'a str>,
    /// The judgement the harness gave each step that is marked done and has not changed since, by step ID. A done step
    /// with no entry is shown as declared.
    pub judgements: &'a [(&'a str, JudgementState)],
    /// The headings of the groups the plan's steps are listed in.
    pub headings: PlanHeadings<'a>,
    /// The pull request of the task's item and its review; the default is an item with no pull request.
    pub review: BriefingReview<'a>,
}

/// The headings of the four groups of the plan's steps, as the catalog words them. Its default has no words: a briefing
/// is built with the catalog's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlanHeadings<'a> {
    /// The heading of the steps whose check passed.
    pub confirmed: &'a str,
    /// The heading of the steps the agent declared done, with no check that passed or failed.
    pub declared: &'a str,
    /// The heading of the steps the agent declared done and whose check failed.
    pub mismatch: &'a str,
    /// The heading of the steps that are not done.
    pub not_done: &'a str,
}

/// A commit of the task's branch since its base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BriefingCommit<'a> {
    /// The short ID of the commit.
    pub id: &'a str,
    /// The first line of its message.
    pub subject: &'a str,
}

/// A step of the task's plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BriefingStep<'a> {
    /// The step's ID, such as `1.2`.
    pub id: &'a str,
    /// The rest of the step's line.
    pub title: &'a str,
    /// The step's mark, as the plan writes it: `[ ]`, `[>]`, `[x]` or `[-]`.
    pub state: &'a str,
}

/// A message of the task's team that no run of the task had read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BriefingMessage<'a> {
    /// The message's number on the bus.
    pub id: &'a str,
    /// Who sent it.
    pub from: &'a str,
    /// Its text.
    pub text: &'a str,
}

/// The facts of the work that a new session of a task is briefed with.
#[derive(Debug, Clone, Copy)]
pub struct Briefing<'a> {
    /// Why the task is started anew, as the state records it: `no_status`, `hang` or `turn_limit`.
    pub reason: &'a str,
    /// The task.
    pub task: &'a str,
    /// The task's own prompt.
    pub prompt: &'a str,
    /// The branch the work is forked from.
    pub base: &'a str,
    /// The branch the task works on.
    pub branch: &'a str,
    /// The commits on the branch since the base, oldest first.
    pub commits: &'a [BriefingCommit<'a>],
    /// The files the worktree holds that no commit does.
    pub uncommitted: &'a [&'a str],
    /// The task's plan of steps, when it has one.
    pub plan: Option<&'a [BriefingStep<'a>]>,
    /// The messages of the task's team it had not read.
    pub messages: &'a [BriefingMessage<'a>],
    /// The note that older unread messages are left out of [`Briefing::messages`], when they are.
    pub left_out: Option<&'a str>,
    /// The reference of the snapshot of the worktree, taken before the session starts.
    pub snapshot: &'a str,
}

/// The values of the variables of `harness/run/briefing.md` for `facts`. A list with nothing in it is empty: the text
/// of the briefing says what that means.
///
/// # Errors
/// [`VariablesError`] when a value cannot be given to its variable.
pub fn briefing_variables(facts: &Briefing<'_>) -> Result<Variables, VariablesError> {
    briefing_variables_with(facts, &BriefingNotes::default())
}

/// The values of the variables of the briefing for `facts` and `notes`, as [`briefing_variables`] gives them.
///
/// # Errors
/// [`VariablesError`] when a value cannot be given to its variable.
pub fn briefing_variables_with(facts: &Briefing<'_>, notes: &BriefingNotes<'_>) -> Result<Variables, VariablesError> {
    let commits = lines(
        facts
            .commits
            .iter()
            .map(|commit| format!("- {} {}", commit.id, commit.subject)),
    );
    let uncommitted = lines(facts.uncommitted.iter().map(|path| format!("- {path}")));
    let plan = plan_text(facts.plan.unwrap_or_default(), notes.judgements, &notes.headings);
    let runs = lines(
        notes
            .runs
            .iter()
            .map(|run| format!("- {} {}: {}", run.started, run.agent, run.end)),
    );
    let answers = lines(
        notes
            .answers
            .iter()
            .map(|answer| format!("- Question {}, {}: {}", answer.number, answer.title, answer.verdict)),
    );
    let messages = lines(facts.messages.iter().flat_map(|message| {
        std::iter::once(format!("- #{} {}:", message.id, message.from))
            .chain(message.text.lines().map(|line| format!("  {line}")))
    }));
    let review = review::review_values(&notes.review);
    let pairs: [(&str, &str); 18] = [
        ("reason", facts.reason),
        ("task", facts.task),
        ("prompt", facts.prompt.trim_end()),
        ("base", facts.base),
        ("branch", facts.branch),
        ("commits", &commits),
        ("uncommitted", &uncommitted),
        ("snapshot", facts.snapshot),
        ("plan", &plan),
        ("runs", &runs),
        ("runs_left_out", notes.runs_left_out.unwrap_or_default().trim_end()),
        ("answers", &answers),
        (
            "answers_left_out",
            notes.answers_left_out.unwrap_or_default().trim_end(),
        ),
        ("left_out", facts.left_out.unwrap_or_default().trim_end()),
        ("messages", &messages),
        ("pull_request", &review.pull_request),
        ("review_state", &review.review_state),
        ("findings", &review.findings),
    ];
    pairs
        .iter()
        .try_fold(Variables::new(), |values, (name, value)| values.with(name, *value))
}

/// The steps of the plan, grouped as the briefing shows them: the steps marked done by how the harness judged them
/// (`confirmed`, `mismatch`, or `declared` for a judgement that says so or none), then the steps not done, each group
/// under its heading. A group with no step is left out.
fn plan_text(steps: &[BriefingStep<'_>], judgements: &[(&str, JudgementState)], headings: &PlanHeadings<'_>) -> String {
    let (mut confirmed, mut declared, mut mismatch, mut not_done) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for step in steps {
        if step.state != DONE_MARK {
            not_done.push(format!("- {} {} {}", step.state, step.id, step.title));
            continue;
        }
        let line = format!("- {} {}", step.id, step.title);
        let judgement = judgements
            .iter()
            .find(|(id, _)| *id == step.id)
            .map(|(_, state)| *state);
        match judgement {
            Some(JudgementState::Confirmed) => confirmed.push(line),
            Some(JudgementState::Mismatch) => mismatch.push(line),
            Some(JudgementState::Declared) | None => declared.push(line),
        }
    }
    [
        (headings.confirmed, confirmed),
        (headings.declared, declared),
        (headings.mismatch, mismatch),
        (headings.not_done, not_done),
    ]
    .into_iter()
    .filter(|(_, lines)| !lines.is_empty())
    .map(|(heading, lines)| {
        std::iter::once(heading.trim_end().to_owned())
            .chain(lines)
            .collect::<Vec<_>>()
            .join("\n")
    })
    .collect::<Vec<_>>()
    .join("\n")
}

/// The mark of a step done, in the plan's own syntax.
const DONE_MARK: &str = "[x]";

/// `lines`, one under the other.
fn lines(lines: impl Iterator<Item = String>) -> String {
    lines.collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests;
