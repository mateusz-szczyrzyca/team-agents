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
    /// The step's state, as the plan writes it: `to do`, `in hand`, `done` or `given up`.
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
    /// Why the task is started anew, as one paragraph for the agent.
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

/// What the briefing says of a branch with no commit yet.
const NO_COMMITS: &str = "The branch has no commit of its own yet.";

/// What the briefing says of a worktree with no file left uncommitted.
const NO_UNCOMMITTED: &str = "No file of the worktree is left uncommitted.";

/// What the briefing says of a task that keeps no plan.
const NO_PLAN: &str = "The task keeps no plan of steps.";

/// What the briefing says of a task with no unread message.
const NO_MESSAGES: &str = "No message of your team waits unread.";

/// The title of the list of the plan's steps in the briefing.
const PLAN_TITLE: &str = "The task's plan of steps, as it stands in `.agent/plan.md`:";

/// The title of the list of the unread messages in the briefing.
const MESSAGES_TITLE: &str = "Messages of your team that no run of this task has read:";

/// The values of the variables of `harness/run/briefing.md` for `facts`.
///
/// # Errors
/// [`VariablesError`] when a value cannot be given to its variable.
pub fn briefing_variables(facts: &Briefing<'_>) -> Result<Variables, VariablesError> {
    let commits = listed(
        facts
            .commits
            .iter()
            .map(|commit| format!("- {} {}", commit.id, commit.subject)),
        None,
        NO_COMMITS,
    );
    let uncommitted = listed(
        facts.uncommitted.iter().map(|path| format!("- {path}")),
        None,
        NO_UNCOMMITTED,
    );
    let plan = facts.plan.map_or_else(
        || NO_PLAN.to_owned(),
        |steps| {
            listed(
                steps
                    .iter()
                    .map(|step| format!("- {} {}: {}", step.id, step.title, step.state)),
                Some(PLAN_TITLE),
                NO_PLAN,
            )
        },
    );
    let messages = listed(
        facts.messages.iter().flat_map(|message| {
            std::iter::once(format!("- #{} from {}:", message.id, message.from))
                .chain(message.text.lines().map(|line| format!("  {line}")))
        }),
        Some(MESSAGES_TITLE),
        NO_MESSAGES,
    );
    let pairs: [(&str, &str); 10] = [
        ("reason", facts.reason),
        ("task", facts.task),
        ("prompt", facts.prompt.trim_end()),
        ("base", facts.base),
        ("branch", facts.branch),
        ("commits", &commits),
        ("uncommitted", &uncommitted),
        ("snapshot", facts.snapshot),
        ("plan", &plan),
        ("messages", &messages),
    ];
    pairs
        .iter()
        .try_fold(Variables::new(), |values, (name, value)| values.with(name, *value))
}

/// `lines` joined, under `title` and the blank line after it when there is a title, or `empty` when there is no line.
fn listed(lines: impl Iterator<Item = String>, title: Option<&str>, empty: &str) -> String {
    let body = lines.collect::<Vec<_>>().join("\n");
    if body.is_empty() {
        return empty.to_owned();
    }
    match title {
        Some(title) => format!("{title}\n\n{body}"),
        None => body,
    }
}
