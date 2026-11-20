//! The briefing of a task started in a new session: the facts of the work the runs before it left behind, as the text
//! of [`HarnessPrompt::Briefing`](agent_core::prompt::HarnessPrompt::Briefing).

use agent_core::prompt::{Variables, VariablesError};

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
    let commits = lines(
        facts
            .commits
            .iter()
            .map(|commit| format!("- {} {}", commit.id, commit.subject)),
    );
    let uncommitted = lines(facts.uncommitted.iter().map(|path| format!("- {path}")));
    let plan = lines(
        facts
            .plan
            .unwrap_or_default()
            .iter()
            .map(|step| format!("- {} {} {}", step.state, step.id, step.title)),
    );
    let messages = lines(facts.messages.iter().flat_map(|message| {
        std::iter::once(format!("- #{} {}:", message.id, message.from))
            .chain(message.text.lines().map(|line| format!("  {line}")))
    }));
    let pairs: [(&str, &str); 11] = [
        ("reason", facts.reason),
        ("task", facts.task),
        ("prompt", facts.prompt.trim_end()),
        ("base", facts.base),
        ("branch", facts.branch),
        ("commits", &commits),
        ("uncommitted", &uncommitted),
        ("snapshot", facts.snapshot),
        ("plan", &plan),
        ("left_out", facts.left_out.unwrap_or_default().trim_end()),
        ("messages", &messages),
    ];
    pairs
        .iter()
        .try_fold(Variables::new(), |values, (name, value)| values.with(name, *value))
}

/// `lines`, one under the other.
fn lines(lines: impl Iterator<Item = String>) -> String {
    lines.collect::<Vec<_>>().join("\n")
}
